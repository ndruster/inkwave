//! M6 Task 72 — the platform-neutral DOM overlay bridge. M5 shipped the
//! `CompositeOverlay` / `LobbyNameplates` data planes but no DOM node was ever
//! created (zero `web-sys` `Document` usage). This module is the bridge body:
//!
//! - [`DomOp`] is a platform-neutral, keyed, idempotent DOM operation;
//! - [`build_overlay_ops`] is a PURE function mapping the overlay state + plate
//!   data to an operation sequence (reconciled against the previous frame's
//!   keys), headless-testable with a [`RecordingDomSink`];
//! - on wasm32 a `web-sys` sink applies the same ops to real nodes (G-2),
//!   gated behind `#[cfg(target_arch = "wasm32")]`; its *visuals* are a
//!   user-machine review item (D-M6-6), never faked in the sandbox.
//!
//! Upstream anchors: `src/game/showcase.js` `_lobFrame` / `_measureLobby`
//! (L2626-2694, DOM-measured safe rect + kFront), `hud-boss.js` L402-440
//! (damage numbers), `minimap.js` L19-24 (transient FX). No upstream tree edit.

use std::collections::BTreeSet;

use bevy::prelude::Resource;

use crate::fx::composite::{CompositeOverlay, DamageNumber, MapFx, MapFxKind};
use crate::ui::showcase_state::LobbyNameplate;

/// Stable DOM node identity. Every op carries a key so the sink can reconcile
/// (create once, update text/style in place, remove when absent) — mirroring
/// the JS `keyed` plate update (showcase.js L2060-2084).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DomKey {
    /// A lobby nameplate, keyed by (row, slot).
    Plate(usize, usize),
    /// A boss damage number keyed by its birth timestamp (stable across pool
    /// shrink — `composite.rs` uses `remove(0)`, so a pool index would shift
    /// and cause node churn; review P2-1). f64 bits hashed to u64.
    Damage(u64),
    /// A minimap transient FX node, keyed by kind + creation ordinal.
    MapFx(MapFxKind, usize),
}

/// A platform-neutral DOM operation. All numeric values are the *final*
/// resolved state (no incremental delta), so applying an op twice is a no-op.
#[derive(Clone, Debug, PartialEq)]
pub enum DomOp {
    /// Create a node of `tag` under `#iw-overlay` for `key`.
    Create { key: DomKey, tag: &'static str },
    /// Set the node's text content.
    SetText { key: DomKey, text: String },
    /// Set a CSS style property.
    SetStyle {
        key: DomKey,
        prop: &'static str,
        value: String,
    },
    /// Remove the node (and its children).
    Remove { key: DomKey },
}

/// The reconciliation input: everything needed to resolve the overlay nodes.
/// Kept as a plain struct (not a Bevy Query) so the mapping is a pure function.
#[derive(Clone, Debug, Default)]
pub struct OverlayDomInput<'a> {
    pub plates: &'a [LobbyNameplate],
    pub damage: &'a [DamageNumber],
    pub map_fx: &'a [MapFx],
    /// Visible plate count from the composite gate (Task 64). When 0 the
    /// bridge still emits plate ops for projected plates — the gate is about
    /// the post pass, not the DOM plates (kept separate, D-T64-3). Retained
    /// as an anchor surface for the low-tier全关 test (TR-72.1).
    #[allow(dead_code)]
    pub pass_enabled: bool,
}

