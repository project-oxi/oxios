//! Shared helpers for browser tools.
//!
//! Centralizes JS snippet generation and result parsing so that
//! `BrowseTool`, `BrowseExtractTool`, and `BrowseScriptTool` share
//! a single source of truth for DOM interaction logic.

use oxicode_sdk::ToolError;
use serde_json::Value;

use super::engine::OxiosTab;
use super::types::ObservedElement;

// ── Link extraction ───────────────────────────────────────────────

/// JS that returns `[{ text, href }, …]` for every `<a href>` on the page.
pub const JS_ALL_LINKS: &str = r#"(function() {
    var links = document.querySelectorAll('a[href]');
    return Array.from(links).map(function(a) {
        return { text: a.textContent.trim(), href: a.href };
    });
})()"#;

/// JS that returns links inside the element matching `selector`.
pub fn js_links_within(selector: &str) -> String {
    let sel = serde_json::to_string(selector).unwrap_or_default();
    format!(
        r#"(function() {{
            var root = document.querySelector({sel});
            if (!root) return [];
            var links = root.querySelectorAll('a[href]');
            return Array.from(links).map(function(a) {{
                return {{ text: a.textContent.trim(), href: a.href }};
            }});
        }})()"#
    )
}

/// Parse the JSON array returned by the link-extraction snippets.
pub fn parse_link_values(value: Value) -> Vec<(String, String)> {
    let Value::Array(arr) = value else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|item| {
            let href = item.get("href")?.as_str()?.to_string();
            if href.is_empty() {
                return None;
            }
            let text = item
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some((text, href))
        })
        .collect()
}

/// Extract all links from the already-loaded page (no navigation).
pub async fn extract_links(tab: &OxiosTab) -> Result<Vec<(String, String)>, ToolError> {
    let value = tab
        .evaluate(JS_ALL_LINKS)
        .await
        .map_err(|e| e.to_string())?;
    Ok(parse_link_values(value))
}

/// Format link pairs as a numbered markdown list.
pub fn format_links(links: &[(String, String)]) -> String {
    links
        .iter()
        .enumerate()
        .map(|(i, (text, href))| {
            if text.is_empty() {
                format!("{}. {}", i + 1, href)
            } else {
                format!("{}. [{}]({})", i + 1, text, href)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ── Element extraction ────────────────────────────────────────────

/// JS that returns `[{ tag, text, attributes }]` for every element
/// matching `selector`.
pub fn js_query_elements(selector: &str) -> String {
    let sel = serde_json::to_string(selector).unwrap_or_default();
    format!(
        r#"(function() {{
            var els = document.querySelectorAll({sel});
            return Array.from(els).map(function(el) {{
                var attrs = {{}};
                for (var i = 0; i < el.attributes.length; i++) {{
                    attrs[el.attributes[i].name] = el.attributes[i].value;
                }}
                return {{ tag: el.tagName, text: el.textContent.trim(), attributes: attrs }};
            }});
        }})()"#
    )
}

/// Parse the JSON array returned by `js_query_elements`.
pub fn parse_element_values(
    value: Value,
) -> Vec<(String, String, std::collections::HashMap<String, String>)> {
    let Value::Array(arr) = value else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|item| {
            let tag = item.get("tag")?.as_str()?.to_string();
            let text = item
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let attributes = item
                .get("attributes")
                .and_then(|v| v.as_object())
                .map(|map| {
                    map.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect()
                })
                .unwrap_or_default();
            Some((tag, text, attributes))
        })
        .collect()
}

// ── DOM interaction JS builders ───────────────────────────────────

/// JS to set a `<select>` value and fire the `change` event.
pub fn js_set_select_value(selector: &str, value: &str) -> String {
    let sel = serde_json::to_string(selector).unwrap_or_default();
    let val = serde_json::to_string(value).unwrap_or_default();
    format!(
        r#"(function() {{
            var sel = document.querySelector({sel});
            if (!sel) throw new Error('Element not found: ' + {sel});
            sel.value = {val};
            sel.dispatchEvent(new Event('change', {{ bubbles: true }}));
        }})()"#
    )
}

