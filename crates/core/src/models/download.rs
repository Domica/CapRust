//! Streaming model downloader.
//!
//! Downloads are blocking and run on a background thread; the UI reads
//! progress through a callback so it can update the Settings → Models
//! progress bar without locking the whole registry.
//!
//! Files are written to `<path>.part` first and renamed to `<path>` only
//! after the download completes and (when a SHA-256 is configured) the
//! digest matches. This makes partial downloads trivially resumable by
//! discarding the `.part` file on error and starting over.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use sha2::{Digest, Sha256};

/// Called periodically during a download.
/// `bytes_done` and `bytes_total` are best-effort: total is `None` when the
/// server does not send a Content-Length header.
pub type ProgressFn = dyn FnMut(u64, Option<u64>) + Send;

/// Download `url` to `dest`, streaming through a `.part` file.
///
/// `progress` is called after every read chunk (~64 KiB) with the running
/// byte count; passing `None` for `progress` disables callbacks.
///
/// If `expected_sha256` is non-empty, the digest of the downloaded file
/// is verified before the rename. Mismatch returns an error and leaves
/// no dest file behind (the `.part` file is removed).
pub fn download_file(
    url: &str,
    dest: &Path,
    expected_sha256: &str,
    progress: Option<&mut ProgressFn>,
) -> Result<()> {
    if url.is_empty() {
        return Err(anyhow!("model has no download URL configured"));
    }

    let part_path = with_extension_part(dest);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create models dir {}", parent.display()))?;
    }
    // Start fresh each time. No resume support yet — simpler and safe.
    let _ = fs::remove_file(&part_path);

    let resp = ureq::get(url)
        .call()
        .with_context(|| format!("GET {url}"))?;
    let total = resp
        .header("Content-Length")
        .and_then(|s| s.parse::<u64>().ok());

    let mut reader = resp.into_reader();
    let mut out =
        File::create(&part_path).with_context(|| format!("create {}", part_path.display()))?;

    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut done: u64 = 0;
    let mut progress = progress;

    loop {
        let n = reader.read(&mut buf).context("read from HTTP stream")?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).context("write to file")?;
        hasher.update(&buf[..n]);
        done += n as u64;
        if let Some(cb) = progress.as_deref_mut() {
            cb(done, total);
        }
    }
    out.sync_all().context("fsync")?;
    drop(out);

    // Optional integrity check.
    if !expected_sha256.is_empty() {
        let actual = hex_encode(hasher.finalize().as_slice());
        if !actual.eq_ignore_ascii_case(expected_sha256) {
            let _ = fs::remove_file(&part_path);
            return Err(anyhow!(
                "sha256 mismatch: expected {expected_sha256}, got {actual}"
            ));
        }
    }

    fs::rename(&part_path, dest)
        .with_context(|| format!("rename {} -> {}", part_path.display(), dest.display()))?;
    Ok(())
}

/// Delete any leftover `.part` sibling of `dest`, if it exists.
/// Call this to clean up after a cancelled or failed download.
pub fn cleanup_partial(dest: &Path) {
    let _ = fs::remove_file(with_extension_part(dest));
}

/// `<path>` -> `<path>.part`.
fn with_extension_part(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_path_appends_suffix() {
        let p = Path::new("/tmp/foo.bin");
        assert_eq!(with_extension_part(p), PathBuf::from("/tmp/foo.bin.part"));
    }

    #[test]
    fn empty_url_is_error() {
        let dest = std::env::temp_dir().join("caprust-dl-test.bin");
        let err = download_file("", &dest, "", None).unwrap_err();
        assert!(err.to_string().contains("no download URL"));
    }

    #[test]
    fn cleanup_partial_removes_file() {
        let dest = std::env::temp_dir().join("caprust-dl-cleanup.bin");
        let part = with_extension_part(&dest);
        fs::write(&part, b"x").unwrap();
        assert!(part.exists());
        cleanup_partial(&dest);
        assert!(!part.exists());
    }
}
