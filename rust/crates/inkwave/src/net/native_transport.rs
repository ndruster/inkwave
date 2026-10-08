//! Native WebSocket transport — `tungstenite` on a dedicated I/O thread with
//! `std::mpsc` hand-offs, so the Bevy main thread never blocks (spec
//! FR-M2-3; PORT_MAP OQ-2 selection: tungstenite over fastwebsockets, no
//! async runtime).
//!
//! The frame *semantics* (welcome handshake, `m|`/`b|`/`s|` envelopes, the
//! 2 s `"ping"` heartbeat and the RTT EMA) are all delegated to
//! [`inkwave_sim::net::transport::RxState`] — the same code path the fake and
//! wasm transports use — so the three agree byte-for-byte on what a relay
//! frame means. This module only owns the socket, the thread and the timers.
//!
//! Origin hygiene: the Cloudflare relay rejects upgrades whose `Origin` header
//! is not on its allow-list (`ORIGIN_OK`, server/src/index.js L22-23). A
//! desktop binary has no page origin, so we mint a compliant one from the
//! relay host itself (`http(s)://<host>:<port>`) — the list admits
//! localhost/LAN/`:port` forms (and `wss://inkwave-net…` pairs with
//! `https://inkwave-net…`, which the *deployed* allow-list would still
//! reject; that path is for the user-machine checklist, not the sandbox).

use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tungstenite::client::IntoClientRequest;
use tungstenite::http::HeaderValue;
use tungstenite::protocol::CloseFrame;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Error, Message, WebSocket, connect};

use inkwave_sim::net::transport::{
    CONNECT_TIMEOUT_MS, PING_INTERVAL_MS, RxState, Transport, TransportEvent, relay_url, room_url,
};

/// The socket type the blocking client ends up with (plain `ws://` or
/// rustls-wrapped `wss://` — the only TLS backend we compile in).
type Ws = WebSocket<MaybeTlsStream<TcpStream>>;

/// Commands the main thread sends to the I/O thread.
enum Cmd {
    /// `connect(code, name, create)` was called — carry out the handshake.
    Connect { url: String, origin: String },
    /// `Transport::send_raw` — one already-framed text payload.
    Send(String),
    /// `Transport::close` — queue `(1000, "bye")` and stop the loop.
    Close,
}

/// Arm a short read timeout so `read` returns instead of parking the thread
/// forever — this is what lets `close`, the heartbeat and the handshake timer
/// run while the socket is idle. We reach the raw socket through
/// `MaybeTlsStream`: `Plain` for `ws://`, `Rustls` for `wss://`.
fn arm_read_timeout(ws: &mut Ws, ms: u64) {
    let t = Some(Duration::from_millis(ms));
    match ws.get_mut() {
        MaybeTlsStream::Plain(s) => {
            let _ = s.set_read_timeout(t);
        }
        MaybeTlsStream::Rustls(s) => {
            let _ = s.sock.set_read_timeout(t);
        }
        _ => {}
    }
}

