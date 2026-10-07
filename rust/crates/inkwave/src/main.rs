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
mod ink_render;
mod input;
mod ui;
mod world;

struct Args {
    headless: bool,
    frames: u32,
    /// TR-16.1: native render-FPS bench for N seconds (0 = disabled).
    bench: u32,
}

/// Minimal flag parsing (works without extra deps; no-op flags on WASM).
fn parse_args() -> Args {
    let mut args = Args {
        headless: false,
        frames: 10,
        bench: 0,
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
            _ => {}
        }
    }
    args
}

#[derive(Resource)]
struct HeadlessFrames(u32);

fn main() {
    let args = parse_args();
    let mut app = App::new();

    if args.headless {
        // Headless smoke: scheduler only, no renderer/GPU.
        app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(
            Duration::from_secs_f64(1.0 / 60.0),
        )))
        .insert_resource(HeadlessFrames(args.frames))
        .add_systems(Update, exit_after_frames);
    } else {
        // TR-16.1: bench mode pins 1920x1080 + AutoNoVsync (Immediate->Mailbox
        // on a real GPU) so the measured rate is render+submit throughput,
        // not the display refresh cap.
        let bench_mode = args.bench > 0;
        app.add_plugins(DefaultPlugins.set(WindowPlugin {
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
        });
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
