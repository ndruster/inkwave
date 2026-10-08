//! Task 18 — the transport *contract*: the shape of a relay link with zero
//! I/O, so every layer above it (session, replication, playback) is testable
//! against [`FakeTransport`] (spec NFR-M2-1/NFR-M2-3).
//!
//! The behaviour modelled here is `src/net/transport.js` end to end:
//!
//! - **URLs** — `relayURL()` (L10-16) and the `/room/<code>?name=&v=1[&create=1]`
//!   connect URL (L44) via [`relay_url`] / [`room_url`].
//! - **Receive handling** — `Transport.connect`'s `handle` (L49-67): the bare
//!   `"pong"` RTT echo, the `"m|<from>|<json>"` envelope split, the JSON
//!   control frames (`err` rejects the connect, `welcome` resolves it and
//!   starts the heartbeat, `join`/`leave` are control callbacks). Modelled
//!   once in [`RxState::handle`] and reused verbatim by the native and wasm
//!   implementations in the `inkwave` crate.
//! - **Heartbeat** — `_startPing` (L82-90): the first `"ping"` text frame
//!   goes out the moment `welcome` lands, then one every 2 s; `_pingSent`
//!   times the *oldest unanswered* ping and the RTT is the EMA
//!   `rtt ? rtt + (r - rtt) * 0.3 : r` (L53/L64) — [`RTT_ALPHA`] / [`rtt_ema`].
//! - **Handshake timeout** — 8 s from `connect()` to `welcome`, else the
//!   link fails with `'Could not connect'` (L48).
//! - **Link simulation** — the debug `?netlag/netjitter/netspike` SIM (L21-25,
//!   L68-72): every received frame is held to
//!   `max(arrival + lag + rand()*jitter + (rand() < spike ? 250 : 0), last)`
//!   with a single monotone `last` per link, so late packets bunch up exactly
//!   like TCP and a spike stalls everything behind it. [`FakeTransport`]
//!   replays this on a virtual clock with a seeded xorshift RNG
//!   (deterministic, TR-18.1); the two `Math.random()` draws happen per frame
//!   in that order (jitter first, spike second), as in JS.
//!
//! [`FakeRelay`] is the in-process *link peer*: it mimics just enough of the
//! Cloudflare room object (`server/src/index.js`) — welcome/join/leave frames,
//! `b|` fan-out (not back to the sender), `s|to|` routing (sender included,
//! like the real relay), host = earliest member, `lock` (host only) refusing
//! joins with `'Match in progress'`, and the runtime-level `ping`→`pong`
//! answer — so multi-endpoint timing scenarios run without any socket. It is
//! **not** the Task 19 relay shim (a standalone binary for cross-process
//! tests); this is the deterministic unit-test fake.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use super::wire;
use super::{ControlFrame, PROTO, SessionId};

/// `PROD_RELAY` (transport.js L9) — the deployed Worker relay.
pub const PROD_RELAY: &str = "wss://inkwave-net.inkwave.workers.dev";
/// Heartbeat period: `setInterval(ping, 2000)` (transport.js L89).
pub const PING_INTERVAL_MS: f64 = 2000.0;
/// Handshake deadline: `setTimeout(…, 8000)` before `welcome` (L48).
pub const CONNECT_TIMEOUT_MS: f64 = 8000.0;
/// Wi-Fi hiccup hold time: `?netspike` adds a flat 250 ms (L69).
pub const SPIKE_MS: f64 = 250.0;
/// RTT smoothing factor: `rtt + (r - rtt) * 0.3` (L53/L64).
pub const RTT_ALPHA: f64 = 0.3;
/// Local dev relay port: `ws://${hostname}:8787` (`wrangler dev`, L15).
pub const LOCAL_RELAY_PORT: u16 = 8787;
/// Room size cap (`MAX = 8`, server/src/index.js L19).
pub const ROOM_MAX: usize = 8;

/// Round-trip EMA exactly as the JS writes it — a zero previous value is
/// replaced, not blended (`this.rtt ? this.rtt + (r - this.rtt) * 0.3 : r`).
#[must_use]
pub fn rtt_ema(prev: f64, r: f64) -> f64 {
    if prev != 0.0 {
        prev + (r - prev) * RTT_ALPHA
    } else {
        r
    }
}

/// The `local` test of `relayURL()` (transport.js L14): localhost, the IPv6
/// loopback literal, RFC-1918 LAN addresses and `.local` mDNS names.
#[must_use]
pub fn is_local_host(h: &str) -> bool {
    if h == "localhost" || h == "127.0.0.1" || h == "[::1]" || h.ends_with(".local") {
        return true;
    }
    let mut p = h.split('.');
    let (Some(a), Some(b)) = (p.next(), p.next()) else {
        return false;
    };
    // `^(10|192\.168|172\.(1[6-9]|2\d|3[01]))\.` — 172.x is only 16–31.
    a == "10"
        || (a == "192" && b == "168")
        || (a == "172" && b.parse::<u8>().is_ok_and(|v| (16..=31).contains(&v)))
}

/// `relayURL()` (transport.js L10-16): an explicit override wins (with one
/// trailing `/` stripped, like `q.replace(/\/$/, '')`); a page served from
/// localhost/the LAN talks to a local `wrangler dev` relay on :8787; anything
/// else uses [`PROD_RELAY`]. `hostname` is `None` for native builds (a
/// desktop binary has no page origin) — then the override or `PROD_RELAY`
/// applies.
#[must_use]
pub fn relay_url(override_base: Option<&str>, hostname: Option<&str>) -> String {
    if let Some(q) = override_base {
        return q.strip_suffix('/').unwrap_or(q).to_string();
    }
    match hostname {
        Some(h) if is_local_host(h) => format!("ws://{h}:{LOCAL_RELAY_PORT}"),
        _ => PROD_RELAY.to_string(),
    }
}

