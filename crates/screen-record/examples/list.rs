//! List all monitors visible to DXGI.
//!
//! Run with:
//!   cargo run -p caprust-screen-record --example list

fn main() -> anyhow::Result<()> {
    let monitors = caprust_screen_record::enumerate_monitors()?;
    println!("{} monitor(s):", monitors.len());
    for m in &monitors {
        println!(
            "  [{}] {} -- {}x{} at ({}, {}){}",
            m.id,
            m.name,
            m.width,
            m.height,
            m.x,
            m.y,
            if m.primary { " (primary)" } else { "" }
        );
    }
    Ok(())
}
