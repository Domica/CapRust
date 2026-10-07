//! Expose the short git commit hash as `CAPRUST_COMMIT` so the UI can
//! show exactly which binary a screenshot came from. No new
//! dependencies: plain `git` subprocess, `"unknown"` fallback.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // Re-run when git HEAD moves so the hash never goes stale.
    // Paths are absolute (package dir is crates/ui); missing paths
    // (e.g. no .git, worktree file) are simply skipped.
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let git_dir = std::path::Path::new(&manifest)
        .join("..")
        .join("..")
        .join(".git");
    for p in [git_dir.join("HEAD"), git_dir.join("refs")] {
        if p.exists() {
            println!("cargo:rerun-if-changed={}", p.display());
        }
    }

    let commit = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=CAPRUST_COMMIT={commit}");
}
