//! TEST-ONLY relay shim — a faithful native re-implementation of the PROTO v1
//! room semantics of `server/src/index.js` (the Cloudflare Worker + Durable
//! Object relay), running on a plain `ws://` TCP socket.
//!
//! **This is not a replacement for `server/`.** It is never deployed; it
//! exists so the Rust client stack (Transport, NetSession, the net harness)
//! can be exercised end-to-end inside the sandbox, where `wrangler dev` and
//! the deployed relay may be unreachable (spec "relay 可达性" note). The
//! upstream `server/` tree stays byte-for-byte untouched.
//!
//! Semantics replicated line-by-line from `server/src/index.js`:
//!
//! - **Join ladder** (L70-74, in order): `v != PROTO` → `'Please refresh the
//!   page — the game was updated'`; `create && others` → `'Room code taken'`;
//!   `!create && !others` → `'Room not found'`; `others >= MAX` →
//!   `'Room is full'`; `locked && others` → `'Match in progress'`; an empty
//!   room auto-unlocks (L75). `fail` sends `{"t":"err",…}` then
//!   `close(4000, e)` (L67).
//! - **Membership & election**: `members()` is seq-ordered (L58), the host is
//!   the oldest member (L60); `welcome` carries the full roster *including
//!   ourselves* with `host = all[0].id` (L81-82); `join` fans out to everyone
//!   else (L83-84); `_gone` broadcasts `leave` with the re-elected host and
//!   unlocks an emptied room (L133-142).
//! - **Payload relay** (L111-122): `b|<p>` → `m|<from>|<p>` to *everyone else*
//!   (exclusive fan-out); `s|<to>|<p>` → `m|<from>|<p>` to the first member
//!   with that id — the sender included (`sendTo` is not exclusive, L120).
//!   Payloads are forwarded as raw strings, never parsed.
//! - **Control JSON** (L123-127): `{"t":"ping","c":n}` → `{"t":"pong","c":n}`;
//!   `{"t":"lock","v":b}` applies only when the sender is the host.
//! - **Runtime heartbeat** (L52): the bare text `"ping"` is answered `"pong"`
//!   without touching room state (and counts as liveness, L93).
//! - **Hygiene** (L22-24, L28, L38): the `ORIGIN_OK` allow-list admits the
//!   game site plus localhost/LAN/`:port` forms; `CODE = /^[A-Z0-9]{4,8}$/`;
//!   `MSG_MAX` oversized frames are dropped; `RATE`/`BURST_STRIKES` cut off
//!   runaway senders with `close(4008)`; the silence sweep drops stale
//!   sockets with `close(4001, 'Connection timed out')` — thresholds
//!   parameterised in [`ShimConfig`] so tests can run them fast.
//!
//! Deliberate, documented deviations (they cannot affect the Rust↔Rust test
//! scope): session ids are minted from a deterministic counter instead of
//! `Math.random().toString(36)` (L78) — same 4-char uppercase base36 shape;
//! `name` sanitisation uses Rust `is_alphabetic`/`is_numeric` for the JS
//! `\p{L}`/`\p{N}` classes and slices by Unicode scalar instead of UTF-16
//! code units (L76); the sweep tracks one liveness stamp per socket rather
//! than the Worker's three-way `max` (L93) — the shim's own `"ping"` answer
//! refreshes the same stamp, so the observable behaviour matches.

use std::collections::HashMap;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::protocol::CloseFrame;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::{Error, Message, WebSocket, accept_hdr};

use inkwave_sim::net::wire::{self, JVal};
use inkwave_sim::net::{ControlFrame, PROTO, SessionId};

/// Room size cap (`MAX = 8`, server/src/index.js L19).
pub const MAX: usize = 8;
/// Handshake protocol version (`PROTO = 1`, L19).
pub const PROTO_V: u8 = PROTO;

/// Hygiene / sweep tunables. Defaults are the Worker's production constants;
/// [`ShimConfig::relaxed`] loosens them for fast tests (tasks.md Task 19:
/// "MSG_MAX/RATE 检查（可放宽为测试配置）").
#[derive(Debug, Clone)]
pub struct ShimConfig {
    /// `MSG_MAX` (L24): frames longer than this (in Unicode scalars — see the
    /// module docs on the UTF-16 deviation) are dropped, never fanned out.
    pub msg_max: usize,
    /// `RATE` (L24): messages per second before a strike accrues.
    pub rate: u32,
    /// `BURST_STRIKES` (L24): strikes before the socket is cut off.
    pub burst_strikes: u32,
    /// `SILENT_MATCH` (L28): silence budget while the room is locked, ms.
    pub silent_match_ms: u64,
    /// `SILENT_LOBBY` (L28): silence budget while unlocked, ms.
    pub silent_lobby_ms: u64,
    /// `SWEEP` (L28): how often the liveness sweep runs, ms. The shim clamps
    /// the sleep to at most 1000 ms so a test config with a small budget
    /// still reacts quickly; the Worker's DO alarm cadence is not reproduced.
    pub sweep_ms: u64,
    /// Enforce the `ORIGIN_OK` allow-list (L22-23). Tests that connect raw
    /// sockets without a browser-style Origin can switch this off.
    pub check_origin: bool,
    /// Room size cap (default [`MAX`]); parameterised so `'Room is full'`
    /// is reachable without opening eight sockets.
    pub room_max: usize,
}

