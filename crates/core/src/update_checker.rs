//! GitHub Releases update checker.
//!
//! Called once per app startup. Two layers of protection against
//! hammering the API:
//!  1. `AppSettings.check_for_updates` (user opt-out).
//!  2. A cache file at `%APPDATA%/CapRust/last_update_check.json` whose
//!     presence within 24 h short-circuits the network call.
//!
//! All failures are silent. The UI shows a toast only when a strictly
//! newer version is available.
//!
//! Version comparison is done with a minimal hand-rolled semver parser:
//! `<major>.<minor>.<patch>` with an optional `-<prerelease>` suffix.
//! Prerelease versions are considered OLDER than the matching release,
//! which matches the practical intent ("don't nag about rc1 if you're
//! on 1.0.0").

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

/// Result of an update check that found something newer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    /// Version we would update to, e.g. "0.2.0".
    pub latest_version: String,
    /// The currently running version (from CARGO_PKG_VERSION).
    pub current_version: String,
    /// HTML page for the release (browser target).
    pub release_url: String,
    /// ISO 8601 publish timestamp as reported by GitHub (informational).
    pub published_at: String,
}

/// Persistent cache entry. Written after every successful check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    /// Unix seconds when we last hit the API.
    pub last_check_ts: u64,
    /// Version string we saw at that time (for logging).
    pub latest_version_seen: String,
    /// Release HTML URL we saw at that time.
    pub release_url: String,
    /// Unix seconds before which the UI should NOT show an update toast.
    /// Set by "Remind me later" (now + 7 days). None = no snooze.
    #[serde(default)]
    pub snooze_until: Option<u64>,
    /// Version the user explicitly chose to skip. When this matches the
    /// latest version seen, the UI stays silent. Any newer version
    /// clears this and shows the toast again.
    #[serde(default)]
    pub skipped_version: Option<String>,
}

/// Cache TTL: 24 h. Any successful check refreshes it.
pub const CACHE_TTL_SECS: u64 = 24 * 60 * 60;

/// Default snooze duration for "Remind me later".
pub const SNOOZE_DURATION_SECS: u64 = 7 * 24 * 60 * 60;

/// Where the cache lives. Uses %APPDATA%/CapRust on Windows,
/// ~/.caprust on Unix-like.
pub fn cache_path() -> PathBuf {
    let base = std::env::var("APPDATA")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    Path::new(&base)
        .join("CapRust")
        .join("last_update_check.json")
}

/// Read the cache if it exists and is parseable. Err only on I/O that
/// is not "file missing"; missing files return Ok(None).
pub fn read_cache() -> Result<Option<CacheEntry>> {
    let path = cache_path();
    if !path.is_file() {
        return Ok(None);
    }
    let data =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    match serde_json::from_str::<CacheEntry>(&data) {
        Ok(entry) => Ok(Some(entry)),
        Err(e) => {
            // Corrupt cache is not fatal — treat as missing.
            tracing::warn!("update_checker: ignoring corrupt cache: {e}");
            Ok(None)
        }
    }
}

