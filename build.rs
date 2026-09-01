//! Build script: emit the `web_embedded` cfg when the built web UI is present.
//!
//! `web/dist/` is a build artifact — gitignored, and absent in a crates.io
//! tarball (`cargo publish` ships git-tracked files only). When present at
//! compile time (local builds, CI, the release binary job), the React SPA is
//! baked into the binary via `include_dir!` (see `src/embedded_web.rs`) so the
//! daemon serves it with **no first-run download**. When absent
//! (`cargo install` from crates.io), the cfg is not set and the daemon falls
//! back to downloading `web-dist.zip` from GitHub Releases at runtime.
//!
//! This mirrors `src/default_skills.rs` embedding `share/default-skills/`,
//! except default-skills is git-tracked (always present) while `web/dist/` is
//! generated — hence the presence check rather than unconditional embedding.
//!
//! `--all-features` safety: a fresh clone has no `web/dist/`, so the cfg stays
//! off and `include_dir!` never expands. No hard compile error.

fn main() {
    // Track the dist tree itself: without this, cargo never reruns the build
    // script when only web/dist changes, and include_dir! keeps expanding the
    // stale tree on caching runners (stale-UI release incident, 2026-09-01).
    println!("cargo:rerun-if-changed=web/dist");
    let marker = std::path::Path::new("web/dist/index.html");
    if marker.is_file() {
        println!("cargo:rustc-cfg=web_embedded");
        // Make the dist contents visible to rustc's and sccache's fingerprints:
        // a changed tree changes this env, forcing dependent recompilation so
        // include_dir! re-expands instead of reusing a cached expansion.
        println!(
            "cargo:rustc-env=OXIOS_WEB_DIST_HASH={}",
            hash_dir(marker.parent().expect("dist dir"))
        );
    }
    // Declare the custom cfg so rustc's `unexpected_cfgs` lint accepts it
    // (emitted above when web/dist exists; absent otherwise).
    println!("cargo::rustc-check-cfg=cfg(web_embedded)");
}

/// Content fingerprint of the dist tree (sorted relative paths + bytes).
fn hash_dir(root: &std::path::Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    fn walk(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, files);
            } else {
                files.push(p);
            }
        }
    }
    walk(root, &mut files);
    files.sort();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for f in files {
        f.strip_prefix(root).expect("relative").hash(&mut hasher);
        if let Ok(bytes) = std::fs::read(&f) {
            hasher.write(&bytes);
        }
    }
    hasher.finish()
}
