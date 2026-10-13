use std::path::Path;

use caprust_core::commands::UndoStack;
use caprust_core::project::ProjectState;
use serde_json::{json, Value};

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
pub struct Server {
    initialized: bool,
}

impl Server {
    pub fn new() -> Self {
        Self { initialized: false }
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
        // 4 read-only + 11 mutating
        assert_eq!(tools.len(), 15);
    }

    #[test]
    fn unknown_method_is_not_found() {
        let mut s = Server::new();
        let mut f = Fixture::new();
        let raw = r#"{"jsonrpc":"2.0","id":3,"method":"bogus"}"#;
        let out = s.handle_line(raw, &mut f.ctx()).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["error"]["code"], -32601);
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
        let raw = r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"nope","arguments":{}}}"#;
        let out = s.handle_line(raw, &mut f.ctx()).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["error"]["code"], -32602);
    }
}
