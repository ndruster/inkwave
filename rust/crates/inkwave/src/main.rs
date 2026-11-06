//! INKWAVE Rust port — Bevy application entry point.
//!
//! Same binary for both targets:
//! - native window / WASM page: full `DefaultPlugins` (renderer + input);
//! - `--headless`: no window, fixed-timestep `ScheduleRunnerPlugin` — used by
//!   CI/smoke tests and by the future autopilot (FR-16) where a GPU/display is
//!   unavailable. `--frames N` exits after N update frames.
use std::time::Duration;

use bevy::app::{AppExit, ScheduleRunnerPlugin};
use bevy::ecs::message::MessageWriter;
use bevy::prelude::*;
use bevy::window::PresentMode;

#[cfg(not(target_arch = "wasm32"))]
mod bench;

mod actors;
mod audio;
mod boss_render;
// Task 51: the FX system (≡ src/fx/fx.js + fxHooks.js) — data core, batched
// quad renderer and the gameplay → VFX hooks, wired by `WorldPlugin`.
mod fx;
// Task 52 batch E: the ripple table (≡ paint.js L551-587) is consumed by the
// E2 wiring; the quality tier seed stays allow-gated until Task 55.
mod ink_render;
mod ink_ripple;
mod input;
mod level_render;
mod net;
mod nettest;
#[allow(dead_code)] // Task 55 消费后移除
mod quality;
// Task 52: schema/materials/bake/res are all wired through
// `level_render` + `world::build_level_world` (batch E2 bake + upload).
mod texlib;
mod ui;
mod world;

struct Args {
    headless: bool,
    frames: u32,
    /// TR-16.1: native render-FPS bench for N seconds (0 = disabled).
    bench: u32,
    /// Task 24: `--relay=ws://…` relay override (native; the wasm build has
    /// no CLI and reads `?relay=` from the page instead).
    relay: Option<String>,
    /// Task 46 (TR-46.1): `--mode boss` runs the headless Boss Battle smoke
    /// (the offline `DemoSim` boss match, no renderer); default `turf` keeps
    /// the scheduler-only smoke untouched.
    mode: String,
}

/// Minimal flag parsing (works without extra deps; no-op flags on WASM).
fn parse_args() -> Args {
    let mut args = Args {
        headless: false,
        frames: 10,
        bench: 0,
        relay: None,
        mode: "turf".to_string(),
    };
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--headless" => args.headless = true,
            "--frames" => {
                if let Some(v) = iter.next() {
                    args.frames = v.parse().unwrap_or(args.frames);
                }
            }
            "--bench" => {
                if let Some(v) = iter.next() {
                    args.bench = v.parse().unwrap_or(args.bench);
                }
            }
            "--mode" => {
                if let Some(v) = iter.next() {
                    args.mode = v;
                }
            }
            other => {
                if let Some(v) = other.strip_prefix("--relay=") {
                    args.relay = Some(v.to_string());
                }
            }
        }
    }
    args
}

#[derive(Resource)]
struct HeadlessFrames(u32);

/// Task 46 (TR-46.1): headless Boss Battle smoke state — the app's own
/// `DemoSim` driving a boss match with no renderer (≡ the offline menu's
/// `replay_boss` + per-frame `step_once`).
#[derive(Resource)]
struct HeadlessBoss {
    demo: ink_render::DemoSim,
    left: u32,
    total: u32,
}

impl HeadlessBoss {
    fn new(frames: u32) -> Self {
        let layout = inkwave_sim::embedded_tidewater();
        let world = inkwave_sim::collision::CollisionWorld::from_layout(&layout);
        let mut demo = ink_render::DemoSim::new(layout, world, 20261004);
        demo.replay_boss(0.0, "normal");
        Self {
            demo,
            left: frames,
            total: frames,
        }
    }
}

