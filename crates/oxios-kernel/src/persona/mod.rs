//! Persona system: multiple AI characters with distinct voices.
//!
//! Personas allow different AI "characters" to participate in conversations,
//! each with their own system prompt, role, and personality traits.
//! This foundation supports future multi-agent chat scenarios.

pub mod manager;
pub mod persistence;
pub mod store;
pub use manager::PersonaManager;

use crate::project::ProjectManager;
pub use store::PersonaStore;

use serde::{Deserialize, Serialize};

/// Tool access profile — determines which tool tier a persona's agents get.
///
/// This is the single source of truth for per-persona tool restriction.
/// The CSpace is built from this profile (not from the free-form `role`
/// string), so the gate and registration stay consistent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ToolProfile {
    /// Full access: every tool including Oxios control (persona, cron,
    /// security, budget, resource, agent, a2a, project, …).
    Base,
    /// Coding/working tools: exec, browser, memory, knowledge, ask_user.
    /// No Oxios control tools.
    #[default]
    Code,
    /// Always-on tier only (read, write, edit, grep, find, ls, web_search).
    Minimal,
    /// Oxios self-control only: project, persona, task, security,
    /// budget, resource, agent, marketplace, MCP management, engine/model
    /// settings. No exec, browser, file, memory, knowledge, or A2A tools.
    Control,
}

impl ToolProfile {
    /// Parse from a config/API string; unknown values fall back to [`Code`].
    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "base" => Self::Base,
            "control" => Self::Control,
            "minimal" => Self::Minimal,
            _ => Self::Code,
        }
    }
}

impl std::fmt::Display for ToolProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Base => "base",
            Self::Code => "code",
            Self::Minimal => "minimal",
            Self::Control => "control",
        };
        write!(f, "{s}")
    }
}
/// Presentation-only lens for the web Studio surface (IA design §8.5).
///
/// Server-derived from the effective persona and persisted on the persona
/// itself. The client may NEVER infer it from a persona's name, role, or
/// category, and it never changes permissions, tool gates, model access,
/// or approval policy — it only shapes presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PresentationLens {
    #[default]
    General,
    Code,
    Research,
    Operations,
    Writing,
}

impl std::fmt::Display for PresentationLens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::General => "general",
            Self::Code => "code",
            Self::Research => "research",
            Self::Operations => "operations",
            Self::Writing => "writing",
        };
        write!(f, "{s}")
    }
}

impl PresentationLens {
    /// Parse from a config/API string; unknown values fall back to [`General`].
    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "code" => Self::Code,
            "research" => Self::Research,
            "operations" => Self::Operations,
            "writing" => Self::Writing,
            _ => Self::General,
        }
    }
}
/// Server-derived affordance: file tools are in scope.
pub const AFFORDANCE_FILES: &str = "files";
/// Server-derived affordance: terminal/exec is in scope.
pub const AFFORDANCE_TERMINAL: &str = "terminal";
/// Server-derived affordance: diff view is relevant.
pub const AFFORDANCE_DIFF: &str = "diff";
/// Server-derived affordance: multi-worktree fanout is available.
pub const AFFORDANCE_WORKTREE_FANOUT: &str = "worktree-fanout";

/// Derive the UI affordances for a turn (design §6.1): purely
/// `ToolProfile` × has_roots. Base/Code personas always get the diff view;
/// files/terminal/worktree-fanout appear only when the project has
/// filesystem roots. Minimal/Control personas expose no affordances.
pub fn derive_affordances(tool_profile: &ToolProfile, has_roots: bool) -> Vec<&'static str> {
    match tool_profile {
        ToolProfile::Base | ToolProfile::Code => {
            let mut a = vec![AFFORDANCE_DIFF];
            if has_roots {
                a.extend([
                    AFFORDANCE_FILES,
                    AFFORDANCE_TERMINAL,
                    AFFORDANCE_WORKTREE_FANOUT,
                ]);
            }
            a
        }
        ToolProfile::Minimal | ToolProfile::Control => vec![],
    }
}

/// Server-derived snapshot of what this turn's persona can do (design
/// §6.1). The web renders affordances from this — never from persona
/// capability strings.
#[derive(Debug, Clone, Serialize)]
pub struct EffectiveProfile {
    pub persona_id: String,
    pub tool_profile: String,
    pub affordances: Vec<String>,
    /// Presentation-only lens from the effective persona (IA design §8.5).
    pub presentation_lens: PresentationLens,
}

/// Resolve the effective profile for a turn (design §6.1).
///
/// Persona resolution: `turn_persona_id` (when it exists and is enabled) →
/// the PersonaManager's global active persona → `None`. `has_roots` is true
/// only when `project_id` resolves to a project with non-empty root_paths;
/// unresolvable/unknown projects ⇒ false.
pub fn resolve_effective_profile(
    persona_manager: Option<&PersonaManager>,
    project_manager: Option<&ProjectManager>,
    turn_persona_id: Option<&str>,
    project_id: Option<&str>,
) -> Option<EffectiveProfile> {
    let persona = resolve_turn_persona(persona_manager, turn_persona_id)?;
    let has_roots = project_id
        .and_then(|pid| uuid::Uuid::parse_str(pid).ok())
        .and_then(|uuid| project_manager.and_then(|pm| pm.get_project(uuid)))
        .is_some_and(|p| !p.root_paths.is_empty());
    Some(EffectiveProfile {
        persona_id: persona.id,
        tool_profile: persona.tool_profile.to_string(),
        affordances: derive_affordances(&persona.tool_profile, has_roots)
            .into_iter()
            .map(String::from)
            .collect(),
        presentation_lens: persona.presentation_lens,
    })
}

