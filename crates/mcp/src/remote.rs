//! Socket listener that hands request lines to whichever thread owns
//! the project.
//!
//! A blocking socket read cannot run on the thread that owns
//! `ProjectState`: it would freeze the UI between frames. So the
//! socket lives here, on its own thread, and the UI thread drains a
//! queue of lines once per frame.
//!
//! One client at a time. A second connection preempts the first rather
//! than interleaving, because the project and its undo stack are
//! single-owner and two writers would race.
//!
//! "Preempts" rather than "waits" is the whole design. The acceptor
//! used to serve each connection inline, which meant it was blocked in
//! `write_loop` for as long as that client stayed connected -- and not
//! in `accept()` at all. Two things broke from that one fact:
//!
//!   * A client that vanished without the desktop noticing (Wi-Fi
//!     drop, NAT timeout) kept the acceptor parked. New connections
//!     queued in the backlog and were never accepted, so a phone that
//!     reconnected appeared connected and then hung.
//!   * `Drop` wakes the acceptor with one throwaway connection. With a
//!     client attached, nothing was in `accept()` to accept it, so the
//!     port stayed bound and the next bind failed -- which is exactly
//!     what toggling the setting does.
//!
//! So each connection is served on its own thread and the acceptor goes
//! straight back to `accept()`. The newcomer shuts the old socket down,
//! which unblocks its threads immediately.

use anyhow::{Context, Result};
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
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
    /// Which connection is current. A connection that is being torn
    /// down must not clear `connected` on its way out, or it would
    /// report the client that replaced it as gone.
    generation: AtomicUsize,
}