/// `encodeURIComponent` (transport.js L44): unreserved `A-Za-z0-9-_.~` plus
/// the marks `!'()`, everything else percent-encoded from the UTF-8 bytes
/// with uppercase hex.
#[must_use]
pub fn js_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'~'
            | b'!'
            | b'\''
            | b'('
            | b')'
            | b'*' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The connect URL, `transport.js` L44:
/// `${base}/room/${encodeURIComponent(code)}?name=${encodeURIComponent(name)}&v=1[&create=1]`.
#[must_use]
pub fn room_url(base: &str, code: &str, name: &str, create: bool) -> String {
    let mut u = format!(
        "{}/room/{}?name={}&v={}",
        base,
        js_encode(code),
        js_encode(name),
        PROTO
    );
    if create {
        u.push_str("&create=1");
    }
    u
}

// ---------------------------------------------------------------------------
// Receive state & handler (transport.js `handle`, L49-67)
// ---------------------------------------------------------------------------

/// One delivered transport event — the union of `onControl` / `onMessage` /
/// `onClose` and the connect rejection of the JS class, drained as a queue
/// (the "channel semantics" the tasks allow for a poll-driven Bevy app).
#[derive(Debug, Clone, PartialEq)]
pub enum TransportEvent {
    /// `{"t":"welcome",…}` resolved the handshake (L65); `id` is our relay
    /// session id, `host` the elected host, `members` the roster at join.
    Welcome {
        id: SessionId,
        host: SessionId,
        members: Vec<(SessionId, String)>,
    },
    /// `{"t":"join","m":{id,name}}` — a member joined (L66 → `onControl`).
    Join { id: SessionId, name: String },
    /// `{"t":"leave","id","host"}` — a member dropped; `host` is the
    /// re-elected oldest member.
    Leave { id: SessionId, host: SessionId },
    /// `"m|<from>|<json>"` — a relayed game payload (L56-60). The payload
    /// stays a raw JSON string; the session layer decodes it via `wire`.
    Message { from: SessionId, payload: String },
    /// A connect *failure*: the relay's `{"t":"err",…}` frame (L63, the
    /// message is the player-facing text) or the local `'Could not connect'`
    /// (socket error / handshake timeout / close before welcome).
    Error { e: String },
    /// The socket closed after a successful handshake — `onClose(reason)`
    /// (L76). Never emitted for an explicit [`Transport::close`] (JS nulls
    /// `onclose` first, L108).
    Closed { reason: String },
}

/// What [`RxState::handle`] decided about one inbound frame.
#[derive(Debug, Default)]
pub struct Rx {
    /// The frame was a `welcome` — the caller must start the heartbeat and
    /// cancel the 8 s handshake timer (transport.js L65).
    pub welcome: bool,
    /// The frame was `{"t":"err",…}` — while the connect has not settled
    /// this is the rejection message (L63).
    pub err: Option<String>,
    /// Events to surface (`Message` / `Join` / `Leave` / `Welcome`).
    pub events: Vec<TransportEvent>,
}

/// The receive-side state of `Transport` (`id`, `settled`, `rtt`, `_pingSent`,
/// `bytesIn`, transport.js L31-36) plus its `handle` function. Shared by the
/// fake, native and wasm implementations so all three parse relay frames with
/// one byte-identical code path. Time is milliseconds on some monotone clock
/// of the caller's choosing (JS `performance.now()`).
#[derive(Debug, Clone, Default)]
pub struct RxState {
    /// Our relay session id (`welcome.id`, L65).
    pub id: SessionId,
    /// Handshake settled (`welcome` seen or connect failed) — the JS
    /// `settled` flag (L42).
    pub settled: bool,
    /// Smoothed round trip, ms (L34).
    pub rtt: f64,
    /// Send time of the oldest unanswered `"ping"`, `None` when none — the
    /// JS falsy `_pingSent` (L35), modelled as `Option` so a virtual clock
    /// that starts at 0 ms still works.
    pub ping_sent: Option<f64>,
    /// Received payload bytes (L36; counted at handling time, i.e. *after*
    /// any SIM delay, as in JS).
    pub bytes_in: usize,
}

impl RxState {
    /// `handle(ev)` (transport.js L49-67) for one text frame arriving at
    /// `now_ms`.
    pub fn handle(&mut self, s: &str, now_ms: f64) -> Rx {
        self.bytes_in += s.len();
        let mut rx = Rx::default();
        if s == "pong" {
            // Answered by the relay runtime itself (L52-55): RTT against the
            // oldest unanswered ping.
            if let Some(p) = self.ping_sent.take() {
                let r = now_ms - p;
                self.rtt = rtt_ema(self.rtt, r);
            }
            return rx;
        }
        // "m|from|json" — charCodeAt(0)==='m' && charCodeAt(1)==='|' (L56).
        if let Some(rest) = s.strip_prefix("m|") {
            let Some(k) = rest.find('|') else {
                return rx;
            };
            let payload = &rest[k + 1..];
            // JS JSON.parse's before calling onMessage; bad JSON is dropped.
            if wire::parse_json(payload).is_some() {
                rx.events.push(TransportEvent::Message {
                    from: rest[..k].to_string(),
                    payload: payload.to_string(),
                });
            }
            return rx;
        }
        let Some(v) = wire::parse_json(s) else {
            return rx;
        };
        let Some(f) = wire::decode_control(&v) else {
            return rx;
        };
        match f {
            ControlFrame::Err { e } => {
                rx.err = Some(if e.is_empty() {
                    "Could not connect".to_string()
                } else {
                    e
                });
            }
            ControlFrame::Pong { c } => {
                // {"t":"pong","c"} — the older-client RTT echo (L64).
                let r = now_ms - c;
                self.rtt = rtt_ema(self.rtt, r);
            }
            ControlFrame::Welcome { id, host, members } => {
                self.id = id.clone();
                rx.welcome = true;
                rx.events
                    .push(TransportEvent::Welcome { id, host, members });
            }
            ControlFrame::Join { id, name } => {
                rx.events.push(TransportEvent::Join { id, name });
            }
            ControlFrame::Leave { id, host } => {
                rx.events.push(TransportEvent::Leave { id, host });
            }
            // Lock is client→relay only; a relay never sends it.
            ControlFrame::Lock { .. } => {}
        }
        rx
    }
}

