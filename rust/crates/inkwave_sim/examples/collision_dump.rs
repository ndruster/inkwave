//! Dump collision-query results as JSON, for independent cross-validation
//! against `rust/tools/verify/collision_crosscheck.py` (TR-4.1/4.2).
//!
//! The probe list here is duplicated verbatim in the Python script (which
//! reimplements block frames, the spatial hash, groundHeight and raycast from
//! scratch). The Python driver runs this example and compares every value.
#![allow(clippy::approx_constant)]

use glam::vec3;
use inkwave_sim::collision::CollisionWorld;
use inkwave_sim::geometry::{Bounds, Brush, BrushCommon, Mural, StageLayout};

fn common() -> BrushCommon {
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

fn r4(v: f32) -> f64 {
    let v = (v as f64 * 1e4).round() / 1e4;
    if v == 0.0 { 0.0 } else { v }
}

fn main() {
    // ---- synthetic ground: flat box + one 4-m-rise / 8-m-run ramp + mirror
    let brushes = vec![
        Brush::Box {
            min: [-10.0, -1.0, -10.0],
            max: [-4.0, 0.0, 10.0],
            common: common(),
        },
        Brush::Ramp {
            low: [-3.0, 0.0, -4.0],
            high: [-3.0, 2.0, 4.0],
            width: 2.0,
            thickness: 0.3,
            thin: true,
            common: common(),
        },
        Brush::Ramp {
            low: [3.0, 0.0, -4.0],
            high: [3.0, 2.0, 4.0],
            width: 2.0,
            thickness: 0.3,
            thin: true,
            common: common(),
        },
    ];
    let bounds = Bounds {
        min_x: -10.0,
        max_x: 10.0,
        min_z: -10.0,
        max_z: 10.0,
    };
    let synth = CollisionWorld::from_brushes(&brushes, bounds);
    let tide = CollisionWorld::tidewater();
    let layout: StageLayout = inkwave_sim::embedded_tidewater();

    let mut out = String::from("[\n");
    let mut rec = |s: String| {
        out.push_str(&s);
        out.push_str(",\n");
    };

    // block/face structure counts
    rec(format!(
        "{{\"k\":\"struct\",\"world\":\"tide\",\"blocks\":{},\"faces\":{},\"has_rails\":{}}}",
        tide.blocks.len(),
        tide.faces.len(),
        tide.has_rails
    ));
    // one record per surviving face slot (block, axis, sign) — face-id order check
    for (bid, b) in tide.blocks.iter().enumerate() {
        for k in 0..3 {
            for (si, sign) in [1.0_f32, -1.0].into_iter().enumerate() {
                let fid = b.faces[k * 2 + si];
                if fid >= 0 {
                    let n = b.axes[k] * sign;
                    rec(format!(
                        "{{\"k\":\"face\",\"block\":{},\"ax\":{},\"sign\":{},\"nx\":{},\"ny\":{},\"nz\":{}}}",
                        bid,
                        k,
                        if sign > 0.0 { 1 } else { -1 },
                        r4(n.x),
                        r4(n.y),
                        r4(n.z)
                    ));
                }
            }
        }
    }

    // synthetic ground heights (surface y = (z+4)/4 on the ramps)
    for x in [-3.0_f32, 3.0] {
        for z in [-3.0_f32, -1.5, 0.0, 1.5, 3.0] {
            let y = synth.ground_height(x, z, 50.0, false);
            rec(format!(
                "{{\"k\":\"gh\",\"world\":\"synth\",\"x\":{},\"z\":{},\"y\":{}}}",
                r4(x),
                r4(z),
                if y.is_finite() {
                    r4(y).to_string()
                } else {
                    "null".into()
                }
            ));
        }
    }
    // flat box top
    rec(format!(
        "{{\"k\":\"gh\",\"world\":\"synth\",\"x\":{},\"z\":{},\"y\":{}}}",
        r4(-7.0),
        r4(0.0),
        r4(synth.ground_height(-7.0, 0.0, 50.0, false))
    ));

    // synthetic down-ray on the ramp: reports the slope normal
    let h = synth.raycast(vec3(-3.0, 3.0, 0.0), -glam::Vec3::Y, 6.0, false);
    rec(format!(
        "{{\"k\":\"ray\",\"world\":\"synth\",\"ox\":{},\"oy\":{},\"oz\":{},\"dx\":0,\"dy\":-1,\"dz\":0,\"hit\":{},\"dist\":{},\"nx\":{},\"ny\":{},\"nz\":{},\"block\":{},\"face\":{}}}",
        r4(-3.0),
        r4(3.0),
        r4(0.0),
        h.hit,
        r4(h.dist),
        r4(h.normal.x),
        r4(h.normal.y),
        r4(h.normal.z),
        h.block,
        h.face
    ));

    // tidewater ground-height grid (mirror pair points included)
    let xs = [-20.0_f32, -10.0, -5.0, -2.0, 0.0, 2.0, 5.0, 10.0, 20.0];
    let zs = [
        -40.0_f32, -30.0, -20.0, -10.0, -5.0, 0.0, 5.0, 10.0, 20.0, 30.0, 40.0,
    ];
    for &z in &zs {
        for &x in &xs {
            let y = tide.ground_height(x, z, 50.0, false);
            rec(format!(
                "{{\"k\":\"gh\",\"world\":\"tide\",\"x\":{},\"z\":{},\"y\":{}}}",
                r4(x),
                r4(z),
                if y.is_finite() {
                    r4(y).to_string()
                } else {
                    "null".into()
                }
            ));
        }
    }
    // spawn pads exactly
    for pad in &layout.stage.spawn_pads {
        let y = tide.ground_height(pad[0], pad[2], 50.0, false);
        rec(format!(
            "{{\"k\":\"gh\",\"world\":\"spawn\",\"x\":{},\"z\":{},\"y\":{}}}",
            r4(pad[0]),
            r4(pad[2]),
            if y.is_finite() {
                r4(y).to_string()
            } else {
                "null".into()
            }
        ));
    }

    // tidewater rays: horizontal sweeps at several heights + two verticals
    let rays: [(f32, f32, f32, f32, f32, f32); 12] = [
        (-25.0, 1.0, 0.0, 1.0, 0.0, 0.0),
        (25.0, 1.0, 0.0, -1.0, 0.0, 0.0),
        (-25.0, 4.0, 0.0, 1.0, 0.0, 0.0),
        (-25.0, 1.0, -20.0, 1.0, 0.0, 0.0),
        (-25.0, 1.0, 20.0, 1.0, 0.0, 0.0),
        (0.0, 1.0, 45.0, 0.0, 0.0, -1.0),
        (0.0, 1.0, -45.0, 0.0, 0.0, 1.0),
        (0.0, 6.0, -41.8, 0.0, -1.0, 0.0),
        (0.0, 6.0, 41.8, 0.0, -1.0, 0.0),
        (10.0, 3.0, 10.0, -0.7071, 0.0, -0.7071),
        (-10.0, 3.0, -10.0, 0.7071, 0.0, 0.7071),
        (20.0, 2.0, -30.0, -0.6, 0.8, 0.0),
    ];
    for (ox, oy, oz, dx, dy, dz) in rays {
        let d = vec3(dx, dy, dz).normalize();
        let h = tide.raycast(vec3(ox, oy, oz), d, 60.0, true);
        rec(format!(
            "{{\"k\":\"ray\",\"world\":\"tide\",\"ox\":{},\"oy\":{},\"oz\":{},\"dx\":{},\"dy\":{},\"dz\":{},\"hit\":{},\"dist\":{},\"nx\":{},\"ny\":{},\"nz\":{},\"block\":{},\"face\":{}}}",
            r4(ox),
            r4(oy),
            r4(oz),
            r4(d.x),
            r4(d.y),
            r4(d.z),
            h.hit,
            if h.hit {
                r4(h.dist).to_string()
            } else {
                "null".into()
            },
            if h.hit {
                r4(h.normal.x).to_string()
            } else {
                "null".into()
            },
            if h.hit {
                r4(h.normal.y).to_string()
            } else {
                "null".into()
            },
            if h.hit {
                r4(h.normal.z).to_string()
            } else {
                "null".into()
            },
            h.block,
            h.face
        ));
    }

    out.truncate(out.len() - 2); // trailing ",\n"
    out.push_str("\n]\n");
    print!("{out}");
}