impl Default for ShimConfig {
    fn default() -> Self {
        Self {
            msg_max: 65536,
            rate: 90,
            burst_strikes: 4,
            silent_match_ms: 20_000,
            silent_lobby_ms: 150_000,
            sweep_ms: 4_000,
            check_origin: true,
            room_max: MAX,
        }
    }
}

impl ShimConfig {
    /// Everything loose except the origin check: no size/rate cuts, no sweep
    /// pressure. Tests that need *specific* hygiene behaviour build their own
    /// config from [`Default`] instead.
    #[must_use]
    pub fn relaxed() -> Self {
        Self {
            msg_max: usize::MAX,
            rate: u32::MAX,
            burst_strikes: u32::MAX,
            silent_match_ms: u64::MAX,
            silent_lobby_ms: u64::MAX,
            sweep_ms: 3_600_000,
            check_origin: true,
            room_max: MAX,
        }
    }
}

/// One joined socket. `gone` is set by `_gone` (server L137) so a late close
/// never double-broadcasts `leave`; `stop` is the sweep's way to ask the
/// serving thread to close the socket (4001) and leave.
struct Member {
    id: SessionId,
    name: String,
    seq: u32,
    sink: Sender<String>,
    gone: AtomicBool,
    stop: Arc<AtomicBool>,
    /// `seen` (L50/L104): last activity, epoch ms.
    seen_ms: AtomicU64,
}

/// A room — the Durable Object of server/src/index.js, minus the hibernation
/// machinery (a test shim never sleeps).
struct Room {
    /// Live + gone members; `live()` filters, mirroring `members()` (L58).
    members: Mutex<Vec<Arc<Member>>>,
    locked: AtomicBool,
    seq: AtomicU32,
}

impl Room {
    fn new() -> Self {
        Self {
            members: Mutex::new(Vec::new()),
            locked: AtomicBool::new(false),
            seq: AtomicU32::new(0),
        }
    }

    /// `members()` (L58): not-gone, sorted by join order (seq).
    fn live(&self) -> Vec<Arc<Member>> {
        let mut ms: Vec<_> = self
            .members
            .lock()
            .unwrap()
            .iter()
            .filter(|m| !m.gone.load(Ordering::Acquire))
            .cloned()
            .collect();
        ms.sort_by_key(|m| m.seq);
        ms
    }

    /// `host()` (L60): the oldest live member.
    fn host(&self) -> Option<SessionId> {
        self.live().first().map(|m| m.id.clone())
    }

    /// `b|` fan-out (L113): every live member except the sender.
    fn fan_out(&self, skip: &str, frame: &str) {
        for m in self.live() {
            if m.id != skip {
                let _ = m.sink.send(frame.to_string());
            }
        }
    }

    /// `s|to|` routing (L120): the first member with this id — the sender
    /// included, exactly like the Worker.
    fn send_to(&self, to: &str, frame: &str) {
        for m in self.live() {
            if m.id == to {
                let _ = m.sink.send(frame.to_string());
                break;
            }
        }
    }

    /// `_gone` (L133-142): mark the member gone, broadcast `leave` with the
    /// re-elected host to the survivors, unlock an emptied room.
    fn gone(&self, id: &str) {
        if let Some(m) = self.members.lock().unwrap().iter().find(|m| m.id == id)
            && m.gone.swap(true, Ordering::AcqRel)
        {
            return; // already gone (L136 `a.gone` guard)
        }
        // L141 sends to `members()` — which still includes the departing
        // socket; its `send` throws and is swallowed there, so the shim
        // delivers to the survivors (the observable set is identical).
        let ms = self.live();
        if let Some(host) = ms.first() {
            let out = wire::encode_control(&ControlFrame::Leave {
                id: id.to_string(),
                host: host.id.clone(),
            });
            for m in &ms {
                let _ = m.sink.send(out.clone());
            }
        } else {
            self.locked.store(false, Ordering::Release); // L142
        }
    }
}

/// Shared core behind the accept/sweep threads and the public handle.
struct Core {
    cfg: ShimConfig,
    rooms: Mutex<HashMap<String, Arc<Room>>>,
    /// Deterministic session-id mint (see module docs): counter → 4-char
    /// uppercase base36, retried past live collisions like the JS `do/while`.
    id_next: AtomicU32,
    stop: Arc<AtomicBool>,
}

impl Core {
    fn room(&self, code: &str) -> Arc<Room> {
        let mut rs = self.rooms.lock().unwrap();
        Arc::clone(
            rs.entry(code.to_string())
                .or_insert_with(|| Arc::new(Room::new())),
        )
    }

    fn mint_id(&self, taken: &[SessionId]) -> SessionId {
        loop {
            let n = self.id_next.fetch_add(1, Ordering::Relaxed);
            let id = base36_id(n);
            if !taken.contains(&id) {
                return id;
            }
        }
    }
}

fn base36_id(mut n: u32) -> String {
    const D: &[u8; 36] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut out = [b'0'; 4];
    for i in (0..4).rev() {
        out[i] = D[(n % 36) as usize];
        n /= 36;
    }
    String::from_utf8(out.to_vec()).expect("base36 ascii")
}

fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A running shim. Holds the accept + sweep threads; [`Drop`] stops them and
/// asks every live socket to close. `url` is the `ws://127.0.0.1:<port>` base
/// a [`inkwave_sim::net::transport::Transport`] connects to.
pub struct Shim {
    /// `ws://127.0.0.1:<port>` — pass to `relay_url`/`NativeTransport::new`.
    pub url: String,
    core: Arc<Core>,
    accept: Option<JoinHandle<()>>,
    sweep: Option<JoinHandle<()>>,
}

