//! Task 51 Batch E — swim wakes, ported from `src/fx/swimWake.js` (107 lines).
//!
//! Up to 4 swimmers near the camera (the local player always first) each keep a
//! 12-point trail of where they swam — a new point every ~0.75 m, the live head
//! as the last point, breaks where they left the ink (≡ swimWake.js L21-49
//! selection + L50-78 trail). The shader turns each segment into an expanding
//! ripple and a glossy mound over the submerged body.
//!
//! # 批次 E 落地面（D-T51-7⑦，如实登记）
//! - **数据层全量平移**：swimmer 筛选 + 轨迹 ring-buffer + 上传缓冲计算
//!   （≡ `update` L21-105 的 CPU 部分）。`SwimWakeState` 暴露与
//!   `uWake/uWakeB/uSwimH/uSwimF` 一一对应的 `[[f32; 4]; N]` 缓冲 getter。
//! - **上传点顺延**：ink.wgsl 无 uWake 组 uniform（已 grep 确认零匹配），
//!   本任务不扩 ink.wgsl 渲染面（任务书既定事实）——缓冲算好即冻结，
//!   供未来 swim-wake shader pass（Task 55 后处理链 / 水面材质扩展）接线。
//!   JS L22 的 `if (!U || !U.uWake) return` 门在 Rust 恒为「数据照算」（无 U 可查）。
//! - **climb 形态缺口**：JS 筛 `form === 'swim' || 'climb'`（L31），Rust 呈现层
//!   形态只有 [`FxForm`]（Swim ≡ F07 入水；climb 无对应动画态，hooks.rs 批次 D
//!   已登记同一偏差）——Rust 用 `actor_form(a) == Swim` 判定，climb 分支
//!   （L58 `v.y` 三维前向）随之缺省。
//! - `_wakeD2` 在 JS 挂在 actor 对象上（L34）；呈现层不得 mutate sim →
//!   候选距离存在本模块侧表（`cand` 局部向量），排序语义一致。
//! - 本地判定：JS `a.isLocal` ≡ `slot == demo.player_slot`（hooks.rs 先例）。

use bevy::prelude::*;
use glam::Vec3; // sim 侧 glam 0.30（render.rs L33 先例；bevy 面调用点显式转换）
use inkwave_sim::actor::{Actor, Form};

use crate::fx::particles::clamp;

/// ≡ swimWake.js L8 `SLOTS = 4`.
pub const SLOTS: usize = 4;
/// ≡ swimWake.js L8 `PTS = 12`.
pub const PTS: usize = 12;
/// Trail point lifetime, seconds (≡ swimWake.js L8 `LIFE = 1.15`).
pub const LIFE: f32 = 1.15;
/// Min horizontal spacing between trail points, m (≡ swimWake.js L8 `SPACING = 0.75`).
pub const SPACING: f32 = 0.75;
/// Trail-break sentinel in the `t` slot (≡ swimWake.js L9 `BREAK = -9`).
pub const BREAK: f32 = -9.0;
/// Cull radius for non-local swimmers, m (≡ swimWake.js L33 `d2 > 38 * 38`).
pub const CAM_RADIUS: f32 = 38.0;

/// One trail point — `(x, y, z, t)` with `t == BREAK` as the break marker
/// (≡ swimWake.js L64/L72 `{x, y, z, t}`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TrailPt {
    pub p: Vec3,
    pub t: f32,
}

/// One wake slot (≡ swimWake.js L13-15).
#[derive(Clone, Debug)]
pub struct WakeSlot {
    /// Actor index into `Match::actors`, `None` = free slot.
    pub actor: Option<usize>,
    pub pts: Vec<TrailPt>,
    /// Mound strength (≡ `s.k`).
    pub k: f32,
    /// Normalized speed envelope (≡ `s.speedK`).
    pub speed_k: f32,
    /// Swim heading (≡ `s.fwd`, default `(0,0,1)` L14).
    pub fwd: Vec3,
    /// Live head (≡ `s.head`).
    pub head: Vec3,
    /// Last pushed point (≡ `s.last`).
    pub last: Option<Vec3>,
    /// Selected this frame (≡ `s.swimming`).
    pub swimming: bool,
}

