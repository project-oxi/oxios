//! Built-in tool profile presets — the eleven named personas the kernel
//! ships out of the box.
//!
//! Seven are domain-flavoured personas (`coding`, `code-review`,
//! `research`, `writing`, `advisory`, `security-audit`, `operations`)
//! and four are direct equivalents of the legacy role templates
//! (`worker`, `standard`, `operator`, `supervisor`). The legacy
//! surfaces are kept verbatim so downstream code can migrate
//! composition-style before doing any structural cleanup.
//!
//! Every preset shares the same activation scaffold:
//!
//! - `revision: ProfileRevision(1)`,
//! - `extends: vec![]`, `dynamic_providers: vec![]`,
//! - `activation.required_core = [kernel.fs.read, kernel.fs.grep]`,
//!   `max_active_tools = 16`, `sticky_turns = 2`.
//!
//! Catalog references resolve through [`kernel_catalog`] so the
//! selectors stay decoupled from descriptor storage.
//!
//! # Storage
//!
//! The presets live behind a `std::sync::LazyLock<Vec<ToolProfileSpec>>`
//! built once on first deref. [`builtin_profile`] returns a reference
//! into that process-lifetime buffer; the same pattern as
//! [`crate::capability::descriptor::kernel_catalog`].

use std::sync::{Arc, LazyLock};

use super::descriptor::{ToolId, ToolProviderId, ToolTag};
use super::profile::{
    CapabilityRequest, DynamicProviderContract, ProfileRevision, ResourceSelector,
    ToolActivationPolicy, ToolProfileRef, ToolProfileSpec, ToolSelector,
};
use super::types::Rights;

// ─── Static storage ──────────────────────────────────────────────────

/// Names of every built-in preset, in canonical preset-declaration
/// order. Exposed verbatim via [`builtin_profile_names`].
const PRESET_NAMES: &[&str] = &[
    "coding",
    "code-review",
    "research",
    "writing",
    "advisory",
    "security-audit",
    "operations",
    "worker",
    "standard",
    "operator",
    "supervisor",
];

/// Process-lifetime buffer of every built-in preset, built on first
/// deref. Returning `&'static ToolProfileSpec` from
/// [`builtin_profile`] borrows into this vector.
static PRESETS: LazyLock<Vec<ToolProfileSpec>> = LazyLock::new(|| {
    vec![
        make_coding(),
        make_code_review(),
        make_research(),
        make_writing(),
        make_advisory(),
        make_security_audit(),
        make_operations(),
        make_worker(),
        make_standard(),
        make_operator(),
        make_supervisor(),
    ]
});

// ─── Public API ──────────────────────────────────────────────────────

/// Return the built-in preset with the given name, or `None` if no
/// preset matches.
///
/// Names are matched case-sensitively against the canonical preset
/// slugs: `"coding"`, `"code-review"`, `"research"`, `"writing"`,
/// `"advisory"`, `"security-audit"`, `"operations"`, `"worker"`,
/// `"standard"`, `"operator"`, `"supervisor"`.
pub fn builtin_profile(name: &str) -> Option<&'static ToolProfileSpec> {
    PRESETS.iter().find(|p| p.id.as_ref() == name)
}

/// Return the names of every available built-in preset, in the order
/// they are declared at module scope.
pub fn builtin_profile_names() -> Vec<&'static str> {
    PRESET_NAMES.to_vec()
}

// ─── Helpers ─────────────────────────────────────────────────────────

/// Standard activation scaffold shared by every preset: the required
/// core is the two read-only file primitives (the substrate every role
/// needs), the active-tool ceiling stays at 16, and sticky-turns is
/// the two-turn default.
fn activation() -> ToolActivationPolicy {
    ToolActivationPolicy {
        max_active_tools: 16,
        required_core: vec![ToolId::new("kernel.fs.read"), ToolId::new("kernel.fs.grep")],
        sticky_turns: 2,
    }
}

/// Empty `extends` / `dynamic_providers` vectors — every built-in is
/// a leaf profile.
fn leaf_extensions() -> (Vec<ToolProfileRef>, Vec<DynamicProviderContract>) {
    (Vec::new(), Vec::new())
}

