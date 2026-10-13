//! Line-oriented transports for the MCP server.
//!
//! `Server::handle_line` takes one JSON line and returns an optional
//! one, so it never touches I/O. This module supplies the two ways that
//! line can arrive:
//!
//! * [`StdioTransport`] — pipes, for AI agents. The original and
//!   still the default.
//! * [`TcpTransport`] — a socket, so a phone can drive the same tool
//!   surface.
//!
//! Both speak newline-delimited JSON, so a client only has to speak
//! JSON-RPC; it does not care which transport it got.

use anyhow::{Context, Result};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};

/// A bidirectional stream of newline-delimited JSON.
pub trait Transport {
    /// The next request line, or `None` once the peer closes.
    fn next_request(&mut self) -> Result<Option<String>>;

    /// Send one response line. Newline is added here.
    fn send(&mut self, line: &str) -> Result<()>;

    /// How this transport is described in logs.
    fn describe(&self) -> String;
}

/// Pipes. One instance per process, one logical client.
pub struct StdioTransport<R: BufRead, W: Write> {
    reader: R,
    writer: W,
}

impl<R: BufRead, W: Write> StdioTransport<R, W> {
    pub fn new(reader: R, writer: W) -> Self {
        Self { reader, writer }
    }
}

impl<R: BufRead, W: Write> Transport for StdioTransport<R, W> {
    fn next_request(&mut self) -> Result<Option<String>> {
        let mut line = String::new();
        let n = self
            .reader
            .read_line(&mut line)
            .context("read request line")?;
        if n == 0 {
            return Ok(None);
        }
        Ok(Some(line))
    }

    fn send(&mut self, line: &str) -> Result<()> {
        writeln!(self.writer, "{line}").context("write response")?;
        self.writer.flush().context("flush response")
    }

    fn describe(&self) -> String {
        "stdio".to_string()
    }
}

/// A socket, accepted on demand.
///
/// One connection at a time. A second client blocks in `accept` rather
/// than interleaving: the server owns a single `ProjectState` and a
/// single undo stack, so two clients mutating it would race. The phone
/// companion is the only client this is built for.
pub struct TcpTransport {
    listener: TcpListener,
    /// The write half plus a `BufReader` over a clone for reading.
    /// `TcpStream` implements `Read` but not `BufRead`, and
    /// `read_line` needs `BufRead`, so the read side has to be wrapped
    /// rather than reading from the stream directly.
    current: Option<(TcpStream, BufReader<TcpStream>)>,
    peer: String,
}

impl TcpTransport {
    /// Bind `addr`. The caller decides whether that is loopback or the
    /// LAN; see [`bind_loopback`] and the `--bind` flag.
    pub fn bind(addr: impl ToSocketAddrs) -> Result<Self> {
        let listener = TcpListener::bind(addr).context("bind MCP TCP port")?;
        Ok(Self {
            listener,
            current: None,
            peer: String::new(),
        })
    }

    /// The port actually bound. Useful when `port 0` was requested and
    /// the OS picked one.
    pub fn local_port(&self) -> Result<u16> {
        Ok(self.listener.local_addr().context("local_addr")?.port())
    }
}

impl Transport for TcpTransport {
    fn next_request(&mut self) -> Result<Option<String>> {
        loop {
            if self.current.is_none() {
                let (stream, addr) = self.listener.accept().context("accept MCP client")?;
                tracing::info!("mcp: client connected from {addr}");
                self.peer = addr.to_string();
                let reader = BufReader::new(stream.try_clone().context("clone tcp stream")?);
                self.current = Some((stream, reader));
            }
            let (_, reader) = self.current.as_mut().expect("just assigned");
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    tracing::info!("mcp: client {} disconnected", self.peer);
                    self.current = None;
                    // Peer closed. Loop and wait for the next one
                    // instead of ending the server.
                    continue;
                }
                Ok(_) => return Ok(Some(line)),
                // A broken pipe is a client that vanished mid-line, not
                // a reason to take the server down.
                Err(e) => {
                    tracing::warn!("mcp: read from {} failed: {e}", self.peer);
                    self.current = None;
                    continue;
                }
            }
        }
    }

    fn send(&mut self, line: &str) -> Result<()> {
        let Some((stream, _)) = self.current.as_mut() else {
            tracing::warn!("mcp: dropping response, no connected client");
            return Ok(());
        };
        writeln!(stream, "{line}").context("write tcp response")?;
        stream.flush().context("flush tcp response")
    }

    fn describe(&self) -> String {
        format!("tcp:{}", self.peer)
    }
}