/// Map the live overlay state to an idempotent operation sequence for this
/// frame. `prev` is the set of keys that exist in the DOM from last frame;
/// the returned ops reconcile `input` onto it. Pure: no GPU / no DOM handle.
#[must_use]
pub fn build_overlay_ops(input: &OverlayDomInput<'_>, prev: &BTreeSet<DomKey>) -> Vec<DomOp> {
    let mut next: BTreeSet<DomKey> = BTreeSet::new();
    let mut ops = Vec::new();

    // --- nameplates (showcase.js L2056-2090) -------------------------------
    for plate in input.plates {
        if plate.tag.behind || !plate.tag.vis {
            continue;
        }
        let key = DomKey::Plate(plate.tag.row, plate.tag.slot);
        next.insert(key.clone());
        if !prev.contains(&key) {
            ops.push(DomOp::Create {
                key: key.clone(),
                tag: "div",
            });
        }
        let text = if plate.free {
            String::new()
        } else {
            plate.name.clone()
        };
        ops.push(DomOp::SetText {
            key: key.clone(),
            text,
        });
        // Position + visibility are resolved from the projected tag. `s` is
        // the right-vector projected scale (px/metre), forwarded as a CSS
        // custom property so the user-machine stylesheet can drive the size.
        ops.push(DomOp::SetStyle {
            key: key.clone(),
            prop: "transform",
            value: format!(
                "translate({:.2}px,{:.2}px) scale({:.4})",
                plate.tag.x,
                plate.tag.y,
                plate.tag.s.max(0.0)
            ),
        });
        let team = format!("iw-team-{}", plate.team);
        ops.push(DomOp::SetStyle {
            key,
            prop: "class",
            value: team,
        });
    }

    // --- boss damage numbers (hud-boss.js L402-440) ------------------------
    for d in input.damage {
        if d.out {
            continue;
        }
        // Stable key from birth time (not pool index): survives remove(0)/expiry.
        let key = DomKey::Damage(d.born.to_bits());
        next.insert(key.clone());
        if !prev.contains(&key) {
            ops.push(DomOp::Create {
                key: key.clone(),
                tag: "span",
            });
        }
        ops.push(DomOp::SetText {
            key: key.clone(),
            text: d.sum.to_string(),
        });
        let scale = d.display_scale();
        ops.push(DomOp::SetStyle {
            key,
            prop: "transform",
            value: format!("scale({scale:.4})"),
        });
    }

    // --- minimap transient FX (minimap.js L19-24) --------------------------
    for (i, fx) in input.map_fx.iter().enumerate() {
        let key = DomKey::MapFx(fx.kind, i);
        next.insert(key.clone());
        if !prev.contains(&key) {
            ops.push(DomOp::Create {
                key: key.clone(),
                tag: "span",
            });
        }
        ops.push(DomOp::SetStyle {
            key,
            prop: "class",
            value: format!("iw-mapfx-{}", map_fx_class(fx.kind)),
        });
    }

    // --- removals: keys present last frame but absent now ------------------
    for key in prev.iter().filter(|k| !next.contains(*k)) {
        ops.push(DomOp::Remove { key: key.clone() });
    }

    ops
}

/// Apply an op sequence to the key set, producing the key set for next frame.
#[must_use]
pub fn reconcile_keys(prev: &BTreeSet<DomKey>, ops: &[DomOp]) -> BTreeSet<DomKey> {
    let mut next = prev.clone();
    for op in ops {
        match op {
            DomOp::Create { key, .. } => {
                next.insert(key.clone());
            }
            DomOp::Remove { key } => {
                next.remove(key);
            }
            DomOp::SetText { .. } | DomOp::SetStyle { .. } => {}
        }
    }
    next
}

fn map_fx_class(kind: MapFxKind) -> &'static str {
    match kind {
        MapFxKind::Slam => "slam",
        MapFxKind::Boom => "boom",
        MapFxKind::Jump => "jump",
        MapFxKind::Spawn => "spawn",
    }
}

/// Native headless sink: records the op sequence per frame. This is the
/// sandbox substitute for real DOM nodes (AC-M6-2's explicit native口径).
#[derive(Resource, Default, Debug)]
pub struct RecordingDomSink {
    pub frames: Vec<Vec<DomOp>>,
    pub keys: BTreeSet<DomKey>,
}

impl RecordingDomSink {
    /// Cap on remembered frames (review P2-4): a long native session must not
    /// grow this debug/anchor buffer without bound. 10 min @60fps is ample.
    const FRAME_CAP: usize = 36_000;

    /// Apply one frame's ops, recording them and advancing the live key set.
    pub fn apply(&mut self, ops: Vec<DomOp>) {
        self.keys = reconcile_keys(&self.keys, &ops);
        self.frames.push(ops);
        if self.frames.len() > Self::FRAME_CAP {
            self.frames.remove(0);
        }
    }

    /// Total ops across all recorded frames (used by the headless anchors).
    #[must_use]
    #[allow(dead_code)]
    pub fn total_ops(&self) -> usize {
        self.frames.iter().map(Vec::len).sum()
    }
}

/// Main-world system: reconcile `CompositeOverlay` + `LobbyNameplates` into
/// the recording sink every frame (native + wasm both run this; the wasm sink
/// swaps in via its own system in the wasm-only module below).
pub fn overlay_dom_sync(
    overlay: bevy::prelude::Res<CompositeOverlay>,
    plates: bevy::prelude::Res<crate::ui::showcase_state::LobbyNameplates>,
    mut sink: bevy::prelude::ResMut<RecordingDomSink>,
) {
    let input = OverlayDomInput {
        plates: &plates.0,
        damage: &overlay.damage_numbers.items,
        map_fx: &overlay.minimap_fx.items,
        pass_enabled: overlay.pass_enabled,
    };
    let ops = build_overlay_ops(&input, &sink.keys);
    sink.apply(ops);
}