fn provider_tag(provider: &'static str, tag: &'static str) -> ToolSelector {
    ToolSelector::ProviderTag {
        provider: ToolProviderId::new(provider),
        tag: ToolTag::new(tag),
    }
}

fn tool_selector(id: &'static str) -> ToolSelector {
    ToolSelector::Tool(ToolId::new(id))
}

fn kernel_domain(name: &'static str, rights: Rights) -> CapabilityRequest {
    CapabilityRequest {
        resource: ResourceSelector::KernelDomain(Arc::from(name)),
        rights,
    }
}

fn fs_ceiling() -> CapabilityRequest {
    CapabilityRequest {
        resource: ResourceSelector::Fs,
        rights: Rights::READ | Rights::WRITE,
    }
}

fn fs_read_ceiling() -> CapabilityRequest {
    CapabilityRequest {
        resource: ResourceSelector::Fs,
        rights: Rights::READ,
    }
}

fn web_ceiling() -> CapabilityRequest {
    CapabilityRequest {
        resource: ResourceSelector::WebSearch,
        rights: Rights::EXECUTE,
    }
}

fn exec_shell_ceiling() -> CapabilityRequest {
    CapabilityRequest {
        resource: ResourceSelector::Exec {
            mode: Arc::from("shell"),
        },
        rights: Rights::EXECUTE,
    }
}

fn browser_ceiling() -> CapabilityRequest {
    CapabilityRequest {
        resource: ResourceSelector::Browser,
        rights: Rights::EXECUTE,
    }
}

fn memory_rw_ceiling() -> CapabilityRequest {
    CapabilityRequest {
        resource: ResourceSelector::Memory,
        rights: Rights::READ | Rights::WRITE,
    }
}

fn memory_read_ceiling() -> CapabilityRequest {
    CapabilityRequest {
        resource: ResourceSelector::Memory,
        rights: Rights::READ,
    }
}

fn knowledge_rw_ceiling() -> CapabilityRequest {
    CapabilityRequest {
        resource: ResourceSelector::Knowledge,
        rights: Rights::READ | Rights::WRITE,
    }
}

fn knowledge_read_ceiling() -> CapabilityRequest {
    CapabilityRequest {
        resource: ResourceSelector::Knowledge,
        rights: Rights::READ,
    }
}

fn ask_user_ceiling() -> CapabilityRequest {
    kernel_domain("ask_user", Rights::EXECUTE)
}

fn a2a_ceiling() -> CapabilityRequest {
    CapabilityRequest {
        resource: ResourceSelector::A2a,
        rights: Rights::EXECUTE,
    }
}

// ─── Preset constructors ─────────────────────────────────────────────

fn make_coding() -> ToolProfileSpec {
    let (extends, dynamic_providers) = leaf_extensions();
    ToolProfileSpec {
        id: Arc::from("coding"),
        revision: ProfileRevision(1),
        extends,
        capability_ceiling: vec![
            fs_ceiling(),
            web_ceiling(),
            exec_shell_ceiling(),
            browser_ceiling(),
            memory_rw_ceiling(),
            knowledge_rw_ceiling(),
            ask_user_ceiling(),
            a2a_ceiling(),
        ],
        include: vec![
            provider_tag("kernel.fs", "fs"),
            provider_tag("kernel.web", "web"),
            provider_tag("kernel.exec", "exec"),
            provider_tag("kernel.browser", "browser"),
            provider_tag("kernel.memory", "memory"),
            provider_tag("kernel.knowledge", "knowledge"),
            provider_tag("kernel.kernel", "user-interaction"),
            provider_tag("kernel.a2a", "delegation"),
        ],
        exclude: vec![],
        dynamic_providers,
        activation: activation(),
    }
}

