//! Document-plane declaration (oxibrain 0.8): oxios seeds the vault root
//! into `<dir>/documents.toml`. Append-only by `alias`; existing rows are
//! preserved as data (the file is re-serialized, formatting may normalize).
//!
//! [`ensure_vault_root`] is called once at boot (detached — never blocks);
//! a failed seed is benign: the first search auto-materializes the row.

use anyhow::{Context, Result};
use std::path::Path;

/// Ensure a `[[root]]` row with `alias = "vault"` exists. Returns `true`
/// when the file was created or a row appended, `false` when a vault row
/// was already present (never rewritten — log-and-respect user edits).
pub fn ensure_vault_root(toml_path: &Path, vault_path: &Path, space: &str) -> Result<bool> {
    let mut doc: toml::Value = match std::fs::read_to_string(toml_path) {
        Ok(text) if text.trim().is_empty() => toml::Value::Table(Default::default()),
        Ok(text) => text
            .parse::<toml::Value>()
            .with_context(|| format!("parse {}", toml_path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            toml::Value::Table(Default::default())
        }
        Err(e) => return Err(e).with_context(|| format!("read {}", toml_path.display())),
    };

    let table = doc
        .as_table_mut()
        .context("documents.toml root must be a table")?;
    let roots = table
        .entry("root".to_string())
        .or_insert_with(|| toml::Value::Array(Vec::new()));
    let roots = roots
        .as_array_mut()
        .context("`root` must be an array of tables")?;
    if roots
        .iter()
        .any(|r| r.get("alias").and_then(|a| a.as_str()) == Some("vault"))
    {
        return Ok(false);
    }
    roots.push(toml::Value::Table({
        let mut row = toml::map::Map::new();
        row.insert("alias".into(), "vault".into());
        row.insert("path".into(), vault_path.display().to_string().into());
        row.insert("space".into(), space.into());
        row
    }));

    if let Some(parent) = toml_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    std::fs::write(toml_path, toml::to_string_pretty(&doc)?)
        .with_context(|| format!("write {}", toml_path.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_file_with_vault_row_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("documents.toml");
        assert!(ensure_vault_root(&p, Path::new("/Users/x/.oxi/vault"), "personal").unwrap());
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("[[root]]"));
        assert!(text.contains("alias = \"vault\""));
        assert!(text.contains("/Users/x/.oxi/vault"));
        assert!(text.contains("space = \"personal\""));
        // Idempotent: second run is a no-op.
        assert!(!ensure_vault_root(&p, Path::new("/other"), "other").unwrap());
    }

    #[test]
    fn preserves_user_rows_and_respects_existing_vault_alias() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("documents.toml");
        std::fs::write(
            &p,
            "[[root]]\nalias = \"work\"\npath = \"/work/docs\"\nspace = \"work\"\n",
        )
        .unwrap();
        assert!(ensure_vault_root(&p, Path::new("/v"), "personal").unwrap());
        let doc: toml::Value = std::fs::read_to_string(&p).unwrap().parse().unwrap();
        let roots = doc["root"].as_array().unwrap();
        assert_eq!(roots.len(), 2);
        assert!(
            roots
                .iter()
                .any(|r| r.get("alias").and_then(|a| a.as_str()) == Some("work"))
        );
        // Existing vault alias is never rewritten.
        assert!(!ensure_vault_root(&p, Path::new("/different"), "other").unwrap());
        let doc: toml::Value = std::fs::read_to_string(&p).unwrap().parse().unwrap();
        let vault = doc["root"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r.get("alias").and_then(|a| a.as_str()) == Some("vault"))
            .unwrap();
        assert_eq!(vault.get("path").and_then(|p| p.as_str()), Some("/v"));
    }
}