impl BridgeStatus {
    fn new(port: u16) -> Self {
        Self {
            connected: AtomicBool::new(false),
            port: AtomicU64::from(port as u64),
            requests: AtomicU64::new(0),
            generation: AtomicUsize::new(0),
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

/// Owns the listener side.
///
/// Dropping it releases the port. That does not happen on its own: the
/// acceptor thread owns the `TcpListener` and parks in `accept()`, so
/// dropping the channels only ends the current connection. The thread
/// would sit on the socket forever, and the next `spawn` on the same
/// port would fail with "address already in use" -- which is what a
/// LAN toggle does, so the feature would break itself every time the
/// user changed a setting.
pub struct RemoteBridge {
    incoming: Receiver<String>,
    outgoing: Sender<String>,
    status: Arc<BridgeStatus>,
    /// Set on drop. The acceptor checks it and returns.
    shutdown: Arc<AtomicBool>,
    /// Where to poke to wake a parked `accept()`.
    addr: SocketAddr,
}

impl Drop for RemoteBridge {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        // `accept()` is blocking, so setting the flag is not enough --
        // nothing wakes it. One throwaway connection makes it return,
        // and the acceptor sees the flag and drops the listener.
        //
        // Best effort: if this fails the thread is stuck until the
        // process exits, which is no worse than not trying.
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(250));
    }
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

        // Read before the listener is moved into the thread below.
        let addr = listener.local_addr().context("read bound address")?;
        let shutdown = Arc::new(AtomicBool::new(false));
        let acceptor_shutdown = Arc::clone(&shutdown);

        let acceptor_status = Arc::clone(&status);
        std::thread::Builder::new()
            .name("caprust-mcp-acceptor".into())
            .spawn(move || {
                acceptor(
                    listener,
                    incoming_tx,
                    outgoing_rx,
                    acceptor_status,
                    acceptor_shutdown,
                )
            })
            .context("spawn MCP acceptor")?;

        tracing::info!("mcp: remote bridge listening on {host}:{bound}");
        Ok(Self {
            incoming: incoming_rx,
            outgoing: outgoing_tx,
            status,
            shutdown,
            addr,
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

/// The connection currently being served, so a newcomer can evict it.
struct Served {
    /// Kept purely to shut the socket down. `shutdown()` on a socket
    /// blocked in `read` or `write` unblocks it immediately, which is
    /// the only reliable way to interrupt a thread parked in the kernel.
    stream: TcpStream,
}

fn acceptor(
    listener: TcpListener,
    incoming_tx: Sender<String>,
    outgoing_rx: Receiver<String>,
    status: Arc<BridgeStatus>,
    shutdown: Arc<AtomicBool>,
) {
    // mpsc::Receiver is neither Clone nor Send-usable from two threads
    // at once, so the writer threads take turns on it behind a mutex.
    // Only one client is ever served, so contention is a non-issue.
    let outgoing = Arc::new(Mutex::new(outgoing_rx));
    let served: Arc<Mutex<Option<Served>>> = Arc::new(Mutex::new(None));

    loop {
        if shutdown.load(Ordering::Relaxed) {
            tracing::info!("mcp: acceptor shutting down, releasing the port");
            drop(listener);
            return;
        }
        let (stream, addr) = match listener.accept() {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("mcp: accept failed: {e}");
                // A transient error (the peer reset mid-handshake) should
                // not kill the bridge.
                continue;
            }
        };
        // The wake-up connection from Drop is not a client. Accepting
        // it and then blocking again would leave the port held for as
        // long as the process lives.
        if shutdown.load(Ordering::Relaxed) {
            tracing::info!("mcp: acceptor shutting down, releasing the port");
            drop(listener);
            return;
        }

        // A new connection takes the slot. Shutting the old socket down
        // is what releases its threads; without this the previous
        // client would keep holding `connected` and its writer would
        // still be draining replies meant for the newcomer.
        if let Ok(mut slot) = served.lock() {
            if let Some(previous) = slot.take() {
                tracing::info!("mcp: new client from {addr} supersedes the previous one");
                let _ = previous.stream.shutdown(Shutdown::Both);
            }
        }

        let generation = status.generation.fetch_add(1, Ordering::Relaxed) + 1;
        status.generation.store(generation, Ordering::Relaxed);
        // Set before the writer thread starts. `write_loop` treats
        // `connected == false` as "the peer is gone" and exits, tearing
        // the socket down -- so forgetting this line makes every
        // connection die the moment it is accepted. It was lost in the
        // move to per-connection threads, and the symptom was a phone
        // that could connect, then got a broken pipe on its first write.
        status.connected.store(true, Ordering::Relaxed);
        tracing::info!("mcp: remote client connected from {addr} (gen {generation})");

        let read_half = match stream.try_clone() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("mcp: could not split socket: {e}");
                continue;
            }
        };
        // A third clone kept purely for eviction.
        let evict = match stream.try_clone() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("mcp: could not keep an eviction handle: {e}");
                continue;
            }
        };

        let reader_incoming = incoming_tx.clone();
        let reader_status = Arc::clone(&status);
        let reader_gen = generation;
        let writer_out = Arc::clone(&outgoing);
        let writer_status = Arc::clone(&status);
        let writer_gen = generation;

        let spawned = std::thread::Builder::new()
            .name("caprust-mcp-conn".into())
            .spawn(move || {
                let reader = std::thread::Builder::new()
                    .name("caprust-mcp-reader".into())
                    .spawn({
                        let status = Arc::clone(&reader_status);
                        move || read_loop(read_half, reader_incoming, status, reader_gen)
                    });

                let mut write_half = stream;
                write_loop(&mut write_half, &writer_out, &writer_status, writer_gen);
                // Leaving write_loop means the socket is finished with.
                // Shutting it down releases the reader even if the peer
                // never closed.
                let _ = write_half.shutdown(Shutdown::Both);
                if let Ok(handle) = reader {
                    let _ = handle.join();
                }
                tracing::info!("mcp: remote client from {addr} finished (gen {writer_gen})");
            });

        if let Err(e) = spawned {
            tracing::warn!("mcp: could not spawn connection thread: {e}");
            continue;
        }

        if let Ok(mut slot) = served.lock() {
            *slot = Some(Served { stream: evict });
        }
    }
}

