//! Spawn helper: on Windows, hide the console window that
//! `Command::new` would otherwise pop up. Without this, every
//! ffmpeg / ffprobe invocation flashes a black cmd window and
//! steals focus from the editor.
//!
//! Debug builds keep the window (devs want to see ffmpeg stderr).
//! Release builds hide it.

use std::ffi::OsStr;
use std::process::Command;

/// Same as `Command::new(program)`, but on Windows release builds
/// the child is spawned with `CREATE_NO_WINDOW` so no console
/// window appears.
///
/// Use this everywhere a subprocess is spawned from `media-io`.
pub fn silent_command(program: impl AsRef<OsStr>) -> Command {
    let cmd = Command::new(program);
    #[cfg(all(windows, not(debug_assertions)))]
    let cmd = {
        use std::os::windows::process::CommandExt;
        let mut cmd = cmd;
        // CREATE_NO_WINDOW = 0x08000000
        cmd.creation_flags(0x0800_0000);
        cmd
    };
    cmd
}
