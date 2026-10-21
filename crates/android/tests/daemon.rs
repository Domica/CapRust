//! The daemon is the bridge, so it is tested as one: a socket, JSON in,
//! JSON out, and the same calls the companion app makes against the
//! desktop.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;

use caprust_android::daemon::{Daemon, DaemonConfig};
use caprust_android::Paths;
use serde_json::{json, Value};

/// Whether ffmpeg is there. See the note in engine.rs.
fn have_ffmpeg() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("caprust-daemon-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn sample_video(dir: &Path) -> PathBuf {
    let out = dir.join("clip.mp4");
    let o = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=10:duration=2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&out)
        .output()
        .expect("ffmpeg runs");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    out
}

/// A client that speaks the protocol the companion speaks.
struct Client {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    next_id: i64,
    /// The token every call has to carry.
    token: String,
}

impl Client {
    fn connect(port: u16, token: &str) -> Self {
        let stream = TcpStream::connect(("127.0.0.1", port)).expect("the daemon is listening");
        let reader = BufReader::new(stream.try_clone().unwrap());
        Self {
            reader,
            writer: stream,
            next_id: 1,
            token: token.to_string(),
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        // The protocol the desktop's MCP server speaks: a tool goes
        // through `tools/call`, with its name and its arguments in the
        // params. The token rides along there too: this transport has
        // no headers, so the two transports have to stay
        // interchangeable.
        let mut params = json!({
            "name": method,
            "arguments": params,
            "token": self.token,
        });
        if let Some(map) = params.as_object_mut() {
            map.insert("token".into(), Value::String(self.token.clone()));
        }
        let request = json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": params});
        let line = serde_json::to_string(&request).unwrap();
        self.writer.write_all(line.as_bytes()).unwrap();
        self.writer.write_all(b"\n").unwrap();
        self.writer.flush().unwrap();

        let mut reply = String::new();
        self.reader.read_line(&mut reply).expect("a reply");
        let value: Value = serde_json::from_str(reply.trim()).expect("JSON in the reply");
        assert_eq!(
            value.get("id").and_then(|i| i.as_i64()),
            Some(id),
            "the reply is ours"
        );
        value
    }

    /// The tools answer with JSON *text*, wrapped the way MCP wraps
    /// it: `{"content": [{"type": "text", "text": "..."}]}`. This
    /// unwraps that and parses the text.
    fn result_json(&mut self, method: &str, params: Value) -> Value {
        let raw = self.result(method, params);
        let text = raw
            .get("content")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or_else(|| panic!("{method} did not answer with text: {raw:?}"));
        serde_json::from_str(text).unwrap_or(Value::Null)
    }

    fn result(&mut self, method: &str, params: Value) -> Value {
        let reply = self.call(method, params);
        if let Some(err) = reply.get("error") {
            panic!("{method} failed: {err}");
        }
        reply.get("result").cloned().unwrap_or(Value::Null)
    }
}

/// Start the daemon on a free port. Returns the port and the token.
fn start(dir: &Path) -> (u16, String) {
    let paths = Paths {
        ffmpeg: PathBuf::from("ffmpeg"),
        ffprobe: PathBuf::from("ffprobe"),
        models_dir: dir.join("models"),
        work_dir: dir.join("work"),
    };
    let mut config = DaemonConfig::new(paths, dir.join("project.caprust"));
    config.port = 0;

    // A free port, found by taking one and letting it go. The race
    // with another process taking it is the kind that does not happen
    // on a test machine.
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let token = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let token_for_thread = token.clone();
    std::thread::spawn(move || {
        // The daemon logs what goes wrong. Without this the test can
        // only see that the export did not finish, not why.
        let _ = tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_max_level(tracing::Level::INFO)
            .try_init();
        let daemon = Daemon::new(config).expect("the daemon starts");
        *token_for_thread.lock().unwrap() = daemon.token().to_string();
        let _ = daemon.serve(port);
    });
    // Wait for the daemon to have its token.
    for _ in 0..100 {
        let t = token.lock().unwrap().clone();
        if !t.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let t = token.lock().unwrap().clone();
    (port, t)
}

#[test]
fn the_daemon_answers_the_calls_the_companion_makes() {
    if !have_ffmpeg() {
        eprintln!("no ffmpeg on this machine: the test has nothing to say");
        return;
    }
    let dir = scratch("calls");
    let clip = sample_video(&dir);
    let (port, token) = start(&dir);
    std::thread::sleep(std::time::Duration::from_millis(300));

    let mut c = Client::connect(port, &token);

    // The project is empty to begin with.
    let clips = c.result_json("list_clips", json!({}));
    let clips: Vec<Value> = serde_json::from_value(clips).unwrap_or_default();
    assert!(clips.is_empty(), "a fresh project has no clips: {clips:?}");

    // Bring the file in.
    let media = c.result_json("add_media", json!({"path": clip.to_string_lossy()}));
    let media_id = media
        .get("media_id")
        .and_then(|i| i.as_str())
        .expect("the media has an id");
    // The tool itself does not probe; the daemon does it right after,
    // the way the desktop's background job does. Ask for the media
    // again and the length is there.
    let listed = c.result_json("list_media", json!({}));
    let listed: Vec<Value> = serde_json::from_value(listed).unwrap_or_default();
    assert_eq!(listed.len(), 1, "the media is in the library: {listed:?}");
    assert!(
        listed[0]
            .get("duration_ms")
            .and_then(|d| d.as_u64())
            .unwrap_or(0)
            > 0,
        "the media was probed: {listed:?}"
    );

    // Put it on the timeline.
    let added = c.result(
        "add_clip_to_timeline",
        json!({"media_id": media_id, "track_index": 1, "start_ms": 0}),
    );
    assert!(
        added.is_string() || added.is_object(),
        "a clip came back: {added:?}"
    );

    let clips = c.result_json("list_clips", json!({}));
    let clips: Vec<Value> = serde_json::from_value(clips).unwrap_or_default();
    assert_eq!(clips.len(), 1, "the clip is on the timeline: {clips:?}");

    // A frame of the preview, as a JPEG in base64, in the same shape
    // every other answer comes in.
    let frame = c.result("preview_frame", json!({"width": 320}));
    let b64 = frame
        .get("content")
        .and_then(|v| v.get(0))
        .and_then(|v| v.get("text"))
        .and_then(|v| v.as_str())
        .expect("a frame");
    assert!(
        b64.len() > 1000,
        "the frame is a picture: {} bytes",
        b64.len()
    );
    assert!(
        b64.starts_with("/9j/"),
        "the frame is a JPEG: {}",
        &b64[..12.min(b64.len())]
    );
}

#[test]
fn the_daemon_starts_an_export_and_reports_the_progress() {
    if !have_ffmpeg() {
        eprintln!("no ffmpeg on this machine: the test has nothing to say");
        return;
    }
    let dir = scratch("export");
    let clip = sample_video(&dir);
    let (port, token) = start(&dir);
    std::thread::sleep(std::time::Duration::from_millis(300));

    let mut c = Client::connect(port, &token);
    let media = c.result_json("add_media", json!({"path": clip.to_string_lossy()}));
    let media_id = media.get("media_id").and_then(|i| i.as_str()).unwrap();
    c.result(
        "add_clip_to_timeline",
        json!({"media_id": media_id, "track_index": 1, "start_ms": 0}),
    );

    let out = dir.join("story.mp4");
    c.result(
        "start_export",
        json!({"destination": out.to_string_lossy()}),
    );

    // The export runs in the daemon; give it a moment and ask.
    let mut finished = false;
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(250));
        let status = c.result_json("export_status", json!({}));
        let progress = status.get("progress").and_then(|p| p.as_f64());
        if progress.is_some_and(|p| p >= 1.0) {
            finished = true;
            break;
        }
    }
    assert!(finished, "the export finished");
    assert!(out.exists(), "the video is at {}", out.display());
    assert!(std::fs::metadata(&out).unwrap().len() > 1000);
}
