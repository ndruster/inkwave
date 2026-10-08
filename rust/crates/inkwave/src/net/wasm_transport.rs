//! WASM WebSocket transport — the browser's own `WebSocket` (web-sys), the
//! direct port of `src/net/transport.js` for the wasm target: `onmessage`
//! feeds the shared [`RxState`] (same frame semantics as fake/native),
//! `setInterval` is the 2 s heartbeat and `setTimeout` the 8 s handshake
//! deadline, exactly like the JS `setInterval(ping, 2000)` /
//! `setTimeout(…, 8000)`.
//!
//! Everything runs on the browser's single event loop, so no threads and no
//! channels are needed; events queue in a `RefCell` for `poll_events`.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{CloseEvent, MessageEvent, WebSocket};

use inkwave_sim::net::transport::{
    CONNECT_TIMEOUT_MS, PING_INTERVAL_MS, RxState, Transport, TransportEvent, relay_url, room_url,
};

/// `performance.now()` — the same monotone clock `transport.js` stamps pings
/// with (L53/L64/L85).
fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now())
        .unwrap_or(0.0)
}

/// `relayURL()` (transport.js L10-16) read straight from `location`: the
/// `?relay=` override wins, a page served from localhost/the LAN gets the
/// local `wrangler dev` relay, anything else gets `PROD_RELAY`.
#[must_use]
pub fn relay_url_from_location() -> String {
    let Some(win) = web_sys::window() else {
        return relay_url(None, None);
    };
    let loc = win.location();
    let search = loc.search().unwrap_or_default();
    let params = web_sys::UrlSearchParams::new_with_str(&search).ok();
    let over = params.as_ref().and_then(|p| p.get("relay"));
    let host = loc.hostname().ok();
    relay_url(over.as_deref(), host.as_deref())
}

/// Shared link state between the caller and the browser event callbacks.
#[derive(Clone)]
struct Link {
    ws: WebSocket,
    rx: Rc<RefCell<RxState>>,
    events: Rc<RefCell<VecDeque<TransportEvent>>>,
    /// `setInterval` handle of the heartbeat, `0` when stopped (JS `_pingT`).
    ping: Rc<Cell<i32>>,
    /// `setTimeout` handle of the 8 s handshake timer, `0` when cleared.
    timer: Rc<Cell<i32>>,
    bytes_out: Rc<Cell<usize>>,
}

impl Link {
    fn push(&self, ev: TransportEvent) {
        self.events.borrow_mut().push_back(ev);
    }

    /// `clearTimeout` of the handshake timer (JS `done` → L43).
    fn clear_timer(&self) {
        let h = self.timer.get();
        if h != 0 {
            if let Some(w) = web_sys::window() {
                w.clear_timeout_with_handle(h);
            }
            self.timer.set(0);
        }
    }

    /// `_stopPing` (L91): cancel the heartbeat interval.
    fn stop_ping(&self) {
        let h = self.ping.get();
        if h != 0 {
            if let Some(w) = web_sys::window() {
                w.clear_interval_with_handle(h);
            }
            self.ping.set(0);
        }
    }

    /// One heartbeat tick (JS `ping`, L84-87): time the oldest unanswered
    /// ping, then always send — the relay reads silence as a dead link.
    fn send_ping(&self) {
        let now = now_ms();
        {
            let mut rx = self.rx.borrow_mut();
            if rx.ping_sent.is_none() {
                rx.ping_sent = Some(now);
            }
        }
        if self.ws.ready_state() == 1 {
            let _ = self.ws.send_with_str("ping");
            self.bytes_out.set(self.bytes_out.get() + 4);
        }
    }

    /// `_startPing` (L82-90): one ping right away, then every 2 s.
    fn start_ping(&self) {
        self.send_ping();
        self.stop_ping();
        let l = self.clone();
        let cb = Closure::wrap(Box::new(move || l.send_ping()) as Box<dyn FnMut()>);
        if let Some(w) = web_sys::window() {
            if let Ok(h) = w.set_interval_with_callback_and_timeout_and_arguments_0(
                cb.as_ref().unchecked_ref(),
                PING_INTERVAL_MS as i32,
            ) {
                self.ping.set(h);
            }
        }
        cb.forget();
    }

    /// Connect rejected before the handshake settled: the JS `done(reject)`
    /// + the close that follows it, surfaced as the [`TransportEvent::Error`]
    /// + [`TransportEvent::Closed`] pair the native transport emits too.
    /// `closed_reason` mirrors what the subsequent `onclose` reports — the
    /// relay's `err` text for a refusal (server `close(4000, e)`), the plain
    /// `'Disconnected'` for the handshake timeout (JS closes with no reason,
    /// L76).
    fn reject(&self, e: &str, closed_reason: &str) {
        {
            let mut rx = self.rx.borrow_mut();
            if rx.settled {
                return;
            }
            rx.settled = true;
        }
        self.clear_timer();
        self.stop_ping();
        self.push(TransportEvent::Error { e: e.to_string() });
        self.push(TransportEvent::Closed {
            reason: closed_reason.to_string(),
        });
        // `fail`/timeout closes the socket; the close event must not double up.
        self.ws.set_onclose(None);
        let _ = self.ws.close();
    }
}

/// WASM [`Transport`]: a browser `WebSocket` driven by JS event callbacks.
pub struct WasmTransport {
    base: String,
    link: Option<Link>,
    rx: Rc<RefCell<RxState>>,
    events: Rc<RefCell<VecDeque<TransportEvent>>>,
    bytes_out: Rc<Cell<usize>>,
}