// ---------------------------------------------------------------------------
// The trait
// ---------------------------------------------------------------------------

/// The relay link contract every layer above codes against. I/O lives in the
/// `inkwave` crate's native/wasm impls; [`FakeTransport`] is the in-process
/// impl. Events are drained by polling (Bevy `Receive` systems), so no
/// callbacks cross thread or wasm boundaries.
pub trait Transport {
    /// Open the link and join/create the room (transport.js `connect`,
    /// L40-80). The outcome arrives as [`TransportEvent::Welcome`] or
    /// [`TransportEvent::Error`] from [`Transport::poll_events`].
    fn connect(&mut self, code: &str, name: &str, create: bool);
    /// Drain pending events (oldest first).
    fn poll_events(&mut self) -> Vec<TransportEvent>;
    /// `Transport._raw` (L93-99): send one already-framed text payload;
    /// `false` when the socket is not open. Counts `bytesOut`.
    fn send_raw(&mut self, s: &str) -> bool;
    /// `get open` (L103): socket connected and usable.
    fn is_open(&self) -> bool;
    /// Smoothed round trip to the relay, ms (L34).
    fn rtt(&self) -> f64;
    /// `Transport.close` (L105-109): stop the heartbeat and close with
    /// `(1000, "bye")`; no [`TransportEvent::Closed`] follows.
    fn close(&mut self);

    /// `Transport.broadcast` (L100): `"b|" + json`.
    fn broadcast(&mut self, payload_json: &str) -> bool {
        self.send_raw(&wire::envelope_broadcast(payload_json))
    }
    /// `Transport.sendTo` (L101): `"s|" + to + "|" + json`.
    fn send_to(&mut self, to: &str, payload_json: &str) -> bool {
        self.send_raw(&wire::envelope_send_to(to, payload_json))
    }
    /// `Transport.lock` (L102): `{"t":"lock","v":!!v}`, host only.
    fn lock(&mut self, v: bool) -> bool {
        self.send_raw(&wire::encode_control(&ControlFrame::Lock { v }))
    }
}

// ---------------------------------------------------------------------------
// Fake link: FakeRelay + FakeTransport (virtual clock, seeded RNG)
// ---------------------------------------------------------------------------

/// The `?netlag/&netjitter/&netspike` SIM parameters (transport.js L21-25).
#[derive(Debug, Clone, Copy)]
pub struct LinkSim {
    /// One-way extra delay on everything received, ms.
    pub lag_ms: f64,
    /// Random extra on top, ms (delivered in order — late packets bunch).
    pub jitter_ms: f64,
    /// Per-message chance of a flat [`SPIKE_MS`] hold.
    pub spike_p: f64,
}

