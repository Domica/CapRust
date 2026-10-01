//! Crash handler: writes a diagnostic report for every uncaught panic.
//!
//! Installed as early as possible in `main` so a panic anywhere in the
//! process (icon decode, tracing init, eframe event loop) is captured.
//! Reports land in `%APPDATA%/CapRust/crashes/` (or `~/.caprust/crashes/`
//! on non-Windows). Retention: the newest `RETENTION` reports are kept;
//! older ones are pruned on write.
//!
//! No external dependencies: version from `CARGO_PKG_VERSION`, timestamp
//! as a Unix epoch (sortable, no date formatting needed), backtrace via
//! `std::backtrace::Backtrace::force_capture()` so we do not require the
//! user to set `RUST_BACKTRACE=1`.

use std::backtrace::Backtrace;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Number of crash reports kept on disk. Older entries are removed
/// whenever a new report is written.
const RETENTION: usize = 20;

/// Install the panic hook. Chains to the previous hook so the panic
/// is still printed to stderr (helpful in dev, harmless in release).
pub fn install() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(path) = write_crash_log(info) {
            eprintln!("crash log written to {}", path.display());
        }
        original(info);
    }));
}

/// Directory where crash reports are written. `None` if we cannot
/// determine a base directory; caller treats that as "log nothing".
fn crashes_dir() -> Option<PathBuf> {
    let base = std::env::var("APPDATA")
        .or_else(|_| std::env::var("HOME"))
        .ok()?;
    let sub = if cfg!(windows) { "CapRust" } else { ".caprust" };
    Some(PathBuf::from(base).join(sub).join("crashes"))
}

/// Extract a best-effort human string from the panic payload.
fn payload_to_string(info: &std::panic::PanicHookInfo) -> String {
    if let Some(s) = info.payload().downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

/// Write a report for this panic. Returns the path on success.
fn write_crash_log(info: &std::panic::PanicHookInfo) -> Option<PathBuf> {
    let dir = crashes_dir()?;
    fs::create_dir_all(&dir).ok()?;

    let unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "<unknown>".to_string());

    let payload = payload_to_string(info);
    let backtrace = Backtrace::force_capture().to_string();
    let version = env!("CARGO_PKG_VERSION");

    let path = write_report(&dir, unix, version, &location, &payload, &backtrace).ok()?;
    prune(&dir);
    Some(path)
}

/// Core writer. Kept separate from `write_crash_log` so tests can drive
/// it with a temp dir and synthetic values, without instantiating a
/// real `PanicHookInfo`.
fn write_report(
    dir: &Path,
    unix: u64,
    version: &str,
    location: &str,
    payload: &str,
    backtrace: &str,
) -> std::io::Result<PathBuf> {
    let path = dir.join(format!("crash-{unix}.log"));
    let body = format!(
        "CapRust crash report
         ====================
         version:    {version}
         unix_time:  {unix}
         location:   {location}
         
         message:
         {payload}
         
         backtrace:
         {backtrace}
"
    );
    fs::write(&path, body)?;
    Ok(path)
}

/// Keep only the newest `RETENTION` `crash-*.log` files in `dir`.
/// Silent on any I/O error: we are already on the crash path.
fn prune(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut logs: Vec<(u64, PathBuf)> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let stamp = name
                .strip_prefix("crash-")?
                .strip_suffix(".log")?
                .parse::<u64>()
                .ok()?;
            Some((stamp, e.path()))
        })
        .collect();
    if logs.len() <= RETENTION {
        return;
    }
    logs.sort_by_key(|(s, _)| *s);
    let drop_count = logs.len() - RETENTION;
    for (_, p) in logs.into_iter().take(drop_count) {
        let _ = fs::remove_file(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unique scratch dir so parallel test runs cannot collide.
    fn scratch(name: &str) -> PathBuf {
        let pid = std::process::id();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("caprust-crash-{name}-{pid}-{nanos}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn report_filename_is_crash_dash_unix() {
        let dir = scratch("fname");
        let p = write_report(
            &dir,
            1_700_000_000,
            "0.6.0-alpha.1",
            "x.rs:1:1",
            "boom",
            "bt",
        )
        .expect("write");
        assert_eq!(p.file_name().unwrap(), "crash-1700000000.log");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn report_body_contains_all_fields() {
        let dir = scratch("body");
        let p = write_report(
            &dir,
            42,
            "9.9.9-test",
            "crates/ui/src/app.rs:100:7",
            "assertion failed",
            "backtrace line",
        )
        .expect("write");
        let text = fs::read_to_string(&p).expect("read");
        assert!(text.contains("version:    9.9.9-test"));
        assert!(text.contains("unix_time:  42"));
        assert!(text.contains("location:   crates/ui/src/app.rs:100:7"));
        assert!(text.contains("assertion failed"));
        assert!(text.contains("backtrace line"));
        assert!(text.starts_with("CapRust crash report"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_keeps_only_retention() {
        let dir = scratch("prune-count");
        // Write RETENTION + 5 files.
        for i in 1..=(RETENTION as u64 + 5) {
            write_report(&dir, i, "v", "l", "m", "b").expect("write");
        }
        prune(&dir);
        let count = fs::read_dir(&dir).map(|it| it.count()).unwrap_or(0);
        assert_eq!(count, RETENTION);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_removes_oldest_first() {
        let dir = scratch("prune-order");
        // Write RETENTION + 1 files with shuffled stamps; the smallest
        // should be dropped, the rest survive.
        for i in 1..=(RETENTION as u64 + 1) {
            write_report(&dir, i * 10, "v", "l", "m", "b").expect("write");
        }
        prune(&dir);
        let oldest = dir.join("crash-10.log");
        assert!(!oldest.exists(), "oldest report should be pruned");
        let newest = dir.join(format!("crash-{}.log", (RETENTION as u64 + 1) * 10));
        assert!(newest.exists(), "newest report must survive");
        let _ = fs::remove_dir_all(&dir);
    }
}
