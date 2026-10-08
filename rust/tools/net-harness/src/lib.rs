//! Task 19 net-harness — a TEST-ONLY multi-client headless driver.
//!
//! Not a game binary, never shipped, never deployed. It exercises the whole
//! Rust client network stack the way the later match tasks (21–24) will, in
//! two modes:
//!
//! - **fake mode**: `FakeRelay`/`FakeTransport` on a virtual clock with the
//!   seeded `?netlag/&netjitter/&netspike` SIM — fully deterministic, so the
//!   same scenario can be replayed and diffed (TR-18.1 determinism, reused
//!   for scenario-level lag injection here).
//! - **shim mode**: real `tungstenite` sockets against the test-only
//!   `relay-shim` (the `server/src/index.js` re-implementation). Four clients
//!   speak the PROTO v1 wire *raw*; a fifth drives the frames through the
//!   production `inkwave_sim::net::transport::RxState` handler — the exact
//!   code path the native/wasm transports run — so the report cross-checks
//!   the shim against the client stack, not just raw bytes.
//!
//! Each mode joins N clients, injects a lag scenario, runs a broadcast /
//! targeted-send / lock / leave sequence, then collects a per-client state
//! snapshot and prints a consistency report. `cargo run -p net-harness
//! [fake|shim|all]` prints it; the integration tests assert on the same
//! results.
//!
//! Also lands the Task 18 review minors reserved for this harness
//! (tasks.md L68): ② the fake relay now answers the runtime `ping`→`pong`
//! heartbeat, so the 2 s cadence and the multi-sample RTT are asserted
//! end-to-end (`heartbeat_samples`); ⑥ the SIM's two-draw order (jitter
//! first, spike second) is pinned by a replayed-xorshift unit test in
//! `transport.rs`.

use std::net::TcpStream;
use std::time::{Duration, Instant};

use tungstenite::client::IntoClientRequest;
use tungstenite::http::HeaderValue;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket, connect};

use inkwave_sim::net::ControlFrame;
pub use inkwave_sim::net::transport::{
    FakeRelay, FakeTransport, LinkSim, PING_INTERVAL_MS, RxState, Transport, TransportEvent,
    room_url,
};
use inkwave_sim::net::wire::{self};
use relay_shim::{Shim, ShimConfig};

/// The scenario every mode runs: N clients join one room, exchange payloads,
/// the host locks, a stranger is refused, one client leaves.
pub const N: usize = 4;

// ---------------------------------------------------------------------------
// Snapshots & the consistency report
// ---------------------------------------------------------------------------

/// What one client believes the room looks like, after draining its events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Our relay session id (from `welcome`).
    pub id: String,
    /// The host we believe in (from `welcome` / the last `leave`).
    pub host: String,
    /// Full roster in join order (welcome roster + observed joins − leaves).
    pub roster: Vec<String>,
    /// Relay payload ids (`{"i":n}`) we received, in arrival order.
    pub payloads: Vec<u32>,
    /// Whether our own socket is open.
    pub open: bool,
    /// The connect refusal text, if we were refused.
    pub err: Option<String>,
}

impl Snapshot {
    #[must_use]
    fn blank() -> Self {
        Self {
            id: String::new(),
            host: String::new(),
            roster: Vec::new(),
            payloads: Vec::new(),
            open: false,
            err: None,
        }
    }

    /// Fold one delivered event into the belief state.
    fn apply(&mut self, ev: &TransportEvent) {
        match ev {
            TransportEvent::Welcome { id, host, members } => {
                self.id = id.clone();
                self.host = host.clone();
                self.roster = members.iter().map(|(i, _)| i.clone()).collect();
                self.open = true;
            }
            TransportEvent::Join { id, .. } => {
                if !self.roster.contains(id) {
                    self.roster.push(id.clone());
                }
            }
            TransportEvent::Leave { id, host } => {
                self.roster.retain(|m| m != id);
                self.host = host.clone();
            }
            TransportEvent::Message { payload, .. } => {
                if let Some(v) = wire::parse_json(payload)
                    && let Some(n) = v.field("i").and_then(wire::JVal::as_f64)
                {
                    self.payloads.push(n as u32);
                }
            }
            TransportEvent::Error { e } => self.err = Some(e.clone()),
            TransportEvent::Closed { .. } => self.open = false,
        }
    }