impl Shim {
    /// Bind `127.0.0.1:0` and start the accept + sweep loops.
    pub fn spawn(cfg: ShimConfig) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        Ok(Self::with_listener(listener, port, cfg))
    }

    /// Bind an explicit address (the `main.rs` binary path).
    pub fn bind(addr: std::net::SocketAddr, cfg: ShimConfig) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        let port = listener.local_addr()?.port();
        Ok(Self::with_listener(listener, port, cfg))
    }

    fn with_listener(listener: TcpListener, port: u16, cfg: ShimConfig) -> Self {
        let core = Arc::new(Core {
            cfg,
            rooms: Mutex::new(HashMap::new()),
            id_next: AtomicU32::new(0),
            stop: Arc::new(AtomicBool::new(false)),
        });
        let url = format!("ws://127.0.0.1:{port}");

        // Accept loop (non-blocking poll so `Drop` can stop it promptly):
        // one thread per socket, like the Worker's per-DO message handlers.
        listener.set_nonblocking(true).expect("nonblocking");
        let accept = {
            let core = Arc::clone(&core);
            thread::spawn(move || {
                while !core.stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            // Accepted sockets inherit the listener's
                            // non-blocking flag on Linux — the handshake and
                            // the room loop need a blocking socket.
                            let _ = stream.set_nonblocking(false);
                            let core = Arc::clone(&core);
                            thread::spawn(move || serve_connection(stream, core));
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(20));
                        }
                        Err(_) => thread::sleep(Duration::from_millis(20)),
                    }
                }
            })
        };

        // Liveness sweep (server `alarm`, L90-97).
        let sweep = {
            let core = Arc::clone(&core);
            thread::spawn(move || {
                while !core.stop.load(Ordering::Acquire) {
                    thread::sleep(Duration::from_millis(core.cfg.sweep_ms.min(1_000)));
                    let now = epoch_ms();
                    let rooms: Vec<Arc<Room>> =
                        core.rooms.lock().unwrap().values().cloned().collect();
                    for r in rooms {
                        let limit = if r.locked.load(Ordering::Acquire) {
                            core.cfg.silent_match_ms
                        } else {
                            core.cfg.silent_lobby_ms
                        };
                        if limit == u64::MAX {
                            continue;
                        }
                        for m in r.live() {
                            if now.saturating_sub(m.seen_ms.load(Ordering::Acquire)) > limit {
                                m.stop.store(true, Ordering::Release);
                            }
                        }
                    }
                }
            })
        };

        Self {
            url,
            core,
            accept: Some(accept),
            sweep: Some(sweep),
        }
    }
}

/// Serve one TCP connection: run the pre-upgrade gates, then hand the socket
/// to the room's message loop.
// `ErrorResponse` is tungstenite's fixed `Callback` return type — we cannot
// box it without breaking `accept_hdr`.
#[allow(clippy::result_large_err)]
fn serve_connection(stream: TcpStream, core: Arc<Core>) {
    let cap: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let cb_core = Arc::clone(&core);
    let cb_cap = Arc::clone(&cap);
    let cb = move |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
        let target = req
            .uri()
            .path_and_query()
            .map(|p| p.as_str().to_string())
            .unwrap_or_default();
        let origin = req
            .headers()
            .get("Origin")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        // Pre-upgrade gates (server fetch(), L35-40): path shape, CODE, Origin.
        gate(&cb_core, &origin, &target, cb_cap, resp)
    };
    if let Ok(sock) = accept_hdr(stream, cb) {
        let target = cap.lock().unwrap().clone().unwrap_or_default();
        // Re-derive the room code + query for the ladder.
        let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
        let Some(code) = room_code(path) else { return };
        serve_client(sock, core, code.to_ascii_uppercase(), query.to_string());
    }
}

/// The gates that run *before* the upgrade (server L35-40), then capture the
/// request target for the join ladder.
#[allow(clippy::result_large_err)]
fn gate(
    core: &Core,
    origin: &str,
    target: &str,
    cap: Arc<Mutex<Option<String>>>,
    resp: Response,
) -> Result<Response, ErrorResponse> {
    let (path, _query) = target.split_once('?').unwrap_or((target, ""));
    let Some(code) = room_code(path) else {
        return Err(http_err(404, "INKWAVE relay"));
    };
    if !code_valid(&code.to_ascii_uppercase()) {
        return Err(http_err(400, "bad code"));
    }
    if core.cfg.check_origin && !origin_ok(origin) {
        return Err(http_err(403, "forbidden"));
    }
    *cap.lock().unwrap() = Some(target.to_string());
    Ok(resp)
}

fn http_err(code: u16, body: &str) -> ErrorResponse {
    Response::builder()
        .status(code)
        .body(Some(body.to_string()))
        .expect("static response")
}

/// `url.pathname.match(/^\/room\/([A-Za-z0-9]+)$/)` (server L35).
fn room_code(path: &str) -> Option<String> {
    let rest = path.strip_prefix("/room/")?;
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    Some(rest.to_string())
}