impl Default for WakeSlot {
    fn default() -> Self {
        Self {
            actor: None,
            pts: Vec::new(),
            k: 0.0,
            speed_k: 0.0,
            fwd: Vec3::Z,
            head: Vec3::ZERO,
            last: None,
            swimming: false,
        }
    }
}

/// Exponential approach toward `target` (≡ ctx.js L29 `damp`).
#[inline]
fn damp(a: f32, b: f32, rate: f32, dt: f32) -> f32 {
    a + (b - a) * (1.0 - (-rate * dt).exp())
}

/// The F07 swim state (≡ JS `anim.form === 'swim'`; `climb` has no Rust
/// presentation form — hooks.rs 批次 D 已登记同一偏差).
#[inline]
fn form_is_swim(a: &Actor) -> bool {
    a.form == Form::Squid && a.submerged
}

/// The swim-wake data layer. Holds the slot trails plus the four upload
/// buffers mirroring `uWake`/`uWakeB`/`uSwimH`/`uSwimF` (≡ swimWake.js L80-105).
///
/// **上传点顺延（D-T51-7⑦）**：ink.wgsl 无对应 uniform——缓冲只算不传，
/// getter 供未来 pass 接线。
#[derive(Resource, Debug)]
pub struct SwimWakeState {
    slots: [WakeSlot; SLOTS],
    /// Global presentation clock (≡ `G.time`, fed by the system).
    now: f32,
    /// ≡ `U.uWake.value`: `SLOTS * PTS` × `(x, y, z, t)`.
    wake: [[f32; 4]; SLOTS * PTS],
    /// ≡ `U.uWakeB.value`: per-slot `(cx, cy, cz, r + 1.6)`, `(0, -999, 0, 0)` = none.
    wake_b: [[f32; 4]; SLOTS],
    /// ≡ `U.uSwimH.value`: per-slot `(head, k)`.
    swim_h: [[f32; 4]; SLOTS],
    /// ≡ `U.uSwimF.value`: per-slot `(fwd, speedK)`.
    swim_f: [[f32; 4]; SLOTS],
}

impl Default for SwimWakeState {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| WakeSlot::default()),
            now: 0.0,
            wake: [[0.0; 4]; SLOTS * PTS],
            wake_b: [[0.0, -999.0, 0.0, 0.0]; SLOTS],
            swim_h: [[0.0, -999.0, 0.0, 0.0]; SLOTS],
            swim_f: [[0.0; 4]; SLOTS],
        }
    }
}

impl SwimWakeState {
    /// ≡ swimWake.js L19 `reset()`.
    pub fn reset(&mut self) {
        for s in self.slots.iter_mut() {
            s.actor = None;
            s.pts.clear();
            s.k = 0.0;
            s.last = None;
            s.swimming = false;
        }
    }

