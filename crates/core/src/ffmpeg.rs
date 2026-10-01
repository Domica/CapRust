//! Managed FFmpeg downloader.
//!
//! CapRust never links libav*; it invokes `ffmpeg.exe` and `ffprobe.exe`
//! as subprocesses. If the user has no FFmpeg on PATH and no override in
//! Settings, the app can fetch a BtbN static build into a per-user folder
//! and point `detect_ffmpeg` at it.
//!
//! Resolution order (highest priority first):
//!   1. AppSettings.ffmpeg_path / ffprobe_path (explicit user override)
//!   2. PATH (system-wide install)
//!   3. `<managed_ffmpeg_dir>/bin/{ffmpeg,ffprobe}.exe` (downloaded by us)

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use anyhow::{anyhow, Context, Result};

use crate::settings::AppSettings;

/// BtbN rolling build. Static GPL binary, everything CapRust uses
/// (drawtext, xfade, atempo, sidechaincompress, afade, maskedmerge)
/// is in it. The "latest" tag rotates in place, so we cannot pin a
/// hash in source; instead the archive is checked against the
/// `checksums.sha256` published next to it BEFORE anything is extracted
/// or executed. This catches corruption and a tampered archive, but not
/// a compromised release (checksums come from the same origin).
pub const FFMPEG_URL: &str =
    "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip";

/// Progress events the UI poll on.
#[derive(Debug, Clone)]
pub enum FfmpegDownloadEvent {
    Progress { done: u64, total: Option<u64> },
    Extracting,
    Done,
    Failed(String),
}

/// Where the auto-downloaded FFmpeg lives. Uses the user's override
/// if set, otherwise the default `%APPDATA%/CapRust/ffmpeg`.
pub fn managed_dir(settings: &AppSettings) -> PathBuf {
    if let Some(d) = settings.managed_ffmpeg_dir.as_ref() {
        if !d.trim().is_empty() {
            return PathBuf::from(d);
        }
    }
    default_managed_dir()
}

fn default_managed_dir() -> PathBuf {
    let base = std::env::var("APPDATA")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join("CapRust").join("ffmpeg")
}

/// Look for ffmpeg: user override, then PATH, then the managed dir.
pub fn find_ffmpeg(settings: &AppSettings) -> Option<PathBuf> {
    find_tool(settings.ffmpeg_path.as_deref(), "ffmpeg", settings)
}

/// Same for ffprobe.
pub fn find_ffprobe(settings: &AppSettings) -> Option<PathBuf> {
    find_tool(settings.ffprobe_path.as_deref(), "ffprobe", settings)
}

