//! Piper TTS integration via subprocess.
//!
//! Piper is shipped as a self-contained CLI binary per platform. We
//! download it on first narration request (see `ensure_binary`), extract
//! it into `<models_dir>/piper/`, and invoke it as a subprocess for each
//! synthesis.
//!
//! Why subprocess and not an ONNX binding:
//! - Same philosophy as ffmpeg (§10): no libav*, no ONNX runtime, no
//!   version-hell. The binary is a runtime dependency the user accepts
//!   by triggering narration, exactly like ffmpeg is for import/export.
//! - Prebuilt Piper binaries are small (~25 MB per platform) and work
//!   on Windows / Linux / macOS without any local build.
//! - A native ONNX binding would add ~30 MB to every build and lock us
//!   to a specific ort version.
//!
//! Input format: Piper reads the text on stdin and writes a RIFF WAV to
//! the path given by `--output_file`. Model is `<voice>.onnx`; the
//! sibling `<voice>.onnx.json` must sit next to it (it is downloaded by
//! F5's model downloader alongside the ONNX weights).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, Context, Result};

/// Asset suffix for the current OS. Used to pick the right Piper release
/// archive from the GitHub release (see PIPER_BASE_URL).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PiperPlatform {
    LinuxX86_64,
    WindowsX64,
    MacosAarch64,
    MacosX64,
    Unsupported,
}

impl PiperPlatform {
    pub fn current() -> Self {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            return Self::LinuxX86_64;
        }
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        {
            return Self::WindowsX64;
        }
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            return Self::MacosAarch64;
        }
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        {
            return Self::MacosX64;
        }
        #[allow(unreachable_code)]
        Self::Unsupported
    }

    /// Archive filename in the Piper GitHub release.
    pub fn archive_name(self) -> Option<&'static str> {
        match self {
            Self::LinuxX86_64 => Some("piper_linux_x86_64.tar.gz"),
            Self::WindowsX64 => Some("piper_windows_amd64.zip"),
            Self::MacosAarch64 => Some("piper_macos_aarch64.tar.gz"),
            Self::MacosX64 => Some("piper_macos_x64.tar.gz"),
            Self::Unsupported => None,
        }
    }

    /// Executable name inside the extracted `piper/` directory.
    pub fn exe_name(self) -> Option<&'static str> {
        match self {
            Self::WindowsX64 => Some("piper.exe"),
            Self::LinuxX86_64 | Self::MacosAarch64 | Self::MacosX64 => Some("piper"),
            Self::Unsupported => None,
        }
    }
}

/// Base URL for Piper release archives. Pinned to a specific tag so
/// future GitHub deletion of old releases does not break downloads.
pub const PIPER_RELEASE_TAG: &str = "2023.11.14-2";
pub const PIPER_BASE_URL: &str = "https://github.com/rhasspy/piper/releases/download/2023.11.14-2";

/// Where the Piper binary lives: `<models_dir>/piper/piper[.exe]`.
pub fn binary_path(models_dir: &Path) -> PathBuf {
    let exe = PiperPlatform::current().exe_name().unwrap_or("piper");
    models_dir.join("piper").join(exe)
}

/// Ensure the Piper binary exists on disk; download + extract if not.
/// Idempotent and safe to call repeatedly.
pub fn ensure_binary(models_dir: &Path) -> Result<PathBuf> {
    let dest = binary_path(models_dir);
    if dest.is_file() {
        return Ok(dest);
    }

    let platform = PiperPlatform::current();
    let archive_name = platform
        .archive_name()
        .ok_or_else(|| anyhow!("unsupported platform for Piper: {platform:?}"))?;
    let url = format!("{PIPER_BASE_URL}/{archive_name}");

    let piper_dir = models_dir.join("piper");
    std::fs::create_dir_all(&piper_dir)
        .with_context(|| format!("create {}", piper_dir.display()))?;

    let archive_path = piper_dir.join(archive_name);
    tracing::info!("piper: downloading {url} -> {}", archive_path.display());

    let resp = ureq::get(&url)
        .call()
        .with_context(|| format!("GET {url}"))?;
    let mut reader = resp.into_reader();
    let mut out = std::fs::File::create(&archive_path)
        .with_context(|| format!("create {}", archive_path.display()))?;
    std::io::copy(&mut reader, &mut out).context("copy archive")?;
    out.sync_all().context("fsync archive")?;
    drop(out);

    // Extract in a temp subdir so we don't have to reason about existing
    // files inside piper_dir. The archives contain a top-level `piper/`
    // directory, which we then move up one level.
    let extract_dir = piper_dir.join("_extract");
    let _ = std::fs::remove_dir_all(&extract_dir);
    std::fs::create_dir_all(&extract_dir).context("create extract dir")?;

    if archive_name.ends_with(".tar.gz") {
        extract_tar_gz(&archive_path, &extract_dir)?;
    } else if archive_name.ends_with(".zip") {
        extract_zip(&archive_path, &extract_dir)?;
    } else {
        return Err(anyhow!("unknown Piper archive format: {archive_name}"));
    }

    // Archive contents: <extract_dir>/piper/piper[.exe] + libs. Move
    // everything from <extract_dir>/piper/ up to <models_dir>/piper/.
    let inner = extract_dir.join("piper");
    if !inner.is_dir() {
        return Err(anyhow!(
            "piper archive did not contain a piper/ directory at {}",
            inner.display()
        ));
    }
    move_dir_contents(&inner, &piper_dir)?;
    let _ = std::fs::remove_dir_all(&extract_dir);
    let _ = std::fs::remove_file(&archive_path);

    // Set executable bit on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&dest) {
            let mut perm = meta.permissions();
            perm.set_mode(0o755);
            let _ = std::fs::set_permissions(&dest, perm);
        }
    }

    if !dest.is_file() {
        return Err(anyhow!(
            "piper binary still missing after extract: {}",
            dest.display()
        ));
    }
    tracing::info!("piper: ready at {}", dest.display());
    Ok(dest)
}