    /// One frame: swimmer selection ≡ swimWake.js L21-49 + trail bookkeeping
    /// ≡ L50-78 + upload buffers ≡ L80-105. `now` is the presentation clock
    /// (JS `G.time`), `cam` the camera position (JS `camPos`).
    ///
    /// `phase_results`: JS L27 gates on `m.state !== 'results'` — Rust maps
    /// `results` to [`inkwave_sim::match_::Phase::End`] (登记偏差：judge/results
    /// 在 Rust 合并为 End，此处 End 即停筛）。
    pub fn update(
        &mut self,
        dt: f32,
        now: f32,
        actors: &[Actor],
        local: Option<usize>,
        cam: Vec3,
        results: bool,
    ) {
        self.now = now;
        // ── selection ≡ L26-49 ─────────────────────────────────────────────
        // (actor, d2) candidates; local sorts first with d2 = -1 (≡ L34).
        let mut cand: Vec<(usize, f32)> = Vec::new();
        if !results {
            for (idx, a) in actors.iter().enumerate() {
                if !a.alive {
                    continue;
                }
                // JS L31 form swim|climb — climb 无 Rust 对应（模块头偏差登记）。
                if !form_is_swim(a) {
                    continue;
                }
                let d2 = a.pos.distance_squared(cam);
                let is_local = Some(idx) == local;
                if !is_local && d2 > CAM_RADIUS * CAM_RADIUS {
                    continue;
                }
                cand.push((idx, if is_local { -1.0 } else { d2 }));
            }
            // ≡ L36 sort by _wakeD2 asc (local first).
            cand.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap_or(std::cmp::Ordering::Equal));
        }
        for s in self.slots.iter_mut() {
            s.swimming = false;
        }
        for &(idx, _d2) in &cand {
            let mut slot = self
                .slots
                .iter()
                .position(|o| o.actor == Some(idx))
                .or_else(|| self.slots.iter().position(|o| o.actor.is_none()));
            if slot.is_none() && Some(idx) == local {
                // The local player always gets one: take the farthest bot's
                // (≡ L42-45; JS reduce 在空槽上会抛 TypeError——Rust 取「已占用
                // 且 d2 最大」槽，语义即 JS 意图，登记微偏差。JS 比较读槽上的
                // 陈旧 d2 字段；不在本帧候选的槽 Rust 以 0.0 参与，驱逐选择
                // 在「持有者离机」边角可差一槽——登记微偏差）。
                let mut best: Option<(usize, f32)> = None;
                for (si, o) in self.slots.iter().enumerate() {
                    if let Some(held) = o.actor {
                        let hd = cand
                            .iter()
                            .find(|c| c.0 == held)
                            .map(|c| c.1)
                            .unwrap_or(0.0);
                        if best.is_none_or(|(_, bd)| hd > bd) {
                            best = Some((si, hd));
                        }
                    }
                }
                slot = best.map(|(si, _)| si);
                if let Some(si) = slot {
                    let s = &mut self.slots[si];
                    s.pts.clear();
                    s.last = None;
                    s.k = 0.0;
                }
            }
            let Some(si) = slot else { continue };
            let s = &mut self.slots[si];
            if s.actor != Some(idx) {
                s.actor = Some(idx);
                s.pts.clear();
                s.last = None;
                s.k = 0.0;
            }
            s.swimming = true;
        }

        // ── per-slot trail bookkeeping ≡ L50-78 ────────────────────────────
        for s in self.slots.iter_mut() {
            let Some(ai) = s.actor else { continue };
            let a = &actors[ai];
            // Expire old points (a break marker at the front is meaningless) ≡ L54.
            while !s.pts.is_empty() && (s.pts[0].t == BREAK || now - s.pts[0].t > LIFE) {
                s.pts.remove(0);
            }
            if s.swimming {
                let p = a.pos;
                let v = a.vel;
                let sp = v.length();
                if sp > 0.6 {
                    // climb 分支（JS L58 取 v.y）缺省——Rust 无 climb 形态。
                    // JS `.normalize()` 对零向量走 THREE `|| 1` → 保持零；
                    // `normalize_or_zero` 与之等价。
                    s.fwd = Vec3::new(v.x, 0.0, v.z).normalize_or_zero();
                }
                s.speed_k = damp(s.speed_k, clamp(sp / 11.8, 0.0, 1.0), 10.0, dt);
                // Teleports / re-entries start a new trail ≡ L61 (2.5 m).
                if s.last
                    .is_some_and(|last| last.distance_squared(p) > 2.5 * 2.5)
                {
                    if s.pts.last().is_some_and(|bp| bp.t != BREAK) {
                        s.pts.push(TrailPt {
                            p: Vec3::ZERO,
                            t: BREAK,
                        });
                    }
                    s.last = None;
                }
                if s.last
                    .is_none_or(|last| last.distance_squared(p) > SPACING * SPACING)
                    && (sp > 0.8 || s.last.is_none())
                {
                    s.pts.push(TrailPt { p, t: now });
                    s.last = Some(p);
                }
                s.head = p;
                s.k = damp(s.k, 1.0, 14.0, dt);
            } else {
                // Left the ink: mound sinks, trail ages out, slot frees ≡ L70-75.
                if s.pts.last().is_some_and(|bp| bp.t != BREAK) {
                    s.pts.push(TrailPt {
                        p: Vec3::ZERO,
                        t: BREAK,
                    });
                }
                s.last = None;
                s.k = damp(s.k, 0.0, 16.0, dt);
                if s.k < 0.004 && !s.pts.iter().any(|o| o.t != BREAK) {
                    s.actor = None;
                    s.pts.clear();
                    s.k = 0.0;
                }
            }
            while s.pts.len() > PTS - 1 {
                s.pts.remove(0);
            }
        }

