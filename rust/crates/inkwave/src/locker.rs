//! Task 69 — locker (character style catalog) data plane and selection state.
//!
//! Upstream has no `locker.js`. The face is `character-style.js` (literal tables
//! plus `resolveStyle`/`randomStyle`/`styleSwatch`), `menus.js` (tabs, setStyle
//! causes, and `_syncMe` sending style via `net.setMe`), and `showcase.js` (3D
//! preview). `rust/assets/locker.json` carries the extracted tables with the
//! source commit pinned and no hand-copied numbers. This module ports the
//! tables, `resolve_style`, `random_style`, `style_swatch`, the locker selection
//! state machine, and the 6-key style JVal flowing through the already-present
//! `inkwave_sim` style wire field. No wire or geometry change.
//!
//! D-T69-1: style persistence uses a separate key from the JS profile.
//! D-T69-2: 3D preview, cloth shader pattern, and switch animation are GPU review.
#![allow(dead_code)] // data-plane API surface; GPU/UI consumers are M5 follow-ups

use std::sync::OnceLock;

use inkwave_sim::net::wire::JVal;

const DOC: &str = include_str!("../../../assets/locker.json");

fn doc() -> &'static JVal {
    static VAL: OnceLock<JVal> = OnceLock::new();
    VAL.get_or_init(|| inkwave_sim::net::wire::parse_json(DOC).expect("locker.json parses"))
}

/// D-T69-1: a dedicated key for the Rust locker style, never the JS profile's
/// `inkwave.profile` key (the JS owns fields the port does not, so sharing a
/// key would clobber them — same discipline as D-T59-4 settings).
#[cfg(target_arch = "wasm32")]
pub const STORAGE_KEY: &str = "inkwave.rust.style";
#[cfg(not(target_arch = "wasm32"))]
pub const PATH_ENV: &str = "INKWAVE_LOCKER_PATH";

fn field<'a>(v: &'a JVal, key: &str) -> &'a JVal {
    v.field(key)
        .unwrap_or_else(|| panic!("locker.json: missing field {key}"))
}

fn array<'a>(v: &'a JVal, key: &str) -> &'a [JVal] {
    field(v, key)
        .as_array()
        .unwrap_or_else(|| panic!("locker.json: missing array {key}"))
}

fn int(v: &JVal) -> i64 {
    v.as_f64().expect("integer value") as i64
}

fn string(v: &'static JVal) -> &'static str {
    v.as_str().expect("string value")
}

// ------------------------------------------------------------------ style

/// A resolved character look — six append-only indices (JS `resolveStyle`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    pub hair: i64,
    pub skin: i64,
    pub outfit: i64,
    pub eyes: i64,
    pub hat: i64,
    pub brows: i64,
}

/// `wrap(v,n) = ((round(v) % n) + n) % n` (JS L84). Uses i64 arithmetic so a
/// partial/out-of-range value always maps into the table, like JS's `%`.
#[must_use]
pub fn wrap(v: i64, n: i64) -> i64 {
    ((v % n) + n) % n
}

/// Mirror of JS `resolveStyle(st, seed)` L86-94. Present fields wrap into their
/// table; this form takes a fully-specified style (no seed derivation).
#[must_use]
pub fn resolve_style(style: Style) -> Style {
    let counts = table_counts();
    Style {
        hair: wrap(style.hair, counts.hair),
        skin: wrap(style.skin, counts.skin),
        outfit: wrap(style.outfit, counts.outfit),
        eyes: wrap(style.eyes, counts.iris),
        hat: wrap(style.hat, counts.hat),
        brows: wrap(style.brows, counts.brows),
    }
}

/// JS `>>` is a SIGN-PROPAGATING (arithmetic) shift on the int32 of `seed`.
/// Upstream seeds are FNV hashes (`fnv(name) >>> 0`, character.js L54 /
/// menus.js L213), so ~half the values have bit 31 set: JS first coerces them
/// to signed int32, then shifts right (filling with sign bits). Rust must do
/// the same via `as i32` before `>>`; a plain `u32 as i64 >> n` would be a
/// logical shift and disagree for those seeds.
#[inline]
fn js_shr(seed: u32, n: u32) -> i64 {
    i64::from((seed as i32) >> n)
}

