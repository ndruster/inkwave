//! M4 Task 49 批次 C — the Bevy UI integration for the minimap.
//!
//! Drives the pure raster core ([`super::minimap`]) from the live
//! [`DemoSim`] and presents the composed canvas as a UI `ImageNode` in the
//! bottom-left corner, with the TAB-expanded box (≡ `hud.js` `_updMap`
//! L1724-1744) and the player / boss dots on top (≡ `main.js` L1293-1304 +
//! the `map.players` dot writes).
//!
//! JS mapping: `Minimap` ctor + `_build` + `setViewerTeam` (L63-97) →
//! `minimap_update`'s rebuild branch; `update` (L341-364) → the
//! [`MinimapClock`](super::minimap::MinimapClock) tick; `_compose` (L366-377)
//! → compose + texture upload; the HUD dot loop (hud.js L1745-1766) →
//! `minimap_ui`.
//!
//! Placeholder档 declared gaps (same convention as `boss_hud`): the dots are
//! flat colour squares (no circular clip / dark rim), the boss marker is a
//! Rust extension (spec 批次 B), and the deferred live layers (spawn pads,
//! transient fx, clouds, bombs, jump lines, beacons) stay out of scope per
//! D-T49-5 ①.

use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::ui::UiTransform;
use inkwave_sim::geometry::ZoneDef;
use inkwave_sim::match_::{MatchMode, Phase};
use inkwave_sim::tuning::map_theme;

use crate::actors::team_color;
use crate::ink_render::DemoSim;
use crate::ui::minimap as mm;
use crate::ui::{Screen, ScreenRoot, UiState};

/// JS `G.teamHex` concat (minimap.js L345): the team-colour identity that
/// invalidates the clock. The port's team colours are compile-time constants
/// (`actors.rs` `team_color`), so the key never changes mid-match; the
/// viewer-side flip is handled by the rebuild branch instead.
const TEAM_KEY: &str = "ff8a142f5bff";

/// One dot marker: `hud.js` L252 pre-allocates 8 (`mapDots`).
const DOT_POOL: usize = 8;
/// JS `.iw-mdot` size (12 px, centred by the −6 px offset).
const DOT_PX: f32 = 12.0;
/// The boss marker (Rust extension): a slightly larger chip.
const BOSS_PX: f32 = 16.0;

/// Per-match minimap state: the projector + raster tables (rebuilt on a
/// viewer-side flip, ≡ JS `setViewerTeam`), the layer buffers, the refresh
/// clock and the UI spring.
#[derive(Resource, Default)]
pub struct MinimapRes {
    proj: Option<mm::Projector>,
    flip: bool,
    tables: Option<mm::RasterTables>,
    styles: Vec<mm::BlockStyle>,
    scratch: Option<mm::InkScratch>,
    base: Vec<u8>,
    ink: Vec<u8>,
    flash: Vec<u8>,
    owner: Vec<u8>,
    zones: Vec<u8>,
    out: Vec<u8>,
    zone_paths: Vec<Vec<Vec<[f32; 2]>>>,
    /// `(zone count, active objective)` the paths were projected for.
    zone_key: Option<(usize, usize)>,
    clock: mm::MinimapClock,
    /// The base layer must be (re)drawn before the next compose.
    need_base: bool,
    image: Option<Handle<Image>>,
    spring: mm::MapSpring,
    /// JS `L.mapExp` latch (the TAB state stays latched through time's up).
    open: bool,
    /// Normalised boss marker position, `Some` only in a live boss match.
    boss: Option<(f32, f32)>,
}

/// The map box (corner ↔ expanded); its `ImageNode` carries the composed
/// canvas. Dots are absolute children, so they ride the box transform.
#[derive(Component)]
pub struct MinimapRoot;

/// One pooled player dot (JS `mapDots[i]`).
#[derive(Component)]
pub(crate) struct MinimapDot(usize);

/// The boss marker chip.
#[derive(Component)]
pub(crate) struct MinimapBossDot;

/// Spawn the minimap tree under the HUD `ScreenRoot` (placeholder 1×1
/// texture; `minimap_update` swaps in the real canvas on the first build).
/// Registered after `build_ui` in the Startup chain, so the query sees the
/// applied screen trees.
pub fn minimap_startup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    roots: Query<(Entity, &ScreenRoot)>,
) {
    commands.init_resource::<MinimapRes>();
    let Some(hud) = roots
        .iter()
        .find(|(_, r)| r.0 == Screen::Hud)
        .map(|(e, _)| e)
    else {
        return;
    };
    let placeholder = images.add(Image::new(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0u8; 4],
        TextureFormat::Rgba8Unorm,
        default(),
    ));
    let map = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(1.0),
                height: Val::Px(1.0),
                ..default()
            },
            ImageNode {
                image: placeholder,
                ..default()
            },
            MinimapRoot,
            ChildOf(hud),
        ))
        .id();
    for i in 0..DOT_POOL {
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(DOT_PX),
                height: Val::Px(DOT_PX),
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::NONE),
            MinimapDot(i),
            ChildOf(map),
        ));
    }
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            width: Val::Px(BOSS_PX),
            height: Val::Px(BOSS_PX),
            display: Display::None,
            ..default()
        },
        BackgroundColor(Color::NONE),
        MinimapBossDot,
        ChildOf(map),
    ));
}