/// The I/O thread body: owns the socket, the [`RxState`] and the timers.
fn io_loop(
    cmd_rx: Receiver<Cmd>,
    ev_tx: Sender<TransportEvent>,
    open: Arc<AtomicBool>,
    rtt: Arc<Mutex<f64>>,
) {
    let mut ws: Option<Ws> = None;
    let mut rx = RxState::default();
    let mut epoch = Instant::now();
    let mut next_ping = f64::INFINITY;
    let mut connected_at = 0.0;
    let mut connecting = false;
    // `close()` was requested: after the socket is gone, exit.
    let mut closing = false;
    // Consecutive read timeouts since `closing` — a black-holed network never
    // answers our close frame, so after ~0.5 s we stop waiting (`Drop::join`
    // must not hang forever).
    let mut close_timeouts = 0u32;

    loop {
        // Drain commands first so sends interleave with reads.
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                Cmd::Connect { url, origin } => {
                    if ws.is_some() || closing {
                        continue;
                    }
                    epoch = Instant::now();
                    connected_at = 0.0;
                    connecting = true;
                    match build_and_connect(&url, &origin) {
                        Ok(mut sock) => {
                            arm_read_timeout(&mut sock, 50);
                            ws = Some(sock);
                        }
                        Err(e) => {
                            connecting = false;
                            let _ = ev_tx.send(TransportEvent::Error { e });
                            let _ = ev_tx.send(TransportEvent::Closed {
                                reason: "Disconnected".to_string(),
                            });
                        }
                    }
                }
                Cmd::Send(s) => {
                    if let Some(w) = ws.as_mut() {
                        // A dead socket surfaces via the next read().
                        let _ = w.send(Message::text(s));
                    }
                }
                Cmd::Close => {
                    closing = true;
                    if let Some(w) = ws.as_mut() {
                        // transport.js L108: `ws.close(1000, 'bye')`.
                        let _ = w.close(Some(CloseFrame {
                            code: CloseCode::Normal,
                            reason: "bye".into(),
                        }));
                    } else {
                        return;
                    }
                }
            }
        }

        let now = epoch.elapsed().as_secs_f64() * 1000.0;

        // Heartbeat (transport.js L82-90): the first ping fires once welcome
        // has landed, then every 2 s; `_pingSent` times the oldest
        // unanswered one (pongs come back in order).
        if open.load(Ordering::Acquire) && now >= next_ping {
            if rx.ping_sent.is_none() {
                rx.ping_sent = Some(now);
            }
            if let Some(w) = ws.as_mut() {
                let _ = w.send(Message::text("ping"));
            }
            next_ping += PING_INTERVAL_MS;
        }

        // 8 s handshake timer (transport.js L48): reject, then the JS closes
        // the socket — `onclose` fires `onClose('Disconnected')` (L76).
        if connecting && now - connected_at >= CONNECT_TIMEOUT_MS {
            connecting = false;
            open.store(false, Ordering::Release);
            let _ = ev_tx.send(TransportEvent::Error {
                e: "Could not connect".to_string(),
            });
            let _ = ev_tx.send(TransportEvent::Closed {
                reason: "Disconnected".to_string(),
            });
            if let Some(w) = ws.as_mut() {
                let _ = w.close(None);
            }
        }

        // Read whatever is pending (returns within ~50 ms thanks to the
        // timeout; partial frames stay in tungstenite's buffer).
        let Some(w) = ws.as_mut() else {
            if closing {
                return;
            }
            // No probing `try_recv` here — it would consume a `Cmd::Connect`
            // and drop it (the command is drained at the loop top instead).
            std::thread::sleep(Duration::from_millis(10));
            continue;
        };
        match w.read() {
            Err(Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if closing {
                    close_timeouts += 1;
                    if close_timeouts >= 10 {
                        return;
                    }
                }
            }
            Ok(Message::Text(t)) => {
                let out = rx.handle(t.as_str(), now);
                if out.welcome {
                    rx.settled = true;
                    connecting = false;
                    open.store(true, Ordering::Release);
                    next_ping = now; // `_startPing` fires immediately (L88)
                }
                if let Some(e) = out.err
                    && !rx.settled
                {
                    rx.settled = true;
                    connecting = false;
                    open.store(false, Ordering::Release);
                    let _ = ev_tx.send(TransportEvent::Error { e: e.clone() });
                    // `fail` closes the socket with the same text (server
                    // L66); onclose then fires onClose(reason) (L75-76).
                    let _ = ev_tx.send(TransportEvent::Closed { reason: e });
                    let _ = w.close(None);
                }
                for ev in out.events {
                    let _ = ev_tx.send(ev);
                }
                *rtt.lock().unwrap() = rx.rtt;
            }
            Ok(Message::Close(cf)) => {
                // The peer sent a close frame (JS `onclose`, L73-77). While
                // unsettled the connect rejects with the frame's reason and
                // `onClose` is NOT called (JS `done` swallows it); settled →
                // `onClose(reason || 'Disconnected')`.
                let reason = cf
                    .and_then(|f| {
                        if f.reason.is_empty() {
                            None
                        } else {
                            Some(f.reason.to_string())
                        }
                    })
                    .unwrap_or_else(|| "Disconnected".to_string());
                if !rx.settled {
                    rx.settled = true;
                    connecting = false;
                    let e = if reason == "Disconnected" {
                        "Could not connect".to_string()
                    } else {
                        reason
                    };
                    let _ = ev_tx.send(TransportEvent::Error { e });
                } else if open.swap(false, Ordering::AcqRel) {
                    let _ = ev_tx.send(TransportEvent::Closed { reason });
                }
                if closing {
                    return;
                }
                ws = None;
            }
            Err(Error::ConnectionClosed) | Err(_) => {
                // Socket died without a clean close frame: the browser fires
                // `onerror` (reject 'Could not connect', L78) *and* `onclose`
                // (L75-76, now settled → onClose('Disconnected')) — the
                // Error + Closed pair, same as the handshake-timeout path.
                if !rx.settled {
                    rx.settled = true;
                    connecting = false;
                    let _ = ev_tx.send(TransportEvent::Error {
                        e: "Could not connect".to_string(),
                    });
                    let _ = ev_tx.send(TransportEvent::Closed {
                        reason: "Disconnected".to_string(),
                    });
                } else if open.swap(false, Ordering::AcqRel) {
                    let _ = ev_tx.send(TransportEvent::Closed {
                        reason: "Disconnected".to_string(),
                    });
                }
                if closing {
                    return;
                }
                ws = None;
            }
            Ok(_) => {} // binary / protocol ping / pong frames: not used here
        }
    }
}

