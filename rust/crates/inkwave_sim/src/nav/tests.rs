//! Task 9 nav-graph tests: synthetic sampling/edge cases + Tidewater
//! connectivity and A* sanity (the autopilot's nav substrate).

use super::*;
use crate::collision::CollisionWorld;
use crate::geometry::{Bounds, Brush, BrushCommon, Mural};
use crate::tuning::Tuning;
use glam::vec3;

fn flags() -> BrushCommon {
    BrushCommon {
        tag: None,
        color: "#dddddd".into(),
        pattern: 0,
        paint: true,
        solid: true,
        grate: false,
        rail: false,
        roof: false,
        perch: false,
        no_nav: false,
        hidden: false,
        bevel: None,
        no_paint: Vec::new(),
        mural: Vec::<Mural>::new(),
        oct: None,
    }
}

fn bx(min: [f32; 3], max: [f32; 3]) -> Brush {
    Brush::Box {
        min,
        max,
        common: flags(),
    }
}

fn tuning() -> Tuning {
    crate::embedded_tuning()
}

const B_SMALL: Bounds = Bounds {
    min_x: -10.0,
    max_x: 10.0,
    min_z: -10.0,
    max_z: 10.0,
};

// ------------------------------------------------------------------ sampling

#[test]
fn flat_floor_samples_one_node_per_cell() {
    let world =
        CollisionWorld::from_brushes(&[bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0])], B_SMALL);
    let t = tuning();
    let g = NavGraph::without_layout(
        &world,
        [vec3(0.0, 0.0, -5.0), vec3(0.0, 0.0, 5.0)],
        4.2,
        &t.player,
    );
    // 20 m / 1 m step → 20×20 cells, every one sampled at y = 0.
    assert_eq!(g.nodes.len(), 400);
    assert!(g.nodes.iter().all(|n| n.pos.y.abs() < 1e-3));
    assert_eq!(g.valid_ids.len(), 400);
    // interior nodes have all 8 neighbours; edges have fewer.
    let interior = g.nodes.iter().filter(|n| n.nb.len() == 8).count();
    assert!(interior > 300, "most nodes fully connected: {interior}");
}

#[test]
fn roof_top_is_not_sampled() {
    let world = CollisionWorld::from_brushes(
        &[
            bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0]),
            Brush::Box {
                min: [-2.0, 0.0, -2.0],
                max: [2.0, 1.0, 2.0],
                common: BrushCommon {
                    roof: true,
                    ..flags()
                },
            },
        ],
        B_SMALL,
    );
    let t = tuning();
    let g = NavGraph::without_layout(
        &world,
        [vec3(0.0, 0.0, -5.0), vec3(0.0, 0.0, 5.0)],
        4.2,
        &t.player,
    );
    assert!(
        g.nodes.iter().all(|n| n.pos.y < 0.5),
        "roof tops are off-limits for nav"
    );
}

#[test]
fn step_pair_connects_walk_and_jump_edges() {
    // Two 10×10 decks at y=0 and y=1 sharing an edge at z=0; the 1 m height
    // step is a jump edge (0.5 < dy ≤ 1.25), the 4 m drop is a drop edge.
    let world = CollisionWorld::from_brushes(
        &[
            bx([-5.0, -1.0, -10.0], [5.0, 0.0, 0.0]),
            bx([-5.0, 0.0, 0.0], [5.0, 1.0, 10.0]),
            bx([-5.0, -5.0, -10.0], [5.0, -4.0, 10.0]),
        ],
        B_SMALL,
    );
    let t = tuning();
    let g = NavGraph::without_layout(
        &world,
        [vec3(0.0, 0.0, -5.0), vec3(0.0, 0.0, 5.0)],
        4.2,
        &t.player,
    );
    let low = g
        .nodes
        .iter()
        .find(|n| n.pos.z < 0.0 && n.pos.z > -1.0 && n.pos.y.abs() < 1e-3 && n.pos.x.abs() < 4.0)
        .unwrap();
    let high = g
        .nodes
        .iter()
        .find(|n| {
            n.pos.z > 0.0 && n.pos.z < 1.0 && (n.pos.y - 1.0).abs() < 1e-3 && n.pos.x.abs() < 4.0
        })
        .unwrap();
    let jump = low.nb.iter().find(|e| g.nodes[e.to].pos.y > 0.5);
    assert!(jump.is_some(), "up-step must be a jump edge");
    assert_eq!(jump.unwrap().kind, EdgeType::Jump);
    assert_eq!(g.edge_type(high.id, low.id), EdgeType::Drop);
    // walk edges stay flat
    let walk = low
        .nb
        .iter()
        .find(|e| (g.nodes[e.to].pos.y - low.pos.y).abs() < 0.1);
    assert_eq!(walk.unwrap().kind, EdgeType::Walk);
}

