use std::path::Path;

use caprust_core::commands::UndoStack;
use caprust_core::project::ProjectState;
use serde_json::{json, Value};

use crate::auth::{Auth, UNAUTHORIZED};
use crate::rpc::{self, Request, Response, RpcError};
use crate::tools;

/// Borrowed project state a request operates on.
///
/// The standalone binary owns its own `ProjectState`; the running app
/// passes the one it already has open. Borrowing instead of owning is
/// what lets a phone drive a *live* session rather than a headless copy
/// that silently diverges from the window the user is looking at.
pub struct Ctx<'a> {
    pub project: &'a mut ProjectState,
    pub undo: &'a mut UndoStack,
    pub project_path: &'a Path,
    /// Session-wide actions the project alone cannot answer: which
    /// projects exist, and switching to one.
    ///
    /// `None` for the standalone binary, which is pinned to the one
    /// file it was given. A client asking it to switch gets a clear
    /// error rather than a silently ignored call.
    pub session: Option<&'a mut dyn SessionControl>,
}

/// Actions outside the project: the recent list, and loading one.
///
/// Separate from `Ctx.project` because a project swap replaces the very
/// state `Ctx` borrows. The implementation therefore only *records* the
/// request; the host performs the load once the borrow is released.
/// Doing the load inside the call would replace memory the caller is
/// still holding a `&mut` to.
pub trait SessionControl {
    /// Recently opened projects, newest first.
    fn recent(&self) -> Vec<caprust_core::recent::RecentProject>;

    /// Ask to switch to `path`. Returning `Ok` means accepted, not
    /// loaded: the host acts on it after the current batch finishes.
    /// An `Err` is reported to the client verbatim.
    fn request_open(&mut self, path: &Path) -> Result<(), String>;

    /// The current preview frame as base64 JPEG, longest side at most
    /// `max_w` pixels. `Ok(None)` means "nothing decoded yet", which
    /// is a normal state rather than a failure -- the preview has to
    /// decode a frame first.
    ///
    /// Separate from [SessionControl::recent] because it is genuinely
    /// optional: a host with no preview can leave it unimplemented by
    /// returning `Ok(None)`.
    fn preview_frame(&self, max_w: u32) -> Result<Option<String>, String> {
        let _ = max_w;
        Ok(None)
    }

    /// Begin an export. Like [SessionControl::request_open], `Ok`
    /// means accepted, not finished.
    fn request_export(&mut self, destination: &str) -> Result<(), String> {
        let _ = destination;
        Err("this host cannot start an export".into())
    }

    /// Export progress: 0.0 ..= 1.0, `None` when no export has run.
    fn export_progress(&self) -> Option<f32> {
        None
    }
}

/// Protocol state only. Everything with the project arrives per call,
/// so one `Server` serves either host without duplicating logic.
#[derive(Default)]
pub struct Server {
    initialized: bool,
    /// Pairing and token state. The token itself is persisted by the
    /// host in settings; the server holds it for the session.
    auth: Auth,
}

impl Server {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn auth_mut(&mut self) -> &mut Auth {
        &mut self.auth
    }

    /// Adopt a token loaded from settings at startup.
    pub fn load_token(&mut self, token: Option<String>) {
        self.auth.set_token(token);
    }

    pub fn token(&self) -> Option<&str> {
        self.auth.token()
    }

    /// Methods reachable before a token exists.
    ///
    /// `tools/list` is in here on purpose: a client has to be able to
    /// discover the tool surface before it can authenticate, and the
    /// schema is not project data. Everything that actually touches the
    /// project is gated.
    fn is_open_method(method: &str) -> bool {
        matches!(
            method,
            "initialize" | "notifications/initialized" | "ping" | "auth" | "tools/list"
        )
    }

