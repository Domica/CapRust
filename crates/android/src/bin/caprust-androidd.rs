//! The daemon that the phone app starts.
//!
//! It serves the engine over a loopback socket, in the same
//! newline-delimited JSON the desktop's MCP server speaks, so the
//! Kotlin side has one client and not two.

use anyhow::{Context, Result};
use caprust_android::daemon::{Daemon, DaemonConfig};
use caprust_android::Paths;
use std::path::PathBuf;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let mut ffmpeg = PathBuf::from("ffmpeg");
    let mut ffprobe = PathBuf::from("ffprobe");
    let mut models_dir = PathBuf::from("models");
    let mut work_dir = PathBuf::from("work");
    let mut project_path = PathBuf::from("project.caprust");
    let mut port = 0u16;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut next = || args.next().unwrap_or_default();
        match arg.as_str() {
            "--ffmpeg" => ffmpeg = PathBuf::from(next()),
            "--ffprobe" => ffprobe = PathBuf::from(next()),
            "--models" => models_dir = PathBuf::from(next()),
            "--work" => work_dir = PathBuf::from(next()),
            "--project" => project_path = PathBuf::from(next()),
            "--port" => port = next().parse().unwrap_or(0),
            other => {
                eprintln!("unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }

    let paths = Paths {
        ffmpeg,
        ffprobe,
        models_dir,
        work_dir,
    };
    std::fs::create_dir_all(&paths.work_dir)
        .with_context(|| format!("create {}", paths.work_dir.display()))?;

    let mut config = DaemonConfig::new(paths, project_path);
    config.port = port;

    let daemon = Daemon::new(config)?;
    daemon.serve(port)
}