        // ── upload buffers ≡ L80-105（数据层落地；上传点顺延）──────────────
        for si in 0..SLOTS {
            let s = &self.slots[si];
            let base = si * PTS;
            let mut n = 0usize;
            if let Some(_ai) = s.actor {
                let (mut cx, mut cy, mut cz) = (0.0f32, 0.0f32, 0.0f32);
                let mut c = 0.0f32;
                for o in &s.pts {
                    if n < PTS {
                        self.wake[base + n] = [o.p.x, o.p.y, o.p.z, o.t];
                        n += 1;
                    }
                    if o.t != BREAK {
                        cx += o.p.x;
                        cy += o.p.y;
                        cz += o.p.z;
                        c += 1.0;
                    }
                }
                // The live head closes the trail while swimming (age 0 at the body) ≡ L91.
                if s.swimming && n < PTS {
                    self.wake[base + n] = [s.head.x, s.head.y, s.head.z, now];
                    n += 1;
                    cx += s.head.x;
                    cy += s.head.y;
                    cz += s.head.z;
                    c += 1.0;
                }
                if c > 0.0 {
                    cx /= c;
                    cy /= c;
                    cz /= c;
                    let mut r = 0.0f32;
                    for i in 0..n {
                        let w = self.wake[base + i];
                        if w[3] != BREAK {
                            r = r.max(
                                (Vec3::new(w[0], w[1], w[2]) - Vec3::new(cx, cy, cz)).length(),
                            );
                        }
                    }
                    r = r.max((s.head - Vec3::new(cx, cy, cz)).length());
                    self.wake_b[si] = [cx, cy, cz, r + 1.6];
                } else {
                    self.wake_b[si] = [0.0, -999.0, 0.0, 0.0];
                }
                self.swim_h[si] = [s.head.x, s.head.y, s.head.z, s.k];
                self.swim_f[si] = [s.fwd.x, s.fwd.y, s.fwd.z, s.speed_k];
            } else {
                self.wake_b[si] = [0.0, -999.0, 0.0, 0.0];
                self.swim_h[si] = [0.0, -999.0, 0.0, 0.0];
            }
            for i in n..PTS {
                self.wake[base + i] = [0.0, 0.0, 0.0, BREAK];
            }
        }
    }

    // ── getters：未来 swim-wake shader pass 接线面 ──────────────────────────

    /// ≡ `U.uWake.value`（`SLOTS * PTS` × `(x, y, z, t)`）。
    pub fn wake(&self) -> &[[f32; 4]; SLOTS * PTS] {
        &self.wake
    }
    /// ≡ `U.uWakeB.value`（per-slot 包围球 + BREAK 哨兵）。
    pub fn wake_b(&self) -> &[[f32; 4]; SLOTS] {
        &self.wake_b
    }
    /// ≡ `U.uSwimH.value`（per-slot `(head, k)`）。
    pub fn swim_h(&self) -> &[[f32; 4]; SLOTS] {
        &self.swim_h
    }
    /// ≡ `U.uSwimF.value`（per-slot `(fwd, speedK)`）。
    pub fn swim_f(&self) -> &[[f32; 4]; SLOTS] {
        &self.swim_f
    }
    /// Read-only slot view (tests / debug).
    pub fn slots(&self) -> &[WakeSlot; SLOTS] {
        &self.slots
    }
}