    /// Returns `Some(json_line)` for requests, `None` for notifications.
    pub fn handle_line(&mut self, line: &str, ctx: &mut Ctx) -> Option<String> {
        let req: Request = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                let resp = Response::error(Value::Null, rpc::PARSE_ERROR, e.to_string());
                return Some(serialize(&resp));
            }
        };

        let is_notification = req.id.is_none();
        let id = req.id.clone().unwrap_or(Value::Null);

        if req.jsonrpc != "2.0" {
            if is_notification {
                return None;
            }
            let resp = Response::error(id, rpc::INVALID_REQUEST, "expected jsonrpc 2.0");
            return Some(serialize(&resp));
        }

        // Gate before dispatch. The token rides in params because this
        // transport is newline-delimited JSON with no headers, and the
        // two transports have to stay interchangeable.
        if !Self::is_open_method(&req.method) {
            let token = req
                .params
                .as_ref()
                .and_then(|p| p.get("token"))
                .and_then(Value::as_str);
            if !self.auth.verify(token) {
                if is_notification {
                    return None;
                }
                let resp = Response::error(
                    id,
                    UNAUTHORIZED,
                    "missing or invalid token; call `auth` with a pairing code first",
                );
                return Some(serialize(&resp));
            }
        }

        let result = self.dispatch(&req, ctx);

        if is_notification {
            return None;
        }

        let resp = match result {
            Ok(v) => Response::ok(id, v),
            Err(e) => Response::error(id, e.code, e.message),
        };
        Some(serialize(&resp))
    }

    fn dispatch(&mut self, req: &Request, ctx: &mut Ctx) -> Result<Value, RpcError> {
        match req.method.as_str() {
            "initialize" => {
                self.initialized = true;
                Ok(json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": { "tools": {} },
                    "serverInfo": {
                        "name": "caprust-mcp",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }))
            }
            "notifications/initialized" => Ok(Value::Null),
            "ping" => Ok(json!({})),
            // The one call that needs no token, because it is the one
            // that hands one out.
            "auth" => {
                let params = req
                    .params
                    .as_ref()
                    .ok_or_else(|| RpcError::invalid_params("missing params"))?;
                let code = params
                    .get("code")
                    .and_then(Value::as_str)
                    .ok_or_else(|| RpcError::invalid_params("missing pairing code"))?;
                match self.auth.redeem(code) {
                    Ok(token) => Ok(json!({ "token": token })),
                    Err(e) => Err(RpcError::custom(UNAUTHORIZED, e.to_string())),
                }
            }
            "tools/list" => Ok(json!({ "tools": tools::definitions() })),
            "tools/call" => {
                let params = req
                    .params
                    .as_ref()
                    .ok_or_else(|| RpcError::invalid_params("missing params"))?;
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| RpcError::invalid_params("missing tool name"))?;
                let args = params.get("arguments").cloned().unwrap_or(json!({}));

                // Split the struct before borrowing its fields. Taking
                // `ctx.project` and `ctx.session` inline would overlap
                // two mutable borrows of the same `&mut Ctx`.
                let Ctx {
                    project,
                    undo,
                    project_path,
                    session,
                } = ctx;

                let text = tools::call(
                    name,
                    &args,
                    project,
                    undo,
                    project_path,
                    session.as_deref_mut(),
                )
                .map_err(|e| RpcError::invalid_params(e.to_string()))?;

                Ok(json!({
                    "content": [{ "type": "text", "text": text }],
                    "isError": false
                }))
            }
            other => Err(RpcError::method_not_found(other)),
        }
    }
}

