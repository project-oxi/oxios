//! Automation runner — the single execution path for automation runs.
//!
//! Shared by the manual-run HTTP endpoint, the auto-run tick loop, and the
//! `automation` agent tool. Wraps [`KernelHandle::run_goal`] with an overall
//! deadline and, when the verify gate is armed, a verify/repair loop driven
//! by a separate verifier conversation.
//!
//! The whole execution runs from the [`AutomationContextSnapshot`] captured
//! by [`AutomationStore::begin_run`]; a later edit to the automation
//! definition is never observed mid-run.

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::automation::model::AutomationRunTrigger;
use crate::automation::store::AutomationStore;
use crate::kernel_handle::KernelHandle;

/// One execution attempt's outcome.
#[derive(Debug, Clone, Default)]
struct Outcome {
    success: bool,
    summary: String,
    error: Option<String>,
}

/// Execute an automation run end-to-end: open the run (snapshot + run row +
/// definition execution state in one transaction) → run the snapshot's
/// instruction (bounded by `timeout_secs` total) → optionally verify/repair
/// from the snapshot's verify config → finalize.
///
/// The budget is a single deadline shared by the main execution, every
/// verifier call, and every repair re-run — a verify-enabled run can never
/// exceed the caller's ceiling.
///
/// Returns `(run_id, success, summary)`; `run_id` is empty when the run
/// could not even start. Success signal: no provider failure AND evaluation
/// passed (default true — a goal with no acceptance criteria that ran
/// cleanly is a success). This is the opposite of the metrics code's
/// `unwrap_or(false)`, which is a latent bug there and must NOT be copied.
pub async fn execute_automation_run(
    store: Arc<Mutex<AutomationStore>>,
    kernel: Arc<KernelHandle>,
    id: &str,
    trigger: AutomationRunTrigger,
    timeout_secs: u64,
) -> (String, bool, String) {
    // 1. Open the run: snapshot is captured here and is the ONLY definition
    //    data this execution ever sees.
    let run = match store.lock().await.begin_run(id, trigger, None).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(%id, error = %e, "begin_run failed");
            return (String::new(), false, format!("Failed to start run: {e}"));
        }
    };
    let snapshot = run.context_snapshot.clone();
    let instruction = snapshot.instruction.clone();

    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs.max(1));

    // 2. Execute from the snapshot.
    let mut result = run_bounded(&kernel, &instruction, deadline).await;

    // 3. Verify/repair loop — only when the run succeeded and the gate
    //    (from the snapshot) is armed.
    let mut verified = false;
    if snapshot.verify.enabled && result.success {
        let criterion = snapshot
            .verify
            .requirement
            .clone()
            .unwrap_or_else(|| instruction.clone());
        let max_attempts = snapshot.verify.max_iterations.max(1);
        let mut feedback = String::new();
        for attempt in 0..max_attempts {
            let verdict_input = verifier_prompt(&instruction, &criterion, &result.summary);
            let out = run_bounded(&kernel, &verdict_input, deadline).await;
            let (passed, reason) = parse_verdict(&out.summary);
            if passed {
                verified = true;
                feedback = reason;
                break;
            }
            feedback = reason;
            tracing::warn!(%id, attempt = attempt + 1, "verify attempt failed");
            if attempt + 1 < max_attempts {
                // Repair re-run with the verifier's feedback appended.
                result =
                    run_bounded(&kernel, &repair_prompt(&instruction, &feedback), deadline).await;
                if !result.success {
                    break;
                }
            }
        }
        if verified {
            tracing::info!(%id, feedback = %feedback, "automation verified");
        } else if result.success {
            result.success = false;
            result.error = Some(format!(
                "Verification failed after {max_attempts} attempt(s): {feedback}"
            ));
        }
    }

    // 4. Finalize: run row + definition terminal state.
    if let Err(e) = store
        .lock()
        .await
        .finish_run(
            id,
            &run.id,
            result.success,
            Some(result.summary.clone()),
            Some(result.summary.clone()),
            result.error.clone(),
        )
        .await
    {
        tracing::error!(%id, error = %e, "finish_run failed");
    }

    (run.id, result.success, result.summary)
}