fn make_code_review() -> ToolProfileSpec {
    let (extends, dynamic_providers) = leaf_extensions();
    ToolProfileSpec {
        id: Arc::from("code-review"),
        revision: ProfileRevision(1),
        extends,
        capability_ceiling: vec![
            fs_read_ceiling(),
            web_ceiling(),
            browser_ceiling(),
            memory_read_ceiling(),
            knowledge_read_ceiling(),
            ask_user_ceiling(),
        ],
        include: vec![
            provider_tag("kernel.fs", "fs"),
            provider_tag("kernel.web", "web"),
            provider_tag("kernel.browser", "browser"),
            provider_tag("kernel.memory", "memory"),
            provider_tag("kernel.knowledge", "knowledge"),
            provider_tag("kernel.kernel", "user-interaction"),
        ],
        exclude: vec![
            // Reviewer never mutates; ceiling is memory/knowledge
            // READ-only so memory.write / knowledge.write must not
            // even appear in the resolved set.
            tool_selector("kernel.fs.write"),
            tool_selector("kernel.fs.edit"),
            tool_selector("kernel.memory.write"),
            tool_selector("kernel.knowledge.write"),
            // No shell execution either.
            provider_tag("kernel.exec", "exec"),
        ],
        dynamic_providers,
        activation: activation(),
    }
}

fn make_research() -> ToolProfileSpec {
    let (extends, dynamic_providers) = leaf_extensions();
    ToolProfileSpec {
        id: Arc::from("research"),
        revision: ProfileRevision(1),
        extends,
        capability_ceiling: vec![
            fs_read_ceiling(),
            web_ceiling(),
            browser_ceiling(),
            memory_read_ceiling(),
            knowledge_read_ceiling(),
            ask_user_ceiling(),
        ],
        include: vec![
            provider_tag("kernel.fs", "fs"),
            provider_tag("kernel.web", "web"),
            provider_tag("kernel.browser", "browser"),
            provider_tag("kernel.memory", "memory"),
            provider_tag("kernel.knowledge", "knowledge"),
            provider_tag("kernel.kernel", "user-interaction"),
        ],
        exclude: vec![
            // Read-only research — no fs.write / fs.edit.
            tool_selector("kernel.fs.write"),
            tool_selector("kernel.fs.edit"),
            tool_selector("kernel.memory.write"),
            tool_selector("kernel.knowledge.write"),
            // No shell execution.
            provider_tag("kernel.exec", "exec"),
        ],
        dynamic_providers,
        activation: activation(),
    }
}

fn make_writing() -> ToolProfileSpec {
    let (extends, dynamic_providers) = leaf_extensions();
    ToolProfileSpec {
        id: Arc::from("writing"),
        revision: ProfileRevision(1),
        extends,
        capability_ceiling: vec![
            fs_ceiling(),
            web_ceiling(),
            memory_rw_ceiling(),
            knowledge_rw_ceiling(),
            ask_user_ceiling(),
        ],
        include: vec![
            provider_tag("kernel.fs", "fs"),
            provider_tag("kernel.web", "web"),
            provider_tag("kernel.memory", "memory"),
            provider_tag("kernel.knowledge", "knowledge"),
            provider_tag("kernel.kernel", "user-interaction"),
        ],
        exclude: vec![
            // Writing never executes shell or drives a browser.
            provider_tag("kernel.exec", "exec"),
            provider_tag("kernel.browser", "browser"),
        ],
        dynamic_providers,
        activation: activation(),
    }
}

fn make_advisory() -> ToolProfileSpec {
    let (extends, dynamic_providers) = leaf_extensions();
    ToolProfileSpec {
        id: Arc::from("advisory"),
        revision: ProfileRevision(1),
        extends,
        capability_ceiling: vec![
            fs_read_ceiling(),
            memory_read_ceiling(),
            knowledge_read_ceiling(),
            ask_user_ceiling(),
        ],
        include: vec![
            provider_tag("kernel.fs", "fs"),
            provider_tag("kernel.memory", "memory"),
            provider_tag("kernel.knowledge", "knowledge"),
            provider_tag("kernel.kernel", "user-interaction"),
        ],
        exclude: vec![
            // Read-only — no file mutation, no network, no exec.
            tool_selector("kernel.fs.write"),
            tool_selector("kernel.fs.edit"),
            tool_selector("kernel.memory.write"),
            tool_selector("kernel.knowledge.write"),
            provider_tag("kernel.web", "web"),
            provider_tag("kernel.exec", "exec"),
            provider_tag("kernel.browser", "browser"),
        ],
        dynamic_providers,
        activation: activation(),
    }
}