impl WasmTransport {
    /// `relay_base` normally comes from [`relay_url_from_location`].
    #[must_use]
    pub fn new(relay_base: String) -> Self {
        Self {
            base: relay_base,
            link: None,
            rx: Rc::new(RefCell::new(RxState::default())),
            events: Rc::new(RefCell::new(VecDeque::new())),
            bytes_out: Rc::new(Cell::new(0)),
        }
    }

    /// Attach the JS callbacks (transport.js L49-78) to a freshly created
    /// socket and arm the 8 s handshake timer (L48).
    fn attach(&mut self, ws: WebSocket) {
        let link = Link {
            ws,
            rx: self.rx.clone(),
            events: self.events.clone(),
            ping: Rc::new(Cell::new(0)),
            timer: Rc::new(Cell::new(0)),
            bytes_out: self.bytes_out.clone(),
        };

        // `ws.onmessage = handle` (L49-67): feed the shared RxState, then
        // carry out the side effects JS does inline — start the heartbeat on
        // `welcome`, reject on `err`.
        let l = link.clone();
        let onmsg = Closure::wrap(Box::new(move |ev: MessageEvent| {
            let s = ev.data().as_string().unwrap_or_default();
            let now = now_ms();
            let out = l.rx.borrow_mut().handle(&s, now);
            if out.welcome {
                l.rx.borrow_mut().settled = true;
                l.clear_timer();
                l.start_ping();
            }
            if let Some(e) = out.err {
                // The relay's `fail` closes with the same text (server L66),
                // so `onclose` reports it as the reason.
                l.reject(&e, &e);
            }
            for ev in out.events {
                l.push(ev);
            }
        }) as Box<dyn FnMut(MessageEvent)>);
        link.ws.set_onmessage(Some(onmsg.as_ref().unchecked_ref()));
        onmsg.forget();

        // `ws.onclose` (L73-77): stop the heartbeat; unsettled → the connect
        // rejects with the close reason, settled → `onClose(reason)`.
        let l = link.clone();
        let onclose = Closure::wrap(Box::new(move |ev: CloseEvent| {
            l.stop_ping();
            l.clear_timer();
            let reason = if ev.reason().is_empty() {
                "Disconnected".to_string()
            } else {
                ev.reason()
            };
            let unsettled = {
                let mut rx = l.rx.borrow_mut();
                if rx.settled {
                    false
                } else {
                    rx.settled = true;
                    true
                }
            };
            if unsettled {
                l.push(TransportEvent::Error {
                    e: if reason == "Disconnected" {
                        "Could not connect".to_string()
                    } else {
                        reason.clone()
                    },
                });
            }
            l.push(TransportEvent::Closed { reason });
        }) as Box<dyn FnMut(CloseEvent)>);
        link.ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
        onclose.forget();

        // `setTimeout(…, 8000)` (L48): reject, then close the socket. The
        // JS `ws.close()` carries no reason, so `onclose` reports
        // 'Disconnected' (L76) — not the rejection text.
        let l = link.clone();
        let timer_cb = Closure::wrap(Box::new(move || {
            if !l.rx.borrow().settled {
                l.reject("Could not connect", "Disconnected");
            }
        }) as Box<dyn FnMut()>);
        if let Some(w) = web_sys::window() {
            if let Ok(h) = w.set_timeout_with_callback_and_timeout_and_arguments_0(
                timer_cb.as_ref().unchecked_ref(),
                CONNECT_TIMEOUT_MS as i32,
            ) {
                link.timer.set(h);
            }
        }
        timer_cb.forget();

        self.link = Some(link);
    }
}

impl Transport for WasmTransport {
    fn connect(&mut self, code: &str, name: &str, create: bool) {
        if self.link.is_some() {
            return; // JS would leak the old socket; we simply ignore.
        }
        let url = room_url(&self.base, code, name, create);
        // `new WebSocket(url)` inside a try/catch (L46).
        match WebSocket::new(&url) {
            Ok(ws) => self.attach(ws),
            Err(_) => {
                self.events.borrow_mut().push_back(TransportEvent::Error {
                    e: "Could not connect".to_string(),
                });
                self.events.borrow_mut().push_back(TransportEvent::Closed {
                    reason: "Disconnected".to_string(),
                });
            }
        }
    }

    fn poll_events(&mut self) -> Vec<TransportEvent> {
        self.events.borrow_mut().drain(..).collect()
    }

    /// `_raw` (L93-99): readyState must be OPEN, bytes counted, send.
    fn send_raw(&mut self, s: &str) -> bool {
        let Some(l) = self.link.as_ref() else {
            return false;
        };
        if l.ws.ready_state() != 1 {
            return false;
        }
        if l.ws.send_with_str(s).is_err() {
            return false;
        }
        self.bytes_out.set(self.bytes_out.get() + s.len());
        true
    }

    /// `get open` (L103): `readyState === 1`.
    fn is_open(&self) -> bool {
        self.link.as_ref().is_some_and(|l| l.ws.ready_state() == 1)
    }

    fn rtt(&self) -> f64 {
        self.rx.borrow().rtt
    }

    /// `close()` (L105-109): stop the heartbeat, drop `onclose` so no
    /// [`TransportEvent::Closed`] follows, then `ws.close(1000, "bye")`.
    fn close(&mut self) {
        if let Some(l) = self.link.take() {
            l.stop_ping();
            l.clear_timer();
            l.ws.set_onclose(None);
            let _ = l.ws.close_with_code_and_reason(1000, "bye");
        }
    }
}

impl Drop for WasmTransport {
    fn drop(&mut self) {
        self.close();
    }
}
