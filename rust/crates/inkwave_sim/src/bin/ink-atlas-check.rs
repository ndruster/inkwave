//! TR-11.2 headless evidence: replay a real 30 s autopilot splat stream onto
//! the ink atlas and prove there is no unbounded growth.
//!
//! The gameplay grid and the presentation bitmap consume the same recorded
//! stream (`PaintGrid::drain_ink` -> `InkAtlas::splat`), so the atlas sees the
//! exact splat flow a match produces. Checks per step: the pixel buffer never
//! reallocates, the dirty rect stays inside the atlas, and the version only
//! advances when pixels change. Prints a JSON summary for the evidence log.
use inkwave_sim::actor::{ActorInput, FIXED_DT, SimWorld};
use inkwave_sim::bot::{Bot, BotCtx};
use inkwave_sim::collision::CollisionWorld;
use inkwave_sim::embedded_tidewater;
use inkwave_sim::ink_atlas::InkAtlas;
use inkwave_sim::match_::{Match, Phase};
use inkwave_sim::nav::NavGraph;

fn main() {
    let duration = 30.0f32;
    let seed = 20261004u64;
    let layout = embedded_tidewater();
    let world = CollisionWorld::from_layout(&layout);
    let tuning = inkwave_sim::embedded_tuning();
    let nav = NavGraph::new(&world, &layout, &tuning.player);
    let sim = SimWorld::new(&world, &layout);
    let mut m = Match::new(duration, seed, &sim, &tuning.player, &tuning.match_config);
    let n = m.actors.len();
    let mut bots: Vec<Bot> = m
        .actors
        .iter()
        .enumerate()
        .map(|(i, a)| Bot::new(a, &tuning.difficulty.easy, seed, i))
        .collect();
    let mut inputs = vec![ActorInput::default(); n];
    let mut mate_goals = vec![None; n];

    let mut atlas = InkAtlas::new(&world);
    let expect_len = (atlas.size() as usize) * (atlas.size() as usize) * 4;
    let mut steps = 0u32;
    let mut splats = 0u64;
    let mut uploads = 0u64;
    let mut max_dirty_area = 0usize;
    let mut version = atlas.version();

    while m.elapsed() < duration && m.phase != Phase::End {
        if m.phase == Phase::Active {
            let snap = m.actors.clone();
            for (i, b) in bots.iter().enumerate() {
                mate_goals[i] = b.goal_node();
            }
            for i in 0..n {
                let ctx = BotCtx {
                    actors: &snap,
                    nav: &nav,
                    paint: &m.paint,
                    world: &sim,
                    t: &tuning.player,
                    w: &tuning.spritzer,
                    mate_goals: &mate_goals,
                };
                inputs[i] = bots[i].step(FIXED_DT, &mut m.actors[i], &ctx);
            }
        } else {
            for i in 0..n {
                inputs[i] = bots[i].hold(&m.actors[i]);
            }
        }
        m.step(FIXED_DT, &inputs, &sim, &tuning.player, &tuning.spritzer);
        steps += 1;

        for s in m.paint.drain_ink() {
            splats += 1;
            atlas.splat(&world, s.center, s.radius, s.team, &s.opts);
        }
        assert_eq!(atlas.pixels().len(), expect_len, "buffer reallocated");
        if atlas.version() != version {
            uploads += 1;
            version = atlas.version();
            if let Some((x0, y0, x1, y1)) = atlas.take_dirty() {
                assert!(
                    x1 < atlas.size() && y1 < atlas.size(),
                    "dirty out of bounds"
                );
                max_dirty_area =
                    max_dirty_area.max((x1 - x0 + 1) as usize * (y1 - y0 + 1) as usize);
            }
        }
    }

    let cov = m.paint.coverage();
    println!(
        "{{\"seed\":{seed},\"duration\":{duration},\"steps\":{steps},\"splats\":{splats},\
         \"uploads\":{uploads},\"atlas_bytes\":{expect_len},\"max_dirty_texels\":{max_dirty_area},\
         \"coverage\":[{:.4},{:.4}]}}",
        cov[0], cov[1]
    );
}
