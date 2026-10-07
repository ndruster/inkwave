//! Render-FPS bench mode (TR-16.1), native only.
//!
//! `--bench N` opens a 1920x1080 window with `PresentMode::AutoNoVsync`
//! (Immediate/Mailbox on a real GPU, so the numbers are render+submit
//! throughput, not the display refresh cap),
//! auto-starts a match (menu -> Hud via the same `apply_action` path a PLAY
//! button click takes), samples the presented frame interval every render
//! frame, and prints p50/p95 FPS to stderr before exiting.
//!
//! The window must stay foregrounded: minimized/out-of-focus windows throttle
//! the render loop and the stats would measure the throttle, not the GPU.
use std::time::{Duration, Instant};

use bevy::ecs::message::MessageWriter;
use bevy::prelude::*;

use crate::ink_render::DemoSim;
use crate::input::PlayerControls;
use crate::ui::{Action, Screen, UiState, apply_action};

#[derive(Resource)]
pub struct Bench {
    pub duration: Duration,
    pub started: Instant,
    pub auto_started: bool,
    pub prev: Option<Instant>,
    pub intervals: Vec<f64>,
    pub done: bool,
}

/// Auto-start once the UI is up: same transition as clicking PLAY (180 s).
pub fn bench_auto_start(
    mut st: ResMut<UiState>,
    mut pc: ResMut<PlayerControls>,
    mut demo: ResMut<DemoSim>,
    mut b: ResMut<Bench>,
) {
    if b.auto_started || b.done {
        return;
    }
    if st.screen == Screen::Menu {
        let mut quit = false;
        apply_action(&mut st, &mut pc, Action::Play(180), &mut demo, &mut quit);
        b.auto_started = true;
        info!("[bench] auto-started match (menu -> Hud)");
    }
}

/// Sample presented-frame intervals; one entry per render frame.
pub fn bench_sample(mut b: ResMut<Bench>, mut exits: MessageWriter<AppExit>) {
    if b.done {
        return;
    }
    let now = Instant::now();
    if b.started.elapsed() >= b.duration {
        b.done = true;
        report(&b.intervals);
        exits.write(AppExit::Success);
        return;
    }
    if let Some(p) = b.prev {
        b.intervals
            .push(now.duration_since(p).as_secs_f64() * 1000.0);
    }
    b.prev = Some(now);
}

fn report(intervals: &[f64]) {
    if intervals.is_empty() {
        eprintln!("[bench] no render frames sampled (window throttled or minimized?)");
        return;
    }
    let mut s: Vec<f64> = intervals.iter().copied().collect();
    s.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |q: f64| s[((s.len() - 1) as f64 * q).round() as usize];
    let mean = s.iter().sum::<f64>() / s.len() as f64;
    eprintln!(
        "[bench] frames={} mean={:.3}ms p50={:.3}ms p95={:.3}ms | render p50 FPS={:.1} \
         render p95 FPS={:.1} (wall-clock presented-frame rate; keep window foregrounded)",
        s.len(),
        mean,
        pct(0.50),
        pct(0.95),
        1000.0 / pct(0.50),
        1000.0 / pct(0.95),
    );
}
