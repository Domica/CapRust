//! Windows screen recording (Faza Q).
//!
//! This crate is isolated from `core` and `media-io` so the
//! portability rule in DIRECTIVES 23.1 stays intact: neither
//! crate grows a `std::process::Command` or a platform-specific
//! dependency. UI code calls into this crate only under
//! `#[cfg(windows)]`.
//!
//! PR 1 scope: enumerate monitors through DXGI. Capture and
//! encoding land in follow-up PRs.

use anyhow::Result;

#[cfg(windows)]
mod windows_impl;

#[cfg(windows)]
pub mod capture;
#[cfg(windows)]
pub mod record;

/// One monitor as reported by the OS. Coordinates are the
/// desktop-relative top-left corner; `width` / `height` are
/// the current pixel resolution in that coordinate space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorInfo {
    pub id: usize,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// True for the primary monitor. UI can pre-select it.
    pub primary: bool,
}

#[cfg(windows)]
pub fn enumerate_monitors() -> Result<Vec<MonitorInfo>> {
    windows_impl::enumerate_monitors()
}

#[cfg(not(windows))]
pub fn enumerate_monitors() -> Result<Vec<MonitorInfo>> {
    anyhow::bail!("screen recording is only available on Windows")
}

/// Spawn helper: same as `Command::new`, but on Windows
/// release builds the child is spawned with CREATE_NO_WINDOW so
/// no console window appears. Used by `record.rs` when spawning
/// the ffmpeg encoder.
#[cfg(windows)]
pub fn silent_command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    let cmd = std::process::Command::new(program);
    #[cfg(not(debug_assertions))]
    let cmd = {
        use std::os::windows::process::CommandExt;
        let mut cmd = cmd;
        cmd.creation_flags(0x0800_0000);
        cmd
    };
    cmd
}
