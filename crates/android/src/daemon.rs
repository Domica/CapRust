//! The daemon: the engine behind a socket.
//!
//! The phone app does not call the engine directly. It starts this
//! program and talks to it over a loopback socket, in the same
//! newline-delimited JSON the desktop's MCP server speaks. Two
//! reasons for that shape and not JNI:
//!
//! - The Kotlin side gets one client, not two. The companion app
//!   already speaks this protocol against the desktop, and the editor
//!   app can speak it against the phone's own engine without a second
//!   implementation of anything.
//! - A renderer that crashes takes the daemon down, not the app. The
//!   app can say so and start it again.
//!
//! The tools are the MCP ones, unchanged: this daemon serves the same
//! `add_media`, `add_clip_to_timeline`, `list_clips` and the rest, so
//! a client cannot tell the two apart.

use anyhow::{Context, Result};
use caprust_core::commands::UndoStack;
use caprust_core::project::ProjectState;
use caprust_core::recent::RecentProject;
use caprust_mcp_core::server::{Ctx, Server, SessionControl};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use crate::{Paths, Session};

/// What the daemon needs to run.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub paths: Paths,
    /// Where the project is saved.
    pub project_path: PathBuf,
    /// The port to listen on. 0 means "any free port".
    pub port: u16,
}

impl DaemonConfig {
    pub fn new(paths: Paths, project_path: PathBuf) -> Self {
        Self {
            paths,
            project_path,
            port: 0,
        }
    }
}

/// The session the MCP tools talk to.
///
/// The preview and the export are the two things the project alone
/// cannot answer, and both need the project. The project is borrowed
/// while the tools run, so this records what was asked for and the
/// daemon does it once the borrow is released -- the same deferral the
/// desktop uses for a project swap.
#[derive(Debug)]
pub struct PhoneSession {
    /// The playhead the preview renders at.
    pub playhead_ms: u64,
    /// The export that was asked for, and where it goes.
    pub pending_export: Option<PathBuf>,
    /// The export that is running.
    pub export_rx: Option<Receiver<caprust_media_io::exporter::ExportEvent>>,
    /// How far the running export has got.
    pub export_progress: Option<f32>,
    /// The last preview frame, as a JPEG.
    pub last_frame: Option<Vec<u8>>,
    /// The width the last frame was rendered at.
    pub last_frame_w: u32,
}

impl Default for PhoneSession {
    fn default() -> Self {
        Self::new()
    }
}

impl PhoneSession {
    pub fn new() -> Self {
        Self {
            playhead_ms: 0,
            pending_export: None,
            export_rx: None,
            export_progress: None,
            last_frame: None,
            last_frame_w: 320,
        }
    }
}

impl SessionControl for PhoneSession {
    fn recent(&self) -> Vec<RecentProject> {
        Vec::new()
    }

    fn request_open(&mut self, path: &Path) -> Result<(), String> {
        let _ = path;
        Err("the phone app edits one project at a time".into())
    }

    fn preview_frame(&self, _max_w: u32) -> Result<Option<String>, String> {
        match &self.last_frame {
            Some(jpeg) => Ok(Some(base64(jpeg))),
            None => Ok(None),
        }
    }

    fn preview_status(&self) -> Option<(bool, u64)> {
        Some((false, self.playhead_ms))
    }

    fn export_progress(&self) -> Option<f32> {
        self.export_progress
    }

    fn request_export(&mut self, destination: &str) -> Result<(), String> {
        if destination.trim().is_empty() {
            return Err("the export needs a destination".into());
        }
        self.pending_export = Some(PathBuf::from(destination));
        Ok(())
    }
}

/// Base64, without pulling in a crate for it.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// The daemon: a project, an undo stack and a socket.
pub struct Daemon {
    server: Server,
    project: ProjectState,
    undo: UndoStack,
    session: PhoneSession,
    project_path: PathBuf,
    paths: Paths,
    /// The token the client has to send.
    ///
    /// The MCP protocol asks every call to carry one, and the server
    /// refuses the ones that do not. On a phone the daemon and the app
    /// are the same device on loopback, so there is nobody to pair --
    /// but the check is the server's, not the app's, so the daemon
    /// mints a token at startup and hands it to the app. It never
    /// leaves the device.
    token: String,
}

impl Daemon {
    pub fn new(config: DaemonConfig) -> Result<Self> {
        let project = ProjectState {
            aspect_ratio: caprust_core::aspect_ratio::AspectRatio::Portrait9x16,
            ..Default::default()
        };
        let token = uuid::Uuid::new_v4().to_string();
        let mut server = Server::new();
        server.load_token(Some(token.clone()));
        Ok(Self {
            server,
            project,
            undo: UndoStack::default(),
            session: PhoneSession::new(),
            project_path: config.project_path,
            paths: config.paths,
            token,
        })
    }