fn make_security_audit() -> ToolProfileSpec {
    let (extends, dynamic_providers) = leaf_extensions();
    ToolProfileSpec {
        id: Arc::from("security-audit"),
        revision: ProfileRevision(1),
        extends,
        capability_ceiling: vec![
            fs_read_ceiling(),
            web_ceiling(),
            memory_read_ceiling(),
            knowledge_read_ceiling(),
            ask_user_ceiling(),
            kernel_domain("security", Rights::READ),
            kernel_domain("budget", Rights::READ),
            kernel_domain("resource", Rights::READ),
        ],
        include: vec![
            provider_tag("kernel.fs", "fs"),
            provider_tag("kernel.web", "web"),
            provider_tag("kernel.memory", "memory"),
            provider_tag("kernel.knowledge", "knowledge"),
            provider_tag("kernel.kernel", "user-interaction"),
            // Kernel observation surface covered by tag.
            provider_tag("kernel.kernel", "kernel-observe"),
            // Defensive pin: also require the security query tool
            // explicitly so tag-taxonomy drift fails validation
            // rather than silently narrowing the auditor's reach.
            tool_selector("kernel.security.query"),
        ],
        exclude: vec![
            // No write side-effects of any kind.
            tool_selector("kernel.fs.write"),
            tool_selector("kernel.fs.edit"),
            tool_selector("kernel.memory.write"),
            tool_selector("kernel.knowledge.write"),
            // Read-only observer — no shell exec, no browser script.
            provider_tag("kernel.exec", "exec"),
        ],
        dynamic_providers,
        activation: activation(),
    }
}

fn make_operations() -> ToolProfileSpec {
    let (extends, dynamic_providers) = leaf_extensions();
    ToolProfileSpec {
        id: Arc::from("operations"),
        revision: ProfileRevision(1),
        extends,
        capability_ceiling: vec![
            fs_ceiling(),
            web_ceiling(),
            exec_shell_ceiling(),
            memory_rw_ceiling(),
            knowledge_read_ceiling(),
            ask_user_ceiling(),
            // Scoped kernel mutation surface.
            kernel_domain("project", Rights::WRITE),
            kernel_domain("agent", Rights::WRITE),
            // Observe security / budget / resource domains.
            kernel_domain("security", Rights::READ),
            kernel_domain("budget", Rights::READ),
            kernel_domain("resource", Rights::READ),
        ],
        include: vec![
            provider_tag("kernel.fs", "fs"),
            provider_tag("kernel.web", "web"),
            provider_tag("kernel.exec", "exec"),
            provider_tag("kernel.memory", "memory"),
            provider_tag("kernel.knowledge", "knowledge"),
            provider_tag("kernel.kernel", "user-interaction"),
            provider_tag("kernel.kernel", "kernel-observe"),
            // Explicit mutation tools — each is one domain's
            // ApprovalOnly mutator; pinning them makes tag drift
            // loud in `validate_publish`.
            tool_selector("kernel.project.update"),
            tool_selector("kernel.agent.update"),
        ],
        exclude: vec![
            // `kernel.persona.update` is never an operations tool.
            tool_selector("kernel.persona.update"),
            // Knowledge ceiling is R-only — the `knowledge` tag also
            // matches `kernel.knowledge.write` so the writer must be
            // dropped explicitly.
            tool_selector("kernel.knowledge.write"),
        ],
        dynamic_providers,
        activation: activation(),
    }
}

fn make_worker() -> ToolProfileSpec {
    let (extends, dynamic_providers) = leaf_extensions();
    ToolProfileSpec {
        id: Arc::from("worker"),
        revision: ProfileRevision(1),
        extends,
        capability_ceiling: vec![
            fs_ceiling(),
            web_ceiling(),
            exec_shell_ceiling(),
            browser_ceiling(),
        ],
        include: vec![
            provider_tag("kernel.fs", "fs"),
            provider_tag("kernel.web", "web"),
            provider_tag("kernel.exec", "exec"),
            provider_tag("kernel.browser", "browser"),
        ],
        exclude: vec![],
        dynamic_providers,
        activation: activation(),
    }
}