/// `tungstenite::connect` with a compliant `Origin` header added to the
/// auto-generated handshake request (the `IntoClientRequest for &str` path
/// fills Host/Upgrade/`Sec-WebSocket-Key`; we only append Origin).
fn build_and_connect(url: &str, origin: &str) -> Result<Ws, String> {
    let mut req = url
        .into_client_request()
        .map_err(|_| "Could not connect".to_string())?;
    req.headers_mut().insert(
        "Origin",
        HeaderValue::from_str(origin).map_err(|_| "Could not connect".to_string())?,
    );
    let (ws, _resp) = connect(req).map_err(|_| "Could not connect".to_string())?;
    Ok(ws)
}

/// Native [`Transport`]: relay frames flow to/from a background thread; the
/// main thread only touches atomics and the event channel.
pub struct NativeTransport {
    /// Relay base URL (`ws://…` / `wss://…`) resolved from `relayURL()`.
    base: String,
    cmd_tx: Sender<Cmd>,
    ev_rx: Receiver<TransportEvent>,
    open: Arc<AtomicBool>,
    rtt: Arc<Mutex<f64>>,
    io: Option<JoinHandle<()>>,
    bytes_out: usize,
}

impl NativeTransport {
    /// `relay_base` is the output of
    /// [`inkwave_sim::net::transport::relay_url`] (the caller decides CLI/env
    /// vs localhost vs PROD). The I/O thread starts idle and connects when
    /// [`Transport::connect`] is called.
    #[must_use]
    pub fn new(relay_base: String) -> Self {
        let (cmd_tx, cmd_rx) = channel::<Cmd>();
        let (ev_tx, ev_rx) = channel::<TransportEvent>();
        let open = Arc::new(AtomicBool::new(false));
        let rtt = Arc::new(Mutex::new(0.0f64));
        let (o2, r2) = (open.clone(), rtt.clone());
        let io = thread::spawn(move || io_loop(cmd_rx, ev_tx, o2, r2));
        Self {
            base: relay_base,
            cmd_tx,
            ev_rx,
            open,
            rtt,
            io: Some(io),
            bytes_out: 0,
        }
    }

    /// `http(s)://<host>:<port>` for the given `ws(s)://` relay base — the
    /// compliant Origin shape the relay allow-list admits
    /// (server/src/index.js L22-23).
    #[must_use]
    pub fn origin_for(base: &str) -> String {
        let scheme = if base.starts_with("wss://") {
            "https"
        } else {
            "http"
        };
        let host = base.split_once("://").map(|(_, r)| r).unwrap_or(base);
        format!("{scheme}://{host}")
    }

    /// Native address resolution (tasks.md Task 18 / FR-M2-3): a CLI `--relay=`
    /// override wins, then `INKWAVE_RELAY`, then `INKWAVE_HOST` drives the
    /// localhost/LAN probe (`ws://<h>:8787`), else `PROD_RELAY`. A desktop
    /// binary has no page origin, so the LAN probe is opt-in via `INKWAVE_HOST`
    /// (the value the wasm build reads from `location.hostname`).
    #[must_use]
    pub fn resolve_relay_base(cli_override: Option<&str>) -> String {
        let env_relay = std::env::var("INKWAVE_RELAY").ok();
        let host = std::env::var("INKWAVE_HOST").ok();
        relay_url(cli_override.or(env_relay.as_deref()), host.as_deref())
    }
}