/// Rebuild-on-flip + the per-frame clock tick + compose + texture upload
/// (≡ JS `Minimap.update` + `_compose`, minus the deferred live layers).
#[allow(clippy::too_many_arguments)]
pub fn minimap_update(
    time: Res<Time>,
    st: Res<UiState>,
    demo: Res<DemoSim>,
    mut res: ResMut<MinimapRes>,
    mut images: ResMut<Assets<Image>>,
    mut map_img: Query<&mut ImageNode, With<MinimapRoot>>,
) {
    // JS: the minimap only updates while the HUD is live (main.js frame loop);
    // paused / menu frames leave the clock untouched.
    if st.screen != Screen::Hud {
        return;
    }
    let dt = time.delta_secs();
    let m = &mut *res;
    let viewer_team = demo.m.actors.get(demo.player_slot).map_or(0, |a| a.team);
    let flip = viewer_team == 1;
    let mut force = false;
    if m.proj.is_none() || m.flip != flip {
        // JS `setViewerTeam` (L90-97): rebuild the projector + tables, force a
        // full base + ink pass this frame.
        let proj = mm::Projector::new(&demo.world.bounds, mm::MinimapTier::Med.px_per_m(), flip);
        let (styles, perch) = mm::layout_styles(&demo.layout.primitives, &demo.world.blocks);
        let tables = mm::RasterTables::build(&proj, &demo.world, &demo.m.paint, &perch);
        let n = proj.w as usize * proj.h as usize;
        let (pw, ph) = (proj.w, proj.h);
        m.flip = flip;
        m.tables = Some(tables);
        m.styles = styles;
        m.scratch = Some(mm::InkScratch::with_size(n));
        m.base = vec![0u8; n * 4];
        m.ink = vec![0u8; n * 4];
        m.flash = vec![0u8; n * 4];
        m.zones = vec![0u8; n * 4];
        m.out = vec![0u8; n * 4];
        m.owner = vec![0u8; n];
        m.zone_key = None;
        m.clock.invalidate();
        m.need_base = true;
        force = true;
        let image = images.add(Image::new(
            Extent3d {
                width: pw,
                height: ph,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            vec![0u8; n * 4],
            TextureFormat::Rgba8Unorm,
            default(),
        ));
        if let Ok(mut node) = map_img.single_mut() {
            node.image = image.clone();
        }
        m.image = Some(image);
        m.proj = Some(proj);
    }
    let (Some(proj), Some(tables)) = (&m.proj, &m.tables) else {
        return;
    };
    let h = proj.h as usize;
    let theme = map_theme(&demo.tuning.maps, &demo.layout.stage.id, "day");
    let tick = m
        .clock
        .begin(dt, demo.m.paint.version, h, TEAM_KEY, theme, force);
    let mut dirty = false;
    if m.need_base || tick.redraw_base {
        m.base = mm::draw_base(proj, tables, &m.styles, theme == "sunset");
        m.need_base = false;
        dirty = true;
    }
    if let Some(job) = &tick.ink {
        // `scratch` is `Some` whenever `proj` is (both built in the rebuild).
        if let Some(scratch) = &mut m.scratch {
            let flashes = mm::draw_ink(
                proj,
                tables,
                demo.m.paint.grid(),
                job.y0,
                job.y1,
                mm::TEAM_RGB,
                scratch,
                &mut m.ink,
                &mut m.flash,
                &mut m.owner,
            );
            m.clock.end(dt, Some(job), flashes);
            dirty = true;
        }
    } else {
        m.clock.end(dt, None, false);
    }
    // Zone Control overlay (JS `_compose` L376-377 + `_drawZones`): redrawn
    // every frame while live (the holder pulse animates at 4 rad/s).
    if let Some(zc) = &demo.m.zones {
        let obj = &zc.objectives[zc.active];
        let active: Vec<bool> = (0..zc.zones.len())
            .map(|i| obj.zones.contains(&i))
            .collect();
        let owners: Vec<i8> = zc.zones.iter().map(|z| z.owner).collect();
        let key = (zc.zones.len(), zc.active);
        if m.zone_key != Some(key) {
            let defs: Vec<&ZoneDef> = zc.zones.iter().map(|z| &z.def).collect();
            m.zone_paths = mm::zone_paths(proj, &defs);
            m.zone_key = Some(key);
        }
        mm::draw_zones(
            proj,
            &m.zone_paths,
            &active,
            &owners,
            mm::TEAM_RGB,
            m.clock.time,
            &mut m.zones,
        );
        dirty = true;
    }
    // Boss marker (Rust extension, spec 批次 B): only in a boss match, only
    // while the boss lives.
    m.boss = if demo.m.mode == MatchMode::Boss {
        demo.bm
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|bm| (bm.boss.pos, bm.boss.dead)))
            .filter(|(_, dead)| !*dead)
            .map(|(pos, _)| mm::boss_dot(pos.x, pos.z, proj))
    } else {
        None
    };
    if m.clock.flash_t < mm::FLASH_DECAY {
        dirty = true;
    }
    if dirty {
        let zopt = if demo.m.zones.is_some() {
            Some(m.zones.as_slice())
        } else {
            None
        };
        mm::compose(&m.base, &m.ink, &m.flash, m.clock.flash_t, zopt, &mut m.out);
        if let Some(handle) = &m.image
            && let Some(mut img) = images.get_mut(handle)
        {
            img.data = Some(m.out.clone());
        }
    }
}