fn make_standard() -> ToolProfileSpec {
    let (extends, dynamic_providers) = leaf_extensions();
    ToolProfileSpec {
        id: Arc::from("standard"),
        revision: ProfileRevision(1),
        extends,
        capability_ceiling: vec![
            fs_ceiling(),
            web_ceiling(),
            exec_shell_ceiling(),
            browser_ceiling(),
            memory_rw_ceiling(),
            knowledge_rw_ceiling(),
            ask_user_ceiling(),
            a2a_ceiling(),
        ],
        include: vec![
            provider_tag("kernel.fs", "fs"),
            provider_tag("kernel.web", "web"),
            provider_tag("kernel.exec", "exec"),
            provider_tag("kernel.browser", "browser"),
            provider_tag("kernel.memory", "memory"),
            provider_tag("kernel.knowledge", "knowledge"),
            provider_tag("kernel.kernel", "user-interaction"),
            provider_tag("kernel.a2a", "delegation"),
        ],
        exclude: vec![],
        dynamic_providers,
        activation: activation(),
    }
}

fn make_operator() -> ToolProfileSpec {
    let (extends, dynamic_providers) = leaf_extensions();
    ToolProfileSpec {
        id: Arc::from("operator"),
        revision: ProfileRevision(1),
        extends,
        capability_ceiling: vec![
            fs_ceiling(),
            web_ceiling(),
            exec_shell_ceiling(),
            browser_ceiling(),
            memory_rw_ceiling(),
            knowledge_rw_ceiling(),
            ask_user_ceiling(),
            a2a_ceiling(),
            // Kernel observation + mutation across every domain.
            kernel_domain("persona", Rights::READ | Rights::WRITE),
            kernel_domain("project", Rights::READ | Rights::WRITE),
            kernel_domain("agent", Rights::READ | Rights::WRITE),
            kernel_domain("security", Rights::READ | Rights::WRITE),
            kernel_domain("budget", Rights::READ | Rights::WRITE),
            kernel_domain("resource", Rights::READ | Rights::WRITE),
            // Long-tail kernel-mutate domains (Task 9 catalog entries).
            kernel_domain("automation", Rights::READ | Rights::WRITE),
            kernel_domain("marketplace", Rights::READ | Rights::WRITE),
            kernel_domain("program", Rights::READ | Rights::WRITE),
            kernel_domain("calendar", Rights::READ | Rights::WRITE),
            kernel_domain("memo", Rights::READ | Rights::WRITE),
            kernel_domain("timeline", Rights::READ | Rights::WRITE),
            kernel_domain("email", Rights::READ | Rights::WRITE),
            kernel_domain("image_gen", Rights::READ | Rights::WRITE),
        ],
        include: vec![
            provider_tag("kernel.fs", "fs"),
            provider_tag("kernel.web", "web"),
            provider_tag("kernel.exec", "exec"),
            provider_tag("kernel.browser", "browser"),
            provider_tag("kernel.memory", "memory"),
            provider_tag("kernel.knowledge", "knowledge"),
            provider_tag("kernel.kernel", "user-interaction"),
            provider_tag("kernel.a2a", "delegation"),
            provider_tag("kernel.kernel", "kernel-observe"),
            provider_tag("kernel.kernel", "kernel-mutate"),
        ],
        exclude: vec![],
        dynamic_providers,
        activation: activation(),
    }
}

fn make_supervisor() -> ToolProfileSpec {
    // Supervisor is the operator surface; only the slug changes.
    let operator = make_operator();
    ToolProfileSpec {
        id: Arc::from("supervisor"),
        ..operator
    }
}

