//! Embed the app icon into the Windows executable metadata so
//! Explorer, taskbar pins and file properties show it. No-op on
//! other platforms.

fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        // Keep a sensible manifest so future code can add DPI
        // awareness / Windows 11 rounded corners without another
        // build script change.
        res.set("FileDescription", "CapRust");
        res.set("ProductName", "CapRust");
        if let Err(e) = res.compile() {
            // Do not fail the build on icon embedding; log and move on.
            eprintln!("build.rs: winres failed: {e}");
        }
    }
}