fn serialize(resp: &Response) -> String {
    serde_json::to_string(resp).expect("Response serialization cannot fail")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Owns the state a `Ctx` borrows. Needed because `Ctx` holds
    /// mutable borrows, so a helper cannot simply return one.
    struct Fixture {
        project: ProjectState,
        undo: UndoStack,
        path: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                project: ProjectState::default(),
                undo: UndoStack::new(),
                path: PathBuf::from("test.caprust"),
            }
        }

        fn ctx(&mut self) -> Ctx<'_> {
            Ctx {
                project: &mut self.project,
                undo: &mut self.undo,
                project_path: &self.path,
                session: None,
            }
        }

        /// Same, but with a session attached.
        fn ctx_with<'a>(&'a mut self, s: &'a mut FakeSession) -> Ctx<'a> {
            Ctx {
                project: &mut self.project,
                undo: &mut self.undo,
                project_path: &self.path,
                session: Some(s),
            }
        }
    }

    /// Stands in for the running app.
    struct FakeSession {
        opened: Option<String>,
        frame: Option<String>,
        export_dest: Option<String>,
        export_progress: Option<f32>,
    }

    impl crate::server::SessionControl for FakeSession {
        fn recent(&self) -> Vec<caprust_core::recent::RecentProject> {
            vec![caprust_core::recent::RecentProject {
                path: "C:/p/MyCut.caprust".into(),
                name: "My Cut".into(),
                duration_ms: 1500,
                last_opened: 1_700_000_000,
                clip_count: 4,
                base_resolution: 1080,
                frame_rate_label: "30.00".into(),
                first_media_id: None,
            }]
        }
        fn request_open(&mut self, path: &Path) -> Result<(), String> {
            self.opened = Some(path.to_string_lossy().to_string());
            Ok(())
        }
        fn preview_frame(&self, _max_w: u32) -> Result<Option<String>, String> {
            Ok(self.frame.clone())
        }
        fn request_export(&mut self, destination: &str) -> Result<(), String> {
            self.export_dest = Some(destination.to_string());
            Ok(())
        }
        fn export_progress(&self) -> Option<f32> {
            self.export_progress
        }
    }

    /// Drive a tool call the way the phone does: JSON in, JSON out.
    fn call_tool(
        s: &mut Server,
        f: &mut Fixture,
        sess: &mut FakeSession,
        name: &str,
        args: Value,
    ) -> Value {
        let token = s.token().map(str::to_string);
        let raw = json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "tools/call",
            "params": { "name": name, "arguments": args, "token": token }
        })
        .to_string();
        let out = s.handle_line(&raw, &mut f.ctx_with(sess)).unwrap();
        serde_json::from_str(&out).unwrap()
    }

    #[test]
    fn recent_projects_reaches_the_client_over_json_rpc() {
        // The handler tests call the function directly. That cannot catch
        // a tool name wired to the wrong arm, a name the phone sends
        // that the dispatch table does not have, or a result that never
        // makes it back into the content envelope. This drives the whole
        // path instead.
        let mut s = Server::new();
        let mut f = Fixture::new();
        let mut sess = FakeSession {
            opened: None,
            frame: None,
            export_dest: None,
            export_progress: None,
        };
        let token = s.auth_mut().rotate_token();

        let v = call_tool(&mut s, &mut f, &mut sess, "list_recent_projects", json!({}));
        // Hand the token over the same way the app does, so auth is in
        // the path under test.
        let _ = token;

        let text = v["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default();
        assert!(!v["error"].is_object(), "got an error: {v}");
        assert!(text.contains("My Cut"), "body was {text}");
        assert!(text.contains("MyCut.caprust"), "body was {text}");
    }

    #[test]
    fn open_project_reaches_the_host_over_json_rpc() {
        let mut s = Server::new();
        let mut f = Fixture::new();
        let mut sess = FakeSession {
            opened: None,
            frame: None,
            export_dest: None,
            export_progress: None,
        };
        s.auth_mut().rotate_token();

        let f2 = std::env::temp_dir().join(format!("caprust_e2e_{}.caprust", uuid::Uuid::new_v4()));
        std::fs::write(&f2, b"{}").unwrap();
        let v = call_tool(
            &mut s,
            &mut f,
            &mut sess,
            "open_project",
            json!({ "path": f2.to_string_lossy() }),
        );
        assert!(!v["error"].is_object(), "got an error: {v}");
        assert_eq!(sess.opened.as_deref(), Some(f2.to_string_lossy().as_ref()));
        let _ = std::fs::remove_file(&f2);
    }

    #[test]
    fn every_advertised_tool_is_reachable() {
        // tools/list and the dispatch table are maintained separately;
        // a name in one and not the other is a tool the phone is told
        // about and cannot call.
        let mut s = Server::new();
        let mut f = Fixture::new();
        let mut sess = FakeSession {
            opened: None,
            frame: None,
            export_dest: None,
            export_progress: None,
        };
        s.auth_mut().rotate_token();

        let raw = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let out = s.handle_line(raw, &mut f.ctx()).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        let names: Vec<String> = v["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert!(names.len() >= 15);

        for name in &names {
            // Each gets empty arguments. Some will fail on validation --
            // that is fine and expected. What must not happen is
            // "unknown tool", which means the name is advertised but
            // not dispatched.
            let v = call_tool(&mut s, &mut f, &mut sess, name, json!({}));
            let err = v["error"]["message"].as_str().unwrap_or_default();
            assert!(
                !err.contains("unknown tool"),
                "`{name}` is advertised by tools/list but not dispatched"
            );
        }
    }

    #[test]
    fn preview_frame_without_a_frame_is_not_an_error() {
        // The preview has to decode a frame first. Reporting that as a
        // failure makes a warming-up preview look broken.
        let mut s = Server::new();
        let mut f = Fixture::new();
        let mut sess = FakeSession {
            opened: None,
            frame: None,
            export_dest: None,
            export_progress: None,
        };
        s.auth_mut().rotate_token();

        let v = call_tool(&mut s, &mut f, &mut sess, "preview_frame", json!({}));
        assert!(!v["error"].is_object(), "got an error: {v}");
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        let body: Value = serde_json::from_str(text).unwrap();
        assert!(body["frame"].is_null(), "expected a null frame, got {body}");
    }

    #[test]
    fn preview_frame_carries_the_encoded_bytes() {
        let mut s = Server::new();
        let mut f = Fixture::new();
        let mut sess = FakeSession {
            opened: None,
            frame: Some("/9j/4AAQSkZJRg==".into()),
            export_dest: None,
            export_progress: None,
        };
        s.auth_mut().rotate_token();

        let v = call_tool(
            &mut s,
            &mut f,
            &mut sess,
            "preview_frame",
            json!({ "width": 480 }),
        );
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        let body: Value = serde_json::from_str(text).unwrap();
        assert_eq!(body["frame"], "/9j/4AAQSkZJRg==");
        assert_eq!(body["encoding"], "jpeg");
    }

    #[test]
    fn preview_width_is_clamped_to_something_sane() {
        // A client asking for 9000px would otherwise get a base64
        // payload no phone can use over a socket.
        let mut s = Server::new();
        let mut f = Fixture::new();
        let mut sess = FakeSession {
            opened: None,
            frame: Some("x".into()),
            export_dest: None,
            export_progress: None,
        };
        s.auth_mut().rotate_token();

        let v = call_tool(
            &mut s,
            &mut f,
            &mut sess,
            "preview_frame",
            json!({ "width": 9000 }),
        );
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        let body: Value = serde_json::from_str(text).unwrap();
        assert_eq!(body["width"], 1280);
    }

    #[test]
    fn export_start_and_status_travel_together() {
        let mut s = Server::new();
        let mut f = Fixture::new();
        let mut sess = FakeSession {
            opened: None,
            frame: None,
            export_dest: None,
            export_progress: Some(0.42),
        };
        s.auth_mut().rotate_token();

        let v = call_tool(
            &mut s,
            &mut f,
            &mut sess,
            "start_export",
            json!({ "destination": "C:/out.mp4" }),
        );
        assert!(!v["error"].is_object(), "got an error: {v}");
        assert_eq!(sess.export_dest.as_deref(), Some("C:/out.mp4"));

        let v = call_tool(&mut s, &mut f, &mut sess, "export_status", json!({}));
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        let body: Value = serde_json::from_str(text).unwrap();
        assert_eq!(body["started"], true);
        // Compared with a tolerance on purpose: the value is an f32
        // and JSON has no float type, so it comes back as the nearest
        // double. An exact assert here would fail on 0.42 and pass on
        // nothing else, which is a test of serde, not of this code.
        let p = body["progress"].as_f64().unwrap();
        assert!((p - 0.42).abs() < 1e-6, "progress was {p}");
    }

    #[test]
    fn export_status_before_anything_started_says_so() {
        let mut s = Server::new();
        let mut f = Fixture::new();
        let mut sess = FakeSession {
            opened: None,
            frame: None,
            export_dest: None,
            export_progress: None,
        };
        s.auth_mut().rotate_token();

        let v = call_tool(&mut s, &mut f, &mut sess, "export_status", json!({}));
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        let body: Value = serde_json::from_str(text).unwrap();
        // started:false, not an error and not a zero. A phone showing
        // 0% would look like a hung export.
        assert_eq!(body["started"], false);
        assert!(body["progress"].is_null());
    }

    #[test]
    fn start_export_needs_a_destination() {
        let mut s = Server::new();
        let mut f = Fixture::new();
        let mut sess = FakeSession {
            opened: None,
            frame: None,
            export_dest: None,
            export_progress: None,
        };
        s.auth_mut().rotate_token();

        let v = call_tool(&mut s, &mut f, &mut sess, "start_export", json!({}));
        assert!(
            v["error"].is_object(),
            "an empty destination must not be accepted"
        );
        let v = call_tool(
            &mut s,
            &mut f,
            &mut sess,
            "start_export",
            json!({ "destination": "  " }),
        );
        assert!(
            v["error"].is_object(),
            "a blank destination must not be accepted"
        );
        assert!(sess.export_dest.is_none());
    }

    #[test]
    fn initialize_returns_server_info() {
        let mut s = Server::new();
        let mut f = Fixture::new();
        let raw = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let out = s.handle_line(raw, &mut f.ctx()).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["result"]["serverInfo"]["name"], "caprust-mcp");
        assert_eq!(v["result"]["protocolVersion"], "2024-11-05");
    }

    #[test]
    fn tools_list_returns_expected_count() {
        let mut s = Server::new();
        let mut f = Fixture::new();
        let raw = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
        let out = s.handle_line(raw, &mut f.ctx()).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        let tools = v["result"]["tools"].as_array().unwrap();
        // 11 read-only + 11 mutating
        assert_eq!(tools.len(), 22);
    }

    #[test]
    fn unknown_method_is_not_found() {
        let mut s = Server::new();
        let mut f = Fixture::new();
        let token = s.auth_mut().rotate_token();
        // Authenticated, so this gets past the gate and reaches the
        // method lookup.
        let raw = format!(
            r#"{{"jsonrpc":"2.0","id":3,"method":"bogus","params":{{"token":"{token}"}}}}"#
        );
        let out = s.handle_line(&raw, &mut f.ctx()).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["error"]["code"], -32601);
    }

    /// Every method that touches the project has to refuse an
    /// unauthenticated caller. Before pairing existed the server had no
    /// gate at all, so this is the regression that matters.
    #[test]
    fn project_tools_require_a_token() {
        let mut s = Server::new();
        let mut f = Fixture::new();
        s.auth_mut().rotate_token();

        let cases = [
            r#"{"jsonrpc":"2.0","id":1,"method":"get_project_summary"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"list_clips"}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"add_media","arguments":{}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"save_project"}"#,
        ];
        for raw in cases {
            let out = s.handle_line(raw, &mut f.ctx()).unwrap();
            let v: Value = serde_json::from_str(&out).unwrap();
            assert_eq!(
                v["error"]["code"],
                crate::auth::UNAUTHORIZED,
                "must refuse without a token: {raw}"
            );
        }
        // And the project is untouched by the refused calls.
        assert_eq!(f.project.clips.len(), 0);
    }

    #[test]
    fn a_wrong_token_is_refused() {
        let mut s = Server::new();
        let mut f = Fixture::new();
        s.auth_mut().rotate_token();
        let raw = r#"{"jsonrpc":"2.0","id":1,"method":"list_clips","params":{"token":"wrong"}}"#;
        let out = s.handle_line(raw, &mut f.ctx()).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["error"]["code"], crate::auth::UNAUTHORIZED);
    }

    /// The pairing handshake a phone actually runs.
    #[test]
    fn auth_hands_out_a_working_token() {
        let mut s = Server::new();
        let mut f = Fixture::new();
        let code = s.auth_mut().begin_pairing();

        let raw =
            format!(r#"{{"jsonrpc":"2.0","id":1,"method":"auth","params":{{"code":"{code}"}}}}"#);
        let out = s.handle_line(&raw, &mut f.ctx()).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        let token = v["result"]["token"].as_str().expect("token").to_string();

        let call = format!(
            r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"get_project_summary","arguments":{{}},"token":"{token}"}}}}"#
        );
        let out = s.handle_line(&call, &mut f.ctx()).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert!(v.get("error").is_none(), "token should work: {v}");
    }

    #[test]
    fn auth_rejects_a_wrong_code() {
        let mut s = Server::new();
        let mut f = Fixture::new();
        s.auth_mut().begin_pairing();
        let raw = r#"{"jsonrpc":"2.0","id":1,"method":"auth","params":{"code":"000000"}}"#;
        let out = s.handle_line(raw, &mut f.ctx()).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["error"]["code"], crate::auth::UNAUTHORIZED);
    }

    #[test]
    fn notification_returns_none() {
        let mut s = Server::new();
        let mut f = Fixture::new();
        let raw = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        assert!(s.handle_line(raw, &mut f.ctx()).is_none());
    }

    #[test]
    fn parse_error_is_reported() {
        let mut s = Server::new();
        let mut f = Fixture::new();
        let out = s.handle_line("not json", &mut f.ctx()).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["error"]["code"], -32700);
    }

    #[test]
    fn non_2_0_jsonrpc_rejected() {
        let mut s = Server::new();
        let mut f = Fixture::new();
        let raw = r#"{"jsonrpc":"1.0","id":4,"method":"ping"}"#;
        let out = s.handle_line(raw, &mut f.ctx()).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["error"]["code"], -32600);
    }

    #[test]
    fn tools_call_unknown_tool_returns_invalid_params() {
        let mut s = Server::new();
        let mut f = Fixture::new();
        let token = s.auth_mut().rotate_token();
        let raw = format!(
            r#"{{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{{"name":"nope","arguments":{{}},"token":"{token}"}}}}"#
        );
        let out = s.handle_line(&raw, &mut f.ctx()).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["error"]["code"], -32602);
    }
}