impl Transport for NativeTransport {
    fn connect(&mut self, code: &str, name: &str, create: bool) {
        let url = room_url(&self.base, code, name, create);
        let origin = Self::origin_for(&self.base);
        let _ = self.cmd_tx.send(Cmd::Connect { url, origin });
    }

    fn poll_events(&mut self) -> Vec<TransportEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = self.ev_rx.try_recv() {
            out.push(ev);
        }
        out
    }

    fn send_raw(&mut self, s: &str) -> bool {
        if !self.open.load(Ordering::Acquire) {
            return false;
        }
        self.bytes_out += s.len();
        self.cmd_tx.send(Cmd::Send(s.to_string())).is_ok()
    }

    fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }

    fn rtt(&self) -> f64 {
        *self.rtt.lock().unwrap()
    }

    /// `close()` (transport.js L105-109): stop the heartbeat, send
    /// `(1000, "bye")`; no [`TransportEvent::Closed`] follows (JS nulls
    /// `onclose` first, L108).
    fn close(&mut self) {
        self.open.store(false, Ordering::Release);
        let _ = self.cmd_tx.send(Cmd::Close);
    }
}

impl Drop for NativeTransport {
    fn drop(&mut self) {
        self.open.store(false, Ordering::Release);
        let _ = self.cmd_tx.send(Cmd::Close);
        // Dropping `cmd_tx` (below) also ends the thread's `try_recv` loop.
        if let Some(io) = self.io.take() {
            let _ = io.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::net::{TcpListener, TcpStream};

    use tungstenite::accept_hdr;
    use tungstenite::handshake::server::{ErrorResponse, Request, Response};

    use inkwave_sim::net::ControlFrame;
    use inkwave_sim::net::wire;

    use super::*;

    /// A room member: `(id, name, sink)`. `sink` sends frames to that
    /// client's socket.
    type Member = (String, String, Sender<String>);

    /// A miniature of `server/src/index.js`'s Room object, running in-process
    /// on a real TCP socket so `NativeTransport` exercises the whole path:
    /// Origin hygiene, the welcome handshake, `b|`/`s|` fan-out, the runtime
    /// `ping`→`pong` answer and the close handshake. One thread per client.
    ///
    /// Membership is shared behind a mutex (like the Durable Object), so two
    /// clients in the same "room" see each other's join/leave and routed
    /// payloads. The relay only ever forwards raw strings — it never parses
    /// the game payload, exactly like the Worker.
    struct Relay {
        /// seq-ordered members.
        members: Arc<Mutex<Vec<Member>>>,
        locked: Arc<Mutex<bool>>,
        /// Captured `Origin` header values, so a test can assert the client
        /// actually sent a compliant one.
        origins: Arc<Mutex<Vec<String>>>,
    }

    impl Relay {
        fn new() -> Self {
            Self {
                members: Arc::new(Mutex::new(Vec::new())),
                locked: Arc::new(Mutex::new(false)),
                origins: Arc::new(Mutex::new(Vec::new())),
            }
        }

        /// Send `frame` to every member except `skip` (the `b|` fan-out).
        fn fan_out(&self, skip: &str, frame: &str) {
            for (id, _, sink) in self.members.lock().unwrap().iter() {
                if id != skip {
                    let _ = sink.send(frame.to_string());
                }
            }
        }

        /// Send `frame` to exactly one member (the `s|to|` routing; the sender
        /// is reachable too, matching the real relay).
        fn send_to(&self, to: &str, frame: &str) {
            for (id, _, sink) in self.members.lock().unwrap().iter() {
                if id == to {
                    let _ = sink.send(frame.to_string());
                    break;
                }
            }
        }

        /// `_gone` (server L133-143): drop a member and broadcast `leave` with
        /// the re-elected host (the oldest remaining member).
        fn part(&self, id: &str) {
            let host = {
                let mut ms = self.members.lock().unwrap();
                ms.retain(|(m, _, _)| m != id);
                ms.first().map(|(m, _, _)| m.clone())
            };
            if let Some(h) = host {
                let out = wire::encode_control(&ControlFrame::Leave {
                    id: id.to_string(),
                    host: h,
                });
                self.fan_out("", &out);
            }
        }
    }

    /// Drive one client socket: run the join ladder from the parsed query,
    /// then relay frames until the socket closes.
    fn serve_client(mut sock: WebSocket<TcpStream>, relay: &Relay, query: String, id: String) {
        // Parse `?name=&create=&v=` from the request target's query string.
        let mut params = HashMap::new();
        for kv in query.split('&') {
            let mut it = kv.splitn(2, '=');
            if let (Some(k), Some(v)) = (it.next(), it.next()) {
                params.insert(k.to_string(), v.to_string());
            }
        }
        let name = params
            .get("name")
            .cloned()
            .unwrap_or_else(|| "Player".to_string());
        let create = params.get("create").map(|s| s == "1").unwrap_or(false);
        let proto_ok = params.get("v").map(|s| s == "1").unwrap_or(false);

        // Outbound frames for this member flow through a channel so the join
        // ladder and the message loop share one writer.
        let (out_tx, out_rx) = std::sync::mpsc::channel::<String>();
        let rx = out_rx;

        // `fail` (server L67): send the `err` frame, then close the socket.
        fn fail(sock: &mut WebSocket<TcpStream>, e: &str) {
            let _ = sock.send(Message::text(wire::encode_control(&ControlFrame::Err {
                e: e.to_string(),
            })));
            let _ = sock.close(None);
        }

        if !proto_ok {
            fail(&mut sock, "Please refresh the page — the game was updated");
            return;
        }

        // Join ladder (server L70-74), against the *other* members.
        let others: Vec<(String, String)> = {
            let ms = relay.members.lock().unwrap();
            ms.iter()
                .map(|(id, n, _)| (id.clone(), n.clone()))
                .collect()
        };
        if create && !others.is_empty() {
            fail(&mut sock, "Room code taken");
            return;
        }
        if !create && others.is_empty() {
            fail(&mut sock, "Room not found");
            return;
        }
        if others.len() >= 8 {
            fail(&mut sock, "Room is full");
            return;
        }
        if *relay.locked.lock().unwrap() && !others.is_empty() {
            fail(&mut sock, "Match in progress");
            return;
        }
        if others.is_empty() {
            *relay.locked.lock().unwrap() = false;
        }

        // Register (welcome roster includes ourselves, L82). `id` was minted
        // in the handshake callback so it is unique among live members.
        {
            let mut ms = relay.members.lock().unwrap();
            ms.push((id.clone(), name.clone(), out_tx.clone()));
            let all: Vec<(String, String)> =
                ms.iter().map(|(a, b, _)| (a.clone(), b.clone())).collect();
            let host = all[0].0.clone();
            drop(ms);
            // welcome → us; join → everyone else.
            let _ = out_tx.send(wire::encode_control(&ControlFrame::Welcome {
                id: id.clone(),
                host,
                members: all,
            }));
            let j = wire::encode_control(&ControlFrame::Join {
                id: id.clone(),
                name,
            });
            relay.fan_out(&id, &j);
        }

        // Pump loop: forward pending outbound frames, read inbound, relay.
        loop {
            // Drain anything the room wants to send us.
            while let Ok(frame) = rx.try_recv() {
                if sock.send(Message::text(frame)).is_err() {
                    relay.part(&id);
                    return;
                }
            }
            match sock.read() {
                Ok(Message::Text(t)) => {
                    let s = t.as_str();
                    if s == "ping" {
                        // Runtime auto-answer (server L52): never wakes the
                        // room, just echoes.
                        let _ = sock.send(Message::text("pong"));
                        continue;
                    }
                    if let Some(rest) = s.strip_prefix("b|") {
                        relay.fan_out(&id, &format!("m|{id}|{rest}"));
                    } else if let Some(rest) = s.strip_prefix("s|") {
                        let k = rest.find('|').unwrap_or(rest.len());
                        let to = &rest[..k];
                        let payload = &rest[k + 1..];
                        relay.send_to(to, &format!("m|{id}|{payload}"));
                    } else if let Some(v) = wire::parse_json(s)
                        && let Some(ControlFrame::Lock { v }) = wire::decode_control(&v)
                    {
                        // host-only lock (server L126): first member.
                        let host = relay
                            .members
                            .lock()
                            .unwrap()
                            .first()
                            .map(|(m, _, _)| m.clone())
                            .unwrap_or_default();
                        if host == id {
                            *relay.locked.lock().unwrap() = v;
                        }
                    }
                }
                Ok(Message::Close(_)) | Err(_) => {
                    relay.part(&id);
                    return;
                }
                _ => {}
            }
        }
    }

    /// Bind an ephemeral listener and spawn the accept loop; returns the
    /// `ws://` base URL and the shared [`Relay`] (for asserting origins).
    // The handshake callback's `Result<Response, ErrorResponse>` signature is
    // fixed by tungstenite's `Callback` trait — the large `Err` is theirs.
    #[allow(clippy::result_large_err)]
    fn spawn_relay() -> (String, Arc<Relay>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let relay = Arc::new(Relay::new());
        let r = relay.clone();
        std::thread::spawn(move || {
            let mut counter = 0u32;
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let relay = r.clone();
                // Per-connection capture of Origin + request target, filled by
                // the handshake callback and read once the socket upgrades.
                let captured: Arc<Mutex<Option<(String, String, String)>>> =
                    Arc::new(Mutex::new(None));
                let cap = captured.clone();
                let seq = {
                    counter += 1;
                    format!("{:04X}", 0xA0 + counter)
                };
                std::thread::spawn(move || {
                    let seq2 = seq.clone();
                    let cb = move |req: &Request,
                                   resp: Response|
                          -> Result<Response, ErrorResponse> {
                        let origin = req
                            .headers()
                            .get("Origin")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("")
                            .to_string();
                        let target = req.uri().path_and_query().map(|p| p.as_str()).unwrap_or("");
                        let (path, query) = match target.split_once('?') {
                            Some((p, q)) => (p.to_string(), q.to_string()),
                            None => (target.to_string(), String::new()),
                        };
                        *cap.lock().unwrap() = Some((origin, path, query));
                        Ok(resp)
                    };
                    if let Ok(sock) = accept_hdr(stream, cb) {
                        let (origin, _path, query) =
                            captured.lock().unwrap().clone().unwrap_or_default();
                        relay.origins.lock().unwrap().push(origin);
                        serve_client(sock, &relay, query, seq2);
                    }
                });
            }
        });
        (format!("ws://{addr}"), relay)
    }

    /// Poll `t` until `pred` sees the wanted event, or time out. Events that
    /// do not match are kept in `seen` so a later call can still find them
    /// (the relay sends `err` and closes in one burst).
    fn wait_for(
        t: &mut NativeTransport,
        seen: &mut Vec<TransportEvent>,
        pred: impl Fn(&TransportEvent) -> bool,
    ) -> Option<TransportEvent> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            seen.extend(t.poll_events());
            if let Some(i) = seen.iter().position(&pred) {
                return Some(seen.remove(i));
            }
            if Instant::now() > deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn origin_for_maps_ws_scheme() {
        assert_eq!(NativeTransport::origin_for("ws://h:8787"), "http://h:8787");
        assert_eq!(NativeTransport::origin_for("wss://x.dev"), "https://x.dev");
    }

    #[test]
    fn resolve_relay_base_prefers_cli_override() {
        // CLI override wins (and the sim `relay_url` strips one trailing `/`).
        assert_eq!(
            NativeTransport::resolve_relay_base(Some("ws://box:1234/")),
            "ws://box:1234"
        );
        // No override and no env set (the CI default) → PROD_RELAY.
        if std::env::var("INKWAVE_RELAY").is_err() && std::env::var("INKWAVE_HOST").is_err() {
            assert_eq!(
                NativeTransport::resolve_relay_base(None),
                inkwave_sim::net::transport::PROD_RELAY
            );
        }
    }

    #[test]
    fn native_connect_welcome_ping_close() {
        let (base, relay) = spawn_relay();
        let mut t = NativeTransport::new(base);
        let mut seen = Vec::new();
        t.connect("TEST01", "Alice", true);

        let ev = wait_for(&mut t, &mut seen, |e| {
            matches!(e, TransportEvent::Welcome { .. })
        })
        .expect("welcome should arrive");
        match ev {
            TransportEvent::Welcome { id, host, members } => {
                assert_eq!(id, host, "solo host is ourselves");
                assert_eq!(members.len(), 1);
                assert_eq!(members[0].1, "Alice");
            }
            _ => unreachable!(),
        }

        // The client must have sent a compliant Origin header (the whole point
        // of the native hygiene requirement).
        let origins = relay.origins.lock().unwrap().clone();
        assert!(
            origins.iter().any(|o| o.starts_with("http://127.0.0.1:")),
            "expected an http://<host>:<port> Origin, got {origins:?}"
        );

        // Heartbeat: the runtime answers "ping" with "pong", so the RTT stays
        // a real (tiny, non-negative) number after at least one round trip.
        // We can't force a 2 s wait cheaply, but `is_open` must hold.
        assert!(t.is_open());

        // Explicit close: no Closed event follows (JS nulls onclose first).
        t.close();
        std::thread::sleep(Duration::from_millis(150));
        let after = t.poll_events();
        assert!(
            !after
                .iter()
                .any(|e| matches!(e, TransportEvent::Closed { .. })),
            "explicit close must not surface Closed: {after:?}"
        );
        assert!(!t.is_open());
    }

    #[test]
    fn native_broadcast_and_send_to_between_two_clients() {
        let (base, _relay) = spawn_relay();
        let mut a = NativeTransport::new(base.clone());
        let mut b = NativeTransport::new(base.clone());
        let (mut sa, mut sb) = (Vec::new(), Vec::new());

        a.connect("ROOM1", "Alice", true);
        wait_for(&mut a, &mut sa, |e| {
            matches!(e, TransportEvent::Welcome { .. })
        })
        .expect("a welcome");
        b.connect("ROOM1", "Bob", false);
        let bw = wait_for(&mut b, &mut sb, |e| {
            matches!(e, TransportEvent::Welcome { .. })
        })
        .expect("b welcome");
        let (bid, aid) = match bw {
            TransportEvent::Welcome { id, members, .. } => {
                // roster has both, Alice first (host).
                assert_eq!(members.len(), 2);
                (id, members[0].0.clone())
            }
            _ => unreachable!(),
        };

        // a sees b join.
        wait_for(&mut a, &mut sa, |e| {
            matches!(e, TransportEvent::Join { .. })
        })
        .expect("a sees join");

        // Broadcast from a → b receives a Message from a; a does not loop back.
        let payload = r#"{"hello":1}"#;
        assert!(a.broadcast(payload));
        let msg = wait_for(&mut b, &mut sb, |e| {
            matches!(e, TransportEvent::Message { .. })
        })
        .expect("b sees broadcast");
        match msg {
            TransportEvent::Message { from, payload: p } => {
                assert_eq!(from, aid);
                assert_eq!(p, payload);
            }
            _ => unreachable!(),
        }

        // Targeted send from b → a.
        assert!(b.send_to(&aid, r#"{"pong":2}"#));
        let msg2 = wait_for(&mut a, &mut sa, |e| {
            matches!(e, TransportEvent::Message { .. })
        })
        .expect("a sees send_to");
        match msg2 {
            TransportEvent::Message { from, payload: p } => {
                assert_eq!(from, bid);
                assert_eq!(p, r#"{"pong":2}"#);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn native_join_ladder_rejects_unknown_room() {
        let (base, _relay) = spawn_relay();
        let mut t = NativeTransport::new(base);
        let mut seen = Vec::new();
        // Join a room nobody created → "Room not found", then the socket
        // closes: Error + Closed pair (transport.js reject + onclose).
        t.connect("NOPE99", "Ghost", false);
        let err = wait_for(&mut t, &mut seen, |e| {
            matches!(e, TransportEvent::Error { .. })
        })
        .expect("error for unknown room");
        match err {
            TransportEvent::Error { e } => assert_eq!(e, "Room not found"),
            _ => unreachable!(),
        }
        wait_for(&mut t, &mut seen, |e| {
            matches!(e, TransportEvent::Closed { .. })
        })
        .expect("closed after err");
        assert!(!t.is_open());
    }

    // -- TR-19.2: the real NativeTransport driven end-to-end against the
    //    test-only relay shim (`rust/tools/relay-shim`). The shim is a
    //    separate thread server speaking full PROTO v1 room semantics, so
    //    this exercises the whole client stack (Origin hygiene, welcome
    //    handshake, `b|`/`s|` relay, join/leave, lock, close) against a relay
    //    that is NOT the in-process mini-`Relay` above.

    #[test]
    fn native_transport_end_to_end_against_relay_shim() {
        use relay_shim::{Shim, ShimConfig};

        let shim = Shim::spawn(ShimConfig::relaxed()).expect("shim spawn");
        let base = shim.url.clone();

        // A creates the room.
        let mut a = NativeTransport::new(base.clone());
        let (mut sa, mut sb) = (Vec::new(), Vec::new());
        a.connect("SHIM01", "Alice", true);
        let aw = wait_for(&mut a, &mut sa, |e| {
            matches!(e, TransportEvent::Welcome { .. })
        })
        .expect("a welcome from shim");
        let (aid, ahost) = match aw {
            TransportEvent::Welcome { id, host, members } => {
                assert_eq!(id, host, "solo creator is host");
                assert_eq!(members.len(), 1);
                (id, host)
            }
            _ => unreachable!(),
        };

        // B joins through a second transport; both share the one shim room.
        let mut b = NativeTransport::new(base.clone());
        b.connect("SHIM01", "Bob", false);
        let bw = wait_for(&mut b, &mut sb, |e| {
            matches!(e, TransportEvent::Welcome { .. })
        })
        .expect("b welcome from shim");
        let bid = match bw {
            TransportEvent::Welcome { id, host, members } => {
                assert_eq!(host, ahost, "host stays A");
                assert_eq!(members.len(), 2);
                id
            }
            _ => unreachable!(),
        };

        // A observes B's join (the shim's `join` fan-out).
        wait_for(
            &mut a,
            &mut sa,
            |e| matches!(e, TransportEvent::Join { id, .. } if *id == bid),
        )
        .expect("a sees b join");

        // `b|` broadcast A to B; A must not loop back.
        assert!(a.broadcast(r#"{"k":"t","ts":1.5,"a":[]}"#));
        let m = wait_for(&mut b, &mut sb, |e| {
            matches!(e, TransportEvent::Message { .. })
        })
        .expect("b sees a broadcast");
        match m {
            TransportEvent::Message { from, payload } => {
                assert_eq!(from, aid);
                assert_eq!(payload, r#"{"k":"t","ts":1.5,"a":[]}"#);
            }
            _ => unreachable!(),
        }

        // `s|to|` targeted B to A.
        assert!(b.send_to(&aid, r#"{"k":"hit"}"#));
        let m2 = wait_for(&mut a, &mut sa, |e| {
            matches!(e, TransportEvent::Message { .. })
        })
        .expect("a sees b send_to");
        match m2 {
            TransportEvent::Message { from, payload } => {
                assert_eq!(from, bid);
                assert_eq!(payload, r#"{"k":"hit"}"#);
            }
            _ => unreachable!(),
        }

        // Host lock (A) -> a third join is refused with the exact ladder text,
        // surfacing as Error (transport.js reject).
        assert!(a.lock(true));
        std::thread::sleep(Duration::from_millis(80));
        let mut c = NativeTransport::new(base.clone());
        c.connect("SHIM01", "Eve", false);
        let err = wait_for(&mut c, &mut Vec::new(), |e| {
            matches!(e, TransportEvent::Error { .. })
        })
        .expect("locked join errors");
        match err {
            TransportEvent::Error { e } => assert_eq!(e, "Match in progress"),
            _ => unreachable!(),
        }

        // B leaves (explicit close) -> A sees `leave` with host still A.
        b.close();
        wait_for(
            &mut a,
            &mut sa,
            |e| matches!(e, TransportEvent::Leave { id, host } if *id == bid && *host == aid),
        )
        .expect("a sees b leave");

        // A closes cleanly; the shim's Drop tears the room down.
        a.close();
        assert!(!a.is_open());
    }
}