/// `CODE = /^[A-Z0-9]{4,8}$/` (server L29) on the uppercased code (L37).
fn code_valid(code: &str) -> bool {
    (4..=8).contains(&code.len())
        && code
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// `ORIGIN_OK` (server L22-23), hand-compiled from the two regexes:
/// `^https://([a-z0-9-]+\.)?inkwave-aah\.pages\.dev$` or
/// `^https?://(localhost|127.0.0.1|10.x.x.x|192.168.x.x|172.16-31.x.x|
/// [a-z0-9-]+\.local)(:\d+)?$`.
///
/// Known deviation: `parse::<u8>()` admits leading zeros, so an origin like
/// `http://172.016.0.1` passes here but the JS `\.`-anchored octet class
/// rejects it. Only dev-origin shapes are affected; the shim never gates
/// production traffic, so this is recorded rather than fixed.
#[must_use]
pub fn origin_ok(o: &str) -> bool {
    if let Some(h) = o.strip_prefix("https://") {
        if h == "inkwave-aah.pages.dev" {
            return true;
        }
        // `([a-z0-9-]+\.)?inkwave-aah\.pages\.dev` — exactly one label dot.
        if let Some(sub) = h.strip_suffix("inkwave-aah.pages.dev") {
            let Some(lab) = sub.strip_suffix('.') else {
                return false;
            };
            if !lab.is_empty()
                && !lab.contains('.')
                && lab
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            {
                return true;
            }
        }
    }
    let Some(rest) = o
        .strip_prefix("http://")
        .or_else(|| o.strip_prefix("https://"))
    else {
        return false;
    };
    let (host, port) = match rest.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (rest, None),
    };
    if let Some(p) = port
        && (p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    if host == "localhost" || host == "127.0.0.1" {
        return true;
    }
    if let Some(lab) = host.strip_suffix(".local") {
        return !lab.is_empty()
            && lab
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    }
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    let (a, b, c, d) = (parts[0], parts[1], parts[2], parts[3]);
    let oct = |s: &str| !s.is_empty() && s.bytes().all(|x| x.is_ascii_digit());
    if a == "10" && oct(b) && oct(c) && oct(d) {
        return true;
    }
    if a == "192" && b == "168" && oct(c) && oct(d) {
        return true;
    }
    if a == "172" && b.parse::<u8>().is_ok_and(|v| (16..=31).contains(&v)) && oct(c) && oct(d) {
        return true;
    }
    false
}

/// `name` sanitisation (server L76): strip everything outside
/// `\p{L}\p{N} ._\-!?`, keep the first 16 units, fall back to `'Player'`.
/// Rust `is_alphabetic`/`is_numeric` stand in for the Unicode classes; the
/// slice counts Unicode scalars (see module docs).
#[must_use]
pub fn clean_name(raw: &str) -> String {
    let kept: String = raw
        .chars()
        .filter(|c| c.is_alphabetic() || c.is_numeric() || " ._-!?".contains(*c) || *c == '\'')
        .take(16)
        .collect();
    if kept.is_empty() {
        "Player".to_string()
    } else {
        kept
    }
}

/// Percent-decode one query value (what `URLSearchParams.get` hands the
/// Worker). Only `%XX` is handled — `room_url` never emits `+`.
fn qs_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) =
                u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn qs_get(query: &str, key: &str) -> Option<String> {
    for kv in query.split('&') {
        let mut it = kv.splitn(2, '=');
        if it.next() == Some(key) {
            return it.next().map(qs_decode);
        }
    }
    None
}

