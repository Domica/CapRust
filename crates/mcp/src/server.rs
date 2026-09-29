use std::path::PathBuf;

use caprust_core::commands::UndoStack;
use caprust_core::project::ProjectState;
use serde_json::{json, Value};

use crate::rpc::{self, Request, Response, RpcError};
use crate::tools;

pub struct Server {
    project: ProjectState,
    undo: UndoStack,
    project_path: PathBuf,
    initialized: bool,
}

impl Server {
    pub fn new(project: ProjectState, project_path: PathBuf) -> Self {
        Self {
            project,
            undo: UndoStack::new(),
            project_path,
            initialized: false,
        }
    }

    /// Returns `Some(json_line)` for requests, `None` for notifications.
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
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

        let result = self.dispatch(&req);

        if is_notification {
            return None;
        }

        let resp = match result {
            Ok(v) => Response::ok(id, v),
            Err(e) => Response::error(id, e.code, e.message),
        };
        Some(serialize(&resp))
    }

    fn dispatch(&mut self, req: &Request) -> Result<Value, RpcError> {
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

                let text = tools::call(
                    name,
                    &args,
                    &mut self.project,
                    &mut self.undo,
                    &self.project_path,
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

    fn server() -> Server {
        Server::new(ProjectState::default(), PathBuf::from("test.caprust"))
    }

    #[test]
    fn initialize_returns_server_info() {
        let mut s = server();
        let raw = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let out = s.handle_line(raw).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["result"]["serverInfo"]["name"], "caprust-mcp");
        assert_eq!(v["result"]["protocolVersion"], "2024-11-05");
    }

    #[test]
    fn tools_list_returns_expected_count() {
        let mut s = server();
        let raw = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
        let out = s.handle_line(raw).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        let tools = v["result"]["tools"].as_array().unwrap();
        // 4 read-only + 11 mutating
        assert_eq!(tools.len(), 15);
    }

    #[test]
    fn unknown_method_is_not_found() {
        let mut s = server();
        let raw = r#"{"jsonrpc":"2.0","id":3,"method":"bogus"}"#;
        let out = s.handle_line(raw).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["error"]["code"], -32601);
    }

    #[test]
    fn notification_returns_none() {
        let mut s = server();
        let raw = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        assert!(s.handle_line(raw).is_none());
    }

    #[test]
    fn parse_error_is_reported() {
        let mut s = server();
        let out = s.handle_line("not json").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["error"]["code"], -32700);
    }

    #[test]
    fn non_2_0_jsonrpc_rejected() {
        let mut s = server();
        let raw = r#"{"jsonrpc":"1.0","id":4,"method":"ping"}"#;
        let out = s.handle_line(raw).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["error"]["code"], -32600);
    }

    #[test]
    fn tools_call_unknown_tool_returns_invalid_params() {
        let mut s = server();
        let raw = r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"nope","arguments":{}}}"#;
        let out = s.handle_line(raw).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["error"]["code"], -32602);
    }
}