/// xorshift64* — deterministic stand-in for `Math.random()` (NFR-M2-3).
/// Output in `[0, 1)` with the same 53-bit shape JS uses.
fn rand01(state: &mut u64) -> f64 {
    let mut x = *state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *state = x;
    (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
}

/// One endpoint's state inside the shared [`NetState`] (the JS `Transport`
/// instance plus the socket the relay sees).
#[derive(Debug)]
struct Ep {
    id: SessionId,
    name: String,
    seq: usize,
    /// Socket open (`readyState === 1`, transport.js L95).
    open: bool,
    rx: RxState,
    sim: Option<LinkSim>,
    rng: u64,
    /// SIM `last` — the monotone delivery floor (transport.js L69-70).
    last: f64,
    /// Frames in flight toward us: (deliver-at ms, text). Arrival stamps the
    /// SIM hold (as JS `onmessage` does), so the queue is time-sorted.
    inbox: VecDeque<(f64, String)>,
    events: VecDeque<TransportEvent>,
    /// Heartbeat schedule (transport.js `_startPing`/`_stopPing`).
    next_ping: f64,
    /// When `connect()` started, for the 8 s handshake timer (L48).
    connected_at: f64,
    /// A `connect()` is in flight (drives the timeout check).
    connecting: bool,
    /// The `create=1` flag of the in-flight `connect()` (drives the relay's
    /// `Room code taken` / `Room not found` ladder, server/src/index.js
    /// L70-71).
    create: bool,
    /// Sent payload bytes (`bytesOut`, L36).
    bytes_out: usize,
}

#[derive(Debug, Default)]
struct NetState {
    now: f64,
    /// `lock` state (server/src/index.js L126).
    locked: bool,
    /// Test knob: the relay never answers (dead network / unreachable host).
    silent: bool,
    next_seq: usize,
    /// Member ids minted like the room object (base36, server L78); the fake
    /// uses a deterministic counter rendered as 4 uppercase chars.
    next_id: u32,
    members: Vec<Rc<RefCell<Ep>>>,
}

impl Ep {
    /// The SIM hold for one arriving frame (transport.js L69-70): two RNG
    /// draws per frame — jitter first, spike second — clamped by the
    /// monotone `last` floor so spikes stall everything behind them.
    fn schedule(&mut self, arr: f64, frame: String) {
        let t = match self.sim {
            None => arr,
            Some(s) => {
                let r1 = rand01(&mut self.rng);
                let r2 = rand01(&mut self.rng);
                let spike = if r2 < s.spike_p { SPIKE_MS } else { 0.0 };
                let t = (arr + s.lag_ms + r1 * s.jitter_ms + spike).max(self.last);
                self.last = t;
                t
            }
        };
        self.inbox.push_back((t, frame));
    }

    /// Deliver every due frame (inbox is time-sorted by the SIM floor).
    /// Returns whether the `welcome` frame landed (the caller starts the
    /// heartbeat, transport.js L65).
    fn deliver(&mut self, now: f64) -> bool {
        let mut welcomed = false;
        while self.inbox.front().is_some_and(|(t, _)| *t <= now) {
            let (_, frame) = self.inbox.pop_front().expect("front checked");
            let rx = self.rx.handle(&frame, now);
            if rx.welcome {
                self.rx.settled = true;
                self.connecting = false;
                welcomed = true;
            }
            if let Some(e) = rx.err {
                if !self.rx.settled {
                    self.rx.settled = true;
                    self.connecting = false;
                    self.open = false;
                    self.events
                        .push_back(TransportEvent::Error { e: e.clone() });
                    // `fail` closes the socket with the same text (server
                    // L66); the JS `onclose` then fires `onClose(reason)`
                    // because the connect already settled (L75-76).
                    self.events.push_back(TransportEvent::Closed { reason: e });
                }
                // Post-settle err frames are swallowed by JS (L63 returns).
                continue;
            }
            for ev in rx.events {
                self.events.push_back(ev);
            }
        }
        welcomed
    }
}

impl NetState {
    /// Host election: the oldest (lowest-seq) member still in the room
    /// (server/src/index.js L139-141).
    fn host(&self) -> Option<(SessionId, usize, Rc<RefCell<Ep>>)> {
        self.members
            .iter()
            .filter(|m| m.borrow().open)
            .map(|m| {
                let b = m.borrow();
                (b.id.clone(), b.seq, m.clone())
            })
            .min_by_key(|(_, seq, _)| *seq)
    }

    /// Schedule `frame` toward `ep` as arriving at the net clock `arr`.
    fn schedule(&self, ep: &Rc<RefCell<Ep>>, arr: f64, frame: String) {
        ep.borrow_mut().schedule(arr, frame);
    }

    /// A client frame from `from` (`from` must be a current member).
    fn route(&mut self, from: &Rc<RefCell<Ep>>, frame: &str) {
        if self.silent {
            return;
        }
        let now = self.now;
        let from_id = from.borrow().id.clone();
        if frame == "ping" {
            // The relay runtime answers pings itself without waking the room
            // (server/src/index.js L111; transport.js L52).
            self.schedule(from, now, "pong".to_string());
            return;
        }
        if let Some(payload) = frame.strip_prefix("b|") {
            for m in &self.members {
                if Rc::ptr_eq(m, from) || !m.borrow().open {
                    continue;
                }
                self.schedule(m, now, format!("m|{from_id}|{payload}"));
            }
            return;
        }
        if let Some(rest) = frame.strip_prefix("s|") {
            let Some(k) = rest.find('|') else {
                return;
            };
            let to = &rest[..k];
            let payload = &rest[k + 1..];
            for m in &self.members {
                // The real relay does not exclude the sender from `s|`
                // ("sendTo 自己可达", server/src/index.js L118-120).
                if m.borrow().id == to && m.borrow().open {
                    self.schedule(m, now, format!("m|{from_id}|{payload}"));
                }
            }
            return;
        }
        // JSON control frames from clients: only `lock`, host only (L126).
        if frame.starts_with('{')
            && let Some(f) = wire::parse_json(frame).and_then(|v| wire::decode_control(&v))
            && self.host().is_some_and(|(h, _, _)| h == from_id)
            && let ControlFrame::Lock { v } = f
        {
            self.locked = v;
        }
    }

    /// Join handshake (server/src/index.js L61-86): refusal frames reach the
    /// client *then the socket closes* (`fail`, L66); success sends `welcome`
    /// (roster incl. self, host = lowest seq) to the joiner and `join` to
    /// everyone else.
    fn join(&mut self, ep: &Rc<RefCell<Ep>>) {
        if self.silent {
            return;
        }
        let now = self.now;
        let open = self.members.iter().filter(|m| m.borrow().open).count();
        let create = ep.borrow().create;
        // `fail` ladder, in the relay's exact order and wording (L69-73).
        let refusal = if create && open > 0 {
            Some("Room code taken")
        } else if !create && open == 0 {
            Some("Room not found")
        } else if open >= ROOM_MAX {
            Some("Room is full")
        } else if self.locked && open > 0 {
            // L73-74: a lock only refuses joins while members exist; an
            // empty room clears it.
            Some("Match in progress")
        } else {
            None
        };
        if let Some(e) = refusal {
            self.schedule(
                ep,
                now,
                wire::encode_control(&ControlFrame::Err { e: e.into() }),
            );
            return;
        }
        if open == 0 {
            self.locked = false;
        }
        let id = mint_id(&mut self.next_id);
        let seq = self.next_seq;
        self.next_seq += 1;
        {
            let mut b = ep.borrow_mut();
            b.id = id.clone();
            b.seq = seq;
            b.open = true;
        }
        self.members.push(ep.clone());
        // Roster in seq order, self included; host = first (`all[0].id`,
        // server/src/index.js L80-81).
        let roster: Vec<(SessionId, String)> = self
            .members
            .iter()
            .map(|m| (m.borrow().id.clone(), m.borrow().name.clone()))
            .collect();
        let host = self.host().map(|(h, _, _)| h).unwrap_or_else(|| id.clone());
        self.schedule(
            ep,
            now,
            wire::encode_control(&ControlFrame::Welcome {
                id: id.clone(),
                host,
                members: roster,
            }),
        );
        let join = wire::encode_control(&ControlFrame::Join {
            id,
            name: ep.borrow().name.clone(),
        });
        for m in &self.members {
            if !Rc::ptr_eq(m, ep) && m.borrow().open {
                self.schedule(m, now, join.clone());
            }
        }
    }

    /// A member's socket closed: remove it and broadcast `leave` with the
    /// re-elected host (server/src/index.js L136-142).
    fn part(&mut self, ep: &Rc<RefCell<Ep>>) {
        let Some(pos) = self.members.iter().position(|m| Rc::ptr_eq(m, ep)) else {
            return;
        };
        let id = ep.borrow().id.clone();
        self.members.remove(pos);
        let Some((host, _, _)) = self.host() else {
            return;
        };
        let leave = wire::encode_control(&ControlFrame::Leave { id, host });
        let now = self.now;
        for m in &self.members {
            if m.borrow().open {
                self.schedule(m, now, leave.clone());
            }
        }
    }

    /// One endpoint's per-frame work. `ep` is in the member list only after
    /// a successful join, so pump also has to run for endpoints whose join
    /// was refused — [`FakeTransport::advance`] covers those via the net's
    /// member list plus the connecting endpoint itself.
    fn pump(ep: &Rc<RefCell<Ep>>, now: f64) {
        // Deliver due frames; if `welcome` landed, `_startPing` fires the
        // first ping immediately, then every 2 s (transport.js L82-90).
        if ep.borrow_mut().deliver(now) {
            ep.borrow_mut().next_ping = now;
        }
        let mut b = ep.borrow_mut();
        // 8 s handshake timer (transport.js L48): reject, then `ws.close()`
        // — whose `onclose` fires `onClose('Disconnected')` (empty reason,
        // L76) because the connect already settled.
        if b.connecting && now - b.connected_at >= CONNECT_TIMEOUT_MS {
            b.connecting = false;
            b.rx.settled = true;
            b.open = false;
            b.events.push_back(TransportEvent::Error {
                e: "Could not connect".to_string(),
            });
            b.events.push_back(TransportEvent::Closed {
                reason: "Disconnected".to_string(),
            });
        }
        // Heartbeat (transport.js L84-88): `_pingSent` times the oldest
        // unanswered ping (pongs come back in order); the relay runtime
        // answers every "ping" with "pong", SIM-delayed like any frame.
        if b.open && now >= b.next_ping {
            if b.rx.ping_sent.is_none() {
                b.rx.ping_sent = Some(now);
            }
            b.bytes_out += 4;
            b.next_ping += PING_INTERVAL_MS;
            b.schedule(now, "pong".to_string());
        }
        // A zero-lag pong scheduled above is due now — deliver it in the same
        // tick so the RTT reflects the true link (0 ms), not the advance step.
        drop(b);
        ep.borrow_mut().deliver(now);
    }
}

/// Deterministic stand-in for the room's
/// `Math.random().toString(36).slice(2, 6).toUpperCase()` member ids
/// (server/src/index.js L78) — 4 uppercase base36 chars from a counter.
fn mint_id(next: &mut u32) -> SessionId {
    let mut v = *next;
    *next += 1;
    let mut s = String::with_capacity(4);
    for _ in 0..4 {
        let d = (v % 36) as u8;
        s.push((if d < 10 { b'0' + d } else { b'A' + d - 10 }) as char);
        v /= 36;
    }
    s
}

/// The in-process relay peer: enough of `server/src/index.js` for link tests
/// (welcome/join/leave, `b|`/`s|` routing, host election, `lock`,
/// `ping`→`pong`). See the module docs for how it differs from the Task 19
/// shim.
#[derive(Debug, Default)]
pub struct FakeRelay {
    net: Rc<RefCell<NetState>>,
}

impl FakeRelay {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Test knob: the relay never answers (models an unreachable host so the
    /// 8 s handshake timer fires).
    pub fn set_silent(&self, v: bool) {
        self.net.borrow_mut().silent = v;
    }

    /// Host-side lock as the relay sees it (server/src/index.js L126);
    /// prefer driving a real [`Transport::lock`] frame in tests.
    pub fn set_locked(&self, v: bool) {
        self.net.borrow_mut().locked = v;
    }

    /// Current roster (id, name) of open members, join order.
    #[must_use]
    pub fn members(&self) -> Vec<(SessionId, String)> {
        self.net
            .borrow()
            .members
            .iter()
            .map(|m| (m.borrow().id.clone(), m.borrow().name.clone()))
            .collect()
    }

    /// The relay's current host (`None` while empty).
    #[must_use]
    pub fn host(&self) -> Option<SessionId> {
        self.net.borrow().host().map(|(h, _, _)| h)
    }

    /// A fresh endpoint on this relay with the given link simulation and a
    /// deterministic RNG seed (TR-18.1). Not joined until [`Transport::connect`].
    #[must_use]
    pub fn transport(&self, sim: Option<LinkSim>, seed: u64) -> FakeTransport {
        FakeTransport {
            net: self.net.clone(),
            me: Rc::new(RefCell::new(Ep {
                id: String::new(),
                name: String::new(),
                seq: usize::MAX,
                open: false,
                rx: RxState::default(),
                sim,
                rng: seed | 1, // xorshift state must be non-zero
                last: 0.0,
                inbox: VecDeque::new(),
                events: VecDeque::new(),
                next_ping: f64::INFINITY,
                connected_at: 0.0,
                connecting: false,
                create: false,
                bytes_out: 0,
            })),
        }
    }
}

/// One fake endpoint (implements [`Transport`]). Time does not pass by
/// itself: call [`FakeTransport::advance`] with a virtual-clock delta,
/// exactly like driving a deterministic playback test. `advance` moves the
/// *shared* net clock, so every endpoint on one [`FakeRelay`] stays in step.
#[derive(Debug)]
pub struct FakeTransport {
    net: Rc<RefCell<NetState>>,
    me: Rc<RefCell<Ep>>,
}

impl FakeTransport {
    /// Move the shared virtual clock by `dt_ms` and run the link: due
    /// deliveries, handshake timers and heartbeats.
    pub fn advance(&mut self, dt_ms: f64) {
        {
            let mut n = self.net.borrow_mut();
            n.now += dt_ms;
            let now = n.now;
            let mut eps = n.members.clone();
            if !eps.iter().any(|m| Rc::ptr_eq(m, &self.me)) {
                eps.push(self.me.clone());
            }
            for ep in &eps {
                NetState::pump(ep, now);
            }
        }
    }

    /// The virtual clock, ms (test introspection).
    #[must_use]
    pub fn now(&self) -> f64 {
        self.net.borrow().now
    }

    /// `bytesOut` (transport.js L36).
    #[must_use]
    pub fn bytes_out(&self) -> usize {
        self.me.borrow().bytes_out
    }

    /// `bytesIn` (transport.js L36).
    #[must_use]
    pub fn bytes_in(&self) -> usize {
        self.me.borrow().rx.bytes_in
    }

    /// Our relay session id (`welcome.id`, transport.js L65).
    #[must_use]
    pub fn id(&self) -> SessionId {
        self.me.borrow().id.clone()
    }
}

impl Transport for FakeTransport {
    fn connect(&mut self, _code: &str, name: &str, create: bool) {
        let now = self.net.borrow().now;
        {
            let mut b = self.me.borrow_mut();
            b.name = name.to_string();
            b.connecting = true;
            b.create = create;
            b.connected_at = now;
        }
        let mut n = self.net.borrow_mut();
        n.join(&self.me);
    }

    fn poll_events(&mut self) -> Vec<TransportEvent> {
        let mut b = self.me.borrow_mut();
        let out = b.events.iter().cloned().collect::<Vec<_>>();
        b.events.clear();
        out
    }

    fn send_raw(&mut self, s: &str) -> bool {
        // `_raw` (transport.js L93-99): drop when the socket is not open.
        if !self.me.borrow().open {
            return false;
        }
        self.me.borrow_mut().bytes_out += s.len();
        let mut n = self.net.borrow_mut();
        n.route(&self.me, s);
        true
    }

    fn is_open(&self) -> bool {
        self.me.borrow().open
    }

    fn rtt(&self) -> f64 {
        self.me.borrow().rx.rtt
    }

    /// `close()` (transport.js L105-109): the relay sees the socket drop and
    /// broadcasts `leave` to everyone else; we get no `Closed` event.
    fn close(&mut self) {
        {
            let mut b = self.me.borrow_mut();
            b.open = false;
            b.next_ping = f64::INFINITY;
            b.connecting = false;
        }
        let mut n = self.net.borrow_mut();
        n.part(&self.me);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- URL / address resolution (transport.js L9-16, L44) -----------------

    #[test]
    fn relay_url_matches_js_resolution() {
        // ?relay= override wins, one trailing slash stripped (L12).
        assert_eq!(
            relay_url(Some("ws://example.com:9000/"), None),
            "ws://example.com:9000"
        );
        // Local / LAN hostnames → wrangler dev on :8787 (L14-15).
        for h in [
            "localhost",
            "127.0.0.1",
            "[::1]",
            "10.1.2.3",
            "192.168.0.7",
            "172.16.0.1",
            "172.31.255.255",
            "printer.local",
        ] {
            assert_eq!(
                relay_url(None, Some(h)),
                format!("ws://{h}:8787"),
                "{h} is local"
            );
        }
        // Public host or native (no origin) → PROD_RELAY (L15).
        assert_eq!(relay_url(None, Some("inkwave-aah.pages.dev")), PROD_RELAY);
        assert_eq!(relay_url(None, Some("172.32.0.1")), PROD_RELAY);
        assert_eq!(relay_url(None, None), PROD_RELAY);
        assert!(!is_local_host("172.15.0.1"));
        assert!(!is_local_host("11.0.0.1"));
    }

    #[test]
    fn room_url_mirrors_transport_js_l44() {
        assert_eq!(
            room_url("ws://h:8787", "ABC23", "Zoe O' B", true),
            "ws://h:8787/room/ABC23?name=Zoe%20O'%20B&v=1&create=1"
        );
        assert_eq!(
            room_url("wss://r", "BCEF2", "é", false),
            "wss://r/room/BCEF2?name=%C3%A9&v=1"
        );
        // encodeURIComponent leaves -_.~!'() alone, everything else UTF-8 %.
        assert_eq!(js_encode("a-_.~!'()"), "a-_.~!'()");
        assert_eq!(js_encode("a/b?c&d"), "a%2Fb%3Fc%26d");
    }

    // -- RTT / handler primitives -------------------------------------------

    #[test]
    fn rtt_ema_matches_js_expression() {
        assert_eq!(rtt_ema(0.0, 100.0), 100.0);
        assert_eq!(rtt_ema(100.0, 200.0), 130.0);
        assert_eq!(rtt_ema(130.0, 100.0), 121.0);
    }

    #[test]
    fn rx_state_handles_every_js_frame_shape() {
        let mut rx = RxState::default();
        // Bare "pong" only counts while a ping is outstanding (L53).
        let r = rx.handle("pong", 10.0);
        assert!(r.events.is_empty() && rx.rtt == 0.0);
        rx.ping_sent = Some(5.0);
        rx.handle("pong", 105.0);
        assert_eq!(rx.rtt, 100.0);
        assert!(rx.ping_sent.is_none(), "pong consumes the outstanding ping");
        // "m|from|json" split (L56-60); bad JSON dropped.
        let r = rx.handle("m|AB12|{\"k\":\"t\"}", 200.0);
        assert_eq!(
            r.events,
            vec![TransportEvent::Message {
                from: "AB12".into(),
                payload: "{\"k\":\"t\"}".into()
            }]
        );
        assert!(rx.handle("m|AB12|not json", 200.0).events.is_empty());
        // err carries the relay text (L63).
        let r = rx.handle("{\"t\":\"err\",\"e\":\"Room is full\"}", 0.0);
        assert_eq!(r.err.as_deref(), Some("Room is full"));
        // welcome sets id + the start flag (L65).
        let r = rx.handle(
            "{\"t\":\"welcome\",\"id\":\"K3XZ\",\"host\":\"A1B2\",\"members\":[]}",
            0.0,
        );
        assert!(r.welcome);
        assert_eq!(rx.id, "K3XZ");
        // JSON pong echo (L64) and bytesIn accounting (L51).
        let before = rx.bytes_in;
        rx.handle("{\"t\":\"pong\",\"c\":1000}", 1030.0);
        assert!(rx.bytes_in > before);
    }

    // -- Fake link behaviour --------------------------------------------------

    #[test]
    fn fake_handshake_join_routing_and_lock_flow() {
        let relay = FakeRelay::new();
        let mut a = relay.transport(None, 1);
        let mut b = relay.transport(None, 2);
        a.connect("ABC23", "a", true);
        // Welcome lands on the next advance (zero lag ⇒ immediate).
        a.advance(0.0);
        assert_eq!(
            a.poll_events(),
            vec![TransportEvent::Welcome {
                id: a.id(),
                host: a.id(),
                members: vec![(a.id(), "a".into())]
            }]
        );
        assert!(a.is_open());
        // A lagless link has an honest RTT of 0: `_startPing` pings on
        // welcome, the runtime answers "pong" the same tick (transport.js L53).
        assert_eq!(a.rtt(), 0.0);
        assert!(a.bytes_out() >= 4, "the first ping went out");

        b.connect("ABC23", "b", false);
        b.advance(0.0);
        assert_eq!(
            b.poll_events(),
            vec![TransportEvent::Welcome {
                id: b.id(),
                host: a.id(),
                members: vec![(a.id(), "a".into()), (b.id(), "b".into())]
            }]
        );
        // a saw the join control frame (server L82-83).
        assert_eq!(
            a.poll_events(),
            vec![TransportEvent::Join {
                id: b.id(),
                name: "b".into()
            }]
        );
        // `b|` fan-out excludes the sender (server L111-113).
        a.broadcast("{\"k\":\"go\"}");
        b.advance(0.0);
        assert_eq!(
            b.poll_events(),
            vec![TransportEvent::Message {
                from: a.id(),
                payload: "{\"k\":\"go\"}".into()
            }]
        );
        a.advance(0.0);
        assert!(a.poll_events().is_empty(), "no echo back to the sender");
        // `s|to|` reaches the target *and* the sender (server L118-120).
        a.send_to(&a.id(), "{\"k\":\"me\"}");
        a.advance(0.0);
        assert_eq!(
            a.poll_events(),
            vec![TransportEvent::Message {
                from: a.id(),
                payload: "{\"k\":\"me\"}".into()
            }]
        );
        // Host-only lock: a is host (oldest seq), so it takes effect.
        assert!(a.lock(true));
        assert!(relay.net.borrow().locked);
        // A locked room refuses joins with the relay's exact text, then the
        // socket closes (server L66 `fail`): Error then Closed.
        let mut c = relay.transport(None, 3);
        c.connect("ABC23", "c", false);
        c.advance(0.0);
        assert_eq!(
            c.poll_events(),
            vec![
                TransportEvent::Error {
                    e: "Match in progress".into()
                },
                TransportEvent::Closed {
                    reason: "Match in progress".into()
                }
            ]
        );
        assert!(!c.is_open());
        // close() → relay broadcasts leave with the re-elected host (server
        // L138-140); the closer gets no Closed event (JS nulls onclose, L108).
        a.lock(false);
        a.close();
        a.advance(0.0);
        assert!(a.poll_events().is_empty(), "explicit close is silent");
        b.advance(0.0);
        assert_eq!(
            b.poll_events(),
            vec![TransportEvent::Leave {
                id: a.id(),
                host: b.id()
            }]
        );
    }

    #[test]
    fn fake_join_ladder_matches_relay_wording() {
        // server/src/index.js L70-71: create into a non-empty room → "Room
        // code taken"; join an empty room → "Room not found".
        let relay = FakeRelay::new();
        let mut a = relay.transport(None, 1);
        a.connect("ABC23", "a", true);
        a.advance(0.0);
        a.poll_events();
        let mut dup = relay.transport(None, 2);
        dup.connect("ABC23", "dup", true);
        dup.advance(0.0);
        assert_eq!(
            dup.poll_events()[0],
            TransportEvent::Error {
                e: "Room code taken".into()
            }
        );
        let relay2 = FakeRelay::new();
        let mut lone = relay2.transport(None, 3);
        lone.connect("ABC23", "lone", false);
        lone.advance(0.0);
        assert_eq!(
            lone.poll_events()[0],
            TransportEvent::Error {
                e: "Room not found".into()
            }
        );
    }

    #[test]
    fn fake_handshake_times_out_at_8s() {
        let relay = FakeRelay::new();
        relay.set_silent(true);
        let mut a = relay.transport(None, 1);
        a.connect("ABC23", "a", true);
        a.advance(CONNECT_TIMEOUT_MS - 0.001);
        assert!(a.poll_events().is_empty(), "not yet");
        a.advance(0.001);
        // Reject (L48), then `ws.close()` fires onClose('Disconnected') (L76).
        assert_eq!(
            a.poll_events(),
            vec![
                TransportEvent::Error {
                    e: "Could not connect".into()
                },
                TransportEvent::Closed {
                    reason: "Disconnected".into()
                }
            ]
        );
        assert!(!a.is_open());
    }

    #[test]
    fn fake_send_before_open_is_dropped() {
        // `_raw` with no socket returns false (transport.js L95).
        let relay = FakeRelay::new();
        let mut a = relay.transport(None, 1);
        assert!(!a.broadcast("{\"k\":\"t\"}"));
        assert_eq!(a.bytes_out(), 0);
    }

    // -- TR-18.1: SIM timing, deterministic seed ------------------------------

    /// Drain `b` one virtual ms at a time, recording the delivery time of
    /// every `{"i":N}` game message (pongs/welcome ignored).
    fn drain_messages(b: &mut FakeTransport, steps: u32) -> Vec<(f64, String)> {
        let mut out = Vec::new();
        for _ in 0..steps {
            b.advance(1.0);
            for e in b.poll_events() {
                if let TransportEvent::Message { payload, .. } = e {
                    out.push((b.now(), payload));
                }
            }
        }
        out
    }

    #[test]
    fn sim_lag_delays_everything_in_order() {
        let relay = FakeRelay::new();
        let sim = Some(LinkSim {
            lag_ms: 40.0,
            jitter_ms: 0.0,
            spike_p: 0.0,
        });
        let mut a = relay.transport(sim, 11);
        let mut b = relay.transport(sim, 22);
        a.connect("ABC23", "a", true);
        // Welcome is itself SIM-delayed (JS wraps onmessage, L68).
        a.advance(39.0);
        assert!(a.poll_events().is_empty());
        a.advance(1.0);
        assert!(matches!(a.poll_events()[0], TransportEvent::Welcome { .. }));
        b.connect("ABC23", "b", false);
        b.advance(40.0);
        assert!(b.is_open());
        // Three frames 10 ms apart arrive exactly 40 ms late, in order.
        let t0 = b.now();
        for i in 0..3 {
            a.advance(10.0);
            a.broadcast(&format!("{{\"i\":{i}}}"));
        }
        let got = drain_messages(&mut b, 60);
        let payloads: Vec<&str> = got.iter().map(|(_, p)| p.as_str()).collect();
        assert_eq!(payloads, vec!["{\"i\":0}", "{\"i\":1}", "{\"i\":2}"]);
        // Each lands 40 ms after its send (send at t0+10/20/30 → arrive +40).
        for (idx, (t, _)) in got.iter().enumerate() {
            assert!(
                (*t - t0) >= 50.0 + 10.0 * idx as f64 - 0.5,
                "lag ≥ 40 ms: {got:?}"
            );
        }
    }

    #[test]
    fn sim_jitter_bunches_like_tcp_and_is_deterministic() {
        // transport.js L69: `t = max(now + lag + rand()*jit + spike, last)`
        // with a shared monotone `last` — late packets bunch up behind it.
        let sim = Some(LinkSim {
            lag_ms: 0.0,
            jitter_ms: 50.0,
            spike_p: 0.0,
        });
        let run = |seed: u64| -> Vec<(f64, String)> {
            let relay = FakeRelay::new();
            let mut a = relay.transport(sim, seed);
            let mut b = relay.transport(sim, seed ^ 0xdead);
            a.connect("ABC23", "a", true);
            a.advance(120.0);
            b.connect("ABC23", "b", false);
            b.advance(120.0);
            let t0 = b.now();
            let mut got = Vec::new();
            // `a.advance` pumps the *shared* clock (and so b's inbox), so we
            // must poll b every step to catch the real delivery time.
            for i in 0..8 {
                a.advance(10.0);
                a.broadcast(&format!("{{\"i\":{i}}}"));
                for e in b.poll_events() {
                    if let TransportEvent::Message { payload, .. } = e {
                        got.push((b.now() - t0, payload));
                    }
                }
            }
            got.extend(
                drain_messages(&mut b, 200)
                    .into_iter()
                    .map(|(t, p)| (t - t0, p)),
            );
            got
        };
        let r1 = run(7);
        let r2 = run(7);
        assert_eq!(r1, r2, "same seed ⇒ identical timing (NFR-M2-3)");
        assert_eq!(r1.len(), 8);
        // Order preserved (never reordered), and the holds are in [0, 50] ms
        // on top of the 10 ms send cadence — bunching compresses gaps.
        for w in r1.windows(2) {
            assert!(w[1].0 >= w[0].0, "deliveries never reorder: {r1:?}");
        }
        // First frame sent at +10, held by rand()*50 ⇒ arrives in [10, 60].
        assert!(r1[0].0 >= 10.0 && r1[0].0 <= 60.0, "first: {:?}", r1[0]);
        // With jitter, at least one inter-delivery gap is < the 10 ms send
        // cadence (a later packet was held less than an earlier one).
        let gaps: Vec<f64> = r1.windows(2).map(|w| w[1].0 - w[0].0).collect();
        assert!(
            gaps.iter().any(|g| *g < 10.0),
            "jitter bunches gaps below the send cadence: {gaps:?}"
        );
    }

    #[test]
    fn sim_spike_stalls_everything_behind_it() {
        // spike_p = 1.0 ⇒ every frame pays the flat 250 ms (transport.js L69);
        // the monotone floor then queues the rest behind it.
        let relay = FakeRelay::new();
        let sim = Some(LinkSim {
            lag_ms: 0.0,
            jitter_ms: 0.0,
            spike_p: 1.0,
        });
        let mut a = relay.transport(sim, 5);
        let mut b = relay.transport(sim, 6);
        a.connect("ABC23", "a", true);
        a.advance(300.0);
        assert!(!b.is_open());
        b.connect("ABC23", "b", false);
        b.advance(300.0);
        assert!(b.is_open());
        let t0 = b.now();
        for i in 0..3 {
            a.advance(10.0);
            a.broadcast(&format!("{{\"i\":{i}}}"));
        }
        let got = drain_messages(&mut b, 400);
        assert_eq!(got.len(), 3);
        // Every frame pays ≥ 250 ms and the queue stays monotone.
        assert!(got[0].0 - t0 >= 250.0, "spike floor: {got:?}");
        assert!(got[1].0 >= got[0].0 && got[2].0 >= got[1].0);
    }
}