// --- wasm32 real-DOM sink (G-2) ---------------------------------------------
// Applies the SAME `DomOp` sequence to real nodes under `#iw-overlay`, keyed
// by a `data-iw-key` attribute. Only compiled for wasm32; visuals are a
// user-machine review item (D-M6-6) and never faked in the sandbox.

#[cfg(target_arch = "wasm32")]
mod wasm_sink {
    use super::*;
    use bevy::prelude::Res;
    use std::collections::BTreeMap;
    use wasm_bindgen::JsCast;
    use web_sys::{Document, Element, HtmlElement, Node};

    const ROOT_ID: &str = "iw-overlay";

    /// Opaque key string for the DOM attribute (stable across frames).
    fn key_str(key: &DomKey) -> String {
        match key {
            DomKey::Plate(r, s) => format!("p:{r}:{s}"),
            DomKey::Damage(i) => format!("d:{i}"),
            DomKey::MapFx(k, i) => format!("m:{}:{i}", map_fx_class(*k)),
        }
    }

    fn root(doc: &Document) -> Option<Element> {
        doc.get_element_by_id(ROOT_ID)
    }

    /// Apply one op to the document. Failures (no #iw-overlay root, e.g. the
    /// index.html has not been patched yet) are silently ignored so the game
    /// keeps running; the missing root is a user-machine setup item.
    fn apply_op(doc: &Document, nodes: &mut BTreeMap<String, Element>, op: &DomOp) {
        match op {
            DomOp::Create { key, tag } => {
                let k = key_str(key);
                if nodes.contains_key(&k) {
                    return;
                }
                if let (Ok(el), Some(root)) = (doc.create_element(tag), root(doc)) {
                    let _ = el.set_attribute("data-iw-key", &k);
                    if root.append_child(&el).is_ok() {
                        nodes.insert(k, el);
                    }
                }
            }
            DomOp::SetText { key, text } => {
                if let Some(el) = nodes.get(&key_str(key)) {
                    el.set_text_content(Some(text));
                }
            }
            DomOp::SetStyle { key, prop, value } => {
                if let Some(el) = nodes.get(&key_str(key)) {
                    if *prop == "class" {
                        let _ = el.set_attribute("class", value);
                    } else if let Ok(html) = el.clone().dyn_into::<HtmlElement>() {
                        let _ = html.style().set_property(prop, value);
                    }
                }
            }
            DomOp::Remove { key } => {
                let k = key_str(key);
                if let Some(el) = nodes.remove(&k) {
                    if let Some(parent) = el.parent_node() {
                        let _ = parent.remove_child(&el);
                    }
                }
            }
        }
    }

    /// wasm-only system: same reconcile as the native sink but drives real DOM.
    /// Registered by [`super::DomBridgePlugin`] on wasm builds.
    pub fn overlay_dom_wasm_sync(
        overlay: Res<CompositeOverlay>,
        plates: Res<crate::ui::showcase_state::LobbyNameplates>,
        mut state: bevy::prelude::Local<'_, WasmSinkState>,
    ) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let Some(doc) = window.document() else { return };
        let input = OverlayDomInput {
            plates: &plates.0,
            damage: &overlay.damage_numbers.items,
            map_fx: &overlay.minimap_fx.items,
            pass_enabled: overlay.pass_enabled,
        };
        let ops = build_overlay_ops(&input, &state.keys);
        for op in &ops {
            apply_op(&doc, &mut state.nodes.borrow_mut(), op);
        }
        state.keys = reconcile_keys(&state.keys, &ops);
    }

    /// Mutable state behind a `Local` (interior mutability for the node map).
    pub struct WasmSinkState {
        pub keys: BTreeSet<DomKey>,
        pub nodes: std::cell::RefCell<BTreeMap<String, Element>>,
    }

    impl Default for WasmSinkState {
        fn default() -> Self {
            Self {
                keys: BTreeSet::new(),
                nodes: std::cell::RefCell::new(BTreeMap::new()),
            }
        }
    }

    // `Node` is imported to keep the web-sys feature surface explicit; the
    // parent_node() call above returns it before remove_child.
    #[allow(dead_code)]
    fn _node_type(_: &Node) {}
}

