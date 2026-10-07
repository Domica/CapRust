//! Free-space queries for pre-flight guards: ffmpeg renders minutes
//! of output before failing at mux/trailer time when the disk is
//! full, which reads as "blocked preview + CPU churn". Checking up
//! front turns that into one clear toast.

/// Minimum free bytes to start a preview render (temp PCM + frames).
pub const PREVIEW_MIN_FREE_BYTES: u64 = 256 * 1024 * 1024;
/// Minimum free bytes to start an export (full output file).
pub const EXPORT_MIN_FREE_BYTES: u64 = 1024 * 1024 * 1024;
/// Minimum free bytes to start an audio-cache render.
pub const AUDIO_CACHE_MIN_FREE_BYTES: u64 = 256 * 1024 * 1024;

/// Free bytes on the volume containing `dir`. `None` when unknown
/// (non-Windows, or the OS call fails) — callers fail OPEN then,
/// i.e. today's behaviour.
pub fn free_bytes(dir: &std::path::Path) -> Option<u64> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let wide: Vec<u16> = dir
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut free: u64 = 0;
        unsafe {
            GetDiskFreeSpaceExW(
                PCWSTR(wide.as_ptr()),
                Some(&mut free as *mut u64),
                None,
                None,
            )
            .ok()?;
        }
        Some(free)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = dir;
        None
    }
}