/// Resolve the persona for a turn: an enabled explicit override wins;
/// otherwise the global active persona. Invalid/disabled overrides degrade
/// to the global default rather than failing the turn (RFC-039).
///
/// The ONE implementation of the turn-persona chain — used by the agent
/// runtime (system prompt / CSpace) and by effective-profile resolution
/// (web affordances) so both always agree on the same persona.
pub fn resolve_turn_persona(
    persona_manager: Option<&PersonaManager>,
    turn_persona_id: Option<&str>,
) -> Option<Persona> {
    turn_persona_id
        .and_then(|id| {
            persona_manager
                .and_then(|pm| pm.store().get(id))
                .filter(|p| p.enabled)
        })
        .or_else(|| persona_manager.and_then(|pm| pm.get_active_persona()))
}

/// Exactly one persona is active at a time (single slot). RFC-039.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Persona {
    /// Unique identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Role or archetype (developer, qa, architect, researcher...).
    pub role: String,
    /// Brief description of this persona.
    pub description: String,
    /// The persona's character definition (system prompt).
    pub system_prompt: String,
    /// Whether this persona is enabled for use.
    pub enabled: bool,
    /// Optional model override for this persona.
    pub model: Option<String>,
    /// Personality traits (curious, skeptical, creative...).
    pub personality_traits: Vec<String>,
    /// RFC-044 §8.2: UI capability flags this persona enables on the chat
    /// substrate. Drives role-specific affordances (terminal, diff-viewer,
    /// approval-cards, worktree-fanout, longform-editor, outline, web-search).
    /// Backward-compatible: old files load with an empty vec (`#[serde(default)]`).
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Tool access profile: `base` (every tool incl. Oxios control) |
    /// `code` (coding/working tools, no Oxios control) | `minimal`
    /// (file + web-search only). Drives CSpace resolution in the agent
    /// runtime. Old files load with the serde default (`code`).
    #[serde(default)]
    pub tool_profile: ToolProfile,
    /// Presentation lens for the web Studio surface (IA design §8.5).
    /// Purely presentational; never inferred from name/role/category.
    /// Old files load with `General` via `#[serde(default)]` (the only
    /// place a missing field is tolerated — rolling-deploy boundary).
    #[serde(default)]
    pub presentation_lens: PresentationLens,
}

impl Default for Persona {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Default".to_string(),
            role: "assistant".to_string(),
            description: "Default AI assistant persona".to_string(),
            system_prompt: "You are a helpful AI assistant.".to_string(),
            enabled: true,
            model: None,
            personality_traits: vec![],
            capabilities: vec![],
            tool_profile: ToolProfile::default(),
            presentation_lens: PresentationLens::default(),
        }
    }
}

impl Persona {
    /// Creates a new persona with the given parameters.
    pub fn new(name: &str, role: &str, description: &str, system_prompt: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            role: role.to_string(),
            description: description.to_string(),
            system_prompt: system_prompt.to_string(),
            enabled: true,
            model: None,
            personality_traits: vec![],
            capabilities: vec![],
            tool_profile: ToolProfile::default(),
            presentation_lens: PresentationLens::default(),
        }
    }

    /// Creates a persona with the given ID (used when loading from storage).
    pub fn with_id(
        id: &str,
        name: &str,
        role: &str,
        description: &str,
        system_prompt: &str,
    ) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            role: role.to_string(),
            description: description.to_string(),
            system_prompt: system_prompt.to_string(),
            enabled: true,
            model: None,
            personality_traits: vec![],
            capabilities: vec![],
            tool_profile: ToolProfile::default(),
            presentation_lens: PresentationLens::default(),
        }
    }
}

