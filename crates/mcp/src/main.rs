mod rpc;
mod server;
mod tools;

use std::io::{self, BufRead, Write};

use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    // stdout is the JSON-RPC channel — logs go to stderr only (§35.3).
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(io::stderr)
        .init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: caprust-mcp <path.caprust>");
        std::process::exit(2);
    }
    let project_path = std::path::PathBuf::from(&args[1]);

    let project = caprust_core::project_io::load_project(&project_path)
        .with_context(|| format!("failed to load {}", project_path.display()))?;

    tracing::info!(path = %project_path.display(), "project loaded");

    let mut server = server::Server::new(project, project_path.clone());

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = line.context("stdin read failed")?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(resp) = server.handle_line(&line) {
            writeln!(out, "{resp}")?;
            out.flush()?;
        }
    }

    Ok(())
}