fn read_loop(
    read_half: TcpStream,
    incoming: Sender<String>,
    status: Arc<BridgeStatus>,
    generation: usize,
) {
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
    // Only the current connection may report the client as gone. An
    // evicted connection clears the flag on its way out, which would
    // otherwise mark the client that replaced it as disconnected.
    if status.generation.load(Ordering::Relaxed) == generation {
        status.connected.store(false, Ordering::Relaxed);
    }
}

fn write_loop(
    write_half: &mut TcpStream,
    outgoing: &Mutex<Receiver<String>>,
    status: &BridgeStatus,
    generation: usize,
) {
    loop {
        // The generation check has to be INSIDE the lock, immediately
        // before the recv. Checking it out here is not enough: a writer
        // that has been preempted is most likely sitting in
        // `recv_timeout` at the moment its replacement arrives, and a
        // reply queued after that would be handed to the dying writer,
        // written to a socket that is already shut down, and lost. The
        // new client then waits for a reply that was thrown away --
        // which is a phone that appears connected and hangs.
        //
        // Inside the lock, a superseded writer breaks without ever
        // calling recv, so it cannot take what is not its own.
        let next = {
            let Ok(rx) = outgoing.lock() else {
                break;
            };
            if status.generation.load(Ordering::Relaxed) != generation {
                break;
            }
            rx.recv_timeout(POLL)
        };
        match next {
            Ok(resp) => {
                // Replaced between the recv and the write. Writing is
                // harmless -- the socket is closed, so it errors -- but
                // there is nothing to recover and no one to tell.
                if status.generation.load(Ordering::Relaxed) != generation {
                    break;
                }
                if writeln!(write_half, "{resp}").is_err() || write_half.flush().is_err() {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                // The reader clears `connected` on EOF. Checking here is
                // what lets this writer let go of a dead socket instead
                // of holding it until the process ends.
                if !status.connected() && status.generation.load(Ordering::Relaxed) == generation {
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

    #[test]
    fn a_new_connection_preempts_the_previous_client() {
        // The phone dropping off Wi-Fi does not close its socket. The
        // desktop keeps the connection half-open and the acceptor stayed
        // parked serving it, so the phone's reconnect sat in the backlog
        // unanswered: it looked connected and then hung on every call.
        //
        // Newest-wins is the only behaviour that survives that, because
        // it does not require the desktop to notice the old peer is
        // gone before it can accept anyone.
        let bridge = RemoteBridge::spawn("127.0.0.1", 0).unwrap();
        let port = bridge.port();

        // First client connects and is left completely idle, never
        // closed, exactly like a phone that lost its network.
        let _stale = TcpStream::connect(("127.0.0.1", port)).expect("first client");
        std::thread::sleep(Duration::from_millis(200));

        // Second client must be served even though the first is still
        // open and silent.
        let fresh = TcpStream::connect(("127.0.0.1", port)).expect("second client");
        fresh
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut w = fresh.try_clone().unwrap();
        let mut r = BufReader::new(fresh);
        w.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n")
            .unwrap();
        w.flush().unwrap();

        let mut served = 0;
        for _ in 0..40 {
            served += bridge.drain(4, |line| {
                if line.contains("tools/list") {
                    Some("{\"served\":true}\n".to_string())
                } else {
                    None
                }
            });
            if served > 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(served > 0, "the second client was never served");

        // Retry the read, but never re-send: the point is that the one
        // reply produced for this request reaches *this* client. A lost
        // reply shows up as a timeout here, which is what CI caught.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut line = String::new();
        loop {
            match r.read_line(&mut line) {
                Ok(_) => break,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    if std::time::Instant::now() >= deadline {
                        panic!("second client never got its reply: {e}");
                    }
                }
                Err(e) => panic!("second client read failed: {e}"),
            }
        }
        assert!(
            line.contains("served"),
            "second client got no reply: {line}"
        );

        // The status the settings panel shows must agree with reality.
        assert!(
            bridge.status().connected(),
            "the bridge reports no client while one is attached and served"
        );
    }

    #[test]
    fn a_phone_can_list_tools_and_pair_over_the_bridge() {
        // End to end, in the order the phone does it: tools/list, then
        // auth, each write followed by a read of the very next line.
        //
        // This is deliberately the same shape as the client's `raw()`:
        // write a request, read one line, parse it. If this passes, the
        // desktop is serving the phone correctly and any pairing failure
        // belongs to the client.
        use crate::server::{Ctx, Server};
        use caprust_core::commands::UndoStack;
        use caprust_core::project::ProjectState;

        let bridge = RemoteBridge::spawn("127.0.0.1", 0).unwrap();
        let port = bridge.port();
        let mut server = Server::new();

        let client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut w = client.try_clone().unwrap();
        let mut r = BufReader::new(client);

        let mut project = ProjectState::default();
        let mut undo = UndoStack::new();
        let path = std::path::PathBuf::from("test.caprust");

        let mut ask = |raw: &str, w: &mut TcpStream, server: &mut Server| -> serde_json::Value {
            w.write_all(raw.as_bytes()).unwrap();
            w.write_all(b"\n").unwrap();
            w.flush().unwrap();
            let mut handled = 0;
            for _ in 0..40 {
                handled += bridge.drain(8, |line| {
                    let mut ctx = Ctx {
                        project: &mut project,
                        undo: &mut undo,
                        project_path: &path,
                        session: None,
                    };
                    Some(server.handle_line(line, &mut ctx).unwrap_or_default())
                });
                if handled > 0 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            assert!(handled > 0, "nothing drained for {raw}");
            let mut line = String::new();
            r.read_line(&mut line).expect("reply arrives");
            serde_json::from_str(&line).expect("reply is valid JSON")
        };

        // 1. Capability check.
        let listed = ask(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#,
            &mut w,
            &mut server,
        );
        assert!(listed.get("error").is_none(), "tools/list failed: {listed}");
        let tools = listed["result"]["tools"].as_array().expect("tools array");
        assert!(!tools.is_empty(), "tool list came back empty");

        // 2. Pairing. The code is one-shot, so it is minted here and
        // redeemed once, exactly as the settings panel does.
        let code = server.auth_mut().begin_pairing();
        let paired = ask(
            &format!(r#"{{"jsonrpc":"2.0","id":2,"method":"auth","params":{{"code":"{code}"}}}}"#),
            &mut w,
            &mut server,
        );
        assert!(paired.get("error").is_none(), "auth failed: {paired}");
        let token = paired["result"]["token"].as_str().unwrap_or("");
        assert!(!token.is_empty(), "auth succeeded but returned no token");
    }

    #[test]
    fn a_connected_client_is_reported_connected() {
        // Narrowest version of the bug: `connected` was never set true
        // on accept, so write_loop saw "peer gone" and tore the socket
        // down the moment it was accepted. A phone got a broken pipe on
        // its first write.
        let bridge = RemoteBridge::spawn("127.0.0.1", 0).unwrap();
        assert!(!bridge.status().connected(), "nothing is connected yet");

        let client = TcpStream::connect(("127.0.0.1", bridge.port())).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut w = client.try_clone().unwrap();
        let mut r = BufReader::new(client);
        w.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n")
            .unwrap();
        w.flush().unwrap();

        let mut served = 0;
        for _ in 0..40 {
            served += bridge.drain(4, |line| {
                if line.contains("tools/list") {
                    Some("{\"served\":true}\n".to_string())
                } else {
                    None
                }
            });
            if served > 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(served > 0, "the request never reached the bridge");

        let mut line = String::new();
        r.read_line(&mut line).expect("client is served");
        assert!(line.contains("served"), "no reply: {line}");
        assert!(
            bridge.status().connected(),
            "bridge says disconnected while a client is attached and served"
        );
    }

    #[test]
    fn dropping_the_bridge_with_a_client_attached_releases_the_port() {
        // The other port-release tests drop the bridge with nobody
        // connected, so the acceptor is parked in accept() and the
        // wake-up connection works. With a client attached the acceptor
        // is not in accept() at all -- it is blocked in write_loop
        // serving that client -- so the throwaway connection is never
        // accepted, the thread never notices the flag, and the port is
        // still held when the app tries to rebind.
        //
        // That is the ordinary case: the user leaves the phone
        // connected and then turns the setting off and on again.
        let bridge = RemoteBridge::spawn("127.0.0.1", 0).unwrap();
        let port = bridge.port();

        let client = TcpStream::connect(("127.0.0.1", port)).expect("client connects");
        // Bounded, so a failure fails instead of hanging the suite.
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        // Prove the client is actually being served before dropping, so
        // a failure below cannot be blamed on a connection that never
        // arrived.
        let mut w = client.try_clone().unwrap();
        let mut r = BufReader::new(client.try_clone().unwrap());
        w.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n")
            .unwrap();
        w.flush().unwrap();
        // drain() before the request has reached the channel finds nothing,
        // so poll for it rather than calling once and hoping.
        let mut served = 0;
        for _ in 0..40 {
            served += bridge.drain(4, |line| {
                if line.contains("tools/list") {
                    Some("{\"ok\":true}\n".to_string())
                } else {
                    None
                }
            });
            if served > 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(served > 0, "the request never reached the bridge");
        let mut line = String::new();
        r.read_line(&mut line).expect("client is served");
        assert!(line.contains("ok"), "client got no reply: {line}");

        drop(bridge);
        // Client is still open and idle -- it never closes.
        drop(w);
        std::thread::sleep(Duration::from_millis(500));

        let again = RemoteBridge::spawn("127.0.0.1", port);
        assert!(
            again.is_ok(),
            "port {port} was still held after drop with a client attached: {}",
            again.err().map(|e| e.to_string()).unwrap_or_default()
        );
    }

    #[test]
    fn dropping_the_bridge_releases_the_port() {
        // The bug this covers: the acceptor thread owns the listener
        // and parks in accept(), so dropping the bridge left the port
        // held and the next bind failed. That is what the LAN toggle
        // does, so turning the setting off and on again broke remote
        // control for the rest of the session.
        let bridge = RemoteBridge::spawn("127.0.0.1", 0).unwrap();
        let port = bridge.port();
        drop(bridge);

        // Give the acceptor a moment to notice and let go.
        std::thread::sleep(Duration::from_millis(300));

        let again = RemoteBridge::spawn("127.0.0.1", port);
        assert!(
            again.is_ok(),
            "port {port} was still held after drop: {}",
            again.err().map(|e| e.to_string()).unwrap_or_default()
        );
    }

    #[test]
    fn rebinding_the_same_port_twice_in_a_row_works() {
        // Straight from the settings flow: bind, drop, rebind.
        for _ in 0..3 {
            let bridge = RemoteBridge::spawn("127.0.0.1", 0).unwrap();
            let port = bridge.port();
            drop(bridge);
            std::thread::sleep(Duration::from_millis(200));
            let next = RemoteBridge::spawn("127.0.0.1", port).expect("rebind");
            drop(next);
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    #[test]
    fn a_dropped_bridge_reports_no_new_requests() {
        // The wake-up connection from Drop must not be served as a
        // client: nothing comes of it and it would look like a peer
        // joining.
        let bridge = RemoteBridge::spawn("127.0.0.1", 0).unwrap();
        let port = bridge.port();
        drop(bridge);
        std::thread::sleep(Duration::from_millis(400));
        // The port must be closed, not merely idle.
        let reachable = TcpStream::connect_timeout(
            &format!("127.0.0.1:{port}").parse().unwrap(),
            Duration::from_millis(500),
        );
        assert!(reachable.is_err(), "port {port} still accepts connections");
    }

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
