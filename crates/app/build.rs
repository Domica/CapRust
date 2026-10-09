//! Embed the app icon into the Windows executable metadata so
//! Explorer, taskbar pins and file properties show it. No-op on
//! other platforms.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // Same commit-hash export as crates/ui/build.rs: the viewport
    // title shows version + short hash so a screenshot always tells
    // which binary is under test. Plain `git`, "unknown" fallback.
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

    // Copy built-in LUT (.cube) files into the output directory so the
    // render graph can resolve them relative to the executable at runtime.
    // OUT_DIR is <target>/<profile>/build/<pkg>-<hash>/out, so three
    // levels up is <target>/<profile>.
    let out_dir = std::env::var("OUT_DIR").unwrap_or_default();
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let luts_src = std::path::Path::new(&manifest_dir)
        .join("assets")
        .join("luts");
    let luts_dst = std::path::Path::new(&out_dir)
        .join("..")
        .join("..")
        .join("..")
        .join("assets")
        .join("luts");
    if luts_src.is_dir() {
        let _ = std::fs::create_dir_all(&luts_dst);
        if let Ok(rd) = std::fs::read_dir(&luts_src) {
            for entry in rd.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("cube") {
                    let dst = luts_dst.join(entry.file_name());
                    let _ = std::fs::copy(&p, &dst);
                }
            }
        }
    }

    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        // Keep a sensible manifest so future code can add DPI
        // awareness / Windows 11 rounded corners without another
        // build script change.
        res.set("FileDescription", "CapRust");
        res.set("ProductName", "CapRust");
        if let Err(e) = res.compile() {
            // Do not fail the build on icon embedding; log and move on.
            eprintln!("build.rs: winres failed: {e}");
        }
    }
}