    /// Did two clients see exactly the same room?
    #[must_use]
    pub fn agrees_with(&self, other: &Snapshot) -> bool {
        self.roster == other.roster && self.host == other.host
    }
}

/// The outcome of one scenario run, ready to be asserted or printed.
#[derive(Debug)]
pub struct Report {
    pub mode: &'static str,
    /// Snapshots of the `N` in-room clients, join order.
    pub clients: Vec<Snapshot>,
    /// Snapshot of the refused stranger.
    pub stranger: Snapshot,
    /// RTT per client, ms (fake: exact virtual-clock value; shim: wall-clock).
    pub rtts: Vec<f64>,
    pub checks: Vec<(&'static str, bool)>,
}

impl Report {
    #[must_use]
    pub fn ok(&self) -> bool {
        self.checks.iter().all(|(_, v)| *v)
    }

    pub fn print(&self) {
        println!("=== net-harness report: {} mode ===", self.mode);
        for (i, c) in self.clients.iter().enumerate() {
            println!(
                "  client[{i}] id={} host={} roster={:?} payloads={:?} open={} err={:?}",
                c.id, c.host, c.roster, c.payloads, c.open, c.err
            );
        }
        let s = &self.stranger;
        println!(
            "  stranger id={:?} open={} err={:?} payloads={}",
            s.id,
            s.open,
            s.err,
            s.payloads.len()
        );
        println!("  rtt(ms): {:?}", self.rtts);
        for (name, pass) in &self.checks {
            println!("  [{}] {name}", if *pass { "PASS" } else { "FAIL" });
        }
        println!(
            "  overall: {}",
            if self.ok() {
                "CONSISTENT"
            } else {
                "INCONSISTENT"
            }
        );
    }
}

// ---------------------------------------------------------------------------
// Raw socket client (shim mode)
// ---------------------------------------------------------------------------

/// Connect one raw blocking WS client to the shim with a compliant Origin
/// (the allow-list admits `http://<host>:<port>`).
fn connect_raw(
    shim_url: &str,
    code: &str,
    name: &str,
    create: bool,
) -> Result<WebSocket<MaybeTlsStream<TcpStream>>, String> {
    let url = room_url(shim_url, code, name, create);
    let host = shim_url
        .split_once("://")
        .map(|(_, r)| r)
        .unwrap_or(shim_url);
    let mut req = url
        .as_str()
        .into_client_request()
        .map_err(|e| format!("bad request: {e}"))?;
    req.headers_mut().insert(
        "Origin",
        HeaderValue::from_str(&format!("http://{host}")).map_err(|e| e.to_string())?,
    );
    let (sock, _resp) = connect(req).map_err(|e| format!("connect: {e}"))?;
    Ok(sock)
}

fn arm_timeout(sock: &mut WebSocket<MaybeTlsStream<TcpStream>>, ms: u64) {
    if let MaybeTlsStream::Plain(t) = sock.get_mut() {
        let _ = t.set_read_timeout(Some(Duration::from_millis(ms)));
    }
}

fn control_kind(cf: &ControlFrame) -> &'static str {
    match cf {
        ControlFrame::Welcome { .. } => "welcome",
        ControlFrame::Join { .. } => "join",
        ControlFrame::Leave { .. } => "leave",
        ControlFrame::Err { .. } => "err",
        ControlFrame::Pong { .. } => "pong",
        ControlFrame::Lock { .. } => "lock",
    }
}

/// A minimal blocking WS client speaking the PROTO v1 wire directly — this
/// side never touches `RxState`, so the report compares the shim's raw
/// behaviour against the production frame handler.
struct RawClient {
    sock: WebSocket<MaybeTlsStream<TcpStream>>,
    /// Text frames received so far.
    inbox: Vec<String>,
}

impl RawClient {
    fn join(shim_url: &str, code: &str, name: &str, create: bool) -> Result<Self, String> {
        let mut sock = connect_raw(shim_url, code, name, create)?;
        arm_timeout(&mut sock, 50);
        Ok(Self {
            sock,
            inbox: Vec::new(),
        })
    }

    fn send(&mut self, s: &str) {
        let _ = self.sock.send(Message::text(s.to_string()));
    }

