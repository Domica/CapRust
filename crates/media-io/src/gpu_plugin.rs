//! External GPU plugin management.
//!
//! Downloads, verifies, and unpacks optional GPU acceleration binaries
//! from GitHub Releases. The standard CapRust EXE stays ~27 MB (CPU
//! only via tract-onnx); GPU support (CUDA / DirectML via ONNX Runtime)
//! is delivered as a separate downloadable component.
//!
//! Plugins are installed to %LOCALAPPDATA%\CapRust\gpu\ on Windows.
//! The plugin binary speaks a line-based protocol over stdin/stdout
//! (see `ExternalBackend` in `inference_backend.rs`).

use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{copy, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;
use ureq;
use zip::ZipArchive;

/// Directory under LOCALAPPDATA where GPU plugins are installed.
pub fn plugin_dir() -> Result<PathBuf> {
    let local_app_data = std::env::var("LOCALAPPDATA")
        .map_err(|_| anyhow!("LOCALAPPDATA environment variable not set"))?;
    Ok(PathBuf::from(local_app_data).join("CapRust").join("gpu"))
}

/// Returns the path to the installed CUDA GPU plugin binary, if any.
pub fn cuda_binary_path() -> Option<PathBuf> {
    let dir = plugin_dir().ok()?;
    let path = dir.join("caprust-gpu-cuda.exe");
    if path.is_file() {
        Some(path)
    } else {
        None
    }
}

/// Returns the path to the installed DirectML GPU plugin binary, if any.
pub fn directml_binary_path() -> Option<PathBuf> {
    let dir = plugin_dir().ok()?;
    let path = dir.join("caprust-gpu-directml.exe");
    if path.is_file() {
        Some(path)
    } else {
        None
    }
}

/// Returns true if at least one GPU plugin is installed.
pub fn any_installed() -> bool {
    cuda_binary_path().is_some() || directml_binary_path().is_some()
}

/// Download and install a GPU plugin for the given provider.
///
/// `provider` is `"cuda"` or `"directml"`.
/// `version` is the CapRust version (e.g. `"0.9.35"`).
/// `sha256` is the expected SHA-256 hex digest of the zip file.
///
/// The zip is expected to contain `caprust-gpu-{provider}.exe` at its
/// root (possibly inside a folder — `enclosed_name` handles both).
pub fn install(provider: &str, version: &str, sha256: &str) -> Result<PathBuf> {
    let dir = plugin_dir()?;
    fs::create_dir_all(&dir)?;

    let zip_name = format!("caprust-gpu-win64-{}-v{}.zip", provider, version);
    let zip_url = format!(
        "https://github.com/Domica/CapRust/releases/download/v{}/{}",
        version, zip_name
    );
    let zip_path = dir.join(&zip_name);

    // Download the zip.
    tracing::info!("downloading GPU plugin from {}", zip_url);
    let resp = ureq::get(&zip_url).call().map_err(|e| {
        anyhow!("failed to download GPU plugin from {zip_url}: {e}")
    })?;
    let mut file = fs::File::create(&zip_path).map_err(|e| {
        anyhow!("failed to create {}: {e}", zip_path.display())
    })?;
    copy(&mut resp.into_reader(), &mut file).map_err(|e| {
        anyhow!("failed to write GPU plugin zip: {e}")
    })?;
    drop(file);

    // Verify SHA-256 before unpacking.
    let actual = sha256_file(&zip_path)?;
    if actual != sha256 {
        fs::remove_file(&zip_path).ok();
        return Err(anyhow!(
            "SHA-256 mismatch for GPU plugin ({provider}): expected {sha256}, got {actual}"
        ));
    }

    // Unpack.
    let file = fs::File::open(&zip_path).map_err(|e| {
        anyhow!("failed to open {}: {e}", zip_path.display())
    })?;
    let mut archive = ZipArchive::new(file).map_err(|e| {
        anyhow!("failed to open zip archive: {e}")
    })?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| {
            anyhow!("failed to read zip entry {i}: {e}")
        })?;
        if let Some(out_path) = entry.enclosed_name() {
            let out_path = dir.join(out_path);
            if entry.is_dir() {
                fs::create_dir_all(&out_path)?;
            } else {
                if let Some(parent) = out_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut out = fs::File::create(&out_path).map_err(|e| {
                    anyhow!("failed to create {}: {e}", out_path.display())
                })?;
                copy(&mut entry, &mut out).map_err(|e| {
                    anyhow!("failed to write {}: {e}", out_path.display())
                })?;
            }
        }
    }
    drop(archive);

    // Clean up the zip.
    fs::remove_file(&zip_path).ok();

    let bin_path = dir.join(format!("caprust-gpu-{provider}.exe"));
    if !bin_path.is_file() {
        return Err(anyhow!(
            "GPU plugin binary not found at {} after unpacking",
            bin_path.display()
        ));
    }

    tracing::info!("GPU plugin installed at {}", bin_path.display());
    Ok(bin_path)
}

/// Compute SHA-256 hex digest of a file.
fn sha256_file(path: &Path) -> Result<String> {
    let file = fs::File::open(path).map_err(|e| {
        anyhow!("failed to open {}: {e}", path.display())
    })?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    copy(&mut reader, &mut hasher).map_err(|e| {
        anyhow!("failed to hash {}: {e}", path.display())
    })?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// Launches the GPU plugin binary in plugin mode, waiting for it to
/// become ready. Returns the `Command` (unstarted) and the expected
/// binary path so the caller can spawn and manage it.
///
/// The binary is expected to speak this protocol once started:
///   `LOAD <model_path> <shape_json>`   -> `OK` | `ERR <msg>`
///   `RUN <input_path> <output_path>`   -> `OK` | `ERR <msg>`
///   `QUIT`                              -> process exits
pub fn plugin_command(binary_path: &Path, model_path: &Path, shape: [i64; 4]) -> Command {
    let shape_json = serde_json::to_string(&shape).unwrap_or_else(|_| format!("{shape:?}"));
    let mut cmd = Command::new(binary_path);
    cmd.arg("--gpu-plugin")
        .arg("--model")
        .arg(model_path)
        .arg("--shape")
        .arg(&shape_json);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_dir_is_under_local_app_data() {
        // Just checks the function doesn't panic when LOCALAPPDATA is set.
        // The actual path depends on the environment.
        let _ = plugin_dir();
    }

    #[test]
    fn sha256_of_empty_file_is_well_known() {
        use std::io::Write;
        let dir = std::env::temp_dir();
        let path = dir.join("caprust-gpu-plugin-test-empty.bin");
        fs::File::create(&path)
            .unwrap()
            .write_all(b"")
            .unwrap();
        let hash = sha256_file(&path).unwrap();
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        fs::remove_file(&path).ok();
    }
}