/// The box spring + layout and the dot writes (≡ `hud.js` `_updMap`
/// L1714-1766).
#[allow(clippy::too_many_arguments)]
pub fn minimap_ui(
    time: Res<Time>,
    input: Res<ButtonInput<KeyCode>>,
    st: Res<UiState>,
    demo: Res<DemoSim>,
    mut res: ResMut<MinimapRes>,
    mut root_q: Query<&mut Node, With<MinimapRoot>>,
    mut dots: Query<(
        &mut Node,
        &mut BackgroundColor,
        &mut UiTransform,
        &MinimapDot,
    )>,
    mut boss_q: Query<(&mut Node, &mut BackgroundColor), With<MinimapBossDot>>,
    windows: Query<&Window>,
) {
    let m = &mut *res;
    // TAB toggle (JS: the controller latches `map.expanded`; the Rust desk UI
    // drives it straight from the key).
    if st.screen == Screen::Hud && input.just_pressed(KeyCode::Tab) {
        m.open = !m.open;
    }
    // L1714: the big map only opens during live play.
    let target = f32::from(m.open && st.screen == Screen::Hud && demo.m.phase == Phase::Active);
    m.spring.step(target, time.delta_secs());
    let Some(proj) = &m.proj else { return };
    let Some(win) = windows.iter().next() else {
        return;
    };
    // CSS px ≡ physical / scale factor (`innerWidth`/`innerHeight` in JS).
    let winres = &win.resolution;
    let size = winres.physical_size().as_vec2() / winres.scale_factor().max(1.0);
    let aspect = proj.w as f32 / proj.h as f32;
    let (x, y, bw, bh) = mm::map_layout(size.x, size.y, aspect, m.spring.t);
    if let Ok(mut node) = root_q.single_mut() {
        node.left = Val::Px(x);
        node.top = Val::Px(y);
        node.width = Val::Px(bw);
        node.height = Val::Px(bh);
    }
    let viewer_team = demo.m.actors.get(demo.player_slot).map_or(0, |a| a.team);
    let list = mm::player_dots(&demo.m.actors, viewer_team, demo.player_slot, proj);
    for (mut node, mut bg, mut ut, role) in dots.iter_mut() {
        let Some(p) = list.get(role.0) else {
            if node.display != Display::None {
                node.display = Display::None;
            }
            continue;
        };
        node.display = Display::Flex;
        // L1752: `clamp(p.x) * bw` with the dot centred by the −6 px margin.
        node.left = Val::Px(p.x.clamp(0.0, 1.0) * bw - DOT_PX * 0.5);
        node.top = Val::Px(p.y.clamp(0.0, 1.0) * bh - DOT_PX * 0.5);
        let c = team_color(p.team);
        // CSS truth (`styles/hud.css` L391): `.iw-mdot.is-dead b` is opacity
        // .7 — a defensive state, since `main.js` L1296 already filters dead
        // actors out of the dot list (the data source is always alive).
        // `is-tracked` has no opacity rule; its CSS effect is the `::after`
        // ping-ring animation, which the flat-square placeholder档 defers
        // (declared in the module header), so tracked dots stay full alpha.
        let alpha = if !p.alive { 0.7 } else { 1.0 };
        bg.0 = c.with_alpha(alpha);
        // L1765: only the self marker rotates with `p.yaw`.
        let rot = if p.is_self {
            Rot2::radians(p.yaw)
        } else {
            Rot2::default()
        };
        ut.rotation = rot;
    }
    for (mut node, mut bg) in boss_q.iter_mut() {
        match m.boss {
            Some((nx, ny)) => {
                node.display = Display::Flex;
                node.left = Val::Px(nx.clamp(0.0, 1.0) * bw - BOSS_PX * 0.5);
                node.top = Val::Px(ny.clamp(0.0, 1.0) * bh - BOSS_PX * 0.5);
                bg.0 = Color::srgba(1.0, 0.16, 0.16, 0.95);
            }
            None => node.display = Display::None,
        }
    }
}