/// Drive one joined socket: the join ladder, then the message loop (server
/// `fetch` + `webSocketMessage` + `webSocketClose`).
fn serve_client(mut sock: WebSocket<TcpStream>, core: Arc<Core>, code: String, query: String) {
    let room = core.room(&code);
    let cfg = &core.cfg;

    // `fail` (server L67): `{"t":"err",e}` then `close(4000, e)`.
    fn fail(sock: &mut WebSocket<TcpStream>, e: &str) {
        let _ = sock.send(Message::text(wire::encode_control(&ControlFrame::Err {
            e: e.to_string(),
        })));
        let _ = sock.close(Some(CloseFrame {
            code: CloseCode::from(4000),
            reason: e.into(),
        }));
        let _ = sock.flush();
    }

    // L70: `+(v || 0) !== PROTO`.
    let v_ok = qs_get(&query, "v").is_some_and(|v| v.parse::<f64>() == Ok(f64::from(PROTO_V)));
    if !v_ok {
        fail(&mut sock, "Please refresh the page — the game was updated");
        return;
    }
    let create = qs_get(&query, "create").as_deref() == Some("1");
    let name = qs_get(&query, "name")
        .map(|n| clean_name(&n))
        .unwrap_or_else(|| "Player".to_string());

    // Join ladder (L71-75) against the *other* live members.
    let others = room.live();
    if create && !others.is_empty() {
        fail(&mut sock, "Room code taken");
        return;
    }
    if !create && others.is_empty() {
        fail(&mut sock, "Room not found");
        return;
    }
    if others.len() >= cfg.room_max {
        fail(&mut sock, "Room is full");
        return;
    }
    if room.locked.load(Ordering::Acquire) && !others.is_empty() {
        fail(&mut sock, "Match in progress");
        return;
    }
    if others.is_empty() {
        room.locked.store(false, Ordering::Release); // L75
    }

    // Register (L78-84). ids mint deterministically, retried past live ids.
    let taken: Vec<SessionId> = others.iter().map(|m| m.id.clone()).collect();
    let id = core.mint_id(&taken);
    let (sink, rx) = channel::<String>();
    let stop = Arc::new(AtomicBool::new(false));
    let member = Arc::new(Member {
        id: id.clone(),
        name: name.clone(),
        seq: room.seq.fetch_add(1, Ordering::Relaxed),
        sink,
        gone: AtomicBool::new(false),
        stop: Arc::clone(&stop),
        seen_ms: AtomicU64::new(epoch_ms()),
    });
    room.members.lock().unwrap().push(Arc::clone(&member));
    let all = room.live();
    let host = all
        .first()
        .map(|m| m.id.clone())
        .unwrap_or_else(|| id.clone());
    let _ = member
        .sink
        .send(wire::encode_control(&ControlFrame::Welcome {
            id: id.clone(),
            host,
            members: all.iter().map(|m| (m.id.clone(), m.name.clone())).collect(),
        }));
    let j = wire::encode_control(&ControlFrame::Join {
        id: id.clone(),
        name,
    });
    room.fan_out(&id, &j);

    // Message loop. A 100 ms socket read timeout keeps the thread responsive
    // to the sweep's `stop` flag and to outbound frames while idle (a timed
    // out read surfaces as `Error::Io(WouldBlock|TimedOut)`).
    let _ = sock
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(100)));
    let mut rate = RateWindow::new(epoch_ms(), cfg.rate, cfg.burst_strikes);
    let mut exited = false;
    loop {
        while let Ok(frame) = rx.try_recv() {
            if sock.send(Message::text(frame)).is_err() {
                exited = true;
                break;
            }
        }
        if exited {
            break;
        }
        if stop.load(Ordering::Acquire) {
            // sweep verdict (server L94): close(4001, 'Connection timed out').
            let _ = sock.close(Some(CloseFrame {
                code: CloseCode::from(4001),
                reason: "Connection timed out".into(),
            }));
            let _ = sock.flush();
            break;
        }
        match sock.read() {
            Ok(Message::Text(t)) => {
                let s = t.as_str();
                member.seen_ms.store(epoch_ms(), Ordering::Release);
                if s == "ping" {
                    // Runtime auto-response (L52): echo, never touches state.
                    let _ = sock.send(Message::text("pong"));
                    continue;
                }
                // MSG_MAX (L105) — dropped, never fanned out.
                if s.chars().count() > cfg.msg_max {
                    continue;
                }
                // RATE hygiene (L106-109).
                if rate.tick(epoch_ms()) {
                    let _ = sock.close(Some(CloseFrame {
                        code: CloseCode::from(4008),
                        reason: "Too many messages".into(),
                    }));
                    let _ = sock.flush();
                    break;
                }
                relay_one(&room, &id, s, &mut sock);
            }
            Ok(Message::Close(_)) => break,
            Err(Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
            Ok(_) => {}
        }
    }
    room.gone(&id);
}

/// `webSocketMessage` body (server L110-127) for one non-ping text frame.
fn relay_one(room: &Room, from: &str, msg: &str, sock: &mut WebSocket<TcpStream>) {
    if let Some(rest) = msg.strip_prefix("b|") {
        room.fan_out(from, &format!("m|{from}|{rest}"));
        return;
    }
    if let Some(rest) = msg.strip_prefix("s|") {
        let Some(k) = rest.find('|') else { return };
        let to = &rest[..k];
        room.send_to(to, &format!("m|{from}|{}", &rest[k + 1..]));
        return;
    }
    if msg.as_bytes().first() == Some(&b'{')
        && let Some(o) = wire::parse_json(msg)
    {
        match o.field("t").and_then(JVal::as_str) {
            // `{"t":"ping","c":n}` → `{"t":"pong","c":n}` (L125). A missing
            // `c` stringifies to `{t:'pong'}` in JS — mirrored here.
            Some("ping") => {
                let out = match o.field("c").and_then(JVal::as_f64) {
                    Some(c) => wire::encode_control(&ControlFrame::Pong { c }),
                    None => r#"{"t":"pong"}"#.to_string(),
                };
                let _ = sock.send(Message::text(out));
            }
            // `lock` applies only when the sender is the host (L126).
            Some("lock") if room.host().as_deref() == Some(from) => {
                let v = o.field("v").is_some_and(|v| match v {
                    JVal::Bool(b) => *b,
                    JVal::Num(n) => *n != 0.0,
                    JVal::Str(s) => !s.is_empty(),
                    JVal::Null => false,
                    _ => true,
                });
                room.locked.store(v, Ordering::Release);
            }
            _ => {}
        }
    }
}

/// Per-socket rate window (server L106-109): a 1 s window accrues strikes
/// while over `RATE`; three times the budget, or `burst_strikes` strikes,
/// cuts the socket off.
struct RateWindow {
    t: u64,
    n: u32,
    strikes: u32,
    rate: u32,
    burst: u32,
}

impl RateWindow {
    fn new(now: u64, rate: u32, burst: u32) -> Self {
        Self {
            t: now,
            n: 0,
            strikes: 0,
            rate,
            burst,
        }
    }

    /// Count one message; `true` when the sender must be cut off.
    fn tick(&mut self, now: u64) -> bool {
        if now.saturating_sub(self.t) >= 1000 {
            self.strikes = if self.n > self.rate {
                self.strikes + 1
            } else {
                self.strikes.saturating_sub(1)
            };
            self.t = now;
            self.n = 0;
        }
        self.n += 1;
        self.n > self.rate.saturating_mul(3) || self.strikes >= self.burst
    }
}

