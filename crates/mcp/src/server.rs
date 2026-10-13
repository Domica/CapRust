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

                let text = tools::call(name, &args, ctx.project, ctx.undo, ctx.project_path)
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
            }
        }
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
        // 6 read-only + 11 mutating
        assert_eq!(tools.len(), 17);
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