fn extract_tar_gz(archive: &Path, dest_dir: &Path) -> Result<()> {
    let file = std::fs::File::open(archive).context("open tar.gz")?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut ar = tar::Archive::new(gz);
    ar.unpack(dest_dir).context("unpack tar.gz")?;
    Ok(())
}

fn extract_zip(archive: &Path, dest_dir: &Path) -> Result<()> {
    let file = std::fs::File::open(archive).context("open zip")?;
    let mut zip = zip::ZipArchive::new(file).context("read zip")?;
    zip.extract(dest_dir).context("unpack zip")?;
    Ok(())
}

/// Move every entry from `src` into `dst` (creating dst if needed).
/// Uses copy+remove rather than rename to survive cross-device moves.
fn move_dir_contents(src: &Path, dst: &Path) -> Result<()> {
    for entry in std::fs::read_dir(src).context("read src dir")? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if to.exists() {
            let _ = std::fs::remove_file(&to);
        }
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
            let _ = std::fs::remove_dir_all(&from);
        } else {
            std::fs::copy(&from, &to)
                .with_context(|| format!("copy {} -> {}", from.display(), to.display()))?;
            let _ = std::fs::remove_file(&from);
        }
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Synthesize `text` to a WAV using the given Piper binary + ONNX voice.
/// The WAV is written to `output_path`. The `<voice>.onnx.json` config
/// is expected to sit alongside the ONNX file.
///
/// Blocks until Piper exits. Typical latency: 50-300 ms for short text.
pub fn synthesize(
    piper_bin: &Path,
    voice_onnx: &Path,
    text: &str,
    output_path: &Path,
) -> Result<()> {
    if !voice_onnx.is_file() {
        return Err(anyhow!(
            "piper voice model not found: {}",
            voice_onnx.display()
        ));
    }
    let config = voice_onnx.with_extension("onnx.json");
    if !config.is_file() {
        return Err(anyhow!(
            "piper voice config not found: {} (expected next to the ONNX)",
            config.display()
        ));
    }
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }

    let mut child = Command::new(piper_bin)
        .arg("--model")
        .arg(voice_onnx)
        .arg("--output_file")
        .arg(output_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn {}", piper_bin.display()))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .context("write text to piper stdin")?;
    }

    let output = child.wait_with_output().context("wait piper")?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("piper failed ({}): {}", output.status, err.trim()));
    }

    if !output_path.is_file() {
        return Err(anyhow!(
            "piper reported success but {} is missing",
            output_path.display()
        ));
    }
    tracing::info!(
        "piper: synthesized {} chars -> {} ({} bytes)",
        text.len(),
        output_path.display(),
        std::fs::metadata(output_path).map(|m| m.len()).unwrap_or(0)
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_archive_names_are_consistent() {
        for p in [
            PiperPlatform::LinuxX86_64,
            PiperPlatform::WindowsX64,
            PiperPlatform::MacosAarch64,
            PiperPlatform::MacosX64,
        ] {
            assert!(p.archive_name().is_some(), "{p:?} has no archive");
            assert!(p.exe_name().is_some(), "{p:?} has no exe");
        }
        assert!(PiperPlatform::Unsupported.archive_name().is_none());
    }

    #[test]
    fn binary_path_ends_with_expected_name() {
        let dir = std::path::Path::new("/tmp/caprust-piper-test");
        let p = binary_path(dir);
        assert!(p.starts_with(dir));
        let file = p.file_name().unwrap().to_string_lossy();
        assert!(file == "piper" || file == "piper.exe");
    }
}