/// Plugin: registers the DOM bridge. Native uses the recording sink (already
/// in the world Update chain via [`overlay_dom_sync`]); wasm additionally runs
/// the real-DOM sink. Added on both platforms — on wasm the recording sink
/// still runs as a headless-parity mirror (cheap, and keeps anchors identical).
pub struct DomBridgePlugin;

impl bevy::prelude::Plugin for DomBridgePlugin {
    fn build(&self, app: &mut bevy::prelude::App) {
        #[cfg(target_arch = "wasm32")]
        app.add_systems(bevy::prelude::Update, wasm_sink::overlay_dom_wasm_sync);
        let _ = app;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::composite::DamageNumber;
    use crate::ui::showcase_state::{LobbyNameplate, LobbyTag};

    fn plate(row: usize, slot: usize, x: f32, y: f32, vis: bool) -> LobbyNameplate {
        LobbyNameplate {
            tag: LobbyTag {
                x,
                y,
                behind: false,
                s: 1.0,
                vis,
                row,
                slot,
            },
            name: format!("p{slot}"),
            team: 1,
            ready: false,
            host: false,
            you: false,
            emote: None,
            free: false,
        }
    }

    #[test]
    fn task72_plate_create_then_update_then_remove() {
        let mut sink = RecordingDomSink::default();
        let p = plate(0, 0, 100.0, 200.0, true);
        let input = OverlayDomInput {
            plates: &[p],
            ..default_input()
        };
        let ops = build_overlay_ops(&input, &sink.keys);
        assert!(ops.iter().any(|o| matches!(o, DomOp::Create { .. })));
        sink.apply(ops);
        assert_eq!(sink.keys.len(), 1);

        // second frame: no Create, only SetText/SetStyle (idempotent reconcile)
        let p2 = plate(0, 0, 120.0, 220.0, true);
        let input2 = OverlayDomInput {
            plates: &[p2],
            ..default_input()
        };
        let ops2 = build_overlay_ops(&input2, &sink.keys);
        assert!(ops2.iter().all(|o| !matches!(o, DomOp::Create { .. })));
        sink.apply(ops2);

        // third frame: plate hidden → Remove
        let p3 = plate(0, 0, 120.0, 220.0, false);
        let input3 = OverlayDomInput {
            plates: &[p3],
            ..default_input()
        };
        let ops3 = build_overlay_ops(&input3, &sink.keys);
        assert!(ops3.iter().any(|o| matches!(o, DomOp::Remove { .. })));
        sink.apply(ops3);
        assert!(sink.keys.is_empty());
        assert_eq!(sink.frames.len(), 3);
    }

    #[test]
    fn task72_behind_plate_produces_no_node() {
        let mut p = plate(0, 0, 100.0, 200.0, true);
        p.tag.behind = true;
        let input = OverlayDomInput {
            plates: &[p],
            ..default_input()
        };
        let ops = build_overlay_ops(&input, &BTreeSet::new());
        assert!(ops.is_empty(), "behind plate must not create a DOM node");
    }

    #[test]
    fn task72_damage_number_updates_text_in_place() {
        let dmg = DamageNumber {
            sum: 42,
            last: 1.0,
            born: 1.0,
            weak: false,
            out: false,
        };
        let input = OverlayDomInput {
            damage: &[dmg],
            ..default_input()
        };
        let ops = build_overlay_ops(&input, &BTreeSet::new());
        let texts: Vec<&str> = ops
            .iter()
            .filter_map(|o| match o {
                DomOp::SetText { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(texts.contains(&"42"));
    }

    #[test]
    fn task72_low_tier_idle_is_empty_ops() {
        // Empty overlay state → no nodes, no ops (the low-tier idle path).
        let input = default_input();
        let ops = build_overlay_ops(&input, &BTreeSet::new());
        assert!(ops.is_empty());
    }

    #[test]
    fn task72_reconcile_keys_is_deterministic() {
        let k = DomKey::MapFx(MapFxKind::Slam, 0);
        let ops = vec![
            DomOp::Create {
                key: k.clone(),
                tag: "span",
            },
            DomOp::Remove { key: k.clone() },
        ];
        let a = reconcile_keys(&BTreeSet::new(), &ops);
        let b = reconcile_keys(&BTreeSet::new(), &ops);
        assert_eq!(a, b);
        assert!(a.is_empty(), "create then remove net to absent");
    }

    fn default_input<'a>() -> OverlayDomInput<'a> {
        OverlayDomInput {
            plates: &[],
            damage: &[],
            map_fx: &[],
            pass_enabled: false,
        }
    }
}
