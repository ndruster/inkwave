//! Task 53 (D-T53-3) — prop animation: spinners and blink lights.
//!
//! ≡ `PropKit.update` (props.js L3265-3300):
//! - spin: `M(t) = base × rotY/Z(t·speed + phase)` — the archive stores the
//!   WORLD base matrix (props.schema.md), so the per-frame composition is
//!   exactly the JS one;
//! - blink: `s = 0.5 + 0.5·sin(t·rate·2π + phase)`, `k = s²(3−2s)`,
//!   `on = k > 0.6 ? 1 : k/0.6·0.25`, colour = base × (lo + (hi−lo)·on) —
//!   JS drives an InstancedMesh colour attribute; here each bulb is a small
//!   entity with its own unlit `ColorMaterial` (≡ the MeshBasic look).
use bevy::prelude::*;

use super::archive::{Blink, Spin};

/// Spinner entity: animates its transform every frame.
#[derive(Component, Clone, Copy)]
pub struct SpinEntity {
    /// Row-major 3×4 world base matrix (translation in `base[i][3]`).
    base: [[f32; 4]; 3],
    speed: f32,
    phase: f32,
    /// 0 = +Y (turbines), 1 = +Z (fans) — props.js L3270-3275.
    axis: u8,
}

impl SpinEntity {
    #[must_use]
    pub fn from_record(s: Spin) -> Self {
        Self {
            base: s.base,
            speed: s.speed,
            phase: s.phase,
            axis: s.axis,
        }
    }

    /// `base × rot(axis, t·speed + phase)` ≡ props.js L3270-3275.
    #[must_use]
    fn matrix_at(&self, time: f32) -> Mat4 {
        let a = time * self.speed + self.phase;
        let mut base = Mat4::from_cols_array_2d(&{
            let mut c = [[0.0f32; 4]; 4];
            c[0] = [self.base[0][0], self.base[1][0], self.base[2][0], 0.0];
            c[1] = [self.base[0][1], self.base[1][1], self.base[2][1], 0.0];
            c[2] = [self.base[0][2], self.base[1][2], self.base[2][2], 0.0];
            c[3] = [self.base[0][3], self.base[1][3], self.base[2][3], 1.0];
            c
        });
        let axis = if self.axis == 1 { Vec3::Z } else { Vec3::Y };
        base *= Mat4::from_axis_angle(axis, a);
        base
    }
}

/// Blink bulb entity: animates its material colour every frame.
#[derive(Component, Clone, Copy)]
pub struct BlinkEntity {
    color: [f32; 3],
    rate: f32,
    phase: f32,
    lo: f32,
    hi: f32,
}

impl BlinkEntity {
    #[must_use]
    pub fn from_record(b: Blink) -> Self {
        Self {
            color: b.color,
            rate: b.rate,
            phase: b.phase,
            lo: b.lo,
            hi: b.hi,
        }
    }

    /// Pulse factor `lo + (hi−lo)·on` ≡ props.js L3279-3284.
    #[must_use]
    fn pulse_at(&self, time: f32) -> f32 {
        let s = 0.5 + 0.5 * (time * self.rate * std::f32::consts::TAU + self.phase).sin();
        let k = s * s * (3.0 - 2.0 * s);
        let on = if k > 0.6 { 1.0 } else { k / 0.6 * 0.25 };
        self.lo + (self.hi - self.lo) * on
    }
}

/// Animate spinners: `M(t) = base × rot(axis, t·speed + phase)`.
pub fn props_spin_anim(time: Res<Time>, mut q: Query<(&SpinEntity, &mut Transform)>) {
    let t = time.elapsed_secs();
    for (s, mut transform) in q.iter_mut() {
        *transform = Transform::from_matrix(s.matrix_at(t));
    }
}

/// Animate blink bulbs: colour = base × pulse(t) (unlit base colour, ≡ the
/// JS MeshBasic instance colour).
pub fn props_blink_anim(
    time: Res<Time>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    q: Query<(&BlinkEntity, &MeshMaterial3d<StandardMaterial>)>,
) {
    let t = time.elapsed_secs();
    for (b, mat) in q.iter() {
        let p = b.pulse_at(t);
        if let Some(mut m) = mats.get_mut(&mat.0) {
            m.base_color = Color::linear_rgb(b.color[0] * p, b.color[1] * p, b.color[2] * p);
        }
    }
}