    /// Drain everything readable within `ms`, appending to `inbox`.
    fn pump(&mut self, ms: u64) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(ms) {
            match self.sock.read() {
                Ok(Message::Text(t)) => self.inbox.push(t.to_string()),
                Ok(Message::Ping(p)) => {
                    let _ = self.sock.send(Message::Pong(p));
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(ref e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => break,
            }
        }
    }

    /// Pop the first inbox frame that decodes to control kind `want`.
    fn take_control(&mut self, want: &str) -> Option<ControlFrame> {
        let pos = self.inbox.iter().position(|f| {
            f.starts_with('{')
                && wire::parse_json(f)
                    .and_then(|v| wire::decode_control(&v))
                    .is_some_and(|cf| control_kind(&cf) == want)
        })?;
        let f = self.inbox.remove(pos);
        wire::decode_control(&wire::parse_json(&f)?)
    }

    /// Pump until a control frame of kind `want` shows up (bounded by `ms`).
    fn wait_control(&mut self, want: &str, ms: u64) -> Option<ControlFrame> {
        let deadline = Instant::now() + Duration::from_millis(ms);
        loop {
            if let Some(cf) = self.take_control(want) {
                return Some(cf);
            }
            if Instant::now() >= deadline {
                return None;
            }
            self.pump(50);
        }
    }

    /// Pop the first `m|<from>|<json>` frame whose payload parses.
    fn take_message(&mut self) -> Option<String> {
        let pos = self.inbox.iter().position(|f| f.starts_with("m|"))?;
        let f = self.inbox.remove(pos);
        let rest = &f[2..];
        let k = rest.find('|')?;
        let payload = &rest[k + 1..];
        wire::parse_json(payload)?;
        Some(payload.to_string())
    }

    /// Pump until one `m|` frame arrives (bounded by `ms`).
    fn wait_message(&mut self, ms: u64) -> Option<String> {
        let deadline = Instant::now() + Duration::from_millis(ms);
        loop {
            if let Some(p) = self.take_message() {
                return Some(p);
            }
            if Instant::now() >= deadline {
                return None;
            }
            self.pump(50);
        }
    }

    /// Pop the bare `"pong"` the relay runtime answers to our `"ping"`.
    fn take_pong(&mut self) -> bool {
        if let Some(pos) = self.inbox.iter().position(|f| f == "pong") {
            self.inbox.remove(pos);
            true
        } else {
            false
        }
    }
}

/// The `i` number of a relay payload (`{"i":n}`).
fn payload_i(payload: &str) -> u32 {
    wire::parse_json(payload)
        .and_then(|v| v.field("i").and_then(wire::JVal::as_f64))
        .expect("payload i") as u32
}

/// A raw client whose *frame handling* goes through the production
/// `RxState` — the exact code path the native and wasm transports run,
/// including the welcome-triggered 2 s heartbeat and the RTT EMA.
struct ProdClient {
    sock: WebSocket<MaybeTlsStream<TcpStream>>,
    rx: RxState,
    snap: Snapshot,
    next_ping: f64,
    base: Instant,
}

impl ProdClient {
    fn join(shim_url: &str, code: &str, name: &str, create: bool) -> Self {
        let mut sock = connect_raw(shim_url, code, name, create).expect("prod join");
        arm_timeout(&mut sock, 20);
        Self {
            sock,
            rx: RxState::default(),
            snap: Snapshot::blank(),
            next_ping: f64::INFINITY,
            base: Instant::now(),
        }
    }

