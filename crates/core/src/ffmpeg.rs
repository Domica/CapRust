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
///
/// The build is platform-specific: Windows ships a `.zip` of `.exe`
/// files, Linux ships a `.tar.xz` of extensionless binaries. Asking for
/// the win64 zip on Linux gets a pair of files the kernel cannot exec,
/// which used to read as "ffmpeg installed" until a render was tried.
pub const FFMPEG_URL_WIN: &str =
    "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip";
pub const FFMPEG_URL_LINUX: &str =
    "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz";

/// The archive for this OS. `ffmpeg.exe` on Windows, `ffmpeg` on Linux.
pub fn ffmpeg_url() -> &'static str {
    if cfg!(windows) {
        FFMPEG_URL_WIN
    } else {
        FFMPEG_URL_LINUX
    }
}

/// A tool's filename inside the managed dir. Windows needs the `.exe`
/// suffix to run; Linux runs an extensionless file with the exec bit.
pub fn tool_file_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

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
    #[cfg(windows)]
    {
        let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
        PathBuf::from(base).join("CapRust").join("ffmpeg")
    }
    // Linux convention is XDG data, not a folder in $HOME.
    #[cfg(not(windows))]
    {
        let base = std::env::var("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
                std::path::PathBuf::from(home).join(".local").join("share")
            });
        base.join("caprust").join("ffmpeg")
    }
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
    let managed = managed_dir(settings).join("bin").join(tool_file_name(name));
    // Run `-version` rather than trusting the file's existence: a
    // leftover binary from the wrong OS, or a half-extracted archive,
    // reads as "installed" otherwise, and every render fails later.
    if managed.is_file() && verify_binary(&managed) {
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

    // BtbN layout: `<prefix>/bin/{ffmpeg,ffprobe}`. We only need the
    // bin/ files; the -gpl build is static, so no shared libraries.
    // Windows ships a zip of .exe, Linux a tar.xz of extensionless
    // binaries with the exec bit set.
    if url.ends_with(".zip") {
        extract_zip_bin(&zip_path, target_dir)?;
    } else {
        extract_tar_xz_bin(&zip_path, target_dir)?;
    }

    let _ = fs::remove_file(&zip_path);

    let ffmpeg = target_dir.join("bin").join(tool_file_name("ffmpeg"));
    let ffprobe = target_dir.join("bin").join(tool_file_name("ffprobe"));
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

/// Pull the `bin/` members out of a zip into `target_dir/bin/`.
fn extract_zip_bin(zip_path: &Path, target_dir: &Path) -> Result<()> {
    let file = fs::File::open(zip_path).context("open zip")?;
    let mut archive = zip::ZipArchive::new(file).context("read zip")?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(inner) = entry.enclosed_name() else {
            continue;
        };
        if !inner.components().any(|c| c.as_os_str() == "bin") {
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
    Ok(())
}

/// Pull the `bin/` members out of a `.tar.xz` into `target_dir/bin/`,
/// setting the exec bit: a Linux binary without it cannot be run, and
/// the verify step below would then report a bad download.
///
/// The xz layer is decompressed to a temp file first: `lzma_rs` takes a
/// `BufRead` and hands back bytes, and feeding that straight into a tar
/// reader would need a streaming adapter the crate does not ship.
fn extract_tar_xz_bin(archive_path: &Path, target_dir: &Path) -> Result<()> {
    fs::create_dir_all(target_dir).with_context(|| format!("create {}", target_dir.display()))?;
    let xz = fs::File::open(archive_path).context("open tar.xz")?;
    let tar_path = target_dir.join("ffmpeg.tar.part");
    let tar_out =
        fs::File::create(&tar_path).with_context(|| format!("create {}", tar_path.display()))?;
    lzma_rs::xz_decompress(
        &mut std::io::BufReader::new(xz),
        &mut std::io::BufWriter::new(tar_out),
    )
    .context("decompress xz")?;

    let tar_file = fs::File::open(&tar_path).context("open tar")?;
    let mut archive = tar::Archive::new(tar_file);
    for entry in archive.entries().context("read tar")? {
        let mut entry = entry.context("read tar entry")?;
        let inner = entry.path().context("tar entry path")?.into_owned();
        if !inner.components().any(|c| c.as_os_str() == "bin") {
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
        drop(dst);
        // Preserve the mode the archive carried, exec bit included.
        // Without it the verify step below reports a bad download.
        #[cfg(unix)]
        {
            if let Ok(mode) = entry.header().mode() {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&dest, fs::Permissions::from_mode(mode));
            }
        }
    }
    let _ = fs::remove_file(&tar_path);
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
        // Windows: %APPDATA%\CapRust\ffmpeg; Linux: XDG data dir.
        let brand = if cfg!(windows) { "CapRust" } else { "caprust" };
        assert!(text.contains(brand), "{text}");
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
        let brand = if cfg!(windows) { "CapRust" } else { "caprust" };
        assert!(text.contains(brand), "{text}");
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

    #[test]
    fn url_and_tool_name_follow_the_os() {
        let url = ffmpeg_url();
        if cfg!(windows) {
            assert!(url.ends_with(".zip"), "{url}");
            assert_eq!(tool_file_name("ffmpeg"), "ffmpeg.exe");
        } else {
            assert!(url.ends_with(".tar.xz"), "{url}");
            assert_eq!(tool_file_name("ffmpeg"), "ffmpeg");
        }
    }

    /// Build a `.tar.xz` the way BtbN does -- a `bin/` folder of
    /// extensionless binaries with the exec bit -- run the real
    /// extractor on it, and check the files land runnable. The Linux
    /// download path was verified by inspection before; a test keeps
    /// it from regressing back to a Windows-only zip.
    #[test]
    fn extract_tar_xz_pulls_bin_and_keeps_exec_bit() {
        let dir = std::env::temp_dir().join(format!("caprust-tarxz-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");

        // Build the tar in memory: `<prefix>/bin/{ffmpeg,ffprobe}`.
        let tar_path = dir.join("build.tar");
        {
            let out = fs::File::create(&tar_path).expect("tar");
            let mut builder = tar::Builder::new(out);
            for name in ["ffmpeg", "ffprobe"] {
                let path = format!("ffmpeg-master-latest-linux64-gpl/bin/{name}");
                let mut header = tar::Header::new_gnu();
                header.set_size(5);
                header.set_mode(0o755);
                header.set_cksum();
                builder
                    .append_data(&mut header, path, &b"\x7fELF\n"[..])
                    .expect("append");
            }
            builder.finish().expect("finish tar");
        }
        // Compress to xz, so the extractor's decompress step is exercised.
        let xz_path = dir.join("build.tar.xz");
        {
            let input = fs::File::open(&tar_path).expect("read tar");
            let output = fs::File::create(&xz_path).expect("write xz");
            lzma_rs::xz_compress(
                &mut std::io::BufReader::new(input),
                &mut std::io::BufWriter::new(output),
            )
            .expect("xz compress");
        }

        let target = dir.join("out");
        extract_tar_xz_bin(&xz_path, &target).expect("extract");

        // The extractor keeps the archive's own names -- the URL is
        // picked per-OS, so a Linux tar only ever reaches a Linux box.
        let ffmpeg = target.join("bin").join("ffmpeg");
        let ffprobe = target.join("bin").join("ffprobe");
        assert!(ffmpeg.is_file(), "missing {}", ffmpeg.display());
        assert!(ffprobe.is_file(), "missing {}", ffprobe.display());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&ffmpeg).expect("meta").permissions().mode();
            assert_ne!(mode & 0o111, 0, "exec bit lost: mode {mode:o}");
        }

        let _ = fs::remove_dir_all(&dir);
    }
}