/// Resolve a PARTIAL style exactly as JS `resolveStyle` L86-94:
/// `hair = wrap(st.hair ?? seed % HAIR_STYLES, 8)`
/// `skin = wrap(st.skin ?? (seed >> 3) % SKIN, 9)`
/// `outfit = wrap(st.outfit ?? (hair + skin + (seed >> 6)) % OUTFITS, 10)` —
///   note it uses the ALREADY-RESOLVED `hair`/`skin` locals (L87-88), so a
///   provided hair/skin feeds the outfit derivation.
/// `eyes = wrap(st.eyes ?? (seed >> 9) % IRIS, 8)`; hat/brows default to 0.
#[must_use]
pub fn resolve_partial(partial: PartialStyle, seed: u32) -> Style {
    let c = table_counts();
    // Resolve hair/skin first — outfit's derived branch reads these locals.
    let hair = partial
        .hair
        .map_or_else(|| wrap(seed as i64, c.hair), |v| wrap(v, c.hair));
    let skin = partial
        .skin
        .map_or_else(|| wrap(js_shr(seed, 3), c.skin), |v| wrap(v, c.skin));
    let outfit = partial.outfit.map_or_else(
        || wrap(hair + skin + js_shr(seed, 6), c.outfit),
        |v| wrap(v, c.outfit),
    );
    let eyes = partial
        .eyes
        .map_or_else(|| wrap(js_shr(seed, 9), c.iris), |v| wrap(v, c.iris));
    Style {
        hair,
        skin,
        outfit,
        eyes,
        hat: partial.hat.map_or(0, |v| wrap(v, c.hat)),
        brows: partial.brows.map_or(0, |v| wrap(v, c.brows)),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PartialStyle {
    pub hair: Option<i64>,
    pub skin: Option<i64>,
    pub outfit: Option<i64>,
    pub eyes: Option<i64>,
    pub hat: Option<i64>,
    pub brows: Option<i64>,
}

/// `randomStyle(rng)` L97-103: rng() → [0,1); hat worn ~1/3 of the time.
#[must_use]
pub fn random_style(mut rng: impl FnMut() -> f64) -> Style {
    let c = table_counts();
    let pick = |n: i64, r: f64| -> i64 { (n - 1).min((r * n as f64) as i64) };
    Style {
        hair: pick(c.hair, rng()),
        skin: pick(c.skin, rng()),
        outfit: pick(c.outfit, rng()),
        eyes: pick(c.iris, rng()),
        hat: if rng() < 0.34 {
            1 + pick(c.hat - 1, rng())
        } else {
            0
        },
        brows: pick(c.brows, rng()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableCounts {
    pub skin: i64,
    pub outfit: i64,
    pub iris: i64,
    pub hair: i64,
    pub hat: i64,
    pub brows: i64,
}

#[must_use]
pub fn table_counts() -> TableCounts {
    let t = field(doc(), "tables");
    TableCounts {
        skin: array(field(t, "skin"), "tones").len() as i64,
        outfit: array(field(t, "outfit"), "items").len() as i64,
        iris: array(field(t, "iris"), "gradients").len() as i64,
        hair: int(field(field(t, "hair"), "count")),
        hat: array(field(t, "hat"), "kinds").len() as i64,
        brows: array(field(t, "brows"), "kinds").len() as i64,
    }
}

/// `styleSwatch(st)` L106-110: UI chip colours for a resolved style.
#[derive(Clone, Debug, PartialEq)]
pub struct Swatch {
    pub skin: &'static str,
    pub eyes: [&'static str; 2],
    pub shirt: &'static str,
    pub shorts: &'static str,
    pub shoe: &'static str,
    pub sock: &'static str,
    pub strap: &'static str,
}

#[must_use]
pub fn style_swatch(style: Style) -> Swatch {
    let t = field(doc(), "tables");
    let skin = array(field(t, "skin"), "tones");
    let iris = array(field(t, "iris"), "gradients");
    let outfit = array(field(t, "outfit"), "items");
    let o = &outfit[style.outfit as usize];
    let ir = iris[style.eyes as usize]
        .as_array()
        .expect("iris gradient is a 2-array");
    Swatch {
        skin: string(&skin[style.skin as usize]),
        eyes: [string(&ir[0]), string(&ir[1])],
        shirt: string(field(o, "shirt")),
        shorts: string(field(o, "shorts")),
        shoe: string(field(o, "shoe")),
        sock: string(field(o, "sock")),
        strap: string(field(o, "strap")),
    }
}

// ------------------------------------------------------------------ selection state

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockerTab {
    Kids,
    Hair,
    Face,
    Outfit,
}

/// `setStyle(style, cause)` cause set (showcase.js L1159): hair/eyes/skin/outfit/
/// preset/random.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeCause {
    Hair,
    Eyes,
    Skin,
    Outfit,
    Preset,
    Random,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockerAction {
    SelectTab(LockerTab),
    StepItem { tab: LockerTab, delta: i64 },
    ApplyPreset(usize),
    Shuffle(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct LockerState {
    pub tab: LockerTab,
    pub style: Style,
    pub cause: ChangeCause,
    pub dirty: bool,
}

impl Default for LockerState {
    fn default() -> Self {
        Self {
            tab: LockerTab::Kids,
            style: resolve_partial(PartialStyle::default(), 0),
            cause: ChangeCause::Preset,
            dirty: false,
        }
    }
}

impl LockerState {
    #[must_use]
    pub fn apply(&self, action: LockerAction) -> Self {
        let mut next = self.clone();
        next.dirty = true;
        match action {
            LockerAction::SelectTab(tab) => next.tab = tab,
            LockerAction::StepItem { tab, delta } => {
                next.tab = tab;
                let c = table_counts();
                match tab {
                    LockerTab::Hair => next.style.hair = wrap(next.style.hair + delta, c.hair),
                    LockerTab::Face => next.style.skin = wrap(next.style.skin + delta, c.skin),
                    LockerTab::Outfit => {
                        next.style.outfit = wrap(next.style.outfit + delta, c.outfit)
                    }
                    LockerTab::Kids => {}
                }
                next.cause = match tab {
                    LockerTab::Hair => ChangeCause::Hair,
                    LockerTab::Face => ChangeCause::Skin,
                    LockerTab::Outfit => ChangeCause::Outfit,
                    LockerTab::Kids => ChangeCause::Preset,
                };
            }
            LockerAction::ApplyPreset(i) => {
                let p = &array(field(doc(), "tables"), "presets")[i];
                let s = field(p, "style");
                next.style = Style {
                    hair: int(field(s, "hair")),
                    skin: int(field(s, "skin")),
                    outfit: int(field(s, "outfit")),
                    eyes: int(field(s, "eyes")),
                    hat: int(field(s, "hat")),
                    brows: int(field(s, "brows")),
                };
                next.cause = ChangeCause::Preset;
            }
            LockerAction::Shuffle(seed) => {
                let mut s = seed as u64;
                let mut rng = move || {
                    s ^= s << 13;
                    s ^= s >> 7;
                    s ^= s << 17;
                    (s as f64) / u64::MAX as f64
                };
                next.style = random_style(&mut rng);
                next.cause = ChangeCause::Random;
            }
        }
        next
    }
}

/// Build the 6-key `style` JVal that `_syncMe` sends via `net.setMe({style})`.
#[must_use]
pub fn style_jval(style: Style) -> JVal {
    JVal::Obj(vec![
        ("hair".into(), JVal::Num(style.hair as f64)),
        ("skin".into(), JVal::Num(style.skin as f64)),
        ("outfit".into(), JVal::Num(style.outfit as f64)),
        ("eyes".into(), JVal::Num(style.eyes as f64)),
        ("hat".into(), JVal::Num(style.hat as f64)),
        ("brows".into(), JVal::Num(style.brows as f64)),
    ])
}

/// Decode a `style` JVal from the wire (tolerant per-key, defaults derive from
/// seed) back into a `Style`, mirroring JS `resolveStyle`.
#[must_use]
pub fn style_from_jval(v: &JVal, seed: u32) -> Style {
    let num = |k: &str| v.field(k).and_then(JVal::as_f64).map(|x| x as i64);
    resolve_partial(
        PartialStyle {
            hair: num("hair"),
            skin: num("skin"),
            outfit: num("outfit"),
            eyes: num("eyes"),
            hat: num("hat"),
            brows: num("brows"),
        },
        seed,
    )
}

// ------------------------------------------------------------------ persistence

impl Style {
    /// Fixed-field JSON writer (the 6 locked keys, same order as `style_jval`).
    #[must_use]
    pub fn to_json(self) -> String {
        format!(
            "{{\"hair\":{},\"skin\":{},\"outfit\":{},\"eyes\":{},\"hat\":{},\"brows\":{}}}",
            self.hair, self.skin, self.outfit, self.eyes, self.hat, self.brows
        )
    }
}

/// Decode a persisted style document. Tolerant per key: an unparseable field
/// is dropped and derived from the seed (same discipline as settings.rs, JS
/// `loadJSON` keeps other keys). Missing/extra keys are ignored.
#[must_use]
pub fn style_from_json(json: &str, seed: u32) -> Option<Style> {
    let v = inkwave_sim::net::wire::parse_json(json)?;
    if !matches!(v, JVal::Obj(_)) {
        return None;
    }
    Some(style_from_jval(&v, seed))
}

#[cfg(not(target_arch = "wasm32"))]
fn locker_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var(PATH_ENV) {
        return std::path::PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    std::path::PathBuf::from(home)
        .join(".inkwave")
        .join("style.json")
}

/// Native: write the style to the dedicated JSON file (D-T69-1).
#[cfg(not(target_arch = "wasm32"))]
pub fn save_style(style: Style) {
    let path = locker_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, style.to_json());
}

/// Native: read the persisted style (tolerant), or `None` if absent/invalid.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_style(seed: u32) -> Option<Style> {
    let json = std::fs::read_to_string(locker_path()).ok()?;
    style_from_json(&json, seed)
}

/// wasm: persist under the dedicated localStorage key, never the JS profile.
#[cfg(target_arch = "wasm32")]
pub fn save_style(style: Style) {
    if let Some(st) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = st.set_item(STORAGE_KEY, &style.to_json());
    }
}

#[cfg(target_arch = "wasm32")]
pub fn load_style(seed: u32) -> Option<Style> {
    let json = web_sys::window()?
        .local_storage()
        .ok()
        .flatten()?
        .get_item(STORAGE_KEY)
        .ok()
        .flatten()?;
    style_from_json(&json, seed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task69_tables_and_reference_pinned() {
        let c = table_counts();
        assert_eq!(
            (c.skin, c.outfit, c.iris, c.hair, c.hat, c.brows),
            (9, 10, 8, 8, 4, 4)
        );
        // resolveStyle reference values from the extracted JS calls.
        let d = resolve_partial(PartialStyle::default(), 0);
        assert_eq!(
            d,
            Style {
                hair: 0,
                skin: 0,
                outfit: 0,
                eyes: 0,
                hat: 0,
                brows: 0
            }
        );
        let p = resolve_partial(
            PartialStyle {
                outfit: Some(13),
                ..Default::default()
            },
            12345,
        );
        assert_eq!(
            p,
            Style {
                hair: 1,
                skin: 4,
                outfit: 3,
                eyes: 0,
                hat: 0,
                brows: 0
            }
        );
        let sw = style_swatch(Style {
            hair: 0,
            skin: 0,
            outfit: 0,
            eyes: 0,
            hat: 0,
            brows: 0,
        });
        assert_eq!(
            (sw.skin, sw.shirt, sw.shorts),
            ("#ffd9c2", "#f4f2ec", "#27304a")
        );
        assert_eq!(sw.eyes, ["#ffcf3a", "#ff7a00"]);
    }

    #[test]
    fn task69_style_persistence_is_separate_and_tolerant() {
        let style = Style {
            hair: 1,
            skin: 5,
            outfit: 9,
            eyes: 1,
            hat: 0,
            brows: 1,
        };
        // JSON roundtrip.
        let json = style.to_json();
        assert_eq!(
            json,
            "{\"hair\":1,\"skin\":5,\"outfit\":9,\"eyes\":1,\"hat\":0,\"brows\":1}"
        );
        assert_eq!(style_from_json(&json, 0), Some(style));
        // Dedicated key: never the JS profile key (D-T69-1).
        #[cfg(target_arch = "wasm32")]
        assert_eq!(STORAGE_KEY, "inkwave.rust.style");
        // Tolerant per-key: a wrong-typed value is dropped and derived from
        // seed (parse-level failures like `1e999` reject the whole doc, same as
        // settings.rs — per-key tolerance applies only to parsed values).
        let tolerant = "{\"hair\":1,\"outfit\":null,\"brows\":2}";
        let got = style_from_json(tolerant, 12345).expect("tolerant decode");
        assert_eq!(got.hair, 1);
        assert_eq!(got.brows, 2);
        // outfit null → derived from resolved hair/skin + seed, in range.
        let c = table_counts();
        assert!(got.outfit >= 0 && got.outfit < c.outfit);
        assert_eq!(style_from_json("not json", 0), None);
        assert_eq!(style_from_json("[1,2]", 0), None);
    }

    #[test]
    fn task69_resolve_matches_js_partial_and_high_seed() {
        // Data-driven against the reference calls the JS module itself made
        // (locker.json reference.*), so the sign-shift and hair/skin-into-outfit
        // semantics are pinned to upstream, not to hand-written expectations.
        let r = field(doc(), "reference");
        let as_style = |v: &JVal| Style {
            hair: int(field(v, "hair")),
            skin: int(field(v, "skin")),
            outfit: int(field(v, "outfit")),
            eyes: int(field(v, "eyes")),
            hat: int(field(v, "hat")),
            brows: int(field(v, "brows")),
        };
        // outfit derivation uses the already-resolved hair/skin locals.
        let hs = resolve_partial(
            PartialStyle {
                hair: Some(5),
                skin: Some(3),
                ..Default::default()
            },
            12345,
        );
        assert_eq!(hs, as_style(field(r, "resolved_hair_skin")));
        // seed >= 2^31: JS >> is sign-propagating on int32, not a logical shift.
        let hi = resolve_partial(PartialStyle::default(), 2147483648);
        assert_eq!(hi, as_style(field(r, "resolved_high_seed")));
        // sanity: high-seed derivation actually differs from a logical shift.
        assert_ne!(
            hi.skin,
            wrap((2147483648u64 >> 3) as i64, table_counts().skin)
        );
    }

    #[test]
    fn task69_resolve_wrap_and_random_are_deterministic() {
        assert_eq!(wrap(-1, 9), 8);
        assert_eq!(wrap(13, 10), 3);
        assert_eq!(wrap(0, 4), 0);
        // All wrapped fields land in range for out-of-range inputs.
        let s = resolve_style(Style {
            hair: 99,
            skin: -3,
            outfit: 25,
            eyes: 100,
            hat: -1,
            brows: 8,
        });
        let c = table_counts();
        assert!(
            s.hair < c.hair
                && s.skin < c.skin
                && s.outfit < c.outfit
                && s.eyes < c.iris
                && s.hat < c.hat
                && s.brows < c.brows
        );
        // randomStyle with the extractor's reference rng sequence.
        let seq = [0.0f64, 0.99, 0.5, 0.2, 0.8, 0.33, 0.34, 0.1];
        let mut i = 0;
        let r = random_style(|| {
            let v = seq[i % seq.len()];
            i += 1;
            v
        });
        assert_eq!(
            r,
            Style {
                hair: 0,
                skin: 8,
                outfit: 5,
                eyes: 1,
                hat: 0,
                brows: 1
            }
        );
    }

    #[test]
    fn task69_selection_state_machine_and_wire_roundtrip() {
        let st = LockerState::default();
        let tabbed = st.apply(LockerAction::SelectTab(LockerTab::Hair));
        assert_eq!(tabbed.tab, LockerTab::Hair);
        let stepped = st.apply(LockerAction::StepItem {
            tab: LockerTab::Outfit,
            delta: 1,
        });
        assert_eq!(stepped.tab, LockerTab::Outfit);
        assert_eq!(stepped.cause, ChangeCause::Outfit);
        assert!(stepped.dirty);
        assert_eq!(stepped.style.outfit, (st.style.outfit + 1) % 10);
        let preset = st.apply(LockerAction::ApplyPreset(1)); // dash
        assert_eq!(
            preset.style,
            Style {
                hair: 1,
                skin: 5,
                outfit: 9,
                eyes: 1,
                hat: 0,
                brows: 1
            }
        );
        assert_eq!(preset.cause, ChangeCause::Preset);
        let shuffled = st.apply(LockerAction::Shuffle(42));
        assert_eq!(shuffled.cause, ChangeCause::Random);
        // style JVal has exactly the 6 keys _syncMe sends; roundtrip stable.
        let jv = style_jval(preset.style);
        if let JVal::Obj(pairs) = &jv {
            assert_eq!(pairs.len(), 6);
            assert_eq!(pairs[0].0, "hair");
            assert_eq!(pairs[5].0, "brows");
        } else {
            panic!("style must be an object");
        }
        assert_eq!(style_from_jval(&jv, 0), preset.style);
        // Tolerant decode: a missing/extra field still resolves in range.
        let partial = JVal::Obj(vec![
            ("outfit".into(), JVal::Num(3.0)),
            ("junk".into(), JVal::Null),
        ]);
        assert_eq!(style_from_jval(&partial, 0).outfit, 3);
    }
}