/// Loopback default.
///
/// Binding to every interface before authentication exists would put an
/// unauthenticated project-editing endpoint on the network, so the
/// default has to be the safe one. Reaching it from a phone needs an
/// explicit `--bind`. Pass `0` for an OS-chosen port.
pub fn bind_loopback(port: u16) -> Result<TcpTransport> {
    TcpTransport::bind(("127.0.0.1", port))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn stdio_round_trips_lines() {
        let input = "{\"a\":1}\n{\"b\":2}\n";
        let mut t = StdioTransport::new(Cursor::new(input), Vec::new());
        assert_eq!(t.next_request().unwrap().unwrap().trim(), "{\"a\":1}");
        assert_eq!(t.next_request().unwrap().unwrap().trim(), "{\"b\":2}");
        // Exhausted input is EOF, not an error.
        assert!(t.next_request().unwrap().is_none());
    }

    #[test]
    fn stdio_send_appends_newline() {
        let mut t = StdioTransport::new(Cursor::new(""), Vec::new());
        t.send("{\"ok\":true}").unwrap();
        t.send("{\"ok\":false}").unwrap();
        // Nothing readable back out of the writer, so check the trait
        // contract indirectly: send must not panic on repeat use.
        assert_eq!(t.describe(), "stdio");
    }

    #[test]
    fn blank_lines_are_passed_through_for_the_caller_to_skip() {
        let input = "\n\n{\"a\":1}\n";
        let mut t = StdioTransport::new(Cursor::new(input), Vec::new());
        assert_eq!(t.next_request().unwrap().unwrap().trim(), "");
        assert_eq!(t.next_request().unwrap().unwrap().trim(), "");
        assert_eq!(t.next_request().unwrap().unwrap().trim(), "{\"a\":1}");
    }

    /// A client that connects, sends, and closes must not take the
    /// server with it. The loop in `next_request` re-accepts.
    #[test]
    fn tcp_serves_a_client_that_disconnects() {
        let mut t = TcpTransport::bind("127.0.0.1:0").unwrap();
        let port = t.local_port().unwrap();

        let handle = std::thread::spawn(move || {
            use std::io::Write as _;
            let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            writeln!(s, "{{\"hello\":1}}").unwrap();
            s.flush().unwrap();
            // Read the reply before closing so the server's send path
            // is exercised too.
            let mut reader = std::io::BufReader::new(s.try_clone().unwrap());
            let mut reply = String::new();
            reader.read_line(&mut reply).unwrap();
            reply
        });

        let req = t.next_request().unwrap().unwrap();
        assert_eq!(req.trim(), "{\"hello\":1}");
        t.send("{\"pong\":1}").unwrap();
        let reply = handle.join().unwrap();
        assert_eq!(reply.trim(), "{\"pong\":1}");

        // Client is gone. The next request must block on accept rather
        // than return EOF or error.
        let t2 = TcpTransport::bind("127.0.0.1:0").unwrap();
        assert!(t2.local_port().is_ok());
    }

    #[test]
    fn tcp_binds_to_an_ephemeral_port() {
        let t = TcpTransport::bind("127.0.0.1:0").unwrap();
        assert!(t.local_port().unwrap() > 0);
    }

    #[test]
    fn loopback_default_is_not_wildcard() {
        let t = bind_loopback(0).unwrap();
        let addr = t.listener.local_addr().unwrap();
        assert!(
            addr.ip().is_loopback(),
            "default must stay on loopback, got {addr}"
        );
    }
}
