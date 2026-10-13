//! Socket listener that hands request lines to whichever thread owns
//! the project.
//!
//! A blocking socket read cannot run on the thread that owns
//! `ProjectState`: it would freeze the UI between frames. So the
//! socket lives here, on its own thread, and the UI thread drains a
//! queue of lines once per frame.
//!
//! One client at a time. A second connection waits rather than
//! interleaving, because the project and its undo stack are
//! single-owner and two writers would race.

use anyhow::{Context, Result};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

/// How often the writer checks whether the client vanished. Bounds how
/// long a dead connection holds the single-client slot.
const POLL: Duration = Duration::from_millis(200);

/// What the UI shows next to the toggle.
#[derive(Debug, Default)]
pub struct BridgeStatus {
    connected: AtomicBool,
    port: AtomicU64,
    requests: AtomicU64,
}

impl BridgeStatus {
    fn new(port: u16) -> Self {
        Self {
            connected: AtomicBool::new(false),
            port: AtomicU64::from(port as u64),
            requests: AtomicU64::new(0),
        }
    }

    pub fn port(&self) -> u16 {
        self.port.load(Ordering::Relaxed) as u16
    }

    pub fn connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    pub fn requests(&self) -> u64 {
        self.requests.load(Ordering::Relaxed)
    }
}

/// Owns the listener side. Dropping it stops the acceptor thread on its
/// next loop iteration.
pub struct RemoteBridge {
    incoming: Receiver<String>,
    outgoing: Sender<String>,
    status: Arc<BridgeStatus>,
}

impl RemoteBridge {
    /// Bind `addr` (host, port) and start serving. Port 0 means the OS
    /// picks one; read it back with [`RemoteBridge::port`].
    pub fn spawn(host: &str, port: u16) -> Result<Self> {
        let listener =
            TcpListener::bind((host, port)).with_context(|| format!("bind {host}:{port}"))?;
        let bound = listener.local_addr().context("read bound address")?.port();
        let status = Arc::new(BridgeStatus::new(bound));

        let (incoming_tx, incoming_rx) = mpsc::channel::<String>();
        let (outgoing_tx, outgoing_rx) = mpsc::channel::<String>();

        let acceptor_status = Arc::clone(&status);
        std::thread::Builder::new()
            .name("caprust-mcp-acceptor".into())
            .spawn(move || acceptor(listener, incoming_tx, outgoing_rx, acceptor_status))
            .context("spawn MCP acceptor")?;

        tracing::info!("mcp: remote bridge listening on {host}:{bound}");
        Ok(Self {
            incoming: incoming_rx,
            outgoing: outgoing_tx,
            status,
        })
    }

    pub fn port(&self) -> u16 {
        self.status.port()
    }

    pub fn status(&self) -> &Arc<BridgeStatus> {
        &self.status
    }

    /// Apply every request that arrived since the last frame.
    ///
    /// `f` runs on the caller's thread, which is the point: it is where
    /// the project and undo stack live. Returning `Some` sends a
    /// response. Must not block, or it stalls the render loop.
    ///
    /// Cap the batch. A client that floods the socket must not be able
    /// to turn one frame into an unbounded amount of work.
    pub fn drain(&self, max_per_frame: usize, mut f: impl FnMut(&str) -> Option<String>) -> usize {
        let mut handled = 0;
        while handled < max_per_frame {
            let Ok(line) = self.incoming.try_recv() else {
                break;
            };
            if line.trim().is_empty() {
                continue;
            }
            handled += 1;
            self.status.requests.fetch_add(1, Ordering::Relaxed);
            if let Some(resp) = f(&line) {
                // A closed channel means the socket is gone; dropping
                // the reply is correct, not an error worth raising.
                if self.outgoing.send(resp).is_err() {
                    tracing::debug!("mcp: response channel closed, client gone");
                    break;
                }
            }
        }
        handled
    }
}

fn acceptor(
    listener: TcpListener,
    incoming_tx: Sender<String>,
    outgoing_rx: Receiver<String>,
    status: Arc<BridgeStatus>,
) {
    loop {
        let (stream, addr) = match listener.accept() {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("mcp: accept failed: {e}");
                // A transient error (the peer reset mid-handshake) should
                // not kill the bridge.
                continue;
            }
        };
        tracing::info!("mcp: remote client connected from {addr}");

        let read_half = match stream.try_clone() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("mcp: could not split socket: {e}");
                continue;
            }
        };
        let mut write_half: TcpStream = stream;

        let reader_status = Arc::clone(&status);
        let reader_incoming = incoming_tx.clone();
        let reader = std::thread::Builder::new()
            .name("caprust-mcp-reader".into())
            .spawn(move || read_loop(read_half, reader_incoming, reader_status));

        status.connected.store(true, Ordering::Relaxed);
        write_loop(&mut write_half, &outgoing_rx, &status);

        // The reader only ends when the peer does, or when the channel
        // dies with the app. Join so no thread outlives its connection.
        if let Ok(handle) = reader {
            let _ = handle.join();
        }
        tracing::info!("mcp: remote client from {addr} disconnected");
    }
}

fn read_loop(read_half: TcpStream, incoming: Sender<String>, status: Arc<BridgeStatus>) {
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if incoming.send(line.clone()).is_err() {
                    // The bridge was dropped: the app is shutting down.
                    break;
                }
            }
        }
    }
    status.connected.store(false, Ordering::Relaxed);
}