/// The per-frame system: camera + actors polling → [`SwimWakeState::update`].
///
/// 排在 `ui_state_machine` drain 前（本 system 不消费事件，与 fx_hooks 同段，
/// 保证与其余 fx 数据层同帧时序）。
pub fn swim_wake_update(
    time: Res<Time>,
    pc: Res<crate::input::PlayerControls>,
    demo: Res<crate::ink_render::DemoSim>,
    cams: Query<&GlobalTransform, With<Camera>>,
    mut st: ResMut<SwimWakeState>,
) {
    if pc.paused {
        return;
    }
    let dt = time.delta_secs().min(0.25);
    st.now += dt;
    let now = st.now;
    let m = &demo.m;
    let local = Some(demo.player_slot);
    let results = matches!(m.phase, inkwave_sim::match_::Phase::End);
    let cam = cams
        .iter()
        .next()
        .map(|g| Vec3::from_array(g.translation().to_array()))
        .unwrap_or(Vec3::ZERO);
    st.update(dt, now, &m.actors, local, cam, results);
}

#[cfg(test)]
mod tests {
    use super::*;
    use inkwave_sim::actor::Form;

    fn swimmer(pos: Vec3, vel: Vec3) -> Actor {
        let t = inkwave_sim::embedded_tuning();
        let mut a = Actor::new(0, 0, &t.player);
        a.pos = pos;
        a.vel = vel;
        a.form = Form::Squid;
        a.submerged = true;
        a
    }

    #[test]
    fn non_swimmers_are_never_selected() {
        let mut st = SwimWakeState::default();
        let t = inkwave_sim::embedded_tuning();
        let kid = Actor::new(0, 0, &t.player);
        st.update(0.016, 0.0, &[kid], Some(0), Vec3::ZERO, false);
        assert!(st.slots().iter().all(|s| s.actor.is_none()));
        // upload buffers idle: B/H BREAK sentinels, W all BREAK.
        assert_eq!(st.wake_b()[0][1], -999.0);
        assert_eq!(st.swim_h()[0][1], -999.0);
        assert!(st.wake().iter().all(|w| w[3] == BREAK));
    }

    #[test]
    fn local_swimmer_first_far_bots_culled_at_38m() {
        let mut st = SwimWakeState::default();
        let local = swimmer(Vec3::new(10.0, 0.5, 10.0), Vec3::new(3.0, 0.0, 0.0));
        let near_bot = swimmer(Vec3::new(12.0, 0.5, 10.0), Vec3::new(3.0, 0.0, 0.0));
        let far_bot = swimmer(Vec3::new(100.0, 0.5, 100.0), Vec3::new(3.0, 0.0, 0.0));
        let actors = [local, near_bot, far_bot];
        st.update(
            0.016,
            0.0,
            &actors,
            Some(0),
            Vec3::new(10.0, 0.5, 10.0),
            false,
        );
        // local + near bot in; far bot culled (≡ L33 38 m).
        let held: Vec<_> = st.slots().iter().filter_map(|s| s.actor).collect();
        assert!(held.contains(&0) && held.contains(&1) && !held.contains(&2));
        // local sorts first (d2 = -1 ≡ L34) and holds its slot.
        assert_eq!(st.slots()[0].actor, Some(0));
        assert!(st.slots()[0].swimming);
    }

