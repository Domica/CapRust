//! Capture one frame from the primary monitor and save it as PPM.
//!
//! Run with:
//!   cargo run -p caprust-screen-record --example capture
//!
//! Writes `frame.ppm` next to the working directory. PPM is a
//! trivial format (P6 header + RGB bytes) and needs no extra
//! dependency, so this proves the DXGI path end to end without
//! pulling `image` into the crate.

use std::fs::File;
use std::io::{BufWriter, Write};

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    let t0 = std::time::Instant::now();
    let frame = caprust_screen_record::capture::capture_one_frame(0)?;
    let elapsed = t0.elapsed();
    println!(
        "captured {}x{} BGRA ({} bytes) in {:?}",
        frame.width,
        frame.height,
        frame.pixels.len(),
        elapsed
    );

    let out = std::env::current_dir()?.join("frame.ppm");
    let f = File::create(&out)?;
    let mut w = BufWriter::new(f);
    write!(w, "P6\n{} {}\n255\n", frame.width, frame.height)?;
    for px in frame.pixels.chunks_exact(4) {
        // BGRA -> RGB
        w.write_all(&[px[2], px[1], px[0]])?;
    }
    w.flush()?;
    println!("wrote {}", out.display());
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("capture: Windows only");
}