impl Drop for Shim {
    fn drop(&mut self) {
        self.core.stop.store(true, Ordering::Release);
        // Ask every live member's thread to close its socket.
        let rooms: Vec<Arc<Room>> = self.core.rooms.lock().unwrap().values().cloned().collect();
        for r in &rooms {
            for m in r.live() {
                m.stop.store(true, Ordering::Release);
            }
        }
        if let Some(h) = self.accept.take() {
            let _ = h.join();
        }
        if let Some(h) = self.sweep.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpStream;

    use tungstenite::Message;
    use tungstenite::client::{IntoClientRequest, connect};
    use tungstenite::stream::MaybeTlsStream;

    use super::*;

    /// One raw client socket with a read timeout so tests never hang.
    struct Client {
        sock: WebSocket<MaybeTlsStream<TcpStream>>,
    }

    impl Client {
        /// Upgrade against the shim with a compliant Origin (browser shape).
        fn join(url_base: &str, path: &str) -> Self {
            let full = format!("{url_base}{path}");
            let origin = format!("http://{}", &url_base["ws://".len()..]);
            let mut req = full.into_client_request().unwrap();
            req.headers_mut().insert("Origin", origin.parse().unwrap());
            let (sock, _) = connect(req).unwrap();
            if let MaybeTlsStream::Plain(s) = sock.get_ref() {
                let _ = s.set_read_timeout(Some(Duration::from_millis(2_000)));
            }
            Self { sock }
        }

        /// Arm a per-call read timeout on the plain test socket.
        fn set_timeout(&self, ms: u64) {
            if let MaybeTlsStream::Plain(s) = self.sock.get_ref() {
                let _ = s.set_read_timeout(Some(Duration::from_millis(ms)));
            }
        }

        fn send(&mut self, s: &str) {
            self.sock.send(Message::text(s.to_string())).unwrap();
        }

        /// Next text frame (panics on timeout — tests sequence reads so that
        /// never happens).
        fn text(&mut self) -> String {
            loop {
                match self.sock.read().unwrap() {
                    Message::Text(t) => return t.to_string(),
                    Message::Ping(p) => {
                        let _ = self.sock.send(Message::Pong(p));
                    }
                    other => panic!("unexpected frame: {other:?}"),
                }
            }
        }

        /// Read until a Close frame arrives (or timeout → None).
        fn wait_close(&mut self) -> Option<(u16, String)> {
            loop {
                match self.sock.read() {
                    Ok(Message::Close(Some(f))) => {
                        return Some((f.code.into(), f.reason.to_string()));
                    }
                    Ok(Message::Close(None)) => return Some((1005, String::new())),
                    Ok(_) => {}
                    Err(_) => return None,
                }
            }
        }

        /// True when nothing arrives within `ms` (used to assert the `b|`
        /// exclusion — the sender must NOT see its own broadcast).
        fn quiet(&mut self, ms: u64) -> bool {
            self.set_timeout(ms);
            let r = self.sock.read();
            self.set_timeout(2_000);
            match r {
                Err(Error::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    true
                }
                Ok(Message::Close(_)) | Err(_) => true,
                _ => false,
            }
        }
    }

    fn welcome_of(frame: &str) -> (SessionId, SessionId, Vec<(SessionId, String)>) {
        let v = wire::parse_json(frame).expect("welcome json");
        match wire::decode_control(&v).expect("welcome frame") {
            ControlFrame::Welcome { id, host, members } => (id, host, members),
            f => panic!("not a welcome: {f:?}"),
        }
    }

    // -- TR-19.1: the 3-socket scenario, full event sequences ----------------

    #[test]
    fn three_sockets_welcome_election_fanout_leave_and_lock() {
        let shim = Shim::spawn(ShimConfig::relaxed()).unwrap();
        let base = shim.url.as_str();

        // A creates the room.
        let mut a = Client::join(base, "/room/ABCD?name=Alice&v=1&create=1");
        let (id_a, host_a, roster_a) = welcome_of(&a.text());
        assert_eq!(host_a, id_a, "solo join elects itself");
        assert_eq!(roster_a, vec![(id_a.clone(), "Alice".to_string())]);

        // B joins: welcome roster has both, host stays A; A sees the join.
        let mut b = Client::join(base, "/room/ABCD?name=Bob&v=1");
        let (id_b, host_b, roster_b) = welcome_of(&b.text());
        assert_eq!(host_b, id_a);
        assert_eq!(roster_b.len(), 2);
        assert_eq!(roster_b[1], (id_b.clone(), "Bob".to_string()));
        let j = wire::parse_json(&a.text()).unwrap();
        assert_eq!(
            wire::decode_control(&j),
            Some(ControlFrame::Join {
                id: id_b.clone(),
                name: "Bob".to_string()
            })
        );

        // C joins the same way.
        let mut c = Client::join(base, "/room/ABCD?name=Cara&v=1");
        let (id_c, host_c, roster_c) = welcome_of(&c.text());
        assert_eq!(host_c, id_a);
        assert_eq!(roster_c.len(), 3);
        for x in [&mut a, &mut b] {
            let j = wire::parse_json(&x.text()).unwrap();
            assert!(matches!(
                wire::decode_control(&j),
                Some(ControlFrame::Join { id, .. }) if id == id_c
            ));
        }

        // `b|` fan-out: everyone else sees `m|A|…`, the sender sees nothing.
        a.send(&format!("b|{}", r#"{"k":"t","x":1}"#));
        let want = format!("m|{id_a}|{}", r#"{"k":"t","x":1}"#);
        assert_eq!(b.text(), want);
        assert_eq!(c.text(), want);
        assert!(a.quiet(150), "b| must not loop back to the sender");

        // `s|to|` routing includes the sender (sendTo self is reachable).
        b.send(&format!("s|{id_b}|self"));
        assert_eq!(b.text(), format!("m|{id_b}|self"));
        b.send(&format!("s|{id_c}|ping-c"));
        assert_eq!(c.text(), format!("m|{id_b}|ping-c"));
        assert!(b.quiet(150), "s| to C must not also reach B");

        // Host-only lock: B (not host) is ignored; A (host) locks the room.
        b.send(r#"{"t":"lock","v":true}"#);
        assert!(b.quiet(150));
        // While B's bogus lock had no effect, a join still works…
        let mut d = Client::join(base, "/room/ABCD?name=Dana&v=1");
        let (id_d, host_d, _) = welcome_of(&d.text());
        assert_eq!(host_d, id_a);
        // Drain D's join from the other survivors so later reads see `leave`.
        for x in [&mut a, &mut c] {
            let j = wire::parse_json(&x.text()).unwrap();
            assert!(matches!(
                wire::decode_control(&j),
                Some(ControlFrame::Join { id, .. }) if id == id_d
            ));
        }
        a.send(r#"{"t":"lock","v":true}"#);
        // …now joins are refused with the exact ladder text (L74).
        let mut e = Client::join(base, "/room/ABCD?name=Eve&v=1");
        let err = wire::parse_json(&e.text()).unwrap();
        assert_eq!(
            wire::decode_control(&err),
            Some(ControlFrame::Err {
                e: "Match in progress".to_string()
            })
        );
        assert_eq!(
            e.wait_close(),
            Some((4000, "Match in progress".to_string()))
        );

        // Leave + host migration: drop B (middle member) → survivors see
        // `leave` with host still A. Then drop A (the host) → host moves to
        // the oldest survivor C (B is gone, so order is A,C,D).
        drop(b);
        let mut seen_leave = 0;
        for x in [&mut a, &mut c, &mut d] {
            let l = wire::parse_json(&x.text()).unwrap();
            match wire::decode_control(&l) {
                Some(ControlFrame::Leave { id, host }) => {
                    assert_eq!(id, id_b);
                    assert_eq!(host, id_a);
                    seen_leave += 1;
                }
                f => panic!("expected leave, got {f:?}"),
            }
        }
        assert_eq!(seen_leave, 3);

        drop(a);
        for x in [&mut c, &mut d] {
            let l = wire::parse_json(&x.text()).unwrap();
            match wire::decode_control(&l) {
                Some(ControlFrame::Leave { id, host }) => {
                    assert_eq!(id, id_a);
                    assert_eq!(host, id_c, "host migrates to oldest survivor");
                }
                f => panic!("expected leave, got {f:?}"),
            }
        }

        // Last member leaves → the room auto-unlocks (L142).
        drop(c);
        let l = wire::parse_json(&d.text()).unwrap();
        assert!(matches!(
            wire::decode_control(&l),
            Some(ControlFrame::Leave { .. })
        ));
        drop(d);
        // Room now empty: a fresh create must succeed despite the old lock.
        let mut f = Client::join(base, "/room/ABCD?name=Zoe&v=1&create=1");
        let (id_f, host_f, roster_f) = welcome_of(&f.text());
        assert_eq!(host_f, id_f, "solo again");
        assert_eq!(roster_f.len(), 1);
    }

    #[test]
    fn join_ladder_texts_and_runtime_ping() {
        let shim = Shim::spawn(ShimConfig::relaxed()).unwrap();
        let base = shim.url.as_str();

        // Wrong protocol version first (L70).
        let mut bad = Client::join(base, "/room/ABCD?name=X&v=0");
        let e = wire::parse_json(&bad.text()).unwrap();
        assert_eq!(
            wire::decode_control(&e),
            Some(ControlFrame::Err {
                e: "Please refresh the page — the game was updated".to_string()
            })
        );

        // Joining a room nobody holds.
        let mut ghost = Client::join(base, "/room/ZZZZ?name=X&v=1");
        let e = wire::parse_json(&ghost.text()).unwrap();
        assert_eq!(
            wire::decode_control(&e),
            Some(ControlFrame::Err {
                e: "Room not found".to_string()
            })
        );

        // Create, then a second create collides.
        let mut a = Client::join(base, "/room/QQQQ?name=A&v=1&create=1");
        let _ = a.text();
        let mut dup = Client::join(base, "/room/QQQQ?name=B&v=1&create=1");
        let e = wire::parse_json(&dup.text()).unwrap();
        assert_eq!(
            wire::decode_control(&e),
            Some(ControlFrame::Err {
                e: "Room code taken".to_string()
            })
        );

        // Room size cap (parameterised: room_max = 2 here).
        let cfg = ShimConfig {
            room_max: 2,
            ..ShimConfig::relaxed()
        };
        let shim2 = Shim::spawn(cfg).unwrap();
        let mut x = Client::join(&shim2.url, "/room/WWWW?name=X&v=1&create=1");
        let _ = x.text();
        let mut y = Client::join(&shim2.url, "/room/WWWW?name=Y&v=1");
        let _ = y.text();
        let mut z = Client::join(&shim2.url, "/room/WWWW?name=Z&v=1");
        let e = wire::parse_json(&z.text()).unwrap();
        assert_eq!(
            wire::decode_control(&e),
            Some(ControlFrame::Err {
                e: "Room is full".to_string()
            })
        );

        // Runtime heartbeat: bare "ping" → "pong" (L52) and the old-style
        // {"t":"ping","c"} → {"t":"pong","c"} (L125).
        a.send("ping");
        assert_eq!(a.text(), "pong");
        a.send(r#"{"t":"ping","c":12.5}"#);
        let p = wire::parse_json(&a.text()).unwrap();
        assert_eq!(
            wire::decode_control(&p),
            Some(ControlFrame::Pong { c: 12.5 })
        );
    }

    #[test]
    fn hygiene_sweep_drops_a_silent_socket() {
        // Lobby silence budget of 300 ms, sweep every 100 ms.
        let cfg = ShimConfig {
            silent_lobby_ms: 300,
            silent_match_ms: u64::MAX,
            sweep_ms: 100,
            ..ShimConfig::relaxed()
        };
        let shim = Shim::spawn(cfg).unwrap();
        let mut a = Client::join(&shim.url, "/room/SSSS?name=A&v=1&create=1");
        let _ = a.text();
        // Stay silent → the sweep closes us with 4001 (server L94).
        let (code, reason) = a.wait_close().expect("sweep closes the socket");
        assert_eq!(code, 4001);
        assert_eq!(reason, "Connection timed out");
    }

    #[test]
    fn hygiene_rate_and_msg_max() {
        let cfg = ShimConfig {
            rate: 2,
            msg_max: 8,
            ..ShimConfig::relaxed()
        };
        let shim = Shim::spawn(cfg).unwrap();
        let mut a = Client::join(&shim.url, "/room/RRRR?name=A&v=1&create=1");
        let (id_a, _, _) = welcome_of(&a.text());
        let mut b = Client::join(&shim.url, "/room/RRRR?name=B&v=1");
        let _ = b.text();

        // Oversized frames are dropped silently (L105): b must not see it.
        a.send("b|0123456789ABCDEFGHIJ"); // 22 chars > msg_max 8
        assert!(b.quiet(150), "oversized frame must not fan out");

        // Within size: relayed.
        a.send("b|ok");
        assert_eq!(b.text(), format!("m|{id_a}|ok"));

        // Flood past 3× RATE → close(4008, 'Too many messages') (L109).
        for _ in 0..40 {
            let _ = a.sock.send(Message::text("b|1".to_string()));
        }
        let (code, reason) = a.wait_close().expect("rate cutoff");
        assert_eq!(code, 4008);
        assert_eq!(reason, "Too many messages");
    }

    // -- pure helpers ----------------------------------------------------------

    #[test]
    fn origin_ok_matches_worker_regex() {
        // server/src/index.js L22-23 admits…
        assert!(origin_ok("https://inkwave-aah.pages.dev"));
        assert!(origin_ok("https://www.inkwave-aah.pages.dev"));
        assert!(origin_ok("http://localhost:8787"));
        assert!(origin_ok("http://127.0.0.1:34567"));
        assert!(origin_ok("https://10.1.2.3"));
        assert!(origin_ok("http://192.168.0.7:8080"));
        assert!(origin_ok("http://172.16.0.1"));
        assert!(origin_ok("http://172.31.255.255"));
        assert!(origin_ok("http://myhost.local:5000"));
        // …and rejects everything else.
        assert!(!origin_ok(""));
        assert!(!origin_ok("http://evil.example.com"));
        assert!(!origin_ok("https://inkwave-aah.pages.dev.evil.com"));
        assert!(!origin_ok("https://x..inkwave-aah.pages.dev"));
        assert!(!origin_ok("http://localhost:abc"));
        assert!(!origin_ok("http://localhost:"));
        assert!(!origin_ok("http://172.15.0.1"));
        assert!(!origin_ok("http://172.32.0.1"));
        assert!(!origin_ok("http://10."));
        assert!(!origin_ok("ftp://localhost"));
        // `[a-z0-9-]+` admits the leading hyphen — the Worker regex does too.
        assert!(origin_ok("http://-bad.local"));
    }

    #[test]
    fn code_valid_is_uppercase_alnum_4_to_8() {
        assert!(code_valid("ABCD"));
        assert!(code_valid("A1B2C3D4"));
        assert!(!code_valid("ABC"));
        assert!(!code_valid("ABCDEFG9X"));
        assert!(!code_valid("AB-C"));
        assert!(!code_valid("abcd"));
    }

    #[test]
    fn clean_name_strips_and_falls_back() {
        assert_eq!(clean_name("Alice"), "Alice");
        assert_eq!(clean_name("A b!c?d.'e-1"), "A b!c?d.'e-1");
        assert_eq!(clean_name("emoji😀name"), "emojiname");
        assert_eq!(clean_name("<script>"), "script");
        assert_eq!(clean_name("12345678901234567890"), "1234567890123456");
        assert_eq!(clean_name(""), "Player");
        assert_eq!(clean_name("😀😀"), "Player");
        assert_eq!(clean_name("中文名字"), "中文名字");
    }

    #[test]
    fn base36_ids_have_js_shape() {
        assert_eq!(base36_id(0), "0000");
        assert_eq!(base36_id(35), "000Z");
        assert_eq!(base36_id(36), "0010");
        assert_eq!(base36_id(36_u32.pow(4) - 1), "ZZZZ");
    }
}