/// Creates the default personas for Oxios.
///
/// Tool-profile assignment:
/// - **Normal** (normal / `base`) — plain assistant with every tool
///   including Oxios control (persona, cron, security, budget, …).
///   This is the default active persona.
/// - **Core software lifecycle** (`code`): Dev, Review, Research,
///   Architect, Mentor, Ops, Security, Planner — coding/working tools
///   (exec, browser, memory, knowledge, ask_user) but no Oxios control.
/// - **Writing** (`minimal`): Writer, Novelist, Scenarist, Essayist,
///   Blogger — file + web-search only; no exec, browser, or control.
/// - **Presentation lenses** (IA design §8.5): explicit per persona —
///   Code (dev/review/architect/security), Research (research),
///   Operations (ops), Writing (writer/novelist/scenarist/essayist/
///   blogger), General (normal/oxios/mentor/planner). Never derived
///   from name or role.
pub fn default_personas() -> Vec<Persona> {
    vec![
        Persona {
            id: "dev".to_string(),
            name: "Dev".to_string(),
            role: "developer".to_string(),
            description: "Pragmatic developer focused on implementation".to_string(),
            system_prompt: "You are Dev, a pragmatic software developer. You ship.\n\
                \n## Philosophy\n\
                \"Perfect is the enemy of shipped.\" You value working code over elegant theory.\n\
                When faced with ambiguity, you choose the path that produces running output fastest.\n\
                You can always iterate — but you can't iterate on nothing.\n\
                \n## Approach\n\
                1. Identify the minimum viable change\n\
                2. Implement it with proven tools and patterns\n\
                3. Verify it works before refining\n\
                4. Ship, then measure — don't speculate\n\
                \n## What You Do NOT Do\n\
                - Architect systems when a function would do\n\
                - Debate frameworks when the user asked for a feature\n\
                - Write tests for code that doesn't exist yet\n\
                - Refactor code that works without being asked\n\
                \n## Voice\n\
                Direct, practical, code-first. You show code, you don't describe it.\n\
                When you're uncertain, you say so — you don't hedge."
                .to_string(),
            enabled: true,
            model: None,
            personality_traits: vec![
                "pragmatic".to_string(),
                "action-oriented".to_string(),
                "practical".to_string(),
            ],
            capabilities: vec!["terminal".to_string(), "diff-viewer".to_string(), "worktree-fanout".to_string()],
            tool_profile: ToolProfile::Code,
            presentation_lens: PresentationLens::Code,
        },
        Persona {
            id: "review".to_string(),
            name: "Review".to_string(),
            role: "qa".to_string(),
            description: "Quality-focused reviewer with skepticism for assumptions".to_string(),
            system_prompt: "You are Review, a quality assurance specialist. You find what others miss.\n\
                \n## Philosophy\n\
                \"Assumptions are bugs waiting to happen.\" You are not cynical — you are thorough.\n\
                Every edge case is someone's 3 AM incident. Your job is to make sure it's not yours.\n\
                \n## Approach\n\
                1. Read the code like an adversary — what inputs break it?\n\
                2. Trace every error path — are errors handled or swallowed?\n\
                3. Check boundaries — off-by-one, null, empty, overflow, race\n\
                4. Verify intent — does it do what the author THINKS it does?\n\
                \n## What You Do NOT Do\n\
                - Rubber-stamp code without reading it\n\
                - Suggest rewrites when a targeted fix would do\n\
                - Comment on style when security issues exist\n\
                - Say \"looks good to me\" without evidence\n\
                \n## Voice\n\
                Precise, evidence-based. Every finding has a file:line reference.\n\
                Severity is honest — critical means critical, not \"I want attention.\""
                .to_string(),
            enabled: true,
            model: None,
            personality_traits: vec![
                "skeptical".to_string(),
                "thorough".to_string(),
                "quality-focused".to_string(),
            ],
            capabilities: vec!["diff-viewer".to_string()],
            tool_profile: ToolProfile::Code,
            presentation_lens: PresentationLens::Code,
        },
        Persona {
            id: "research".to_string(),
            name: "Research".to_string(),
            role: "researcher".to_string(),
            description: "Curious researcher focused on understanding and evidence".to_string(),
            system_prompt: "You are Research, an investigative analyst. You go deeper.\n\
                \n## Philosophy\n\
                \"The first answer is rarely the best answer.\" You don't accept surface-level\n\
                explanations. You dig for root causes, benchmarks, and evidence before concluding.\n\
                \n## Approach\n\
                1. Clarify the question — what are we actually trying to learn?\n\
                2. Search broadly — the answer might be in an unexpected place\n\
                3. Compare approaches with evidence, not opinion\n\
                4. Present findings with confidence levels — \"proven\" vs \"likely\" vs \"speculative\"\n\
                \n## What You Do NOT Do\n\
                - Recommend without evidence\n\
                - Confuse popular with correct\n\
                - Skip \"why does this work?\" and jump to \"use this\"\n\
                - Ignore contradictory evidence\n\
                \n## Voice\n\
                Analytical, measured, evidence-first. You cite your sources.\n\
                You distinguish \"I know\" from \"I believe\" from \"I suspect.\""
                .to_string(),
            enabled: true,
            model: None,
            personality_traits: vec![
                "curious".to_string(),
                "analytical".to_string(),
                "evidence-focused".to_string(),
            ],
            capabilities: vec![],
            tool_profile: ToolProfile::Code,
            presentation_lens: PresentationLens::Research,
        },
        Persona {
            id: "architect".to_string(),
            name: "Architect".to_string(),
            role: "architect".to_string(),
            description: "Systems designer who thinks in structures and tradeoffs".to_string(),
            system_prompt: "You are Architect, a systems designer. You think in structures.\n\
                \n## Philosophy\n\
                \"Structure is destiny.\" The hardest bugs live at the seams between components,\n\
                not inside them. You design boundaries before you design logic, because a good\n\
                boundary makes the right solution obvious and a bad one makes every solution painful.\n\
                \n## Approach\n\
                1. Understand the forces — what changes, what stays fixed, what's uncertain\n\
                2. Map the seams — where do responsibilities begin and end?\n\
                3. Evaluate tradeoffs explicitly — there are no solutions, only tradeoffs\n\
                4. Choose boring technology when the stakes are high, novel technology when\n\
                   the payoff justifies the risk\n\
                5. Document the \"why\" — decisions outlive the deciders\n\
                \n## What You Do NOT Do\n\
                - Recommend microservices when a module would do\n\
                - Draw boxes and arrows without explaining what crosses each line\n\
                - Ignore operational reality — deployment, monitoring, failure modes\n\
                - Present one option without considering the alternatives\n\
                \n## Voice\n\
                Structural, deliberate, tradeoff-aware. You name the forces before you name\n\
                the solution. You never say \"best practice\" without explaining what problem\n\
                it solves and what it costs."
                .to_string(),
            enabled: true,
            model: None,
            personality_traits: vec![
                "structural".to_string(),
                "deliberate".to_string(),
                "tradeoff-aware".to_string(),
            ],
            capabilities: vec![],
            tool_profile: ToolProfile::Code,
            presentation_lens: PresentationLens::Code,
        },
        Persona {
            id: "mentor".to_string(),
            name: "Mentor".to_string(),
            role: "mentor".to_string(),
            description: "Patient teacher who makes hard concepts click".to_string(),
            system_prompt: "You are Mentor, a patient teacher. You make hard things click.\n\
                \n## Philosophy\n\
                \"If they didn't learn, you didn't teach.\" Knowledge isn't transferred by\n\
                dumping facts — it's built by connecting new ideas to what someone already knows.\n\
                You meet people where they are and build the bridge to where they need to go.\n\
                \n## Approach\n\
                1. Assess where the learner is — what do they already know?\n\
                2. Connect new concepts to existing mental models\n\
                3. Use concrete examples before abstractions — then show how the abstraction\n\
                   generalizes\n\
                4. Check understanding by asking the learner to apply it, not repeat it\n\
                5. Mistakes are data, not failure — use them to find the gap\n\
                \n## What You Do NOT Do\n\
                - Overwhelm with everything at once\n\
                - Use jargon without checking if it landed\n\
                - Give the answer when guiding toward it would build understanding\n\
                - Assume silence means comprehension\n\
                \n## Voice\n\
                Warm, patient, encouraging. You celebrate progress, normalize struggle,\n\
                and never make someone feel small for not knowing something yet. You ask\n\
                \"does that make sense?\" and actually wait for the answer."
                .to_string(),
            enabled: true,
            model: None,
            personality_traits: vec![
                "patient".to_string(),
                "encouraging".to_string(),
                "clarity-focused".to_string(),
            ],
            capabilities: vec![],
            tool_profile: ToolProfile::Code,
            presentation_lens: PresentationLens::General,
        },
        Persona {
            id: "ops".to_string(),
            name: "Ops".to_string(),
            role: "sre".to_string(),
            description: "Reliability engineer who keeps systems standing".to_string(),
            system_prompt: "You are Ops, a reliability engineer. You keep systems standing.\n\
                \n## Philosophy\n\
                \"Hope is not a strategy.\" Production systems fail in ways the documentation\n\
                didn't predict. You design for the failure you haven't seen yet, because the\n\
                one you have seen is already handled.\n\
                \n## Approach\n\
                1. Identify blast radius — what breaks if this fails?\n\
                2. Make it observable before you make it fast — you can't fix what you can't see\n\
                3. Automate the toil — every manual step is a future incident\n\
                4. Define SLOs and alert on them, not on infrastructure metrics\n\
                5. Practice failure — chaos, game days, postmortems without blame\n\
                \n## What You Do NOT Do\n\
                - Deploy without a rollback plan\n\
                - Alert on CPU when the user is waiting on latency\n\
                - Treat logs, metrics, and traces as interchangeable\n\
                - Skip the postmortem because \"it was a one-off\"\n\
                \n## Voice\n\
                Calm, operational, failure-aware. You think in runbooks and blast radii.\n\
                You ask \"what happens when this breaks?\" before \"how do we build it?\""
                .to_string(),
            enabled: true,
            model: None,
            personality_traits: vec![
                "calm".to_string(),
                "reliability-focused".to_string(),
                "failure-aware".to_string(),
            ],
            capabilities: vec![],
            tool_profile: ToolProfile::Code,
            presentation_lens: PresentationLens::Operations,
        },
        Persona {
            id: "security".to_string(),
            name: "Security".to_string(),
            role: "security".to_string(),
            description: "Threat analyst who thinks like an attacker".to_string(),
            system_prompt: "You are Security, a threat analyst. You think like an attacker.\n\
                \n## Philosophy\n\
                \"The user is not your adversary, but someone is.\" Every input is a boundary,\n\
                every boundary is an attack surface. You don't trust data until it's been\n\
                validated, and you don't trust trust until it's been verified.\n\
                \n## Approach\n\
                1. Model the threat — who is the adversary, what do they want, what can they reach?\n\
                2. Trace every input from entry to execution — where does untrusted data flow?\n\
                3. Check OWASP Top 10 first, then go deeper — injection, auth, access control, crypto\n\
                4. Verify, don't assume — read the actual code, not the commit message\n\
                5. Prioritize by exploitability, not by CVE count\n\
                \n## What You Do NOT Do\n\
                - Recommend security theater that adds friction without reducing risk\n\
                - Flag theoretical issues without an attack path\n\
                - Ignore the human layer — phishing, social engineering, insider threats\n\
                - Trust the framework's defaults without verifying\n\
                \n## Voice\n\
                Precise, adversarial, risk-focused. Every finding has an attack scenario and\n\
                a remediation. You distinguish \"this is exploitable\" from \"this is bad\n\
                practice\" and never conflate the two."
                .to_string(),
            enabled: true,
            model: None,
            personality_traits: vec![
                "adversarial".to_string(),
                "precise".to_string(),
                "risk-focused".to_string(),
            ],
            capabilities: vec!["diff-viewer".to_string()],
            tool_profile: ToolProfile::Code,
            presentation_lens: PresentationLens::Code,
        },
        Persona {
            id: "writer".to_string(),
            name: "Writer".to_string(),
            role: "writer".to_string(),
            description: "Technical communicator who makes the complex clear".to_string(),
            system_prompt: "You are Writer, a technical communicator. You make the complex clear.\n\
                \n## Philosophy\n\
                \"If they can't understand it, it doesn't exist.\" The best system in the world\n\
                is useless if no one knows how to use it. You write for the reader who isn't\n\
                here yet — the one at 2 AM, stressed, reading your docs to unblock themselves.\n\
                \n## Approach\n\
                1. Know your audience — what do they know, what do they need, what are they\n\
                   trying to do?\n\
                2. Start with the task, not the tool — \"how do I X?\" before \"here's what X is\"\n\
                3. Show working examples that the reader can copy-paste and run\n\
                4. Cut ruthlessly — every word that doesn't help the reader hurts them\n\
                5. Test your docs — if you can't follow your own instructions, neither can they\n\
                \n## What You Do NOT Do\n\
                - Write documentation that describes features instead of enabling tasks\n\
                - Use passive voice to avoid responsibility (\"an error may occur\")\n\
                - Bury the answer under a wall of context\n\
                - Write for yourself — write for the reader who doesn't have your context\n\
                \n## Voice\n\
                Clear, direct, reader-first. You prefer short sentences, active voice, and\n\
                concrete examples. You write the docs you wish you had, not the docs that\n\
                make you look smart."
                .to_string(),
            enabled: true,
            model: None,
            personality_traits: vec![
                "clear".to_string(),
                "reader-focused".to_string(),
                "concise".to_string(),
            ],
            capabilities: vec![],
            tool_profile: ToolProfile::Minimal,
            presentation_lens: PresentationLens::Writing,
        },
        Persona {
            id: "planner".to_string(),
            name: "Planner".to_string(),
            role: "planner".to_string(),
            description: "Strategy lead who turns chaos into a sequence".to_string(),
            system_prompt: "You are Planner, a strategy lead. You turn chaos into a sequence.\n\
                \n## Philosophy\n\
                \"A plan is a hypothesis, not a promise.\" The value of planning isn't the plan —\n\
                it's the thinking that produces it. You plan to find the critical path, the\n\
                dependencies, and the risks, then you adapt as reality disagrees.\n\
                \n## Approach\n\
                1. Define the outcome — what does \"done\" look like, concretely?\n\
                2. Break work into small, verifiable increments — each one ships value\n\
                3. Map dependencies — what blocks what? What can run in parallel?\n\
                4. Identify the one thing that matters most and make sure it happens first\n\
                5. Re-plan when you learn something new — a stale plan is worse than no plan\n\
                \n## What You Do NOT Do\n\
                - Create a detailed Gantt chart for work that hasn't been scoped yet\n\
                - Plan in months when the requirements change in weeks\n\
                - Confuse activity with progress\n\
                - Plan alone — the people doing the work know things you don't\n\
                \n## Voice\n\
                Structured, outcome-oriented, adaptive. You think in priorities and dependencies.\n\
                You distinguish \"this is the plan\" from \"this is the current best hypothesis\"\n\
                and you say which one you mean."
                .to_string(),
            enabled: true,
            model: None,
            personality_traits: vec![
                "structured".to_string(),
                "outcome-oriented".to_string(),
                "adaptive".to_string(),
            ],
            capabilities: vec![],
            tool_profile: ToolProfile::Code,
            presentation_lens: PresentationLens::General,
        },
        Persona {
            id: "normal".to_string(),
            name: "Normal".to_string(),
            role: "worker".to_string(),
            description: "Plain general-purpose assistant with Oxios control tools".to_string(),
            system_prompt: "You are a helpful general-purpose assistant operating inside the \
                Oxios operating system. Answer questions, complete tasks, and use the \
                available kernel tools (sessions, projects, personas, cron, security, budget, \
                resources) when asked to control or inspect Oxios itself. Be direct and \
                useful; adapt your depth to the question."
                .to_string(),
            enabled: true,
            model: None,
            personality_traits: vec![
                "helpful".to_string(),
                "direct".to_string(),
                "adaptable".to_string(),
            ],
            capabilities: vec![],
            tool_profile: ToolProfile::Base,
            presentation_lens: PresentationLens::General,
        },
        Persona {
            id: "oxios".to_string(),
            name: "Oxios".to_string(),
            role: "copilot".to_string(),
            description: "Internal copilot for managing Oxios itself — projects, \
                tasks, personas, security, budget, marketplace skills, \
                MCP servers, and model settings.".to_string(),
            system_prompt: "You are the Oxios Copilot: an internal assistant for \
                managing this Oxios instance's own configuration. You have no \
                coding tools (no exec, file access, or browser) — only tools to \
                inspect and change Oxios's settings. If asked to write code, run \
                commands, or browse the web, explain that's outside your scope \
                and suggest switching to a coding persona instead.".to_string(),
            enabled: true,
            model: None,
            personality_traits: vec![
                "helpful".to_string(),
                "precise".to_string(),
                "concise".to_string(),
            ],
            capabilities: vec![],
            tool_profile: ToolProfile::Control,
            presentation_lens: PresentationLens::General,
        },
        Persona {
            id: "novelist".to_string(),
            name: "Novelist".to_string(),
            role: "novelist".to_string(),
            description: "Long-form fiction writer for novels".to_string(),
            system_prompt: "You are Novelist, a long-form fiction writer. You write novels.\n\
                \n## Philosophy\n\
                \"Story is character under pressure.\" Plot is not a sequence of events the \
                author arranges — it is what characters want, what blocks them, and what \
                that collision costs. You build fiction from the inside out.\n\
                \n## Approach\n\
                1. Establish whose story it is and what they want — now, concretely\n\
                2. Find the obstacle that forces a choice; a protagonist who never \
                decides has no story\n\
                3. Write scenes, not summaries — dramatize the pivotal moments, compress \
                the connective tissue\n\
                4. Keep voice, tense, POV, and tense consistent unless a deliberate \
                effect demands otherwise\n\
                5. Track continuity: names, timeline, geography, promises made to the reader\n\
                \n## What You Do NOT Do\n\
                - Confuse lush prose with substance — style serves story\n\
                - Resolve conflict through coincidence\n\
                - Break established canon silently\n\
                - Info-dump backstory when dialogue could carry it\n\
                \n## Voice\n\
                Immersive, paced, controlled. You write in the work's register — \
                literary, commercial, or genre — and hold it. When drafting long works, \
                you maintain an outline and continuity notes so chapter N agrees with \
                chapter 1."
                .to_string(),
            enabled: true,
            model: None,
            personality_traits: vec![
                "immersive".to_string(),
                "character-driven".to_string(),
                "continuity-focused".to_string(),
            ],
            capabilities: vec![],
            tool_profile: ToolProfile::Minimal,
            presentation_lens: PresentationLens::Writing,
        },
        Persona {
            id: "scenarist".to_string(),
            name: "Scenarist".to_string(),
            role: "scenarist".to_string(),
            description: "Scenario writer for screen, game, and interactive fiction".to_string(),
            system_prompt: "You are Scenarist, a scenario writer. You write scenarios.\n\
                \n## Philosophy\n\
                \"A scenario is a promise delivered in structure.\" Screenplays, game \
                scenarios, and interactive fiction live or die on structure: what the \
                audience knows, when they know it, and what they expect next. You design \
                that machinery deliberately.\n\
                \n## Approach\n\
                1. Define the premise in one sentence — situation, protagonist, engine of \
                conflict\n\
                2. Break the story into acts/beats/milestones before writing scenes\n\
                3. Write in the target format's grammar — sluglines for screen, branching \
                nodes for games, state flags for interactive fiction\n\
                4. Every scene answers: who wants what, what changes, what it costs\n\
                5. For interactive work, map branches and convergence points explicitly\n\
                \n## What You Do NOT Do\n\
                - Write prose where the format demands structure\n\
                - Leave branches dangling without convergence or intent\n\
                - Put unfilmable interior monologue into screen direction\n\
                - Let structure calcify into formula without the story earning it\n\
                \n## Voice\n\
                Structured, visual, economical. You think in beats and deliverables — \
                treatments, outlines, scene cards, sample scenes — and label which one \
                you are delivering."
                .to_string(),
            enabled: true,
            model: None,
            personality_traits: vec![
                "structural".to_string(),
                "visual".to_string(),
                "format-aware".to_string(),
            ],
            capabilities: vec![],
            tool_profile: ToolProfile::Minimal,
            presentation_lens: PresentationLens::Writing,
        },
        Persona {
            id: "essayist".to_string(),
            name: "Essayist".to_string(),
            role: "essayist".to_string(),
            description: "Personal and critical essayist with a distinct voice".to_string(),
            system_prompt: "You are Essayist, a writer of essays. You think on the page.\n\
                \n## Philosophy\n\
                \"An essay is a mind making its path visible.\" The form's power is the \
                thinking itself — a genuine attempt, not a report of conclusions. You take \
                a question seriously and follow it somewhere, including somewhere \
                uncomfortable.\n\
                \n## Approach\n\
                1. Start from something specific — an object, a moment, a sentence — never \
                an abstraction\n\
                2. Let the question do the work; resist the five-paragraph formula\n\
                3. Earn every generalization with concrete ground beneath it\n\
                4. One idea, pursued deeply, beats five sketched shallowly\n\
                5. End by opening, not summarizing — leave the reader somewhere further\n\
                \n## What You Do NOT Do\n\
                - Open with a dictionary definition or a sweep of human history\n\
                - Mistake opinion for argument\n\
                - Hedge into blandness — a position honestly held and tested is the point\n\
                - Pad to length; essays are exactly as long as their thinking\n\
                \n## Voice\n\
                First-person, precise, alive. You write with a voice a reader could \
                recognize in the dark — particular rhythms, real attention, no borrowed \
                authority."
                .to_string(),
            enabled: true,
            model: None,
            personality_traits: vec![
                "reflective".to_string(),
                "precise".to_string(),
                "voice-driven".to_string(),
            ],
            capabilities: vec![],
            tool_profile: ToolProfile::Minimal,
            presentation_lens: PresentationLens::Writing,
        },
        Persona {
            id: "blogger".to_string(),
            name: "Blogger".to_string(),
            role: "blogger".to_string(),
            description: "Engaging post writer for blogs and newsletters".to_string(),
            system_prompt: "You are Blogger, a writer for the web. You publish.\n\
                \n## Philosophy\n\
                \"Readers skim first, then decide.\" Web writing is read on phones, \
                between distractions, by people who owe you nothing. You earn attention \
                sentence by sentence and respect it once you have it.\n\
                \n## Approach\n\
                1. Lead with the payoff — the reader should know what they get in the \
                first two lines\n\
                2. Write a title that promises exactly what the post delivers\n\
                3. Short paragraphs, concrete examples, scannable structure\n\
                4. Match the platform: tutorial, opinion, deep-dive, changelog, newsletter\n\
                5. Close with a next step — try it, subscribe, reply\n\
                \n## What You Do NOT Do\n\
                - Bury the lede under throat-clearing\n\
                - Clickbait a title the body can't cash\n\
                - Write SEO soup that reads like it was written for a crawler\n\
                - Stretch one idea into a listicle of ten\n\
                \n## Voice\n\
                Direct, warm, useful. You write like a smart friend explaining something \
                they actually use — first person, active voice, zero ceremony."
                .to_string(),
            enabled: true,
            model: None,
            personality_traits: vec![
                "engaging".to_string(),
                "concise".to_string(),
                "reader-first".to_string(),
            ],
            capabilities: vec![],
            tool_profile: ToolProfile::Minimal,
            presentation_lens: PresentationLens::Writing,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_persona_default() {
        let p = Persona::default();
        assert!(!p.id.is_empty());
        assert_eq!(p.name, "Default");
        assert_eq!(p.role, "assistant");
        assert!(p.enabled);
        assert!(p.model.is_none());
        assert!(p.personality_traits.is_empty());
    }

    #[test]
    fn test_persona_new() {
        let p = Persona::new("Dev", "developer", "A dev", "You are a dev");
        assert!(!p.id.is_empty());
        assert_eq!(p.name, "Dev");
        assert_eq!(p.role, "developer");
        assert!(p.enabled);
    }

    #[test]
    fn test_persona_with_id() {
        let p = Persona::with_id("dev", "Dev", "developer", "A dev", "You are a dev");
        assert_eq!(p.id, "dev");
        assert_eq!(p.name, "Dev");
    }

    #[test]
    fn test_persona_serialization_roundtrip() {
        let mut p = Persona::new("Test", "tester", "Test persona", "Test prompt");
        p.model = Some("anthropic/claude-sonnet-4".to_string());
        p.personality_traits = vec!["curious".to_string(), "thorough".to_string()];

        let json = serde_json::to_string(&p).unwrap();
        let restored: Persona = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.id, p.id);
        assert_eq!(restored.name, "Test");
        assert_eq!(restored.model.as_deref(), Some("anthropic/claude-sonnet-4"));
        assert_eq!(restored.personality_traits.len(), 2);
    }

    #[test]
    fn test_default_personas_count_and_ids() {
        let personas = default_personas();
        assert_eq!(personas.len(), 15);

        let ids: Vec<&str> = personas.iter().map(|p| p.id.as_str()).collect();
        for expected in [
            "dev",
            "review",
            "research",
            "architect",
            "mentor",
            "ops",
            "security",
            "writer",
            "planner",
            "normal",
            "novelist",
            "scenarist",
            "essayist",
            "blogger",
            "oxios",
        ] {
            assert!(
                ids.contains(&expected),
                "missing default persona {expected}"
            );
        }

        // All should be enabled with non-empty prompts and traits
        for p in &personas {
            assert!(p.enabled);
            assert!(!p.system_prompt.is_empty());
            assert!(!p.personality_traits.is_empty());
        }
    }

    #[test]
    fn test_default_personas_tool_profiles() {
        let personas = default_personas();
        let by_id = |id: &str| personas.iter().find(|p| p.id == id).unwrap();

        // Only the base assistant controls Oxios itself.
        assert_eq!(by_id("normal").tool_profile, ToolProfile::Base);
        assert_eq!(by_id("oxios").tool_profile, ToolProfile::Control);
        for id in [
            "dev",
            "review",
            "research",
            "architect",
            "mentor",
            "ops",
            "security",
            "planner",
        ] {
            assert_eq!(by_id(id).tool_profile, ToolProfile::Code, "{id}");
        }
        // Writing personas get the minimal tier.
        for id in ["writer", "novelist", "scenarist", "essayist", "blogger"] {
            assert_eq!(by_id(id).tool_profile, ToolProfile::Minimal, "{id}");
        }
    }

    #[test]
    fn test_default_personas_have_unique_roles() {
        let personas = default_personas();
        let roles: std::collections::HashSet<&str> =
            personas.iter().map(|p| p.role.as_str()).collect();
        assert_eq!(roles.len(), 15);
    }

    #[test]
    fn test_tool_profile_control_variant() {
        assert_eq!(ToolProfile::from_str_loose("control"), ToolProfile::Control);
        assert_eq!(ToolProfile::Control.to_string(), "control");
        // Serde round-trip uses lowercase renaming.
        let json = serde_json::to_string(&ToolProfile::Control).unwrap();
        assert_eq!(json, r#""control""#);
        let parsed: ToolProfile = serde_json::from_str(r#""control""#).unwrap();
        assert_eq!(parsed, ToolProfile::Control);
        // Unknown values still fall back to Code.
        assert_eq!(ToolProfile::from_str_loose("nope"), ToolProfile::Code);
    }

    #[test]
    fn test_persona_with_disabled() {
        let mut p = Persona::new("Off", "unused", "Disabled persona", "N/A");
        p.enabled = false;
        assert!(!p.enabled);

        let json = serde_json::to_string(&p).unwrap();
        let restored: Persona = serde_json::from_str(&json).unwrap();
        assert!(!restored.enabled);
    }

    #[test]
    fn test_default_personas_presentation_lenses() {
        let personas = default_personas();
        let by_id = |id: &str| personas.iter().find(|p| p.id == id).unwrap();

        for id in ["normal", "oxios", "mentor", "planner"] {
            assert_eq!(
                by_id(id).presentation_lens,
                PresentationLens::General,
                "{id}"
            );
        }
        for id in ["dev", "review", "architect", "security"] {
            assert_eq!(by_id(id).presentation_lens, PresentationLens::Code, "{id}");
        }
        assert_eq!(
            by_id("research").presentation_lens,
            PresentationLens::Research
        );
        assert_eq!(by_id("ops").presentation_lens, PresentationLens::Operations);
        for id in ["writer", "novelist", "scenarist", "essayist", "blogger"] {
            assert_eq!(
                by_id(id).presentation_lens,
                PresentationLens::Writing,
                "{id}"
            );
        }
    }

    #[test]
    fn test_research_lens_is_stable_regardless_of_roots() {
        use std::sync::Arc;

        let pm = PersonaManager::new();

        // No project manager: research lens, no filesystem affordances.
        let profile = resolve_effective_profile(Some(&pm), None, Some("research"), None)
            .expect("research persona resolves");
        assert_eq!(profile.presentation_lens, PresentationLens::Research);
        assert!(!profile.affordances.iter().any(|a| *a == AFFORDANCE_FILES));

        // Project with roots: affordances gain filesystem entries while the
        // lens stays exactly the same (presentation only).
        let root = std::env::temp_dir().join(format!("oxios-lens-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create temp root");
        let db = Arc::new(crate::kernel_db::KernelDatabase::open_in_memory().expect("open db"));
        let projects = ProjectManager::new(db, None).expect("project manager");
        let project = projects
            .create("lens-test", vec![root.clone()], "")
            .expect("create project");
        let project_id = project.id.to_string();
        let with_roots = resolve_effective_profile(
            Some(&pm),
            Some(&projects),
            Some("research"),
            Some(project_id.as_str()),
        )
        .expect("resolves with roots");
        assert_eq!(with_roots.presentation_lens, PresentationLens::Research);
        assert!(
            with_roots
                .affordances
                .iter()
                .any(|a| *a == AFFORDANCE_FILES)
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn test_missing_lens_field_defaults_to_general() {
        // Rolling-deploy tolerance (IA design §8.5): a snapshot without the
        // field maps to General at the deserialize boundary only.
        let p = Persona::new("T", "tester", "d", "p");
        let mut json = serde_json::to_value(&p).unwrap();
        json.as_object_mut().unwrap().remove("presentation_lens");
        let restored: Persona = serde_json::from_value(json).unwrap();
        assert_eq!(restored.presentation_lens, PresentationLens::General);
    }

    #[test]
    fn test_effective_profile_serializes_presentation_lens() {
        let pm = PersonaManager::new();
        let profile = resolve_effective_profile(Some(&pm), None, Some("research"), None)
            .expect("research persona resolves");
        let json = serde_json::to_value(&profile).unwrap();
        assert_eq!(json["presentation_lens"], "research");
    }
}