    /// The token the client has to send with every call.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Listen and serve, one client at a time.
    ///
    /// The port and the token are printed on stdout the moment they
    /// are known, so the app that starts this does not have to guess
    /// either: one line, `port token`.
    pub fn serve(mut self, port: u16) -> Result<()> {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .with_context(|| format!("listen on 127.0.0.1:{port}"))?;
        let actual = listener.local_addr()?.port();
        println!("{} {}", actual, self.token);
        use std::io::Write as _;
        std::io::stdout().flush().ok();

        for stream in listener.incoming() {
            let stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("the daemon could not accept a connection: {e}");
                    continue;
                }
            };
            if let Err(e) = self.serve_one(stream) {
                tracing::warn!("the daemon closed a connection: {e}");
            }
        }
        Ok(())
    }

    /// Serve one client until it goes away.
    fn serve_one(&mut self, stream: std::net::TcpStream) -> Result<()> {
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut writer = stream;
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => return Ok(()),
                Ok(_) => {}
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let reply = self.handle(trimmed);
            writer.write_all(reply.as_bytes())?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
    }

    /// Answer one request.
    pub fn handle(&mut self, line: &str) -> String {
        // The preview is answered here, not by the tool: it needs the
        // project, and the project is borrowed while the tools run.
        if let Some(reply) = self.answer_preview(line) {
            return reply;
        }

        let request: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": {"code": -32700, "message": format!("bad JSON: {e}")},
                })
                .to_string()
            }
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);

        let reply = {
            let mut ctx = Ctx {
                project: &mut self.project,
                undo: &mut self.undo,
                project_path: &self.project_path,
                session: Some(&mut self.session),
            };
            self.server.handle_line(line, &mut ctx)
        };

        let reply = match reply {
            Some(r) => r,
            None => return String::new(),
        };
        // Anything the session recorded is done now, with the project
        // free.
        self.run_pending();
        let _ = id;
        reply
    }

    /// The preview, answered with the project to ourselves.
    ///
    /// Returns the result value, which is wrapped in the content array
    /// the protocol uses.
    fn answer_preview(&mut self, line: &str) -> Option<Value> {
        let request: Value = serde_json::from_str(line).ok()?;
        if request.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
            return None;
        }
        let name = request
            .get("params")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str());
        if name != Some("preview_frame") {
            return None;
        }
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let width = request
            .get("params")
            .and_then(|p| p.get("arguments"))
            .and_then(|a| a.get("width"))
            .and_then(|w| w.as_u64())
            .unwrap_or(320)
            .clamp(64, 1280) as u32;

        match self.render_preview(width) {
            Ok(Some(jpeg)) => {
                let b64 = base64(&jpeg);
                Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{"type": "text", "text": b64}],
                        "isError": false,
                    },
                }))
            }
            Ok(None) => {
                Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{"type": "text", "text": ""}],
                        "isError": false,
                    },
                }))
            }
            Err(e) => {
                Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32000, "message": format!("{e:#}")},
                }))
            }
        }
    }

    /// The preview, answered as a string the way the other answers are.
    fn answer_preview_text(&mut self, line: &str) -> Option<String> {
        Some(self.answer_preview(line)?.to_string())
    }

    /// Render the preview frame at the playhead.
    fn render_preview(&mut self, width: u32) -> Result<Option<Vec<u8>>> {
        let (w, h) = self.project.project_dimensions();
        let at = self.session.playhead_ms;
        let frame = crate::render_preview_frame(&self.project, &self.paths, w, h, at)?;
        let bytes = std::fs::read(&frame)?;
        self.session.last_frame = Some(bytes.clone());
        self.session.last_frame_w = width;
        Ok(Some(bytes))
    }

    /// Probe the media that has not been probed yet.
    ///
    /// The desktop does this in a background job. The daemon has no
    /// background, so it does it between calls: a file that arrived
    /// without a length is a clip without a length, and the export
    /// would render nothing.
    fn probe_new_media(&mut self) {
        let pending: Vec<(uuid::Uuid, String)> = self
            .project
            .media
            .items
            .iter()
            .filter(|m| !m.probe_done)
            .map(|m| (m.id, m.path.clone()))
            .collect();
        for (id, path) in pending {
            let probe = caprust_media_io::ffprobe::probe(&self.paths.ffprobe, Path::new(&path));
            let Ok(probe) = probe else { continue };
            if let Some(item) = self.project.media.items.iter_mut().find(|m| m.id == id) {
                item.duration_ms = probe.duration_ms;
                item.has_audio = probe.has_audio;
                item.probe_done = true;
            }
        }
    }

    /// Do what the session recorded: probe the new media, start the
    /// export, keep the progress fresh.
    fn run_pending(&mut self) {
        self.probe_new_media();

        if let Some(destination) = self.session.pending_export.take() {
            let (w, h) = self.project.project_dimensions();
            let engine = Session::with_render_size(self.project.clone(), self.paths.clone(), w, h);
            match engine.export(&destination) {
                Ok(rx) => {
                    self.session.export_rx = Some(rx);
                    self.session.export_progress = Some(0.0);
                }
                Err(e) => {
                    tracing::warn!("the export did not start: {e:#}");
                    self.session.export_progress = None;
                }
            }
        }

        // Drain whatever the export has to say.
        let mut done = false;
        if let Some(rx) = &self.session.export_rx {
            while let Ok(ev) = rx.try_recv() {
                match ev {
                    caprust_media_io::exporter::ExportEvent::Progress(p) => {
                        self.session.export_progress = Some(p)
                    }
                    caprust_media_io::exporter::ExportEvent::Finished { .. } => {
                        self.session.export_progress = Some(1.0);
                        done = true;
                    }
                    caprust_media_io::exporter::ExportEvent::Failed(msg) => {
                        tracing::warn!("the export failed: {msg}");
                        self.session.export_progress = None;
                        done = true;
                    }
                    caprust_media_io::exporter::ExportEvent::Log(line) => {
                        tracing::info!("ffmpeg: {line}");
                    }
                    _ => {}
                }
            }
        }
        if done {
            self.session.export_rx = None;
        }
    }
}