/// Run one goal bounded by the remaining budget. Timeout produces a failed
/// outcome (the deadline is shared, so a timed-out phase ends the run).
async fn run_bounded(kernel: &KernelHandle, goal: &str, deadline: tokio::time::Instant) -> Outcome {
    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
    if remaining.is_zero() {
        return Outcome {
            success: false,
            summary: String::new(),
            error: Some("Deadline exhausted before this phase could start".into()),
        };
    }
    match tokio::time::timeout(remaining, kernel.run_goal(goal, None)).await {
        Ok(Ok(r)) => {
            let success = r.failure_class.is_none() && r.evaluation_passed.unwrap_or(true);
            let summary = r.output.clone().unwrap_or_else(|| r.response.clone());
            Outcome {
                success,
                summary,
                error: None,
            }
        }
        Ok(Err(e)) => Outcome {
            success: false,
            summary: String::new(),
            error: Some(e.to_string()),
        },
        Err(_) => Outcome {
            success: false,
            summary: String::new(),
            error: Some("Run timed out".into()),
        },
    }
}

/// Verifier prompt — asks a separate conversation for a PASS/FAIL verdict
/// against the acceptance criterion. Structural output: the first line must
/// be exactly `PASS` or `FAIL`.
pub fn verifier_prompt(instruction: &str, criterion: &str, result: &str) -> String {
    format!(
        "You are a task verifier. Decide whether the task result meets the acceptance criterion.\n\
         \n## Task instruction\n{instruction}\n\
         \n## Acceptance criterion\n{criterion}\n\
         \n## Task result\n{result}\n\
         \nRespond with PASS or FAIL on the FIRST line (exactly one word), followed by one sentence of reasoning. \
         If FAIL, the next line must state precisely what is missing or wrong so the executor can repair it."
    )
}

/// Repair prompt — re-runs the original instruction with the verifier's
/// feedback appended so the executor can fix the previous attempt.
pub fn repair_prompt(instruction: &str, feedback: &str) -> String {
    format!(
        "{instruction}\n\n---\nA verifier rejected your previous attempt:\n{feedback}\n\
         Address the feedback and complete the task again."
    )
}

/// Parse a verifier verdict from its output.
///
/// First non-empty line decides: leading markdown/bullet markers and
/// whitespace are tolerated, match is ASCII case-insensitive. Anything
/// unparseable is a FAIL (an unreadable verdict must never accept work).
/// Returns `(passed, reason)` — the reason carries the verifier's sentence
/// (PASS) or the failure explanation (FAIL).
pub fn parse_verdict(verifier_output: &str) -> (bool, String) {
    let first_line = verifier_output
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let cleaned: String = first_line
        .trim_start_matches(['#', '>', '*', '-', ' '])
        .to_string();
    let lower = cleaned.to_ascii_lowercase();
    let (consumed, passed) = if let Some(rest) = lower.strip_prefix("pass") {
        (lower.len() - rest.len(), true)
    } else if let Some(rest) = lower.strip_prefix("fail") {
        (lower.len() - rest.len(), false)
    } else {
        return (
            false,
            format!(
                "verifier produced no parseable verdict: {}",
                &verifier_output.chars().take(300).collect::<String>()
            ),
        );
    };
    let reason = cleaned
        .get(consumed..)
        .unwrap_or_default()
        .trim_start_matches([':', '—', '–', '-', ' ', '.'])
        .trim()
        .to_string();
    if reason.is_empty() {
        (
            passed,
            if passed {
                "passed".into()
            } else {
                "failed".into()
            },
        )
    } else {
        (passed, reason)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_verdict_pass_variants() {
        assert!(parse_verdict("PASS — looks good").0);
        assert_eq!(parse_verdict("pass").1, "passed");
        assert!(!parse_verdict("**FAIL**\nMissing pairing 3").0);
        assert_eq!(
            parse_verdict("FAIL: output had only 2 pairings").1,
            "output had only 2 pairings"
        );
        assert!(parse_verdict("  \n\nPass. All criteria met").0);
        assert!(!parse_verdict("# FAIL").0);
    }

    #[test]
    fn parse_verdict_unparseable_fails() {
        let (passed, reason) = parse_verdict("I think it's fine");
        assert!(!passed);
        assert!(reason.starts_with("verifier produced no parseable verdict"));
        assert!(!parse_verdict("").0);
        assert!(!parse_verdict("\n  \n").0);
    }

    #[test]
    fn prompts_carry_their_inputs() {
        let v = verifier_prompt("do the thing", "must contain BANANA", "the thing, done");
        assert!(v.contains("do the thing"));
        assert!(v.contains("must contain BANANA"));
        assert!(v.contains("the thing, done"));
        assert!(v.contains("PASS or FAIL"));

        let r = repair_prompt("do the thing", "no BANANA found");
        assert!(r.starts_with("do the thing"));
        assert!(r.contains("no BANANA found"));
        assert!(r.contains("Address the feedback"));
    }
}