fn headless_boss_step(mut hb: ResMut<HeadlessBoss>, mut exit: MessageWriter<AppExit>) {
    if hb.left > 0 {
        hb.left -= 1;
        hb.demo.step_once();
        return;
    }
    let d = &hb.demo;
    let bmg = d.bm.lock().unwrap();
    let bm = bmg
        .as_ref()
        .expect("headless boss: the BossMode must be live");
    // TR-46.1: the boss event stream must reach the app (JS
    // `emit('boss:*')`) and the boss clock must advance. 600 frames @60 fps
    // = 10 s of a 240 s match, so no settlement is expected — the state must
    // simply be progressing.
    assert!(
        !d.boss_events.is_empty(),
        "headless boss: no boss events after {} frames",
        hb.total
    );
    assert!(
        d.boss_events.iter().any(|e| matches!(
            e,
            inkwave_sim::boss::BossEvent::Move(_)
                | inkwave_sim::boss::BossEvent::Hp { .. }
                | inkwave_sim::boss::BossEvent::Intro
        )),
        "headless boss: no Move/Hp/Intro events in the stream"
    );
    assert!(
        bm.boss.bt > 0.0,
        "headless boss: the boss clock never advanced"
    );
    println!(
        "[inkwave] headless boss: frames={} bt={:.1} hp={:.0}/{:.0} phase={} events={}",
        hb.total,
        bm.boss.bt,
        bm.boss.hp,
        bm.boss.max_hp,
        bm.boss.phase,
        d.boss_events.len()
    );
    exit.write(AppExit::Success);
}

fn main() {
    let args = parse_args();
    let mut app = App::new();

    if args.headless {
        if args.mode == "boss" {
            // Task 46 TR-46.1: headless Boss Battle smoke — drive the app's
            // own `DemoSim` boss match frame by frame (no renderer) and
            // assert the boss event stream / clock advance.
            app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(
                Duration::from_secs_f64(1.0 / 60.0),
            )))
            .insert_resource(HeadlessBoss::new(args.frames))
            .add_systems(Update, headless_boss_step);
        } else {
            // Headless smoke: scheduler only, no renderer/GPU.
            app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(
                Duration::from_secs_f64(1.0 / 60.0),
            )))
            .insert_resource(HeadlessFrames(args.frames))
            .add_systems(Update, exit_after_frames);
        }
    } else {
        // TR-16.1: bench mode pins 1920x1080 + AutoNoVsync (Immediate->Mailbox
        // on a real GPU) so the measured rate is render+submit throughput,
        // not the display refresh cap.
        let bench_mode = args.bench > 0;
        // Task 24: resolve the relay base once at boot (`--relay=` > env >
        // localhost probe > PROD on native; `location` + `?relay=` on wasm).
        // WorldPlugin reads it to build the session resource.
        // Task 26: the test-only net smoke seam (wasm `?net…` query params;
        // the config is always `None` on native — no CLI seam).
        app.insert_resource(net::default_relay_base(args.relay.as_deref()))
            .insert_resource(nettest::NetTest::from_location())
            .add_plugins(DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: format!("INKWAVE (Rust port M1, sim {})", inkwave_sim::SIM_VERSION),
                    resolution: (1920, 1080).into(),
                    present_mode: if bench_mode {
                        PresentMode::AutoNoVsync
                    } else {
                        default()
                    },
                    ..default()
                }),
                ..default()
            }))
            .add_plugins(world::WorldPlugin {
                layout: inkwave_sim::embedded_tidewater(),
            })
            // Task 46 C1: the boss presentation tier (silhouette / telegraph
            // markers / crablets), driven off DemoSim.bm.
            .add_plugins(boss_render::BossRenderPlugin);
        #[cfg(not(target_arch = "wasm32"))]
        if bench_mode {
            app.insert_resource(bench::Bench {
                duration: Duration::from_secs(args.bench as u64),
                started: std::time::Instant::now(),
                auto_started: false,
                prev: None,
                intervals: Vec::new(),
                done: false,
            })
            .add_systems(Update, bench::bench_auto_start)
            .add_systems(Last, bench::bench_sample);
        }
    }

    app.add_systems(Startup, boot_banner);
    app.run();
}

fn boot_banner() {
    println!(
        "[inkwave] boot: sim crate {} (headless = {})",
        inkwave_sim::SIM_VERSION,
        cfg!(not(any(target_arch = "wasm32")))
    );
}

fn exit_after_frames(mut count: ResMut<HeadlessFrames>, mut exit: MessageWriter<AppExit>) {
    count.0 -= 1;
    if count.0 == 0 {
        println!("[inkwave] headless frames complete");
        exit.write(AppExit::Success);
    }
}