/// Write the cache after a successful check. Best-effort.
pub fn write_cache(entry: &CacheEntry) -> Result<()> {
    let path = cache_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(entry).context("serialize cache")?;
    std::fs::write(&path, json).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Snooze the update toast for the default duration. Preserves any
/// other cache fields. Called by the UI when the user picks
/// "Remind me later".
pub fn snooze_default(latest_version: &str, release_url: &str) -> Result<()> {
    let mut entry = read_cache()?.unwrap_or_else(|| CacheEntry {
        last_check_ts: now_secs(),
        latest_version_seen: latest_version.to_string(),
        release_url: release_url.to_string(),
        snooze_until: None,
        skipped_version: None,
    });
    entry.latest_version_seen = latest_version.to_string();
    entry.release_url = release_url.to_string();
    entry.snooze_until = Some(now_secs() + SNOOZE_DURATION_SECS);
    write_cache(&entry)
}

/// Mark a version as skipped. The UI stays silent for that exact
/// version until a newer one appears. Called by "Skip this version".
pub fn skip_version(version: &str) -> Result<()> {
    let mut entry = read_cache()?.unwrap_or_else(|| CacheEntry {
        last_check_ts: now_secs(),
        latest_version_seen: version.to_string(),
        release_url: String::new(),
        snooze_until: None,
        skipped_version: None,
    });
    entry.skipped_version = Some(version.to_string());
    write_cache(&entry)
}

/// Should the UI show a toast for `info`, given the current cache?
/// False when the user is snoozing, or has skipped exactly this version.
pub fn should_notify(info: &UpdateInfo) -> bool {
    let Ok(Some(entry)) = read_cache() else {
        return true;
    };
    if let Some(until) = entry.snooze_until {
        if now_secs() < until {
            tracing::debug!(
                "update_checker: snoozed for {} more seconds",
                until.saturating_sub(now_secs())
            );
            return false;
        }
    }
    if let Some(skipped) = entry.skipped_version {
        if skipped == info.latest_version {
            tracing::debug!(
                "update_checker: user skipped version {}",
                info.latest_version
            );
            return false;
        }
    }
    true
}

/// Check GitHub Releases for the latest version.
///
/// Returns `Ok(Some(info))` only if a strictly newer version exists.
/// Returns `Ok(None)` if already up to date, cache is fresh, or the
/// user has disabled checks (the caller passes `enabled=false`).
/// Returns Err on network / parse failures — the UI is expected to
/// log and ignore.
pub fn check(current_version: &str, enabled: bool) -> Result<Option<UpdateInfo>> {
    if !enabled {
        return Ok(None);
    }

    // Throttle: return early if cache is fresh. Even if the cache
    // recorded a newer version previously, we don't want to re-nag more
    // than once a day. The UI can show a persistent badge separately
    // if it wants.
    if let Ok(Some(entry)) = read_cache() {
        let age = now_secs().saturating_sub(entry.last_check_ts);
        if age < CACHE_TTL_SECS {
            tracing::debug!(
                "update_checker: cache fresh ({}s old), skipping network",
                age
            );
            return Ok(None);
        }
    }

    let url = "https://api.github.com/repos/Domica/CapRust/releases/latest";
    tracing::debug!("update_checker: GET {url}");

    let resp = ureq::get(url)
        .set("User-Agent", "CapRust-update-checker")
        .set("Accept", "application/vnd.github+json")
        .call()
        .with_context(|| format!("GET {url}"))?;

    let body = resp
        .into_string()
        .context("read GitHub releases response")?;
    let parsed: serde_json::Value =
        serde_json::from_str(&body).context("parse GitHub releases JSON")?;

    let tag = parsed
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("GitHub response missing tag_name"))?;
    let html_url = parsed
        .get("html_url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("GitHub response missing html_url"))?;
    let published_at = parsed
        .get("published_at")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Tags are typically prefixed with 'v'. Strip it if present.
    let latest = tag.trim_start_matches('v').to_string();

    // Always record the check time, even if we are up to date — the
    // throttle depends on it.
    let _ = write_cache(&CacheEntry {
        last_check_ts: now_secs(),
        latest_version_seen: latest.clone(),
        release_url: html_url.to_string(),
        // Fresh check overwrites ts but preserves any existing snooze /
        // skip. Read what was there before and carry it forward.
        snooze_until: read_cache().ok().flatten().and_then(|c| c.snooze_until),
        skipped_version: read_cache().ok().flatten().and_then(|c| c.skipped_version),
    });

    match parse_version(&latest).zip(parse_version(current_version)) {
        Some((latest_v, current_v)) => {
            if latest_v > current_v {
                Ok(Some(UpdateInfo {
                    latest_version: latest,
                    current_version: current_version.to_string(),
                    release_url: html_url.to_string(),
                    published_at,
                }))
            } else {
                tracing::info!(
                    "update_checker: up to date (current={current_version}, latest={latest})"
                );
                Ok(None)
            }
        }
        None => {
            tracing::warn!(
                "update_checker: could not parse versions (current={current_version}, latest={latest})"
            );
            Ok(None)
        }
    }
}

// ---------------------------------------------------------------------------
// Minimal semver parser
// ---------------------------------------------------------------------------

/// Parsed version. Prerelease is deliberately coarse: any suffix makes
/// the version sort BEFORE the equivalent release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// 0 for stable, 1 for prerelease (sorts lower than stable).
    pub prerelease_rank: u8,
}

/// Parse `MAJOR.MINOR.PATCH[-anything]`. Returns None on malformed input.
pub fn parse_version(s: &str) -> Option<Version> {
    let (core, suffix) = match s.split_once('-') {
        Some((c, _)) => (c, true),
        None => (s, false),
    };
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(Version {
        major,
        minor,
        patch,
        // Stable (no suffix) sorts AFTER prerelease with identical numbers.
        prerelease_rank: if suffix { 0 } else { 1 },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_semver() {
        let v = parse_version("1.2.3").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
        assert_eq!(v.prerelease_rank, 1);
    }

    #[test]
    fn parse_prerelease() {
        let v = parse_version("1.2.3-rc.1").unwrap();
        assert_eq!(v.prerelease_rank, 0);
        assert!(v < parse_version("1.2.3").unwrap());
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(parse_version("").is_none());
        assert!(parse_version("1.2").is_none());
        assert!(parse_version("1.2.3.4").is_none());
        assert!(parse_version("v1.2.3").is_none()); // 'v' prefix handled by caller
        assert!(parse_version("a.b.c").is_none());
    }

    #[test]
    fn comparison_is_numeric_not_lexicographic() {
        // "10" must beat "9" even though "1" < "9" as strings.
        assert!(parse_version("1.10.0").unwrap() > parse_version("1.9.0").unwrap());
        // And the inverse must also hold numerically.
        assert!(parse_version("0.2.0").unwrap() < parse_version("0.10.0").unwrap());
    }

    #[test]
    fn cache_path_lives_under_caprust_dir() {
        let p = cache_path();
        assert!(p.to_string_lossy().contains("CapRust"));
        assert!(p.to_string_lossy().ends_with("last_update_check.json"));
    }

    #[test]
    fn cache_entry_deserializes_without_snooze_or_skip() {
        // Old-format cache from a previous install must still load.
        let json = r#"{
            "last_check_ts": 100,
            "latest_version_seen": "0.1.0",
            "release_url": "https://x"
        }"#;
        let entry: CacheEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.last_check_ts, 100);
        assert!(entry.snooze_until.is_none());
        assert!(entry.skipped_version.is_none());
    }

    #[test]
    fn cache_entry_roundtrips_snooze_and_skip() {
        let entry = CacheEntry {
            last_check_ts: 42,
            latest_version_seen: "0.2.0".into(),
            release_url: "https://x".into(),
            snooze_until: Some(1000),
            skipped_version: Some("0.2.0".into()),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: CacheEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.snooze_until, Some(1000));
        assert_eq!(back.skipped_version.as_deref(), Some("0.2.0"));
    }
}