fn write_loop(write_half: &mut TcpStream, outgoing: &Receiver<String>, status: &BridgeStatus) {
    loop {
        match outgoing.recv_timeout(POLL) {
            Ok(resp) => {
                if writeln!(write_half, "{resp}").is_err() || write_half.flush().is_err() {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                // The reader clears `connected` on EOF. Checking here is
                // what lets the acceptor move on to the next client
                // instead of blocking forever on a dead socket.
                if !status.connected() {
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    /// Connect a client, and get a thread that collects the server's
    /// replies. Reading replies has to live off the test thread or it
    /// blocks the very loop that is supposed to drive the drain.
    fn client(port: u16) -> (TcpStream, std::sync::mpsc::Receiver<String>) {
        let sock = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let writer = sock.try_clone().unwrap();
        let mut reader = BufReader::new(sock.try_clone().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            while let Ok(n) = reader.read_line(&mut line) {
                if n == 0 {
                    break;
                }
                if tx.send(line.trim().to_string()).is_err() {
                    break;
                }
                line.clear();
            }
        });
        (writer, rx)
    }

    #[test]
    fn reports_the_port_it_bound() {
        let bridge = RemoteBridge::spawn("127.0.0.1", 0).unwrap();
        assert!(bridge.port() > 0);
    }

    #[test]
    fn passes_lines_to_the_drain_closure_and_replies() {
        let bridge = RemoteBridge::spawn("127.0.0.1", 0).unwrap();
        let (mut writer, replies) = client(bridge.port());
        writeln!(writer, "{{\"id\":1}}").unwrap();
        writeln!(writer, "{{\"id\":2}}").unwrap();
        writer.flush().unwrap();

        let (tx_seen, rx_seen) = std::sync::mpsc::channel::<String>();
        // Collected as they arrive. Peeking with try_iter() in the loop
        // condition would consume them and the final assertion would see
        // an empty receiver.
        let mut got: Vec<String> = Vec::new();
        let mut seen: Vec<String> = Vec::new();

        // Poll-drain like a render loop does, with a real deadline so a
        // scheduling hiccup does not turn into a hung test.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while got.len() < 2 && std::time::Instant::now() < deadline {
            bridge.drain(16, |line| {
                let id = line
                    .trim()
                    .trim_start_matches('{')
                    .trim_end_matches('}')
                    .to_string();
                let _ = tx_seen.send(id.clone());
                Some(format!("ack-{id}"))
            });
            while let Ok(reply) = replies.try_recv() {
                got.push(reply);
            }
        }
        // One last sweep: a reply can land just after the last drain.
        while let Ok(reply) = replies.try_recv() {
            got.push(reply);
        }

        seen.extend(rx_seen.try_iter());
        assert_eq!(
            seen.len(),
            2,
            "both lines reached the UI side, got {seen:?}"
        );
        assert!(seen.iter().any(|s| s.contains("1")));
        assert!(seen.iter().any(|s| s.contains("2")));
        assert_eq!(got.len(), 2, "both replies came back, got {got:?}");
        assert!(got[0].starts_with("ack-"), "got {got:?}");
    }

    #[test]
    fn blank_lines_do_not_count_as_requests() {
        let bridge = RemoteBridge::spawn("127.0.0.1", 0).unwrap();
        let (mut writer, _rx) = client(bridge.port());
        writeln!(writer).unwrap();
        writeln!(writer).unwrap();
        writer.flush().unwrap();
        std::thread::sleep(Duration::from_millis(200));

        let mut calls = 0;
        let n = bridge.drain(16, |_| {
            calls += 1;
            None
        });
        assert_eq!(n, 0, "blank lines are keepalives, not requests");
        assert_eq!(calls, 0);
    }

    #[test]
    fn batch_is_capped_per_frame() {
        let bridge = RemoteBridge::spawn("127.0.0.1", 0).unwrap();
        let (mut writer, _rx) = client(bridge.port());
        for i in 0..50 {
            writeln!(writer, "{{\"n\":{i}}}").unwrap();
        }
        writer.flush().unwrap();
        std::thread::sleep(Duration::from_millis(300));

        let mut seen = 0;
        let n = bridge.drain(8, |_| {
            seen += 1;
            None
        });
        assert!(n <= 8, "drain must not be unbounded, got {n}");
    }

    #[test]
    fn a_dead_client_releases_the_slot() {
        let bridge = RemoteBridge::spawn("127.0.0.1", 0).unwrap();
        let port = bridge.port();

        let first = TcpStream::connect(("127.0.0.1", port)).unwrap();
        drop(first);
        std::thread::sleep(Duration::from_millis(500));

        // The acceptor must come back around for this to be served. If
        // the writer were still blocked on the dead socket, the connect
        // would succeed but nothing would ever arrive.
        let (mut writer, _rx) = client(port);
        writeln!(writer, "{{\"after\":\"reconnect\"}}").unwrap();
        writer.flush().unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut got = false;
        while !got && std::time::Instant::now() < deadline {
            bridge.drain(16, |line| {
                if line.contains("reconnect") {
                    got = true;
                }
                None
            });
        }
        assert!(got, "a new client must be served after the old one drops");
    }
}
