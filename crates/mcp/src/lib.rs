//! CapRust MCP server, as a library.
//!
//! The binary in this package is a thin wrapper around this lib. The
//! app also links it, so a phone can drive a live session rather than a
//! headless copy that diverges from the window the user is looking at.
//!
//! ```no_run
//! use caprust_mcp_core::{server::{Ctx, Server}, transport::Transport};
//!
//! fn on_line(server: &mut Server, ctx: &mut Ctx, line: &str) {
//!     if let Some(reply) = server.handle_line(line, ctx) {
//!         // send `reply` back over whatever transport delivered `line`
//!         let _ = reply;
//!     }
//! }
//! ```

pub mod auth;
pub mod remote;
pub mod rpc;
pub mod server;
pub mod tools;
pub mod transport;