    /// Read for up to `ms`, feeding `RxState.handle` exactly like
    /// `native_transport::io_loop` does.
    fn pump(&mut self, ms: u64) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(ms) {
            let now = self.base.elapsed().as_secs_f64() * 1000.0;
            // Heartbeat (transport.js L82-90): first ping on welcome, then
            // every 2 s; `_pingSent` times the oldest unanswered one.
            if self.snap.open && now >= self.next_ping {
                if self.rx.ping_sent.is_none() {
                    self.rx.ping_sent = Some(now);
                }
                let _ = self.sock.send(Message::text("ping"));
                self.next_ping += PING_INTERVAL_MS;
            }
            match self.sock.read() {
                Ok(Message::Text(t)) => {
                    let out = self.rx.handle(t.as_str(), now);
                    if out.welcome {
                        self.rx.settled = true;
                        self.next_ping = now;
                    }
                    for ev in out.events {
                        self.snap.apply(&ev);
                    }
                }
                Ok(Message::Close(_)) => break,
                Err(tungstenite::Error::Io(ref e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => break,
                Ok(_) => {}
            }
        }
    }

    #[must_use]
    fn rtt(&self) -> f64 {
        self.rx.rtt
    }
}

// ---------------------------------------------------------------------------
// Fake mode: virtual clock, seeded SIM
// ---------------------------------------------------------------------------

fn drain_all(eps: &mut [FakeTransport], snaps: &mut [Snapshot], steps: usize, dt: f64) {
    for _ in 0..steps {
        for ep in eps.iter_mut() {
            ep.advance(dt);
        }
    }
    for (i, ep) in eps.iter_mut().enumerate() {
        for ev in ep.poll_events() {
            snaps[i].apply(&ev);
        }
    }
}

/// Run the N-client scenario on a `FakeRelay` with one `LinkSim` per client.
/// `seed` drives every endpoint's RNG (TR-18.1 determinism).
pub fn run_fake(sim: Option<LinkSim>, seed: u64) -> Report {
    let relay = FakeRelay::new();
    let mut eps: Vec<FakeTransport> = (0..N)
        .map(|i| relay.transport(sim, seed.wrapping_mul(31).wrapping_add(i as u64 + 1)))
        .collect();
    let mut snaps: Vec<Snapshot> = (0..N).map(|_| Snapshot::blank()).collect();

    // Everyone joins one room (client 0 creates).
    for (i, ep) in eps.iter_mut().enumerate() {
        ep.connect("HARNESS", &format!("c{i}"), i == 0);
    }
    drain_all(&mut eps, &mut snaps, 50, 10.0);
    let ids: Vec<String> = snaps.iter().map(|s| s.id.clone()).collect();

    // Broadcast one payload per client, then a targeted 0→1.
    let mut expected: Vec<Vec<u32>> = (0..N)
        .map(|r| (0..N as u32).filter(|s| *s != r as u32).collect())
        .collect();
    for (s, ep) in eps.iter_mut().enumerate() {
        ep.broadcast(&format!("{{\"i\":{s}}}"));
    }
    drain_all(&mut eps, &mut snaps, 40, 20.0);
    eps[0].send_to(&ids[1], "{\"i\":100}");
    expected[1].push(100);
    drain_all(&mut eps, &mut snaps, 40, 20.0);

    // Host (client 0) locks; the stranger joins and is refused.
    assert!(eps[0].lock(true));
    let mut stranger = relay.transport(sim, seed.wrapping_mul(31).wrapping_add(999));
    stranger.connect("HARNESS", "x", false);
    stranger.advance(500.0);
    let mut s_snap = Snapshot::blank();
    for ev in stranger.poll_events() {
        s_snap.apply(&ev);
    }
    s_snap.open = stranger.is_open();

    // Client 2 leaves; the survivors re-elect.
    eps[2].close();
    drain_all(&mut eps, &mut snaps, 40, 20.0);
    snaps[2].open = false;

    let rtts: Vec<f64> = eps.iter().map(|e| e.rtt()).collect();
    let mut checks: Vec<(&str, bool)> = Vec::new();

    // Rosters/host agree across the survivors.
    let mut all_agree = true;
    for a in snaps.iter() {
        for b in snaps.iter() {
            if a.open && b.open && !a.agrees_with(b) {
                all_agree = false;
            }
        }
    }
    checks.push(("survivor rosters/host agree", all_agree));

    // Every live client received every relay payload exactly once, in send
    // order (the SIM never reorders — monotone `last` floor).
    let order_ok = (0..N)
        .filter(|&i| i != 2)
        .all(|i| snaps[i].payloads == expected[i]);
    checks.push(("payload order identical on every survivor", order_ok));

    // The stranger saw no payload and got the exact refusal text.
    checks.push((
        "stranger refused 'Match in progress', no payloads",
        s_snap.err.as_deref() == Some("Match in progress")
            && s_snap.payloads.is_empty()
            && !s_snap.open,
    ));

    // Host stays the oldest survivor after client 2 leaves.
    let host_ok = snaps.iter().all(|s| !s.open || s.host == ids[0]);
    checks.push(("host stays the oldest survivor after leave", host_ok));

    // Lock truthfulness (Task 18 minor ①): the relay's own `locked` view is
    // observable through a fresh probe join.
    let mut probe = relay.transport(None, 0xdead_beef);
    probe.connect("HARNESS", "probe", false);
    probe.advance(0.0);
    let probe_refused = probe
        .poll_events()
        .iter()
        .any(|e| matches!(e, TransportEvent::Error { e } if e == "Match in progress"));
    checks.push(("relay.locked observable via probe join", probe_refused));

    // RTT: a lagless link answers every ping at the same virtual instant
    // (EMA 0); with a SIM the hold is `lag + rand*jitter + spike` per
    // sample, so the EMA can only ever sit at or above `lag_ms`.
    let rtt_ok = match sim {
        None => rtts.iter().all(|r| *r == 0.0),
        Some(s) => rtts.iter().all(|r| *r >= s.lag_ms),
    };
    checks.push(("rtt matches the link sim", rtt_ok));

    Report {
        mode: "fake",
        clients: snaps,
        stranger: s_snap,
        rtts,
        checks,
    }
}

/// Drive a fake link for `ticks` × 10 ms and return the per-tick RTT
/// samples plus the total heartbeat bytes sent (Task 18 minor ②: the fake
/// relay now answers the runtime `ping`→`pong`, so the 2 s cadence and the
/// multi-sample EMA are observable end-to-end). 10 ms steps keep the pong
/// delivery from being quantised upward by a coarse virtual tick.
#[must_use]
pub fn heartbeat_samples(sim: Option<LinkSim>, ticks: usize) -> (Vec<f64>, usize) {
    let relay = FakeRelay::new();
    let mut a = relay.transport(sim, 7);
    a.connect("HBTEST", "a", true);
    a.advance(0.0);
    let mut out = Vec::with_capacity(ticks);
    for _ in 0..ticks {
        a.advance(10.0);
        out.push(a.rtt());
    }
    let bytes = a.bytes_out();
    (out, bytes)
}

// ---------------------------------------------------------------------------
// Shim mode: raw sockets + the production RxState handler
// ---------------------------------------------------------------------------

/// Run the N-client scenario against a freshly spawned shim.
pub fn run_shim() -> Report {
    let shim = Shim::spawn(ShimConfig::relaxed()).expect("shim spawn");
    let mut raw: Vec<Option<RawClient>> = Vec::new();
    let mut ids: Vec<String> = Vec::new();
    let mut snaps: Vec<Snapshot> = Vec::new();

    for i in 0..N {
        let mut c =
            RawClient::join(&shim.url, "SHIMRM", &format!("c{i}"), i == 0).expect("raw join");
        c.pump(800);
        let w = c.take_control("welcome").expect("welcome");
        let (id, host, members) = match w {
            ControlFrame::Welcome { id, host, members } => (id, host, members),
            other => panic!("not welcome: {other:?}"),
        };
        ids.push(id.clone());
        snaps.push(Snapshot::new_from(&id, &host, &members));
        raw.push(Some(c));
    }
    // Fold the join fan-out frames already in the inboxes.
    for c in raw.iter_mut() {
        let Some(c) = c else { continue };
        c.pump(300);
    }
    for (i, slot) in raw.iter_mut().enumerate() {
        let Some(c) = slot else { continue };
        while let Some(cf) = c.take_control("join") {
            if let ControlFrame::Join { id, .. } = cf {
                snaps[i].apply(&TransportEvent::Join {
                    id,
                    name: String::new(),
                });
            }
        }
    }

    // Production-handler client on the same room.
    let mut prod = ProdClient::join(&shim.url, "SHIMRM", "prod", false);
    prod.pump(800);
    let prod_expected = prod.snap.roster.len() == N + 1 && prod.snap.open;
    // Fold prod's join fan-out into the raw clients' rosters, too.
    for (i, slot) in raw.iter_mut().enumerate() {
        let Some(c) = slot else { continue };
        c.pump(300);
        while let Some(cf) = c.take_control("join") {
            if let ControlFrame::Join { id, .. } = cf {
                snaps[i].apply(&TransportEvent::Join {
                    id,
                    name: String::new(),
                });
            }
        }
    }

    // Broadcast one payload per client. The relay fans out per-connection on
    // its own threads, so we send one frame and collect it from every
    // recipient before sending the next — that pins the order every client
    // observes.
    let mut expected: Vec<Vec<u32>> = (0..N)
        .map(|r| (0..N as u32).filter(|s| *s != r as u32).collect())
        .collect();
    for s in 0..N {
        raw[s]
            .as_mut()
            .unwrap()
            .send(&wire::envelope_broadcast(&format!("{{\"i\":{s}}}")));
        for i in 0..N {
            if i == s || raw[i].is_none() {
                continue;
            }
            let payload = raw[i]
                .as_mut()
                .unwrap()
                .wait_message(2000)
                .expect("broadcast delivery");
            let n = payload_i(&payload);
            snaps[i].payloads.push(n);
            prod.pump(120);
        }
    }

    // Targeted 0→1 (`sendTo` reaches the addressee only; the sender *could*
    // address itself, but here it does not).
    raw[0]
        .as_mut()
        .unwrap()
        .send(&wire::envelope_send_to(&ids[1], "{\"i\":100}"));
    expected[1].push(100);
    let payload = raw[1]
        .as_mut()
        .unwrap()
        .wait_message(2000)
        .expect("sendTo delivery");
    snaps[1].payloads.push(payload_i(&payload));
    prod.pump(300);

    // Host locks; stranger refused.
    raw[0]
        .as_mut()
        .unwrap()
        .send(&wire::encode_control(&ControlFrame::Lock { v: true }));
    std::thread::sleep(Duration::from_millis(200));
    let mut stranger = RawClient::join(&shim.url, "SHIMRM", "x", false).expect("stranger join");
    let mut s_snap = Snapshot::blank();
    if let Some(ControlFrame::Err { e }) = stranger.wait_control("err", 1000) {
        s_snap.err = Some(e);
    }

    // Client 2 leaves; survivors re-elect.
    raw[2] = None; // drop closes the socket → relay `_gone` → leave fan-out
    snaps[2].open = false;
    for (i, slot) in raw.iter_mut().enumerate() {
        let Some(c) = slot else { continue };
        c.pump(500);
        while let Some(cf) = c.take_control("leave") {
            if let ControlFrame::Leave { id, host } = cf {
                snaps[i].apply(&TransportEvent::Leave { id, host });
            }
        }
    }
    prod.pump(500);

    // Heartbeat through the shim: the runtime answers bare `"ping"` with
    // bare `"pong"` (server/src/index.js L52).
    raw[0].as_mut().unwrap().send("ping");
    raw[0].as_mut().unwrap().pump(500);
    let pong_ok = raw[0].as_mut().unwrap().take_pong();

    let rtts = vec![prod.rtt()];
    let mut checks: Vec<(&str, bool)> = Vec::new();

    let prod_snap = prod.snap.clone();
    let survivors: Vec<&Snapshot> = snaps
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 2)
        .map(|(_, s)| s)
        .chain(std::iter::once(&prod_snap))
        .collect();
    let all_agree = survivors
        .iter()
        .all(|a| survivors.iter().all(|b| a.agrees_with(b)));
    checks.push((
        "survivor rosters/host agree, incl. the RxState client",
        all_agree && prod_expected,
    ));