// ─── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::descriptor::kernel_catalog;
    use crate::capability::resolver::{ResolutionStatus, resolve};
    use crate::capability::{CSpace, Capability, ResourceRef, Rights};
    use crate::types::AgentId;

    #[test]
    fn all_presets_pass_publish_validation() {
        for name in builtin_profile_names() {
            let p = builtin_profile(name).unwrap();
            p.validate_publish(kernel_catalog())
                .unwrap_or_else(|e| panic!("preset {name} invalid: {e}"));
        }
    }

    #[test]
    fn code_review_has_no_write_and_no_exec() {
        let agent = AgentId::new_v4();
        let mut bounding = CSpace::new(agent);
        for (r, rights) in [
            (ResourceRef::Fs, Rights::READ | Rights::WRITE),
            (ResourceRef::WebSearch, Rights::EXECUTE),
            (ResourceRef::Browser, Rights::EXECUTE),
            (
                ResourceRef::Exec {
                    mode: "shell".into(),
                },
                Rights::EXECUTE,
            ),
            (ResourceRef::Memory, Rights::READ | Rights::WRITE),
            (ResourceRef::Knowledge, Rights::READ | Rights::WRITE),
            (
                ResourceRef::KernelDomain {
                    domain: "ask_user".into(),
                },
                Rights::EXECUTE,
            ),
        ] {
            bounding.insert(Capability::kernel(r, rights));
        }

        let resolved = resolve(
            builtin_profile("code-review").unwrap(),
            &bounding,
            agent,
            None,
        );
        let names = resolved.active_registered_names();
        assert!(names.contains(&"read".to_string()));
        assert!(
            !names.contains(&"write".to_string()),
            "code-review must not write: {names:?}"
        );
        assert!(!names.contains(&"edit".to_string()));
        assert!(!names.contains(&"exec".to_string()));
        assert!(names.contains(&"memory_read".to_string()));
        assert!(!names.contains(&"memory_write".to_string()));
    }

    #[test]
    fn worker_profile_replaces_legacy_template_surface() {
        let agent = AgentId::new_v4();
        let mut bounding = CSpace::new(agent);
        bounding.insert(Capability::kernel(
            ResourceRef::Fs,
            Rights::READ | Rights::WRITE,
        ));
        bounding.insert(Capability::kernel(ResourceRef::WebSearch, Rights::EXECUTE));
        bounding.insert(Capability::kernel(
            ResourceRef::Exec {
                mode: "shell".into(),
            },
            Rights::EXECUTE,
        ));
        bounding.insert(Capability::kernel(ResourceRef::Browser, Rights::EXECUTE));

        let resolved = resolve(builtin_profile("worker").unwrap(), &bounding, agent, None);
        let names = resolved.active_registered_names();
        for expected in [
            "read",
            "write",
            "edit",
            "grep",
            "find",
            "ls",
            "web_search",
            "get_search_results",
            "exec",
        ] {
            assert!(
                names.contains(&expected.to_string()),
                "worker missing {expected}: {names:?}"
            );
        }
    }

    #[test]
    fn operations_gets_scoped_kernel_mutation_not_persona() {
        let agent = AgentId::new_v4();
        let p = builtin_profile("operations").unwrap();
        let mut bounding = CSpace::new(agent);
        for (r, rights) in [
            (ResourceRef::Fs, Rights::READ | Rights::WRITE),
            (ResourceRef::WebSearch, Rights::EXECUTE),
            (
                ResourceRef::Exec {
                    mode: "shell".into(),
                },
                Rights::EXECUTE,
            ),
            (ResourceRef::Memory, Rights::READ | Rights::WRITE),
            (ResourceRef::Knowledge, Rights::READ),
            (
                ResourceRef::KernelDomain {
                    domain: "ask_user".into(),
                },
                Rights::EXECUTE,
            ),
            (
                ResourceRef::KernelDomain {
                    domain: "project".into(),
                },
                Rights::WRITE,
            ),
            (
                ResourceRef::KernelDomain {
                    domain: "agent".into(),
                },
                Rights::WRITE,
            ),
            (
                ResourceRef::KernelDomain {
                    domain: "persona".into(),
                },
                Rights::WRITE,
            ),
        ] {
            bounding.insert(Capability::kernel(r, rights));
        }

        let resolved = resolve(p, &bounding, agent, None);
        let status_of = |name: &str| {
            resolved
                .tools
                .iter()
                .find(|t| t.descriptor.registered_name == name)
                .map(|t| &t.status)
        };
        // project/agent mutate: ApprovalOnly class → RequiresApproval
        // (authorized, gated). (cron was removed with the cron tool.)
        assert!(matches!(
            status_of("project"),
            Some(ResolutionStatus::RequiresApproval)
        ));
        // persona is NOT included by operations → not even present.
        assert!(status_of("persona").is_none());
    }
}
