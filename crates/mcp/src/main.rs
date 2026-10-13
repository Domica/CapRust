//! Thin binary over `caprust_mcp_core`. All logic lives in the lib so
//! the app can drive the same tool surface against a live session.

use anyhow::{Context, Result};
use std::io::{self, BufWriter};
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

use caprust_mcp_core::{
    server,
    transport::{self, Transport},
};

/// How this process was asked to talk to clients.
struct Options {
    project_path: PathBuf,
    mode: Mode,
}

enum Mode {
    /// Newline-delimited JSON on stdin/stdout. What AI agents expect.
    Stdio,
    /// A socket. `--bind 0.0.0.0` to reach it from a phone.
    Tcp { bind: String, port: u16 },
}

const USAGE: &str = "\
caprust-mcp — JSON-RPC control surface for a CapRust project

USAGE:
    caprust-mcp <path.caprust>                    stdio (AI agents)
    caprust-mcp --tcp <path.caprust>              tcp on 127.0.0.1, OS-chosen port
    caprust-mcp --bind <addr> --port <n> <path>   tcp on a specific address

TCP binds to loopback by default. Opening a port to the network
exposes project editing to anyone who can reach it, so use an explicit
--bind to do it deliberately.
";

fn parse_args() -> Result<Options> {
    let mut path: Option<String> = None;
    let mut tcp = false;
    let mut bind: Option<String> = None;
    let mut port: Option<u16> = None;

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--tcp" => tcp = true,
            "--bind" => bind = Some(it.next().context("--bind needs an address")?),
            "--port" => {
                let raw = it.next().context("--port needs a number")?;
                port = Some(raw.parse().with_context(|| format!("bad port {raw}"))?);
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            other if other.starts_with('-') => anyhow::bail!("unknown flag {other}\n\n{USAGE}"),
            other => path = Some(other.to_string()),
        }
    }

    let Some(project_path) = path else {
        anyhow::bail!("no project file given\n\n{USAGE}");
    };
    Ok(Options {
        project_path: PathBuf::from(project_path),
        mode: if tcp {
            Mode::Tcp {
                bind: bind.unwrap_or_else(|| "127.0.0.1".to_string()),
                port: port.unwrap_or(0),
            }
        } else {
            if bind.is_some() || port.is_some() {
                anyhow::bail!("--bind/--port need --tcp\n\n{USAGE}");
            }
            Mode::Stdio
        },
    })
}

fn main() -> Result<()> {
    // stdout is the JSON-RPC channel in stdio mode — logs go to stderr
    // only (§35.3). In TCP mode stdout is free, but keeping the rule
    // means switching transports never changes where logs land.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(io::stderr)
        .init();

    let opts = parse_args()?;

    let mut project = caprust_core::project_io::load_project(&opts.project_path)
        .with_context(|| format!("failed to load {}", opts.project_path.display()))?;
    tracing::info!(path = %opts.project_path.display(), "project loaded");

    // The server borrows the project rather than owning it. Here the
    // binary owns it; in the app it will be the open session's state.
    let mut server = server::Server::new();
    let mut undo = caprust_core::commands::UndoStack::new();
    let mut ctx = server::Ctx {
        project: &mut project,
        undo: &mut undo,
        project_path: &opts.project_path,
    };

    match opts.mode {
        Mode::Stdio => {
            let t =
                transport::StdioTransport::new(io::stdin().lock(), BufWriter::new(io::stdout()));
            serve(&mut server, t, &mut ctx)
        }
        Mode::Tcp { bind, port } => {
            // Loopback goes through bind_loopback so the safe default
            // is the named path, not just whatever the default string
            // happens to be today.
            let t = if bind == "127.0.0.1" {
                transport::bind_loopback(port).context("bind MCP loopback")?
            } else {
                tracing::warn!(
                    "mcp: listening on {bind} exposes project editing to the network with no authentication"
                );
                transport::TcpTransport::bind((bind.as_str(), port)).context("bind MCP TCP port")?
            };
            let local = t
                .local_port()
                .with_context(|| format!("could not read the bound port on {bind}:{port}"))?;
            // Printed on stdout because a caller scripting the launch
            // needs the port when it asked for 0, and it must not be
            // confused with a JSON-RPC frame in stdio mode.
            println!("listening on {bind}:{local}");
            serve(&mut server, t, &mut ctx)
        }
    }
}

/// Read requests until the peer goes away. Blank lines are ignored:
/// keepalives and half-written frames both show up as empty input and
/// are not JSON-RPC errors worth reporting.
fn serve(server: &mut server::Server, mut t: impl Transport, ctx: &mut server::Ctx) -> Result<()> {
    tracing::info!("mcp: serving on {}", t.describe());
    while let Some(line) = t.next_request()? {
        if line.trim().is_empty() {
            continue;
        }
        if let Some(resp) = server.handle_line(&line, ctx) {
            t.send(&resp)?;
        }
    }
    tracing::info!("mcp: transport closed, stopping");
    Ok(())
}