#[test]
fn wet_ring_marks_water_edges() {
    // A deck over open water: ground_height under it (skip_grates) is -inf
    // only where nothing supports; a grate deck reads wet on its rim.
    let mut f = flags();
    f.grate = true;
    f.paint = false;
    let world = CollisionWorld::from_brushes(
        &[
            bx([-10.0, -1.0, -10.0], [-6.0, 0.0, 10.0]),
            Brush::Box {
                min: [-6.0, -0.2, -10.0],
                max: [6.0, 0.0, 10.0],
                common: f,
            },
        ],
        B_SMALL,
    );
    let t = tuning();
    let g = NavGraph::without_layout(
        &world,
        [vec3(-8.0, 0.0, 0.0), vec3(8.0, 0.0, 0.0)],
        4.2,
        &t.player,
    );
    // grate tops are sampled (kids walk them) and flagged over water.
    let over = g.nodes.iter().filter(|n| n.over_water).count();
    assert!(over > 0, "grate over a void must sample as over_water");
    let rim = g
        .nodes
        .iter()
        .find(|n| n.pos.x > -6.0 && n.pos.x < -4.0)
        .unwrap();
    assert!(rim.wet >= 1, "node near the water edge is wet: {}", rim.wet);
}

#[test]
fn unreachable_island_is_pruned() {
    let world = CollisionWorld::from_brushes(
        &[
            bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0]),
            bx([-2.0, 6.0, -2.0], [2.0, 7.0, 2.0]),
        ],
        B_SMALL,
    );
    let t = tuning();
    let g = NavGraph::without_layout(
        &world,
        [vec3(0.0, 0.0, -5.0), vec3(0.0, 0.0, 5.0)],
        4.2,
        &t.player,
    );
    for n in &g.nodes {
        if n.pos.y > 3.0 {
            assert!(!g.is_valid(n.id), "island top must be pruned");
        } else {
            assert!(g.is_valid(n.id), "main deck stays valid");
        }
    }
}

#[test]
fn nearest_snaps_to_the_grid_and_path_walks_it() {
    let world =
        CollisionWorld::from_brushes(&[bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0])], B_SMALL);
    let t = tuning();
    let g = NavGraph::without_layout(
        &world,
        [vec3(0.0, 0.0, -5.0), vec3(0.0, 0.0, 5.0)],
        4.2,
        &t.player,
    );
    let a = g.nearest(vec3(-4.3, 0.0, -4.1), 1.2, true).unwrap();
    // stay clear of the team-1 spawn zone (barrier + 0.6 around (0, 5))
    let b = g.nearest(vec3(4.2, 0.0, 2.3), 1.2, false).unwrap();
    assert_ne!(a, b);
    let p = g.path(a, b, 0).unwrap();
    assert_eq!(*p.first().unwrap(), a);
    assert_eq!(*p.last().unwrap(), b);
    assert!(p.len() >= 9, "at least one node per metre of travel");
    // every hop is a real edge
    for w in p.windows(2) {
        assert!(g.nodes[w[0]].nb.iter().any(|e| e.to == w[1]));
    }
}

// ------------------------------------------------------------------ tidewater

#[test]
fn tidewater_graph_is_connected_between_the_spawn_pads() {
    let l = crate::embedded_tidewater();
    let world = CollisionWorld::from_layout(&l);
    let t = tuning();
    let g = NavGraph::new(&world, &l, &t.player);
    assert!(g.nodes.len() > 1000, "Tidewater must sample a dense graph");
    let pads = &l.stage.spawn_pads;
    let a = g
        .nearest(pads[0].into(), 2.0, true)
        .expect("team-0 pad has nav");
    // team 0 may not route *into* the enemy spawn zone (JS path() rule), so
    // aim for the closest node outside the barrier instead.
    let pad1 = Vec3::from(pads[1]);
    let b = g
        .valid_ids
        .iter()
        .filter(|&&id| g.nodes[id].zone != 1)
        .min_by(|&&x, &&y| {
            g.nodes[x]
                .pos
                .distance_squared(pad1)
                .partial_cmp(&g.nodes[y].pos.distance_squared(pad1))
                .unwrap()
        })
        .copied()
        .expect("nav exists outside the enemy zone");
    let p = g.path(a, b, 0).expect("pads are mutually reachable");
    assert!(p.len() > 20, "the pads are ~84 m apart: {p:?}", p = p.len());
    // team-0 routes must not pass through the team-1 spawn zone
    for id in &p {
        assert_ne!(g.nodes[*id].zone, 1, "team 0 routed through the enemy pad");
    }
    // the graph must cover most of the stage's walkable area
    let frac = g.valid_ids.len() as f32 / g.nodes.len() as f32;
    assert!(frac > 0.5, "prune keeps the playable majority: {frac:.2}");
}