    let order_ok = (0..N)
        .filter(|&i| i != 2)
        .all(|i| snaps[i].payloads == expected[i]);
    checks.push(("broadcast order identical (raw clients)", order_ok));

    checks.push((
        "production RxState client sees the same order",
        prod_snap.payloads == (0..N as u32).collect::<Vec<u32>>(),
    ));

    checks.push((
        "stranger refused 'Match in progress'",
        s_snap.err.as_deref() == Some("Match in progress"),
    ));

    let host_ok = snaps
        .iter()
        .enumerate()
        .all(|(i, s)| i == 2 || s.host == ids[0])
        && prod_snap.host == ids[0];
    checks.push(("host stays oldest survivor after leave", host_ok));

    checks.push(("runtime ping→pong answers", pong_ok));

    checks.push((
        "heartbeat RTT sampled through the shim",
        prod.rtt() > 0.0 && prod.rtt() < 2000.0,
    ));

    Report {
        mode: "shim",
        clients: snaps,
        stranger: s_snap,
        rtts,
        checks,
    }
}

impl Snapshot {
    fn new_from(id: &str, host: &str, members: &[(String, String)]) -> Self {
        let mut s = Snapshot::blank();
        s.apply(&TransportEvent::Welcome {
            id: id.to_string(),
            host: host.to_string(),
            members: members.to_vec(),
        });
        s
    }
}
