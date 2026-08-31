//! Oxios sub-agent runner — in-process delegation via oxicode-sdk 0.54.0+.
//!
//! Wraps [`oxicode_sdk::SdkSubagentRunner`] (which wraps an `Oxicode` instance)
//! and exposes it as an [`oxicode_agent::SubagentRunner`] for the `subagent`
//! tool's in-process path. Each `run_isolated` call builds a fresh `Agent`
//! with an empty context (full isolation from the parent), runs it, and
//! returns only the final text + usage.
//!
//! # Security model
//!
//! The sub-agent built by `SdkSubagentRunner` has **zero tools** — it
//! calls `self.oxi.agent(config).build()` with no `.tool()` / `.coding_tools()`
//! / `.kernel_tools()` registration. A tool-less agent can only do pure
//! text generation: no file access, no bash, no network, no side effects.
//! This makes the sandbox-escape vector from RFC-035 §4.3 currently moot
//! for this runner.
//!
//! **Defense-in-depth upgrade path:** if a future `SdkSubagentRunner`
//! version starts honoring the `_tools` parameter or auto-registers
//! built-in tools, switch this module to delegate through
//! `AgentLifecycleManager::execute_directive` instead (RFC-035 Q2-B),
//! which inherits `allowed_tools`/`network_access`/`max_execution_time_secs`/
//! `access_manager` by construction. The "wrinkle" (ExecutionResult has
//! no token usage) is resolvable by sourcing usage from `AgentEvent::Usage`.
//!
//! # Depth safety
//!
//! The runner sets the forked agent's `subagent_depth` to `depth + 1`.
//! The SDK hardcodes the in-process max to 3 (`subagent.rs:649`), so
//! recursion is bounded without env vars (concurrent `set_var` is UB).

use std::sync::Arc;

use oxicode_sdk::SdkSubagentRunner;

/// Oxios's in-process sub-agent runner.
///
/// Constructed once at boot from the [`OxiosEngine`]'s `Oxicode` instance
/// and shared via `Arc` into every agent build path. When wired into
/// `AgentConfig.subagent_runner`, the `subagent` tool prefers this
/// in-process path over shelling out to the CLI binary.
#[derive(Clone)]
pub struct OxiosSubagentRunner {
    inner: SdkSubagentRunner,
    /// Parent's resolved chat model (e.g. `zai/glm-5.3-flash`). The SDK's
    /// reference runner keeps `AgentConfig::default()`'s hardcoded Anthropic
    /// model when the `model` argument is `None`; that provider usually has
    /// no credential in the Oxios auth store, so the fork fails its first
    /// stream call with `MissingApiKey`. We fall back to this instead.
    default_model: Option<String>,
}

impl std::fmt::Debug for OxiosSubagentRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OxiosSubagentRunner")
            .field("inner", &"<SdkSubagentRunner>")
            .field("default_model", &self.default_model)
            .finish()
    }
}

impl OxiosSubagentRunner {
    /// Create from an [`oxicode_sdk::Oxicode`] engine instance and the
    /// parent's resolved chat model.
    ///
    /// The `Oxicode` instance is `Arc`-backed, so this is clone-cheap and
    /// safe to share across concurrent tasks.
    pub fn new(oxi: oxicode_sdk::Oxicode, default_model: Option<String>) -> Self {
        Self {
            inner: SdkSubagentRunner::new(oxi),
            default_model,
        }
    }

    /// Return an `Arc<dyn SubagentRunner>` suitable for wiring into
    /// `AgentConfig::subagent_runner`.
    pub fn into_trait_object(self) -> Arc<dyn oxicode_agent::SubagentRunner> {
        Arc::new(self)
    }
}

#[async_trait::async_trait]
impl oxicode_agent::SubagentRunner for OxiosSubagentRunner {
    async fn run_isolated(
        &self,
        agent_name: &str,
        task: &str,
        system_prompt: Option<&str>,
        model: Option<&str>,
        tools: &[String],
        cwd: &std::path::Path,
        depth: u8,
    ) -> anyhow::Result<oxicode_agent::ForkResult> {
        // Fall back to the parent's resolved model when the caller passes
        // none: the SDK keeps `AgentConfig::default()`'s hardcoded Anthropic
        // id otherwise, and that provider has no credential in the Oxios
        // auth store (MissingApiKey on the first stream call). Inheriting
        // the parent's model also gives the fork the same provider/key the
        // parent turn already resolved.
        let model = model.or(self.default_model.as_deref());
        // Delegate to the SDK's reference implementation. The sub-agent
        // is tool-less (see Security model above), so no sandbox escape.
        self.inner
            .run_isolated(agent_name, task, system_prompt, model, tools, cwd, depth)
            .await
    }
}