    #[test]
    fn trail_pushes_spaced_points_and_head_closes() {
        let mut st = SwimWakeState::default();
        let mut a = swimmer(Vec3::ZERO, Vec3::new(5.0, 0.0, 0.0));
        let actors = std::slice::from_mut(&mut a);
        // Walk 1 m along +x in 0.1 m steps: SPACING 0.75 → ≥1 point pushed.
        for i in 0..10 {
            actors[0].pos = Vec3::new(i as f32 * 0.1, 0.4, 0.0);
            st.update(0.05, i as f32 * 0.05, actors, Some(0), Vec3::ZERO, false);
        }
        let s = &st.slots()[0];
        assert!(!s.pts.is_empty());
        assert!(s.k > 0.0);
        // head buffer tracks the live position; fwd = travel direction.
        assert_eq!(st.swim_h()[0][0], actors[0].pos.x);
        assert!((st.swim_f()[0][0] - 1.0).abs() < 1e-3);
        // bounding sphere r > 0 once a point exists.
        assert!(st.wake_b()[0][3] > 1.6);
    }

    #[test]
    fn teleport_inserts_break_and_leak_frees_slot() {
        let mut st = SwimWakeState::default();
        let mut a = swimmer(Vec3::ZERO, Vec3::new(5.0, 0.0, 0.0));
        let actors = std::slice::from_mut(&mut a);
        for i in 0..5 {
            actors[0].pos = Vec3::new(i as f32 * 0.9, 0.4, 0.0);
            st.update(0.05, i as f32 * 0.05, actors, Some(0), Vec3::ZERO, false);
        }
        // Teleport > 2.5 m ≡ L61 → break marker.
        actors[0].pos = Vec3::new(50.0, 0.4, 0.0);
        st.update(0.05, 1.0, actors, Some(0), Vec3::ZERO, false);
        assert!(st.slots()[0].pts.iter().any(|p| p.t == BREAK));
        // Leave the ink (kid form) → k decays, slot frees after trail ages ≡ L70-75.
        actors[0].form = Form::Kid;
        actors[0].submerged = false;
        for i in 0..40 {
            st.update(
                0.05,
                1.0 + i as f32 * 0.05,
                actors,
                Some(0),
                Vec3::ZERO,
                false,
            );
        }
        assert_eq!(st.slots()[0].actor, None);
    }

    #[test]
    fn results_phase_stops_selection() {
        let mut st = SwimWakeState::default();
        let a = swimmer(Vec3::ZERO, Vec3::new(5.0, 0.0, 0.0));
        st.update(0.016, 0.0, &[a], Some(0), Vec3::ZERO, true);
        assert!(st.slots().iter().all(|s| s.actor.is_none()));
    }

    #[test]
    fn reset_clears_slots() {
        let mut st = SwimWakeState::default();
        let mut a = swimmer(Vec3::ZERO, Vec3::new(5.0, 0.0, 0.0));
        let actors = std::slice::from_mut(&mut a);
        st.update(0.05, 0.0, actors, Some(0), Vec3::ZERO, false);
        assert!(st.slots()[0].actor.is_some());
        st.reset();
        assert!(st.slots().iter().all(|s| s.actor.is_none()));
    }

    #[test]
    fn buffers_have_no_nan() {
        let mut st = SwimWakeState::default();
        let mut a = swimmer(Vec3::ZERO, Vec3::new(5.0, 0.0, 0.0));
        let actors = std::slice::from_mut(&mut a);
        for i in 0..30 {
            actors[0].pos = Vec3::new((i as f32 * 0.3).cos() * 4.0, 0.4, i as f32 * 0.3);
            st.update(
                0.05,
                i as f32 * 0.05,
                actors,
                Some(0),
                Vec3::splat(2.0),
                false,
            );
        }
        for w in st
            .wake()
            .iter()
            .chain(st.wake_b())
            .chain(st.swim_h())
            .chain(st.swim_f())
        {
            assert!(w.iter().all(|v| v.is_finite()));
        }
    }
}