/// JS to check a checkbox (only if not already checked).
pub fn js_check(selector: &str) -> String {
    let sel = serde_json::to_string(selector).unwrap_or_default();
    format!(
        r#"(function() {{
            var el = document.querySelector({sel});
            if (!el) throw new Error('Element not found: ' + {sel});
            if (!el.checked) el.click();
        }})()"#
    )
}

/// JS to uncheck a checkbox (only if currently checked).
pub fn js_uncheck(selector: &str) -> String {
    let sel = serde_json::to_string(selector).unwrap_or_default();
    format!(
        r#"(function() {{
            var el = document.querySelector({sel});
            if (!el) throw new Error('Element not found: ' + {sel});
            if (el.checked) el.click();
        }})()"#
    )
}

// ── Accessibility-surface observation ─────────────────────────────

/// JS that walks the page's interactive elements, stamps each with a stable
/// `data-oxios-ref="eN"` attribute, and returns structured element info.
pub const JS_OBSERVE: &str = r#"(function() {
    var SEL = 'a[href], button, input, textarea, select, summary, [role], [tabindex], [onclick]';
    var els = document.querySelectorAll(SEL);
    var out = [];
    var n = 0;
    for (var i = 0; i < els.length; i++) {
        var el = els[i];
        var cs = getComputedStyle(el);
        if (cs.getPropertyValue('display') === 'none') continue;
        if (cs.getPropertyValue('visibility') === 'hidden') continue;
        if (cs.getPropertyValue('opacity') === '0') continue;
        if (el.getAttribute('hidden') !== null) continue;
        if (el.getAttribute('aria-hidden') === 'true') continue;
        if (el.getAttribute('disabled') !== null) continue;
        if (cs.getPropertyValue('pointer-events') === 'none') continue;
        var tag = (el.tagName || '').toLowerCase();
        var role = el.getAttribute('role');
        if (!role) {
            var type = (el.getAttribute('type') || '').toLowerCase();
            if (tag === 'a') role = 'link';
            else if (tag === 'button' || type === 'button' || type === 'submit' || type === 'reset' || tag === 'summary') role = 'button';
            else if (tag === 'input' || tag === 'textarea') role = 'textbox';
            else if (tag === 'select') role = 'combobox';
            else if (type === 'checkbox') role = 'checkbox';
            else if (type === 'radio') role = 'radio';
            else if (tag === 'option') role = 'option';
            else role = tag;
        }
        var name = el.getAttribute('aria-label') || el.getAttribute('title') || '';
        name = name.trim();
        if (!name) {
            name = (el.textContent || '').trim().replace(/\s+/g, ' ');
        }
        name = name.slice(0, 80);
        n++;
        var ref = 'e' + n;
        try { el.setAttribute('data-oxios-ref', ref); } catch (e) {}
        out.push({
            ref_id: ref,
            role: role,
            name: name,
            tag: tag,
            selector: '[data-oxios-ref="' + ref + '"]',
            visible: true,
            interactive: true
        });
    }
    return out;
})()"#;

/// Parse the JSON array returned by [`JS_OBSERVE`] into [`ObservedElement`]s.
pub fn parse_observed_elements(value: Value) -> Vec<ObservedElement> {
    let Some(arr) = value.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|e| {
            let ref_id = e.get("ref_id")?.as_str()?.to_string();
            let s = |k: &str| {
                e.get(k)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            };
            Some(ObservedElement {
                ref_id,
                role: s("role"),
                name: s("name"),
                tag: s("tag"),
                selector: s("selector"),
                visible: e
                    .get("visible")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
                interactive: e
                    .get("interactive")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
            })
        })
        .collect()
}