fn find_tool(override_path: Option<&str>, name: &str, settings: &AppSettings) -> Option<PathBuf> {
    if let Some(p) = override_path {
        if !p.trim().is_empty() {
            let path = PathBuf::from(p);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    if let Some(p) = which_in_path(name) {
        return Some(p);
    }
    let managed = managed_dir(settings)
        .join("bin")
        .join(format!("{name}.exe"));
    if managed.is_file() {
        return Some(managed);
    }
    None
}

fn which_in_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    let candidates: Vec<String> = if cfg!(windows) {
        vec![format!("{name}.exe"), name.to_string()]
    } else {
        vec![name.to_string()]
    };
    for dir in std::env::split_paths(&path_var) {
        for c in &candidates {
            let candidate = dir.join(c);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Run `<path> -version` and check the banner. Guards against a
/// truncated download, an HTML error page, or a replaced binary.
pub fn verify_binary(path: &Path) -> bool {
    let out = match std::process::Command::new(path).arg("-version").output() {
        Ok(o) => o,
        Err(_) => return false,
    };
    if !out.status.success() {
        return false;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.contains("ffmpeg version") || text.contains("ffprobe version")
}

/// Download + extract FFmpeg into `target_dir` on a background thread.
/// The returned receiver yields progress events until `Done` or `Failed`.
pub fn spawn_ffmpeg_download(target_dir: PathBuf, url: String) -> Receiver<FfmpegDownloadEvent> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        if let Err(e) = download_and_extract(&target_dir, &url, &tx) {
            let _ = tx.send(FfmpegDownloadEvent::Failed(e.to_string()));
        }
    });
    rx
}

fn download_and_extract(
    target_dir: &Path,
    url: &str,
    tx: &mpsc::Sender<FfmpegDownloadEvent>,
) -> Result<()> {
    fs::create_dir_all(target_dir).with_context(|| format!("create {}", target_dir.display()))?;
    let zip_path = target_dir.join("ffmpeg.zip.part");
    let _ = fs::remove_file(&zip_path);

    let resp = ureq::get(url)
        .call()
        .with_context(|| format!("GET {url}"))?;
    let total = resp
        .header("Content-Length")
        .and_then(|s| s.parse::<u64>().ok());
    let mut reader = resp.into_reader();

    let mut out =
        fs::File::create(&zip_path).with_context(|| format!("create {}", zip_path.display()))?;
    let mut buf = [0u8; 64 * 1024];
    let mut done: u64 = 0;
    loop {
        let n = reader.read(&mut buf).context("read HTTP stream")?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).context("write zip")?;
        done += n as u64;
        let _ = tx.send(FfmpegDownloadEvent::Progress { done, total });
    }
    out.sync_all().context("fsync zip")?;
    drop(out);

    let _ = tx.send(FfmpegDownloadEvent::Extracting);
    let name = url.rsplit('/').next().unwrap_or_default();
    let sums_url = format!(
        "{}/checksums.sha256",
        url.rsplit_once('/').map_or(url, |p| p.0)
    );
    let sums = ureq::get(&sums_url)
        .call()
        .with_context(|| format!("GET {sums_url}"))?
        .into_string()
        .context("read checksums")?;
    let expected =
        checksum_for(&sums, name).ok_or_else(|| anyhow!("no checksum for {name} in {sums_url}"))?;
    if let Err(e) = crate::models::download::verify_sha256_file(&zip_path, expected) {
        let _ = fs::remove_file(&zip_path);
        return Err(e.context("ffmpeg archive failed verification"));
    }

    // BtbN layout: `<prefix>/bin/{ffmpeg,ffprobe}.exe`. We only need
    // the bin/ files; the -gpl build is static, so no shared DLLs.
    let file = fs::File::open(&zip_path).context("open zip")?;
    let mut archive = zip::ZipArchive::new(file).context("read zip")?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(inner) = entry.enclosed_name() else {
            continue;
        };
        let in_bin = inner.components().any(|c| c.as_os_str() == "bin");
        if !in_bin {
            continue;
        }
        let Some(file_name) = inner.file_name() else {
            continue;
        };
        let dest = target_dir.join("bin").join(file_name);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut dst =
            fs::File::create(&dest).with_context(|| format!("create {}", dest.display()))?;
        std::io::copy(&mut entry, &mut dst)
            .with_context(|| format!("extract {}", dest.display()))?;
    }

    let _ = fs::remove_file(&zip_path);

    let ffmpeg = target_dir.join("bin").join("ffmpeg.exe");
    let ffprobe = target_dir.join("bin").join("ffprobe.exe");
    if !ffmpeg.is_file() || !verify_binary(&ffmpeg) {
        return Err(anyhow!(
            "extracted ffmpeg did not verify at {}",
            ffmpeg.display()
        ));
    }
    if !ffprobe.is_file() || !verify_binary(&ffprobe) {
        return Err(anyhow!(
            "extracted ffprobe did not verify at {}",
            ffprobe.display()
        ));
    }

    let _ = tx.send(FfmpegDownloadEvent::Done);
    Ok(())
}

/// Find `name` in `sha256sum`-style text (`<hex>  <file>`).
fn checksum_for<'a>(sums: &'a str, name: &str) -> Option<&'a str> {
    sums.lines().find_map(|l| {
        let (hash, file) = l.split_once(char::is_whitespace)?;
        (file.trim_start_matches(['*', ' ']) == name).then_some(hash)
    })
}
#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> AppSettings {
        AppSettings::default()
    }

    #[test]
    fn managed_dir_default_under_caprust() {
        let d = managed_dir(&s());
        let text = d.to_string_lossy();
        assert!(text.contains("CapRust"), "{text}");
        assert!(text.ends_with("ffmpeg"), "{text}");
    }

    #[test]
    fn managed_dir_respects_override() {
        let mut st = s();
        st.managed_ffmpeg_dir = Some("D:\\\\MyFFmpeg".into());
        assert_eq!(managed_dir(&st), PathBuf::from("D:\\\\MyFFmpeg"));
    }

    #[test]
    fn managed_dir_ignores_blank_override() {
        let mut st = s();
        st.managed_ffmpeg_dir = Some("   ".into());
        let text = managed_dir(&st).to_string_lossy().to_string();
        assert!(text.contains("CapRust"));
    }

    #[test]
    fn verify_binary_rejects_missing_file() {
        assert!(!verify_binary(Path::new("C:\\\\nope\\\\ffmpeg.exe")));
    }

    #[test]
    fn checksum_for_finds_exact_filename() {
        let sums = "aaa  ffmpeg-master-latest-win64-gpl-shared.zip\nbbb  ffmpeg-master-latest-win64-gpl.zip\n";
        assert_eq!(
            checksum_for(sums, "ffmpeg-master-latest-win64-gpl.zip"),
            Some("bbb")
        );
        assert_eq!(checksum_for(sums, "missing.zip"), None);
    }
    #[test]
    fn verify_binary_accepts_real_ffmpeg_when_present() {
        let Some(p) = which_in_path("ffmpeg") else {
            return;
        };
        assert!(verify_binary(&p));
    }
}
