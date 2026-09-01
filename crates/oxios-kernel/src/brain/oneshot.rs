//! One-shot `oxibrain` CLI invocations — the `admin` namespace (machine
//! lifecycle: `admin index`, `admin extract --pending`), NOT the 14-op
//! agent surface. Kernel-constructed argv — the trusted host
//! tools/provisioner pattern, NOT the agent `ExecTool` sandbox path.
//!
//! [`run_cli`] is used by the boot path (document index warm-up) and the
//! 600 s extraction-drain timer; both log-and-continue on failure.

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Output;

/// Run `oxibrain <args...> --dir <dir>` and collect its output.
pub async fn run_cli(binary: &Path, dir: &Path, args: &[&str]) -> Result<Output> {
    tokio::process::Command::new(binary)
        .args(args)
        .arg("--dir")
        .arg(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // A caller that abandons the future — the drain timer's 30-min
        // timeout — must not leave the child running: without this flag the
        // dropped Child survived as an orphan (still burning GPU/CPU) while
        // the timer spawned a duplicate 30 min later.
        .kill_on_drop(true)
        .output()
        .await
        .with_context(|| format!("run {} {:?}", binary.display(), args))
}

/// oxibrain's stock CLI `admin index --embed` reports this even when the
/// lexical reconcile above it has already committed — no non-test caller
/// attaches an embedder port. The boot path and the `brain reindex` arm
/// both degrade exactly this case: retry without `--embed` and continue.
/// Revisit when oxibrain ships a built-in dense embedder or a stable flag
/// to disable the embed step.
pub fn embed_gap(stderr: &str) -> bool {
    stderr.contains("no embedding port configured")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kernel-constructed argv reaches the binary: the caller's args come
    /// first, then `--dir <dir>`. Stdout is captured for the caller.
    #[tokio::test]
    async fn run_cli_appends_dir_and_captures_args() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("oxibrain-stub");
        std::fs::write(&bin, "#!/bin/sh\necho \"$@\"\n").unwrap();
        make_executable(&bin);
        let dir = tmp.path().join("brain");
        let out = run_cli(&bin, &dir, &["admin", "index", "--documents", "--embed"])
            .await
            .unwrap();
        assert!(out.status.success());
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("index"));
        assert!(stdout.contains("--documents"));
        assert!(stdout.contains("--embed"));
        assert!(stdout.contains("--dir"));
        assert!(stdout.contains(dir.to_str().unwrap()));
    }

    /// A binary that cannot spawn surfaces as `Err`, not a panic.
    #[test]
    fn embed_gap_detects_known_upstream_marker() {
        assert!(embed_gap("Error: no embedding port configured\n"));
        assert!(embed_gap("[dense] no embedding port configured (skipped)"));
        assert!(!embed_gap("permission denied"));
        assert!(!embed_gap(""));
    }

    #[tokio::test]
    async fn run_cli_missing_binary_is_err() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(
            run_cli(&tmp.path().join("nope"), tmp.path(), &["probe"])
                .await
                .is_err()
        );
    }

    fn make_executable(p: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(p).unwrap().permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(p, perm).unwrap();
    }
}
