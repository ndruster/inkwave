//! Task 51 Batch E — the full-screen feedback FX, ported from
//! `src/fx/screenfx.js` (1068 lines).
//!
//! JS runs one composite `ShaderPass` over the whole scene: speed lines,
//! swim tint, blast punch, chromatic split, ink on the camera lens, edge goo,
//! low-HP heartbeat vignette, kill flash, splat flood, respawn reveal… all
//! driven by a 12-subscription event surface (≡ screenfx.js L589-667) feeding
//! one effect-state machine (≡ L569-576 `s`) whose per-frame decay/threshold
//! math writes ~30 uniforms (≡ L544-557 uniform 声明 + L816-866 每帧输出).
//!
//! # 批次 E 落地面（D-T51-7①，如实登记）
//! - **事件驱动效果状态机全量平移**：触发/衰减/阈值 → 各 uniform 值的纯数据
//!   计算 ≡ `_bind` L589-667 + `update` L780-868 + `_sim` L881-1008 +
//!   `_simFlood` L1010-1032 + `_decayAll` L870-879。`ScreenFxState` 持 `s`
//!   全部状态 + uniform 面全部输出值，供未来 pass 接线。
//! - **LensInk 数据核平移**：镜头墨点粒子模拟（splat/droplet/滑落/拖尾/回收）
//!   ≡ L398-503 的 CPU 部分——它是 `uLensOn` 与 flood/reveal 水花的真数据源。
//!   GPU 面（instanced 渲染/RT/resize，L362-396/L505-527）顺延 pass。
//! - **composite pass 不接（D-T51-7①）**：`R.setExtraPass` L532-587 属
//!   Task 55 后处理链范围（Non-Goal 一致）。**如实登记：pass 未接、
//!   uniform 无上传目标**——本模块只把值算好冻结在 Resource 里。
//!
//! # 事件面（12 订阅 ≡ L589-667）——驱动 vs 缺口
//! - ✅ `splatted` ≡ `MatchEvent::Splat`（victim==local → flood）
//! - ✅ `respawn` ≡ `MatchEvent::Respawn`（local → reveal）
//! - ✅ `special:use` ≡ `MatchEvent::SpecialUse`（local → auraPulse/chroma/storm）
//! - ✅ `special:slam` ≡ `MatchEvent::SpecialSlam`（blast + 本地软化）
//! - ✅ `match:state` ≡ `MatchEvent::Phase`（Intro→reset、playing(Active)→
//!   satPop、judge/results(End)→fadeout；登记偏差：Rust End 合并 judge/results）
//! - ✅ `match:count` ≡ `MatchEvent::Countdown`（urg 基线）
//! - ⚠️ `hit(killed)` ≡ `Splat.attacker==local` 等价驱动 kill 闪光（同一时刻；
//!   HitSend 无 killed 旗标——偏差②）
//! - ⚠️ `superjump`/`superjump:land`：**polling 驱动**（`Actor::super_jump`
//!   charge/flight 沿边沿 + flight→落地沿边，hooks.rs 批次 D polling 先例；
//!   JS typed event 面 Rust 无 → `land()` 双路去重窗保留）
//! - ❌ `damage`（本地受击分档 chroma/punch + 伤害迸发 lens splat）：无
//!   MatchEvent 对应（本地路由留在 SimEvent 不上总线——hooks.rs 同一缺口原因）。
//!   `_damageSplat`/`_edgePoint` 数据层已平移，等事件面接线。
//! - ❌ `shake`（位置化震屏请求 → blast 去重）：无 MatchEvent。
//! - ❌ `bomb:explode`：无 MatchEvent（hooks.rs 已登记同缺口）。
//! - ❌ `weapon:impact`（敌墨近镜溅 droplet）：无 MatchEvent（hooks.rs 同缺口）。
//!
//! # 其它缺口/偏差（逐条）
//! ① settings 面（`G.settings.quality/cameraShake`、`reducedMotion()`，
//!   L800-808）未移植 → 强度系数 `I = 1.0` 常量、taps/lens scale 固定 high。
//! ③ `G.mode === 'match'` / `m.attract`（L593/L783）：Rust 呈现层 demo 恒为
//!   对局内 → `live()` ≡ phase ∈ {Intro, Active, Finish}；`inMatch` ≡ true。
//!   菜单态（`pc.paused`）走 `_decayAll`（≡ JS 场外衰减路径）；对局内
//!   `m.paused` 冻结（≡ JS sdt=0，L791）。
//! ④ `climb` 形态无 Rust 对应（hooks.rs 批次 D 同偏差）→ `swimming` 判定
//!   仅 Swim；emerge 条件「form 非 swim/climb」≡「非 Swim」。
//! ⑤ 随机流：JS 全局 `Math.random`——Rust 用独立 [`Rng`] 流（fx 各模块
//!   既定做法；本模块无 golden，仅确定性冒烟）。
//! ⑥ `G.hud?.attachScreenFX`（L585）挂点、`test()` 调试面（L1036-1065）、
//!   `debugHold`/`force`（L812-813）：未移植（lab/audit 面，登记顺延）；
//!   `debugHold` 恒 0。
//! ⑦ `uRes`/`uAspect` 的 GPU 上传与 `lens.resize` RT 尺寸（L795-803）：
//!   pass 侧顺延；`_aspect` 数据面从主窗口取（headless 无窗 → 16/9 默认，
//!   ≡ JS L577 初值）。投影面：JS `_v.project(cam)`（ndc，behind ≡ z>1）→
//!   Rust [`Projector`] 用相机基向量手算 ndc，behind ≡ z≤0（远平面不建模，
//!   影响 droplet/`_blast`/`_damageSplat` 三处落点——27 m 衰减门内无实际
//!   差异，登记微偏差）。

use bevy::prelude::*;
use glam::Vec3; // sim 侧 glam 0.30（render.rs L33 先例；bevy 相机面调用点显式转换）
use inkwave_sim::actor::{Actor, Form, SplatCause};
use inkwave_sim::match_::{Match, MatchEvent, Phase};
use inkwave_sim::special::world::ObjectBody;

use super::hooks::team_lin;
use crate::audio::synth::Rng;
use crate::fx::particles::clamp;

/// ≡ screenfx.js L28 `SEA`.
pub const SEA: [f32; 3] = [0.06, 0.34, 0.62];
/// ≡ screenfx.js L29 `HOT`.
pub const HOT: [f32; 3] = [1.0, 0.16, 0.05];
/// ≡ screenfx.js L29 `WHITE`.
pub const WHITE: [f32; 3] = [1.0, 1.0, 1.0];

const TAU: f32 = std::f32::consts::TAU;

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
#[inline]
fn damp(a: f32, b: f32, rate: f32, dt: f32) -> f32 {
    a + (b - a) * (1.0 - (-rate * dt).exp())
}
/// ≡ screenfx.js L26 局部 `easeOut`（文件内定义覆盖 ctx.js：cubic + clamp）。
#[inline]
fn ease_out(x: f32) -> f32 {
    1.0 - (1.0 - clamp(x, 0.0, 1.0)).powi(3)
}
/// ≡ screenfx.js L27 局部 `easeInOut`（同上：cubic + clamp）。
#[inline]
fn ease_in_out(x: f32) -> f32 {
    let x = clamp(x, 0.0, 1.0);
    if x < 0.5 {
        4.0 * x * x * x
    } else {
        1.0 - (-2.0 * x + 2.0).powi(3) / 2.0
    }
}

/// resolve a `(team, slot)` identity to a roster index（hooks.rs L823 同构）。
fn resolve(m: &Match, id: (usize, usize)) -> Option<usize> {
    m.actors
        .iter()
        .position(|a| a.team == id.0 && a.slot == id.1)
}

// ───────────────────────────────────────────── LensInk（数据核 ≡ L361-530）

/// Sprite capacity (≡ screenfx.js L358 `MAXS = 220`).
pub const MAXS: usize = 220;
/// Channels ≡ L359 `CH_ENEMY/CH_OWN/CH_WATER`.
pub const CH_ENEMY: usize = 0;
pub const CH_OWN: usize = 1;
pub const CH_WATER: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LensKind {
    Drop,
    Arm,
    Sat,
    Trail,
}

/// One lens sprite (≡ L411-416 `add()` 的字段集).
#[derive(Clone, Copy, Debug)]
struct LensPart {
    kind: LensKind,
    ch: usize,
    x: f32,
    y: f32,
    r: f32,
    rx: f32,
    ry: f32,
    rot: f32,
    i0: f32,
    i: f32,
    life: f32,
    stick: f32,
    pop: f32,
    grow: f32,
    age: f32,
    slide: bool,
    hold: f32,
    seed: f32,
    vx: f32,
    vy: f32,
    trail_t: f32,
    mass: f32,
}

impl Default for LensPart {
    fn default() -> Self {
        Self {
            kind: LensKind::Drop,
            ch: 0,
            x: 0.0,
            y: 0.0,
            r: 0.0,
            rx: 0.0,
            ry: 0.0,
            rot: 0.0,
            i0: 1.0,
            i: 1.0,
            life: 1.5,
            stick: 99.0,
            pop: 0.09,
            grow: 0.0,
            age: 0.0,
            slide: false,
            hold: 0.0,
            seed: 0.0,
            vx: 0.0,
            vy: 0.0,
            trail_t: 0.0,
            mass: 1.0,
        }
    }
}

/// CPU lens simulation (≡ LensInk L398-503；GPU 面 L362-396/L505-527 顺延 pass).
#[derive(Debug, Default)]
pub struct LensInk {
    parts: Vec<LensPart>,
    pool: Vec<LensPart>,
}

impl LensInk {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn count(&self) -> usize {
        self.parts.len()
    }

    /// ≡ L398-409 `_new()`（含满员时「最弱 trail 优先回收」；JS 在倒序遍历中
    /// splice(k<i) 会跳过一项——Rust `remove(k)` 同位置语义照搬。饱和角例：
    /// 池满（=MAXS）时 JS 循环内即时回收影响同帧后续 splat 的拖尾生成，
    /// Rust 拖尾延后一步生成，回收选择/跳项可差一项——仅饱和边角，留档）。
    fn new_part(&mut self, rng: &mut Rng) -> usize {
        if self.parts.len() >= MAXS {
            let mut k = usize::MAX;
            let mut best = f32::INFINITY;
            for (i, p) in self.parts.iter().enumerate() {
                let s = if p.kind == LensKind::Trail {
                    p.i
                } else {
                    p.i + 10.0
                };
                if s < best {
                    best = s;
                    k = i;
                }
            }
            if k < self.parts.len() {
                let p = self.parts.remove(k);
                self.pool.push(p);
            }
        }
        let mut p = self.pool.pop().unwrap_or_default();
        p.vx = 0.0;
        p.vy = 0.0;
        p.rot = 0.0;
        p.age = 0.0;
        p.slide = false;
        p.trail_t = 0.0;
        p.hold = 0.0;
        p.pop = 1.0;
        p.grow = 0.0;
        p.mass = 1.0;
        p.seed = rng.next_f64() as f32 * 100.0; // ≡ L406 `Math.random()*100`
        self.parts.push(p);
        self.parts.len() - 1
    }

    /// ≡ L411-416 `add(kind, ch, x, y, r, {rx, ry, rot, I, life, stick, pop})`.
    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        rng: &mut Rng,
        kind: LensKind,
        ch: usize,
        x: f32,
        y: f32,
        r: f32,
        rx: f32,
        ry: f32,
        rot: f32,
        i: f32,
        life: f32,
        stick: f32,
        pop: f32,
    ) -> usize {
        let idx = self.new_part(rng);
        let p = &mut self.parts[idx];
        p.kind = kind;
        p.ch = ch;
        p.x = x;
        p.y = y;
        p.r = r;
        p.rx = rx;
        p.ry = ry;
        p.rot = rot;
        p.i0 = i;
        p.i = i;
        p.life = life;
        p.stick = stick;
        p.pop = pop;
        p.grow = if pop > 0.0 { 0.0 } else { 1.0 };
        idx
    }

    /// ≡ L420-453 `splat(ch, cx, cy, size, aspect, {arms, sats, life})`——
    /// lumpy core + bulb-tipped arms + flung satellites.
    #[allow(clippy::too_many_arguments)] // JS option-object 参数面直译
    pub fn splat(
        &mut self,
        rng: &mut Rng,
        ch: usize,
        cx: f32,
        cy: f32,
        size: f32,
        aspect: f32,
        arms: usize,
        sats: usize,
        life: f32,
    ) {
        let rot0 = rng.next_f64() as f32 * TAU;
        // (x + cos(a)*d/aspect, y + sin(a)*d) ≡ L422
        let p = |x: f32, y: f32, a: f32, d: f32| -> (f32, f32) {
            (x + (a.cos() * d) / aspect, y + a.sin() * d)
        };
        // rnd 先取值再入参（保持 JS 实参求值顺序 = 随机流次序，借用不重叠）。
        let life_c = life * rnd(rng, 0.95, 1.15);
        let stick_c = rnd(rng, 0.25, 0.55);
        let core = self.add(
            rng,
            LensKind::Drop,
            ch,
            cx,
            cy,
            size * 0.92,
            size * 0.92,
            size * 0.92,
            0.0,
            1.4,
            life_c,
            stick_c,
            0.09,
        );
        self.parts[core].mass = 1.0;
        // lumps make the core silhouette irregular ≡ L426-432
        let nl = 3 + ((rng.next_f64() as f32 * 2.0) as usize);
        for i in 0..nl {
            let a = rot0 + (i as f32 / nl as f32) * TAU + rnd(rng, -0.5, 0.5);
            let (x, y) = p(cx, cy, a, size * rnd(rng, 0.3, 0.55));
            let r = size * rnd(rng, 0.5, 0.72);
            let life_l = life * rnd(rng, 0.75, 1.0);
            let stick_l = rnd(rng, 0.5, 1.2);
            let l = self.add(
                rng,
                LensKind::Drop,
                ch,
                x,
                y,
                r,
                r,
                r,
                0.0,
                1.25,
                life_l,
                stick_l,
                0.09,
            );
            self.parts[l].mass = 0.6;
        }
        // arms: fat necks (elongated metaballs) ending in bulbs ≡ L434-443
        for i in 0..arms {
            let a = rot0 + ((i as f32 + rnd(rng, -0.3, 0.3)) / arms as f32) * TAU;
            let long = rng.next_f64() < 0.3;
            let l = size
                * if long {
                    rnd(rng, 1.35, 1.8)
                } else {
                    rnd(rng, 0.85, 1.2)
                };
            let (nx, ny) = p(cx, cy, a, l * 0.5);
            let nr = size * rnd(rng, 0.2, 0.3) * if long { 0.8 } else { 1.0 };
            let life_a = life * rnd(rng, 0.6, 0.85);
            self.add(
                rng,
                LensKind::Arm,
                ch,
                nx,
                ny,
                size * 0.3,
                nr,
                l * 0.5,
                a - std::f32::consts::FRAC_PI_2,
                1.3,
                life_a,
                99.0,
                0.09,
            );
            let (bx, by) = p(cx, cy, a, l);
            let br = size
                * if long {
                    rnd(rng, 0.26, 0.34)
                } else {
                    rnd(rng, 0.3, 0.42)
                };
            let life_b = life * rnd(rng, 0.7, 1.0);
            let stick_b = rnd(rng, 0.4, 1.1);
            let b = self.add(
                rng,
                LensKind::Drop,
                ch,
                bx,
                by,
                br,
                br,
                br,
                0.0,
                1.35,
                life_b,
                stick_b,
                0.09,
            );
            self.parts[b].mass = 0.4;
        }
        // satellites flung outward ≡ L445-451
        for _ in 0..sats {
            let a = rng.next_f64() as f32 * TAU;
            let d = size * rnd(rng, 1.45, 2.5);
            let sr = size * rnd(rng, 0.07, 0.16);
            let (x, y) = p(cx, cy, a, d);
            let st = rng.next_f64() < 0.4;
            let life_s = rnd(rng, 0.5, 1.3);
            self.add(
                rng,
                LensKind::Sat,
                ch,
                x,
                y,
                sr,
                sr * if st { 0.7 } else { 1.0 },
                sr * if st { 1.9 } else { 1.0 },
                a - std::f32::consts::FRAC_PI_2,
                1.3,
                life_s,
                99.0,
                0.05,
            );
        }
    }

    /// ≡ L456-461 `droplet(ch, x, y, r, {slide, life})`.
    #[allow(clippy::too_many_arguments)] // JS option-object 参数面直译
    pub fn droplet(
        &mut self,
        rng: &mut Rng,
        ch: usize,
        x: f32,
        y: f32,
        r: f32,
        slide: f32,
        life: f32,
    ) {
        let d = self.add(
            rng,
            LensKind::Drop,
            ch,
            x,
            y,
            r,
            r,
            r,
            0.0,
            1.3,
            life,
            slide,
            0.05,
        );
        self.parts[d].mass = 0.3;
    }

    /// ≡ L463 `clear()`.
    pub fn clear(&mut self) {
        while let Some(p) = self.parts.pop() {
            self.pool.push(p);
        }
    }

    /// ≡ L465-503 `update(dt, aspect)`——grow/fade/slide/stick-slip/trail/回收。
    pub fn update(&mut self, rng: &mut Rng, dt: f32, aspect: f32) {
        // 拖尾生成先记账、循环后统一 spawn（借用不跨 add；单 part 单帧至多 1 条，
        // 收集顺序 ≡ JS 倒序遍历中的 spawn 相对序，字段值取收缩前 ≡ L492-494）。
        let mut pending: Vec<(f32, f32, f32, f32, f32, usize)> = Vec::new();
        let mut i = self.parts.len();
        while i > 0 {
            i -= 1;
            let p = &mut self.parts[i];
            p.age += dt;
            if p.grow < 1.0 {
                p.grow = (p.grow + dt / p.pop.max(0.01)).min(1.0);
            }
            // fade out after life ≡ L471-474
            let fade = if p.age > p.life {
                1.0 - (p.age - p.life) / if p.kind == LensKind::Trail { 0.9 } else { 0.55 }
            } else {
                1.0
            };
            p.i = if p.kind == LensKind::Trail {
                p.i0 * clamp(1.0 - p.age / p.life, 0.0, 1.0)
            } else {
                p.i0 * clamp(fade, 0.0, 1.0)
            };
            if p.i <= 0.01 {
                let dead = self.parts.remove(i);
                self.pool.push(dead);
                continue;
            }
            if p.kind == LensKind::Drop {
                if !p.slide && p.age > p.stick && p.r > 0.012 {
                    p.slide = true;
                }
                if p.slide {
                    // stick-slip: heavy drops run, pause, run ≡ L480-481
                    p.hold -= dt;
                    if p.hold <= 0.0 && rng.next_f64() < dt as f64 * 0.45 {
                        p.hold = rnd(rng, 0.06, 0.22);
                    }
                    let g = if p.hold > 0.0 {
                        0.0
                    } else {
                        1.05 * (p.r / 0.05) * (0.55 + 0.45 * p.mass)
                    };
                    p.vy -= g * dt;
                    p.vy *= (-dt * if p.hold > 0.0 { 12.0 } else { 2.1 }).exp();
                    p.vx = (p.age * 2.3 + p.seed).sin() * 0.006
                        + (p.age * 6.1 + p.seed * 2.0).sin() * 0.003;
                    let dx = p.vx * dt;
                    let dy = p.vy * dt;
                    p.x += dx;
                    p.y += dy;
                    let sp = -p.vy;
                    // thin film trail per ~0.6 r travelled ≡ L490-495
                    p.trail_t += (dx * aspect).hypot(dy);
                    if p.trail_t > p.r * 0.55 {
                        p.trail_t = 0.0;
                        pending.push((
                            p.x,
                            p.y + p.r * 0.45,
                            p.r * 0.55,
                            p.r * 0.42,
                            p.r * 0.72,
                            p.ch,
                        ));
                        p.r *= 0.965; // ≡ JS L494
                    }
                    p.ry = p.r * (1.0 + (sp * 9.0).min(0.9));
                    p.rx = p.r * (1.0 - (sp * 2.5).min(0.25));
                    if p.r < 0.01 {
                        p.slide = false;
                        p.life = p.life.min(p.age + 0.2);
                    }
                } else {
                    p.rx = p.r;
                    p.ry = p.r;
                }
                if p.y < -0.1 {
                    let dead = self.parts.remove(i);
                    self.pool.push(dead);
                }
            }
        }
        for (x, y, r, rx, ry, ch) in pending {
            self.add(
                rng,
                LensKind::Trail,
                ch,
                x,
                y,
                r,
                rx,
                ry,
                0.0,
                0.5,
                1.2,
                99.0,
                0.0,
            );
        }
    }
}

// ───────────────────────────────────────────── 效果状态机（≡ L569-576 `s`）

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloodMode {
    In,
    Reveal,
    Fadeout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JumpPhase {
    Charge,
    Flight,
}

/// 每帧算好的 uniform 输出面（≡ L544-557 uniform 声明 + L816-852 写入）。
/// pass 未接（D-T51-7①）——这些值冻结在 Resource 里供 Task 55 接线。
#[derive(Clone, Debug, Default)]
pub struct SfxUniforms {
    pub u_time: f32,
    pub u_aspect: f32,
    pub u_speed: f32,
    pub u_speed_tint: [f32; 3],
    pub u_stretch: f32,
    pub u_punch: f32,
    pub u_punch_pos: [f32; 2],
    pub u_blast: f32,
    pub u_blast_pos: [f32; 2],
    pub u_blast_ring: f32,
    pub u_blast_color: [f32; 3],
    pub u_chroma: f32,
    pub u_lens_on: bool,
    pub u_lens_col_a: [f32; 3],
    pub u_lens_col_b: [f32; 3],
    pub u_edge_ink: [f32; 4],
    pub u_aura: [f32; 4],
    pub u_heart: [f32; 4],
    pub u_hurt: f32,
    pub u_urgency: [f32; 4],
    pub u_swim: [f32; 4],
    pub u_focus: f32,
    pub u_charge: [f32; 4],
    pub u_shimmer: [f32; 4],
    pub u_flood: [f32; 4],
    pub u_flood_drip: f32,
    pub u_flood_clear: f32,
    pub u_hole: f32,
    pub u_hole_rim: [f32; 3],
    pub u_flash: [f32; 4],
    pub u_desat: f32,
    pub u_sat: f32,
    pub u_kill: [f32; 4],
    /// ≡ L862-866 `any` —— pass 启用判据（pass 本体顺延，值先算好）。
    pub pass_enabled: bool,
}

impl SfxUniforms {
    /// JS 的 uniform 初值（L546-557）与 `Default::default` 的零值不同，
    /// 显式构造 ≡ ScreenFX 构造期。
    fn from_js_defaults() -> Self {
        Self {
            u_aspect: 16.0 / 9.0,
            u_speed_tint: WHITE,
            u_punch_pos: [0.5, 0.5],
            u_blast_pos: [0.5, 0.5],
            u_blast_color: [1.0, 0.9, 0.8],
            u_lens_col_a: [0.1, 0.2, 1.0],
            u_lens_col_b: [1.0, 0.4, 0.05],
            u_heart: [0.6, 0.02, 0.06, 0.0],
            u_urgency: [HOT[0], HOT[1], HOT[2], 0.0],
            u_charge: [1.0, 1.0, 1.0, 0.0],
            u_hole_rim: WHITE,
            u_flash: [1.0, 1.0, 1.0, 0.0],
            u_sat: 1.0,
            u_kill: [1.0, 1.0, 1.0, 0.0],
            ..Default::default()
        }
    }
}

/// 效果状态变量（≡ L569-576 `this.s`，字段一一对应）。
#[derive(Clone, Debug)]
pub struct SfxVars {
    pub speed: f32,
    pub stretch: f32,
    pub punch: f32,
    pub punch_v: f32,
    pub blast: f32,
    pub blast_t: f32,
    pub chroma: f32,
    pub edge_ink: f32,
    pub aura: f32,
    pub aura_pulse: f32,
    pub heart: f32,
    pub heart_ph: f32,
    pub hurt: f32,
    pub urg: f32,
    pub urg_base: f32,
    pub swim: f32,
    pub focus: f32,
    pub charge_pulse: f32,
    pub shimmer: f32,
    pub kill: f32,
    pub flash: f32,
    pub desat: f32,
    pub sat: f32,
    pub sat_pop: f32,
    pub flood: f32,
    pub flood_drip: f32,
    pub hole: f32,
    pub flood_mode: Option<FloodMode>,
    pub flood_t: f32,
    pub hole_t: f32,
    pub jump: Option<JumpPhase>,
    pub jump_charge: f32,
    pub was_jump: bool,
    pub step_t: f32,
    pub rain_t: f32,
    pub emerge_t: f32,
    pub last_form: SfxForm,
    pub dmg_acc: f32,
    pub dmg_t: f32,
    pub dmg_atk: Option<usize>,
    /// ≡ `lastBlast {t,x,y,z}` L575.
    pub last_blast: (f32, f32, f32, f32),
    /// ≡ `s._fullCharge` L975.
    pub full_charge: bool,
}

/// 形态（JS `anim.form` kid/squid/swim/climb 面；climb 无 Rust 对应——偏差④）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SfxForm {
    Kid,
    Squid,
    Swim,
}

impl Default for SfxVars {
    fn default() -> Self {
        Self {
            speed: 0.0,
            stretch: 0.0,
            punch: 0.0,
            punch_v: 0.0,
            blast: 0.0,
            blast_t: 9.0, // ≡ L570 blastT: 9
            chroma: 0.0,
            edge_ink: 0.0,
            aura: 0.0,
            aura_pulse: 0.0,
            heart: 0.0,
            heart_ph: 0.0,
            hurt: 0.0,
            urg: 0.0,
            urg_base: 0.0,
            swim: 0.0,
            focus: 0.0,
            charge_pulse: 0.0,
            shimmer: 0.0,
            kill: 0.0,
            flash: 0.0,
            desat: 0.0,
            sat: 1.0, // ≡ L572 sat: 1
            sat_pop: 0.0,
            flood: 0.0,
            flood_drip: 0.0,
            hole: 0.0,
            flood_mode: None,
            flood_t: 0.0,
            hole_t: 0.0,
            jump: None,
            jump_charge: 0.0,
            was_jump: false,
            step_t: 0.0,
            rain_t: 0.0,
            emerge_t: 0.0,
            last_form: SfxForm::Kid, // ≡ L574 lastForm 'kid'
            dmg_acc: 0.0,
            dmg_t: 0.0,
            dmg_atk: None,
            last_blast: (-9.0, 0.0, 0.0, 0.0),
            full_charge: false,
        }
    }
}

/// 相机投影（JS `_v.project(cam)` 的 CPU 等价：返回 ndc x/y ∈ [-1,1] + behind）。
#[derive(Clone, Copy, Debug)]
pub struct Projector {
    pub pos: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub fwd: Vec3,
    pub tan_h: f32,
    pub aspect: f32,
}

impl Projector {
    /// ≡ `_v.copy(p).project(cam)` → `(ndc_x, ndc_y, behind)`；
    /// behind ≡ JS `z > 1` 的近似（z≤0，模块头偏差⑦）。
    pub fn project(&self, p: Vec3) -> (f32, f32, bool) {
        let v = p - self.pos;
        let z = v.dot(self.fwd);
        if z <= 1e-4 {
            return (-v.dot(self.right), -v.dot(self.up), true);
        }
        (
            v.dot(self.right) / (z * self.tan_h * self.aspect),
            v.dot(self.up) / (z * self.tan_h),
            false,
        )
    }
}

/// The screen-fx state machine + uniform surface (Resource).
#[derive(Resource)]
pub struct ScreenFxState {
    pub s: SfxVars,
    pub u: SfxUniforms,
    pub lens: LensInk,
    pub rng: Rng,
    /// ≡ `this.time`（sim 时钟，paused 冻结 L792）。
    pub time: f32,
    /// ≡ `this._aspect`（L798 每帧由 drawing buffer 尺寸算）。
    pub aspect: f32,
    /// ≡ `this._landT`（L724 0.3 s 去重窗）。
    land_t: f32,
    /// superjump polling 沿边沿状态（≡ JS typed event 的 Rust 等价）。
    sj_prev: Option<bool>,
}

impl Default for ScreenFxState {
    fn default() -> Self {
        Self {
            s: SfxVars::default(),
            u: SfxUniforms::from_js_defaults(),
            lens: LensInk::new(),
            rng: Rng::new(0x5c8e_2f13),
            time: 0.0,
            aspect: 16.0 / 9.0,
            land_t: -9.0,
            sj_prev: None,
        }
    }
}

#[inline]
fn rnd(rng: &mut Rng, a: f32, b: f32) -> f32 {
    a + (b - a) * (rng.next_f64() as f32)
}

#[inline]
fn sfx_form(a: &Actor) -> SfxForm {
    if a.form == Form::Squid {
        if a.submerged {
            SfxForm::Swim
        } else {
            SfxForm::Squid
        }
    } else {
        SfxForm::Kid
    }
}

/// ≡ L593 `live()`（attract/mode 面未移植——偏差③：phase ∈ {Intro, Active, Finish}）。
fn live(m: &Match) -> bool {
    matches!(m.phase, Phase::Intro | Phase::Active | Phase::Finish)
}

impl ScreenFxState {
    // ── 事件面（≡ _bind L589-667）──────────────────────────────────────────

    /// 消费一帧 `MatchEvent`（`.iter()` 只读，须排 `ui_state_machine` drain 前）。
    pub fn on_events(&mut self, m: &Match, local: Option<usize>, proj: Option<&Projector>) {
        for e in m.events.iter() {
            self.on_match_event(e, m, local, proj);
        }
    }

    pub fn on_match_event(
        &mut self,
        e: &MatchEvent,
        m: &Match,
        local: Option<usize>,
        proj: Option<&Projector>,
    ) {
        match e {
            MatchEvent::Splat {
                victim_team,
                victim_slot,
                attacker,
                cause,
            } => {
                // ≡ splatted L608-611：live && victim==local → flood。
                if live(m) && resolve(m, (*victim_team, *victim_slot)) == local {
                    let water = matches!(cause, SplatCause::Water);
                    self.start_flood(*attacker, water, m, local);
                }
                // ≡ hit(killed) L604-607——Rust 用 Splat attacker 等价（偏差②）。
                if live(m) && attacker.is_some() && resolve(m, attacker.unwrap()) == local {
                    self.s.kill = 1.0;
                    self.s.chroma = (self.s.chroma + 0.3).min(1.0);
                    self.kick_punch(0.012);
                }
            }
            MatchEvent::Respawn { team, slot } => {
                // ≡ respawn L612：isLocal(actor) → reveal。JS 事件面隐含
                // live() 门（handler 只在 playing 派发）；Rust 无门，但
                // Respawn 事件仅 playing 期产生——不可达，登记留档。
                if resolve(m, (*team, *slot)) == local {
                    self.start_reveal(*team);
                }
            }
            MatchEvent::SpecialUse { owner, id } => {
                // ≡ special:use L620-625（isLocal 含 live 门）。
                if live(m) && resolve(m, *owner) == local {
                    self.s.aura_pulse = 1.0;
                    self.s.chroma = (self.s.chroma + 0.25).min(1.0);
                    if id == "storm" {
                        self.s.aura = self.s.aura.max(0.9);
                    }
                }
            }
            MatchEvent::SpecialSlam {
                owner,
                pos,
                radius: _,
            } => {
                // ≡ special:slam L637-641（blast 去重包装 L627-634）。
                if live(m) {
                    let color = team_lin(owner.0);
                    self.blast_event(*pos, 1.0, Some(color), proj);
                    if resolve(m, *owner) == local {
                        // your own slam: keep the punch, soften the blur/glow
                        self.s.blast *= 0.55;
                        self.s.chroma *= 0.7;
                    }
                }
            }
            MatchEvent::Phase(p) => {
                // ≡ match:state L642-648（judge/results ≡ End——模块头偏差登记）。
                match p {
                    Phase::Intro => self.reset(),
                    Phase::Active => {
                        self.s.sat_pop = 1.0;
                        self.s.chroma = (self.s.chroma + 0.35).min(1.0);
                        self.kick_punch(0.015);
                    }
                    Phase::End => {
                        self.s.flood_mode = if self.s.flood > 0.0 {
                            Some(FloodMode::Fadeout)
                        } else {
                            None
                        };
                        self.s.flood_t = 0.0;
                        self.lens.clear();
                    }
                    _ => {}
                }
            }
            MatchEvent::Countdown { n } if live(m) => {
                // ≡ match:count L662-666。
                self.s.urg = 1.0;
                self.s.urg_base = clamp((11.0 - *n as f32) / 10.0, 0.0, 1.0) * 0.35;
            }
            // ❌ 缺口（模块头逐条登记）：damage / shake / bomb:explode /
            //    weapon:impact 无 MatchEvent 对应；superjump 走 polling（_sim 内）。
            _ => {}
        }
    }

    // ── helpers（≡ L669-771）───────────────────────────────────────────────

    /// ≡ L669 `_kickPunch`.
    pub fn kick_punch(&mut self, v: f32) {
        self.s.punch_v += v * 22.0;
    }

    /// ≡ L627-634 `blast()` 去重包装。
    fn blast_event(
        &mut self,
        pos: Vec3,
        amount: f32,
        color: Option<Vec3>,
        proj: Option<&Projector>,
    ) {
        let lb = self.s.last_blast;
        let same = self.time - lb.0 < 0.05
            && (lb.1 - pos.x).abs() + (lb.2 - pos.y).abs() + (lb.3 - pos.z).abs() < 0.8;
        if same {
            if let Some(c) = color {
                self.u.u_blast_color = c.lerp(Vec3::from(WHITE), 0.35).to_array();
            }
            return;
        }
        self.s.last_blast = (self.time, pos.x, pos.y, pos.z);
        self.blast(pos, amount, color, proj);
    }

    /// ≡ L671-689 `_blast`.
    pub fn blast(&mut self, pos: Vec3, amount: f32, color: Option<Vec3>, proj: Option<&Projector>) {
        let Some(cam) = proj else { return }; // JS `if (!cam) return` L673
        let d = cam.pos.distance(pos);
        let k = amount * clamp(1.0 - (d - 3.0) / 24.0, 0.0, 1.0);
        if k < 0.04 {
            return;
        }
        let (nx, ny, behind) = cam.project(pos);
        let on = !behind && nx.abs() < 1.25 && ny.abs() < 1.25;
        if on {
            self.u.u_blast_pos = [nx * 0.5 + 0.5, ny * 0.5 + 0.5];
            self.s.blast = self.s.blast.max(k * 1.1).min(1.2);
            self.s.blast_t = 0.0;
            self.u.u_blast_color = color
                .unwrap_or(Vec3::from(WHITE))
                .lerp(Vec3::from(WHITE), 0.35)
                .to_array();
            self.u.u_punch_pos = self.u.u_blast_pos;
        }
        self.s.chroma = (self.s.chroma + k * 0.9).min(1.3);
        self.kick_punch(0.02 * k);
    }

    /// ≡ L693-707 `_startFlood`。`local`（roster index）≡ JS `this._local`
    /// （`frame` 每帧刷新，无本地 actor 时 JS 走 attacker/water 分支不受影响）。
    pub fn start_flood(
        &mut self,
        attacker: Option<(usize, usize)>,
        water: bool,
        m: &Match,
        local: Option<usize>,
    ) {
        // JS col = water ? SEA : attacker ? teamColor(attacker.team) : teamColor(local.enemyTeam)
        let col = if water {
            Vec3::from(SEA)
        } else if let Some((at, _)) = attacker {
            team_lin(at)
        } else {
            let t = local
                .and_then(|li| m.actors.get(li))
                .map(|a| 1 - a.team)
                .unwrap_or(1);
            team_lin(t)
        };
        self.u.u_flood = [col.x, col.y, col.z, self.s.flood];
        self.u.u_flood_clear = if water { 1.0 } else { 0.0 };
        self.s.flood_mode = Some(FloodMode::In);
        self.s.flood_t = 0.0;
        self.s.hole = 0.0;
        self.s.jump = None;
        self.s.chroma = (self.s.chroma + 0.9).min(1.4);
        self.kick_punch(-0.03);
        if water {
            for _ in 0..34 {
                let x = self.rng.next_f64() as f32;
                let y = rnd(&mut self.rng, 0.05, 1.0);
                let r = rnd(&mut self.rng, 0.01, 0.034);
                let slide = rnd(&mut self.rng, 0.3, 1.2);
                let life = rnd(&mut self.rng, 1.8, 3.6);
                self.lens
                    .droplet(&mut self.rng, CH_WATER, x, y, r, slide, life);
            }
        }
    }

    /// ≡ L709-720 `_startReveal`.
    pub fn start_reveal(&mut self, team: usize) {
        let col = team_lin(team);
        self.u.u_flood = [col.x, col.y, col.z, 1.0];
        self.u.u_flood_clear = 0.0;
        self.u.u_hole_rim = col
            .lerp(Vec3::from(WHITE), 0.55)
            .to_array()
            .map(|v| v * 1.6);
        self.s.flood = 1.0;
        self.s.flood_mode = Some(FloodMode::Reveal);
        self.s.flood_t = 0.0;
        self.s.hole = 0.0001;
        self.s.flood_drip = 0.05;
        self.lens.clear();
        self.s.desat = self.s.desat.min(0.3);
        self.s.shimmer = 1.0;
    }

    /// ≡ L722-734 `_land`（actor 只取 team）。
    pub fn land(&mut self, team: usize) {
        if self.time - self.land_t < 0.3 {
            return; // typed event + fallback 去重窗
        }
        self.land_t = self.time;
        self.s.jump = None;
        self.u.u_punch_pos = [0.5, 0.3];
        self.u.u_blast_pos = [0.5, 0.18];
        self.u.u_blast_color = team_lin(team).lerp(Vec3::from(WHITE), 0.3).to_array();
        self.s.blast = self.s.blast.max(0.75);
        self.s.blast_t = 0.0;
        self.s.chroma = (self.s.chroma + 0.7).min(1.3);
        self.kick_punch(0.045);
        self.s.flash = self.s.flash.max(0.35);
    }

    /// ≡ L737-757 `_damageSplat`（damage 事件缺口——数据层已平移，待接线）。
    pub fn damage_splat(
        &mut self,
        amount: f32,
        attacker_pos: Option<Vec3>,
        proj: Option<&Projector>,
    ) {
        let a = self.aspect;
        let k = clamp(amount / 70.0, 0.18, 1.2);
        let mut ang: Option<f32> = None;
        if let (Some(p), Some(cam)) = (attacker_pos, proj) {
            let (mut dx, mut dy, behind) = cam.project(p + Vec3::Y);
            if behind {
                dx = -dx;
                dy = -dy;
            }
            if !behind && dx.abs() < 1.0 && dy.abs() < 1.0 {
                let mut v = if dx >= 0.0 { 0.0 } else { std::f32::consts::PI };
                v += rnd(&mut self.rng, -0.35, 0.35);
                ang = Some(v);
            } else {
                ang = Some(dy.atan2(dx * a));
            }
        }
        let ang = ang.unwrap_or_else(|| self.rng.next_f64() as f32 * TAU);
        let n = if k > 0.7 { 2 } else { 1 };
        for i in 0..n {
            let aa = ang
                + if i != 0 {
                    rnd(&mut self.rng, -0.7, 0.7)
                } else {
                    rnd(&mut self.rng, -0.18, 0.18)
                };
            let size = (0.045 + 0.05 * k)
                * if i != 0 { 0.6 } else { 1.0 }
                * rnd(&mut self.rng, 0.85, 1.15);
            let inset = rnd(&mut self.rng, 0.2, 0.9) * size;
            let (x, y) = self.edge_point(aa, inset);
            let arms = 6 + ((self.rng.next_f64() as f32 * 4.0) as usize);
            let sats = 5 + ((self.rng.next_f64() as f32 * 5.0) as usize);
            let life = 1.5 + k * 0.9;
            self.lens
                .splat(&mut self.rng, CH_ENEMY, x, y, size, a, arms, sats, life);
        }
    }

    /// ≡ L761-771 `_edgePoint`（屏幕边框取点，避开准星与角色）。
    pub fn edge_point(&mut self, ang: f32, inset: f32) -> (f32, f32) {
        let a = self.aspect;
        let (cx, cy) = (ang.cos(), ang.sin());
        let t = ((a / 2.0) / cx.abs().max(1e-3)).min(0.5 / cy.abs().max(1e-3));
        let mut x = a / 2.0 + cx * t;
        let mut y = 0.5 + cy * t;
        x -= cx.signum() * inset * if cx.abs() > 0.25 { 1.0 } else { 0.3 };
        y -= cy.signum() * inset * if cy.abs() > 0.25 { 1.0 } else { 0.3 };
        let mut u = x / a;
        let v = y;
        if v < 0.5 && (u - 0.5).abs() < 0.2 {
            let sign = if (u - 0.5).abs() < 1e-9 {
                if self.rng.next_f64() < 0.5 { -1.0 } else { 1.0 }
            } else {
                (u - 0.5).signum()
            };
            u = 0.5 + sign * rnd(&mut self.rng, 0.22, 0.3);
        }
        (clamp(u, 0.02, 0.98), clamp(v, 0.03, 0.97))
    }

    /// ≡ L773-777 `reset()`（只清 Object.assign 列出的键 + lens.clear）。
    pub fn reset(&mut self) {
        self.lens.clear();
        // ≡ JS L773-777 is a PARTIAL reset (Object.assign of listed keys):
        // blastT/heartPh/chargePulse/wasJump/lastBlast/fullCharge survive —
        // mirror that instead of a full default.
        let old = std::mem::take(&mut self.s);
        self.s.blast_t = old.blast_t;
        self.s.heart_ph = old.heart_ph;
        self.s.charge_pulse = old.charge_pulse;
        self.s.was_jump = old.was_jump;
        self.s.last_blast = old.last_blast;
        self.s.full_charge = old.full_charge;
    }

    // ── 每帧（≡ update L780-868 + _sim L881-1008 + _simFlood L1010-1032）───

    /// 一帧：`dt` 墙钟步进、`m` 只读 sim、`local` 本地 roster 下标、
    /// `proj` 相机投影（无相机 = headless，blast/splat 落点自动跳过 ≡ JS `!cam` 门）。
    pub fn frame(&mut self, dt: f32, m: &Match, local: Option<usize>, proj: Option<&Projector>) {
        if let Some(p) = proj {
            self.aspect = p.aspect;
        }
        let paused = m.paused;
        let sdt = if paused { 0.0 } else { dt };
        self.time += sdt;
        if !paused {
            self.sim(sdt, m, local, proj);
        }
        self.write_uniforms(m, local);
    }

    /// ≡ L870-879 `_decayAll`（场外衰减——Rust 用于 `pc.paused` 菜单态，偏差③）。
    pub fn decay_all(&mut self, dt: f32) {
        let s = &mut self.s;
        let k = (-dt * 6.0).exp();
        for v in [
            &mut s.speed,
            &mut s.stretch,
            &mut s.blast,
            &mut s.chroma,
            &mut s.edge_ink,
            &mut s.aura,
            &mut s.aura_pulse,
            &mut s.heart,
            &mut s.hurt,
            &mut s.urg,
            &mut s.swim,
            &mut s.focus,
            &mut s.charge_pulse,
            &mut s.shimmer,
            &mut s.kill,
            &mut s.flash,
            &mut s.desat,
            &mut s.sat_pop,
            &mut s.flood,
        ] {
            *v *= k;
            if *v < 0.001 {
                *v = 0.0;
            }
        }
        s.punch *= k;
        s.punch_v = 0.0;
        s.sat = damp(s.sat, 1.0, 6.0, dt);
        s.hole = 0.0;
        s.flood_mode = None;
        self.lens.update(&mut self.rng, dt, self.aspect);
    }

    /// ≡ L881-1008 `_sim`.
    fn sim(&mut self, dt: f32, m: &Match, local: Option<usize>, proj: Option<&Projector>) {
        let a: Option<&Actor> = local.and_then(|li| m.actors.get(li));
        let alive = a.is_some_and(|x| x.alive);

        // --- damage bursts → lens splats ≡ L887（damage 缺口：dmgT 恒 0，
        //     结构保留待接线）
        if self.s.dmg_t > 0.0 {
            self.s.dmg_t -= dt;
            if self.s.dmg_t <= 0.0 {
                let (acc, atk) = (self.s.dmg_acc, self.s.dmg_atk);
                if acc > 0.0 && alive {
                    let apos = atk.and_then(|i| m.actors.get(i)).map(|x| x.pos);
                    self.damage_splat(acc, apos, proj);
                }
                self.s.dmg_acc = 0.0;
                self.s.dmg_atk = None;
            }
        }

        // --- superjump polling 沿边沿（≡ JS superjump 事件 L613-618：
        //     phase 'charge'/'flight' 两态各自进入沿触发；None→charge 首沿同此。
        //     JS handler 含 live() 门；Rust polling 无门，但 End 期
        //     super_jump 无新沿——不可达，登记留档）
        let sj = a.filter(|x| x.alive).and_then(|x| x.super_jump.as_ref());
        let cur = sj.map(|j| j.charge);
        if cur != self.sj_prev {
            match cur {
                Some(true) => {
                    self.s.jump = Some(JumpPhase::Charge);
                }
                Some(false) => {
                    // charge → flight：launch geyser ≡ L617
                    self.s.jump = Some(JumpPhase::Flight);
                    self.s.flash = 0.5;
                    self.s.chroma = (self.s.chroma + 0.6).min(1.2);
                    self.kick_punch(-0.04);
                }
                None => {}
            }
            self.sj_prev = cur;
        }

        // --- speed + super jump flight ≡ L890-902
        let hs = if alive {
            a.map(|x| x.vel.x.hypot(x.vel.z)).unwrap_or(0.0)
        } else {
            0.0
        };
        let form = if alive {
            a.map(sfx_form).unwrap_or(SfxForm::Kid)
        } else {
            SfxForm::Kid
        };
        let swimming = form == SfxForm::Swim; // climb ≡ Swim 并入（偏差④）
        let mut speed_t = if swimming {
            clamp((hs - 6.5) / 5.3, 0.0, 1.0) * 0.55
        } else {
            0.0
        };
        let mut stretch_t = if swimming {
            clamp((hs - 7.0) / 4.8, 0.0, 1.0) * 0.045
        } else {
            0.0
        };
        if let Some(j) = sj.filter(|j| !j.charge) {
            // flight ≡ L896-900（fast launch, float at the apex, dive）
            let kk = clamp(j.t / (if j.dur == 0.0 { 1.2 } else { j.dur }), 0.0, 1.0);
            speed_t = 0.55 + 0.45 * (kk * std::f32::consts::PI).cos().abs();
            stretch_t = 0.06 * (0.4 + 0.6 * (kk * std::f32::consts::PI).cos().abs());
        }
        self.s.speed = damp(
            self.s.speed,
            speed_t,
            if speed_t > self.s.speed { 5.0 } else { 3.0 },
            dt,
        );
        self.s.stretch = damp(self.s.stretch, stretch_t, 4.0, dt);

        // --- landing edge（typed event 缺省 → polling fallback 主路径 ≡ L903-906）
        let sj_flight = sj.is_some_and(|j| !j.charge);
        let was_flight = self.s.was_jump;
        self.s.was_jump = sj_flight;
        if was_flight
            && !sj_flight
            && alive
            && let Some(x) = a
        {
            let team = x.team;
            self.land(team);
        }

        // --- charge glow ≡ L908
        if sj.is_some_and(|j| j.charge) {
            self.s.jump_charge = (self.s.jump_charge + dt / 0.75).min(1.0);
        } else {
            self.s.jump_charge = (self.s.jump_charge - dt * 3.0).max(0.0);
        }

        // --- swim tint + emerge droplets ≡ L911-919
        self.s.swim = damp(
            self.s.swim,
            if alive && form == SfxForm::Swim {
                1.0
            } else {
                0.0
            },
            7.0,
            dt,
        );
        if alive
            && self.s.last_form == SfxForm::Swim
            && form != SfxForm::Swim
            && (hs > 7.0 || a.is_some_and(|x| x.vel.y > 4.0))
            && self.s.emerge_t <= 0.0
        {
            self.s.emerge_t = 0.6;
            let n = 2 + ((self.rng.next_f64() as f32 * 3.0) as usize);
            for _ in 0..n {
                let side = if self.rng.next_f64() < 0.5 {
                    rnd(&mut self.rng, 0.04, 0.3)
                } else {
                    rnd(&mut self.rng, 0.7, 0.96)
                };
                let y = rnd(&mut self.rng, 0.03, 0.22);
                let r = rnd(&mut self.rng, 0.008, 0.016);
                let slide = rnd(&mut self.rng, 0.05, 0.2);
                let life = rnd(&mut self.rng, 0.45, 0.8);
                self.lens
                    .droplet(&mut self.rng, CH_OWN, side, y, r, slide, life);
            }
        }
        self.s.emerge_t -= dt;
        self.s.last_form = form;

        // --- enemy ink underfoot ≡ L922-931（JS onEnemy 三元：Rust 字段恒有值）
        let in_enemy = alive && a.is_some_and(|x| x.on_enemy);
        self.s.edge_ink = damp(
            self.s.edge_ink,
            if in_enemy { 1.0 } else { 0.0 },
            if in_enemy { 9.0 } else { 3.0 },
            dt,
        );
        if in_enemy && hs > 0.8 {
            self.s.step_t -= dt * (0.6 + hs / 3.0);
            if self.s.step_t <= 0.0 {
                self.s.step_t = rnd(&mut self.rng, 0.22, 0.4);
                let side = if self.rng.next_f64() < 0.5 {
                    rnd(&mut self.rng, 0.03, 0.34)
                } else {
                    rnd(&mut self.rng, 0.66, 0.97)
                };
                let y = rnd(&mut self.rng, 0.02, 0.14);
                let r = rnd(&mut self.rng, 0.012, 0.024);
                let slide = rnd(&mut self.rng, 0.1, 0.4);
                let life = rnd(&mut self.rng, 0.6, 1.1);
                self.lens
                    .droplet(&mut self.rng, CH_ENEMY, side, y, r, slide, life);
            }
        }

        // --- storm rain ≡ L933-951（G.projectiles.clouds ≡ specials.world kind=="cloud"）
        if alive {
            for w in m.specials.world.iter() {
                if w.kind != "cloud" || w.dead {
                    continue;
                }
                let (t, dur) = match &w.body {
                    ObjectBody::Cloud(c) => (c.t, c.dur),
                    _ => continue,
                };
                if let Some(x) = a {
                    let dx = w.pos.x - x.pos.x;
                    let dz = w.pos.z - x.pos.z;
                    if dx * dx + dz * dz < 3.6 * 3.6 && t < dur {
                        self.s.rain_t -= dt;
                        if self.s.rain_t <= 0.0 {
                            self.s.rain_t = rnd(&mut self.rng, 0.05, 0.12);
                            let ch = if w.team == x.team { CH_OWN } else { CH_ENEMY };
                            let mut u = self.rng.next_f64() as f32;
                            let v = rnd(&mut self.rng, 0.15, 1.0);
                            if (u - 0.5).abs() < 0.16 && (v - 0.5).abs() < 0.2 {
                                u += 0.3
                                    * if (u - 0.5).abs() < 1e-9 {
                                        1.0
                                    } else {
                                        (u - 0.5).signum()
                                    };
                            }
                            let r = rnd(&mut self.rng, 0.008, 0.02);
                            let slide = rnd(&mut self.rng, 0.02, 0.2);
                            let life = rnd(&mut self.rng, 0.5, 1.1);
                            self.lens.droplet(
                                &mut self.rng,
                                ch,
                                clamp(u, 0.02, 0.98),
                                v,
                                r,
                                slide,
                                life,
                            );
                        }
                        break; // JS `break` ≡ L948（首朵雨云生效）
                    }
                }
            }
        }

        // --- low HP heartbeat ≡ L953-963
        let hp = if alive {
            a.map(|x| clamp(x.hp / 100.0, 0.0, 1.0)).unwrap_or(1.0)
        } else {
            1.0
        };
        let low_k = if alive {
            clamp((0.5 - hp) / 0.38, 0.0, 1.0)
        } else {
            0.0
        };
        self.s.hurt = damp(self.s.hurt, low_k, 4.0, dt);
        if low_k > 0.0 {
            let bpm = 80.0 + 70.0 * low_k;
            self.s.heart_ph = (self.s.heart_ph + dt * bpm / 60.0) % 1.0;
            let p = self.s.heart_ph;
            let beat =
                (-((p - 0.04) / 0.05).powi(2)).exp() + 0.7 * (-((p - 0.24) / 0.055).powi(2)).exp();
            self.s.heart = low_k * beat;
        } else {
            self.s.heart = damp(self.s.heart, 0.0, 6.0, dt);
        }

        // --- special aura ≡ L965-969
        let special = alive && a.is_some_and(|x| x.special_active.is_some());
        self.s.aura_pulse = (self.s.aura_pulse - dt * 1.4).max(0.0);
        let aura_t = if special { 0.75f32 } else { 0.0 }
            .max(self.s.jump_charge * 0.9)
            .max(self.s.aura_pulse * 0.9);
        self.s.aura = damp(
            self.s.aura,
            aura_t,
            if aura_t > self.s.aura { 10.0 } else { 2.5 },
            dt,
        );

        // --- charger focus tunnel ≡ L971-977
        let charge = if alive {
            local
                .and_then(|li| m.runners.get(li))
                .filter(|wr| wr.charging_flag())
                .map(|wr| wr.charge())
                .unwrap_or(0.0)
        } else {
            0.0
        };
        self.s.focus = damp(
            self.s.focus,
            if charge > 0.0 {
                0.25 + 0.75 * charge
            } else {
                0.0
            },
            8.0,
            dt,
        );
        if charge >= 0.999 && !self.s.full_charge {
            self.s.full_charge = true;
            self.s.charge_pulse = 1.0;
        }
        if charge < 0.999 {
            self.s.full_charge = false;
        }
        self.s.charge_pulse = (self.s.charge_pulse - dt * 2.4).max(0.0);

        // --- spawn shimmer ≡ L980
        let shimmer_t = if alive && a.is_some_and(|x| x.invuln > 0.0) && m.phase == Phase::Active {
            0.7
        } else {
            0.0
        };
        self.s.shimmer = damp(self.s.shimmer, shimmer_t, 5.0, dt);

        // --- final 10 s urgency ≡ L983-984
        self.s.urg = (if m.phase == Phase::Active && m.time <= 10.2 {
            self.s.urg_base
        } else {
            0.0
        })
        .max(self.s.urg - dt * 1.6);
        if m.phase != Phase::Active {
            self.s.urg_base = 0.0;
        }

        // --- time's up freeze / judge ≡ L987-990
        let desat_t = if m.phase == Phase::Finish {
            0.45
        } else if self.s.flood_mode.is_some()
            && self.s.flood_mode != Some(FloodMode::Reveal)
            && self.s.flood_mode != Some(FloodMode::Fadeout)
        {
            0.62
        } else {
            0.0
        };
        self.s.desat = damp(self.s.desat, desat_t, 5.0, dt);
        self.s.sat = damp(
            self.s.sat,
            if m.phase == Phase::Finish { 0.92 } else { 1.0 },
            4.0,
            dt,
        );
        self.s.sat_pop = (self.s.sat_pop - dt * 2.2).max(0.0);

        // --- one-shots decay ≡ L993-997
        self.s.kill = (self.s.kill - dt * 2.4).max(0.0);
        self.s.flash = (self.s.flash - dt * if self.s.flash > 0.6 { 4.5 } else { 2.6 }).max(0.0);
        self.s.chroma = (self.s.chroma - dt * 3.2).max(0.0);
        self.s.blast = (self.s.blast - dt * 2.4).max(0.0);
        self.s.blast_t += dt;
        // --- punch spring ≡ L999-1000（critically damped, sub-stepped）
        let w_p = 16.0f32;
        let n_p = ((w_p * dt) / 0.12).ceil().max(1.0) as usize;
        let h_p = dt / n_p as f32;
        for _ in 0..n_p {
            self.s.punch_v += (-w_p * w_p * self.s.punch - 2.0 * w_p * self.s.punch_v) * h_p;
            self.s.punch += self.s.punch_v * h_p;
        }
        if self.s.jump_charge > 0.0 {
            self.s.punch = lerp(self.s.punch, -0.012 * self.s.jump_charge, 0.2);
        }

        // --- flood/reveal sequencer + lens ≡ L1004-1007
        self.sim_flood(dt);
        self.lens.update(&mut self.rng, dt, self.aspect);
    }

    /// ≡ L1010-1032 `_simFlood`.
    fn sim_flood(&mut self, dt: f32) {
        let s = &mut self.s;
        let Some(mode) = s.flood_mode else {
            s.flood = (s.flood - dt * 2.0).max(0.0);
            s.hole = 0.0;
            return;
        };
        s.flood_t += dt;
        let t = s.flood_t;
        match mode {
            FloodMode::In => {
                // surge to full cover, hold, recede to a heavy dripping frame ≡ L1015-1020
                if t < 0.24 {
                    s.flood = ease_in_out(t / 0.24);
                } else if t < 0.5 {
                    s.flood = 1.0;
                } else {
                    s.flood = lerp(1.0, 0.17, ease_out((t - 0.5) / 0.8));
                }
                s.flood_drip = (if t < 0.5 {
                    0.05
                } else {
                    0.05 + (t - 0.5) * 0.08
                })
                .min(0.4);
            }
            FloodMode::Reveal => {
                // iris opens from the centre with a bright rim ≡ L1022-1027
                s.flood = 1.0;
                s.flood_drip = 0.05;
                let k = clamp((t - 0.1) / 0.72, 0.0, 1.0);
                s.hole = if t < 0.1 {
                    0.0001
                } else {
                    (k.powf(1.7) * 0.75 + ease_out(k) * 0.25) * 1.32
                };
                if t > 0.86 {
                    s.flood_mode = None;
                    s.flood = 0.0;
                    s.hole = 0.0;
                }
            }
            FloodMode::Fadeout => {
                s.flood = (s.flood - dt * 2.5).max(0.0);
                if s.flood <= 0.0 {
                    s.flood_mode = None;
                }
            }
        }
    }

    /// ≡ L816-866 uniform 写入（`I = 1.0`——settings 面未移植，偏差①）。
    fn write_uniforms(&mut self, m: &Match, local: Option<usize>) {
        let i_scale = 1.0f32; // cameraShake × reducedMotion（缺口①）
        let s = &self.s;
        let u = &mut self.u;
        u.u_time = self.time;
        u.u_speed = s.speed * i_scale;
        u.u_stretch = s.stretch * i_scale;
        u.u_punch = clamp(s.punch, -0.08, 0.08) * i_scale;
        u.u_blast = s.blast * i_scale;
        u.u_blast_ring = ease_out(s.blast_t / 0.55) * 0.9;
        u.u_chroma = s.chroma.min(1.5) * i_scale;
        u.u_edge_ink[3] = s.edge_ink;
        u.u_aura[3] = s.aura * (0.55 + 0.45 * i_scale);
        u.u_heart[3] = s.heart;
        u.u_hurt = s.hurt;
        u.u_urgency[3] = s.urg;
        u.u_swim[3] = s.swim;
        u.u_focus = s.focus;
        u.u_charge[3] = s.charge_pulse * i_scale;
        u.u_shimmer[3] = s.shimmer;
        u.u_kill[3] = s.kill * (0.5 + 0.5 * i_scale);
        u.u_flash[3] = s.flash * (0.3 + 0.7 * i_scale);
        u.u_desat = s.desat;
        u.u_sat = s.sat + s.sat_pop * 0.28;
        u.u_flood[3] = s.flood;
        u.u_flood_drip = s.flood_drip;
        u.u_hole = s.hole;
        if let Some(a) = local.and_then(|li| m.actors.get(li)) {
            let own = team_lin(a.team);
            let enemy = team_lin(1 - a.team);
            u.u_lens_col_a = enemy.to_array();
            u.u_lens_col_b = own.to_array();
            u.u_edge_ink = [enemy.x, enemy.y, enemy.z, s.edge_ink];
            let hc = enemy.lerp(Vec3::new(0.55, 0.0, 0.04), 0.55);
            u.u_heart = [hc.x, hc.y, hc.z, s.heart];
            let aura_w = u.u_aura[3];
            u.u_aura = [own.x * 1.6, own.y * 1.6, own.z * 1.6, aura_w];
            u.u_swim = [own.x, own.y, own.z, s.swim];
            u.u_speed_tint = own.lerp(Vec3::from(WHITE), 0.6).to_array();
            let kill_w = u.u_kill[3];
            u.u_kill = [own.x * 1.3, own.y * 1.3, own.z * 1.3, kill_w];
            u.u_shimmer = [own.x + 0.3, own.y + 0.3, own.z + 0.3, s.shimmer];
            let charge_w = u.u_charge[3];
            u.u_charge = [own.x + 0.5, own.y + 0.5, own.z + 0.5, charge_w];
        }
        // --- lens field ≡ L856-859（render 顺延 pass）
        u.u_lens_on = self.lens.count() > 0;
        u.u_aspect = self.aspect;
        // --- pass 启用判据 ≡ L862-866（pass 本体顺延 Task 55）
        u.pass_enabled = u.u_lens_on
            || s.speed > 0.002
            || s.stretch > 0.0005
            || s.punch.abs() > 0.0004
            || s.blast > 0.002
            || s.chroma > 0.004
            || s.edge_ink > 0.002
            || s.aura > 0.002
            || s.heart > 0.002
            || s.hurt > 0.002
            || s.urg > 0.002
            || s.swim > 0.002
            || s.focus > 0.002
            || s.charge_pulse > 0.002
            || s.shimmer > 0.002
            || s.kill > 0.002
            || s.flash > 0.002
            || s.desat > 0.002
            || (s.sat - 1.0).abs() > 0.002
            || s.sat_pop > 0.002
            || s.flood > 0.001;
    }
}

/// The per-frame system：事件消费（drain 前）+ 状态机推进。
pub fn screen_fx_update(
    time: Res<Time>,
    pc: Res<crate::input::PlayerControls>,
    demo: Res<crate::ink_render::DemoSim>,
    cams: Query<(&Projection, &GlobalTransform), With<Camera>>,
    windows: Query<&Window>,
    mut st: ResMut<ScreenFxState>,
) {
    let dt = time.delta_secs();
    if pc.paused {
        // 菜单态 ≡ JS 场外 `_decayAll`（偏差③登记）。
        st.decay_all(dt);
        return;
    }
    let m = &demo.m;
    let local = Some(demo.player_slot);
    let aspect = windows
        .iter()
        .next()
        .map(|w| {
            let r = &w.resolution;
            (r.physical_width() as f32) / (r.physical_height() as f32).max(1.0)
        })
        .unwrap_or(16.0 / 9.0); // headless 无窗 ≡ JS L577 初值（模块头偏差⑦）
    let proj = cams.iter().next().and_then(|(pr, g)| {
        // ≡ render.rs L331-336 先例：SRT 分解 + 逐基向量桥接 glam 0.30。
        let (_s, r, t) = g.to_scale_rotation_translation();
        let fov = match pr {
            Projection::Perspective(p) => p.fov,
            _ => return None,
        };
        let cv = |v: bevy::math::Vec3| Vec3::from_array(v.to_array());
        Some(Projector {
            pos: cv(t),
            right: cv(r * bevy::math::Vec3::X),
            up: cv(r * bevy::math::Vec3::Y),
            fwd: cv(r * bevy::math::Vec3::NEG_Z),
            tan_h: (fov * 0.5).tan(),
            aspect,
        })
    });
    st.on_events(m, local, proj.as_ref());
    st.frame(dt, m, local, proj.as_ref());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ink_render::DemoSim;
    use inkwave_sim::collision::CollisionWorld;

    fn demo() -> DemoSim {
        let layout = inkwave_sim::embedded_tidewater();
        let world = CollisionWorld::from_layout(&layout);
        DemoSim::new(layout, world, 20261004)
    }

    fn push(d: &mut DemoSim, e: MatchEvent) {
        d.m.events.push(e);
    }

    fn ident(d: &DemoSim, i: usize) -> (usize, usize) {
        (d.m.actors[i].team, d.m.actors[i].slot)
    }

    #[test]
    fn countdown_sets_urgency_baseline() {
        let mut d = demo();
        d.m.phase = Phase::Active;
        push(&mut d, MatchEvent::Countdown { n: 3 });
        let mut st = ScreenFxState::default();
        st.on_events(&d.m, Some(d.player_slot), None);
        assert_eq!(st.s.urg, 1.0);
        assert!((st.s.urg_base - 0.28).abs() < 1e-6); // (11-3)/10*0.35
        // 非 live（End）→ 不动作
        d.m.events.clear();
        d.m.phase = Phase::End;
        push(&mut d, MatchEvent::Countdown { n: 3 });
        let mut st2 = ScreenFxState::default();
        st2.on_events(&d.m, Some(d.player_slot), None);
        assert_eq!(st2.s.urg, 0.0);
    }

    #[test]
    fn splat_local_victim_starts_flood_and_attacker_kill_flash() {
        let mut d = demo();
        d.m.phase = Phase::Active;
        let li = d.player_slot;
        let (team, slot) = ident(&d, li);
        push(
            &mut d,
            MatchEvent::Splat {
                victim_team: team,
                victim_slot: slot,
                attacker: Some((1 - team, 0)),
                cause: SplatCause::Weapon,
            },
        );
        let mut st = ScreenFxState::default();
        st.on_events(&d.m, Some(li), None);
        assert_eq!(st.s.flood_mode, Some(FloodMode::In));
        assert_eq!(st.s.jump, None);
        assert!(st.s.chroma >= 0.9);
        // attacker 是对手 → kill 不触发
        assert_eq!(st.s.kill, 0.0);
        // 再来一发 attacker==local → kill 闪光（偏差②等价路径）
        d.m.events.clear();
        push(
            &mut d,
            MatchEvent::Splat {
                victim_team: 1 - team,
                victim_slot: 0,
                attacker: Some((team, slot)),
                cause: SplatCause::Weapon,
            },
        );
        st.on_events(&d.m, Some(li), None);
        assert_eq!(st.s.kill, 1.0);
    }

    #[test]
    fn water_splat_adds_34_droplets() {
        let mut d = demo();
        d.m.phase = Phase::Active;
        let li = d.player_slot;
        let (team, slot) = ident(&d, li);
        push(
            &mut d,
            MatchEvent::Splat {
                victim_team: team,
                victim_slot: slot,
                attacker: None,
                cause: SplatCause::Water,
            },
        );
        let mut st = ScreenFxState::default();
        st.on_events(&d.m, Some(li), None);
        assert_eq!(st.u.u_flood_clear, 1.0);
        assert!(st.lens.count() >= 34);
    }

    #[test]
    fn respawn_local_starts_reveal() {
        let mut d = demo();
        d.m.phase = Phase::Active;
        let li = d.player_slot;
        let (team, slot) = ident(&d, li);
        push(&mut d, MatchEvent::Respawn { team, slot });
        let mut st = ScreenFxState::default();
        st.on_events(&d.m, Some(li), None);
        assert_eq!(st.s.flood_mode, Some(FloodMode::Reveal));
        assert_eq!(st.s.flood, 1.0);
        assert_eq!(st.s.shimmer, 1.0);
    }

    #[test]
    fn phase_transitions_reset_and_fadeout() {
        let mut st = ScreenFxState::default();
        let mut d = demo();
        d.m.phase = Phase::Active;
        push(&mut d, MatchEvent::Phase(Phase::Active));
        st.on_events(&d.m, Some(d.player_slot), None);
        assert_eq!(st.s.sat_pop, 1.0);
        // flood 进行中转 End → fadeout
        st.s.flood = 0.5;
        d.m.events.clear();
        d.m.phase = Phase::End;
        push(&mut d, MatchEvent::Phase(Phase::End));
        st.on_events(&d.m, Some(d.player_slot), None);
        assert_eq!(st.s.flood_mode, Some(FloodMode::Fadeout));
    }

    #[test]
    fn slam_blasts_with_dedup() {
        let mut d = demo();
        d.m.phase = Phase::Active;
        let pos = Vec3::new(1.0, 1.0, 4.0);
        push(
            &mut d,
            MatchEvent::SpecialSlam {
                owner: (1, 0),
                pos,
                radius: 3.0,
            },
        );
        let mut st = ScreenFxState::default();
        let proj = Projector {
            pos: Vec3::ZERO,
            right: Vec3::X,
            up: Vec3::Y,
            fwd: Vec3::Z,
            tan_h: 0.4,
            aspect: 16.0 / 9.0,
        };
        st.on_events(&d.m, Some(d.player_slot), Some(&proj));
        assert!(st.s.blast > 0.0);
        let b = st.s.blast;
        // 同帧去重（<0.05 s 同位）→ 不再叠加。剔除 sim 种入的 Phase 事件：
        // 真实对局事件逐帧排空，harness 队列重复处理会让 Intro 的 reset()
        // 干扰去重语义（JS reset 保留 lastBlast，去重跨 reset 仍有效）。
        d.m.events.retain(|e| !matches!(e, MatchEvent::Phase(_)));
        st.on_events(&d.m, Some(d.player_slot), Some(&proj));
        assert_eq!(st.s.blast, b);
    }

    #[test]
    fn reset_is_partial_like_js_object_assign() {
        // ≡ screenfx.js L773-777: Object.assign 只回写列出的键——
        // blastT/heartPh/chargePulse/wasJump/lastBlast/fullCharge 跨 reset 保留。
        let mut st = ScreenFxState::default();
        st.s.blast = 1.0;
        st.s.blast_t = 0.5;
        st.s.heart_ph = 0.7;
        st.s.charge_pulse = 0.9;
        st.s.was_jump = true;
        st.s.last_blast = (0.3, 1.0, 2.0, 3.0);
        st.s.full_charge = true;
        st.reset();
        assert_eq!(st.s.blast, 0.0);
        assert_eq!(st.s.blast_t, 0.5);
        assert_eq!(st.s.heart_ph, 0.7);
        assert_eq!(st.s.charge_pulse, 0.9);
        assert!(st.s.was_jump);
        assert_eq!(st.s.last_blast, (0.3, 1.0, 2.0, 3.0));
        assert!(st.s.full_charge);
    }

    #[test]
    fn flood_in_sequence_covers_and_recedes() {
        let mut st = ScreenFxState::default();
        st.s.flood_mode = Some(FloodMode::In);
        let mut d = demo();
        d.m.phase = Phase::Active;
        d.m.paused = false;
        for _ in 0..40 {
            st.sim(0.05, &d.m, Some(d.player_slot), None);
        }
        // t≈2 s：surge→hold→recede 到 0.17 渐近
        assert!((st.s.flood - 0.17).abs() < 0.1);
        assert!(st.s.flood_drip > 0.05);
    }

    #[test]
    fn reveal_iris_opens_and_finishes() {
        let mut st = ScreenFxState::default();
        st.start_reveal(0);
        let mut d = demo();
        d.m.phase = Phase::Active;
        d.m.paused = false;
        for _ in 0..30 {
            st.sim(0.05, &d.m, Some(d.player_slot), None);
        }
        // t > 0.86 → mode 清空、flood/hole 归零
        assert_eq!(st.s.flood_mode, None);
        assert_eq!(st.s.flood, 0.0);
    }

    #[test]
    fn punch_spring_is_stable_and_settles() {
        let mut st = ScreenFxState::default();
        st.kick_punch(0.045);
        let mut d = demo();
        d.m.phase = Phase::Active;
        d.m.paused = false;
        let mut max_abs = 0.0f32;
        for _ in 0..200 {
            st.sim(0.016, &d.m, Some(d.player_slot), None);
            max_abs = max_abs.max(st.s.punch.abs());
        }
        assert!(max_abs < 0.08); // 临界阻尼不过冲（uPunch 钳制面内）
        assert!(st.s.punch.abs() < 1e-3);
    }

    #[test]
    fn lens_splat_recycles_at_capacity() {
        let mut st = ScreenFxState::default();
        for _ in 0..40 {
            st.lens
                .splat(&mut st.rng, CH_ENEMY, 0.5, 0.5, 0.08, 16.0 / 9.0, 6, 6, 2.1);
        }
        assert!(st.lens.count() <= MAXS);
        st.lens.update(&mut st.rng, 0.05, 16.0 / 9.0);
        assert!(st.lens.count() <= MAXS);
    }

    #[test]
    fn edge_point_stays_in_bounds() {
        let mut st = ScreenFxState::default();
        for k in 0..24 {
            let (x, y) = st.edge_point(k as f32 * 0.26, 0.05);
            assert!((0.02..=0.98).contains(&x));
            assert!((0.03..=0.97).contains(&y));
        }
    }

    #[test]
    fn full_frame_smoke_no_nan() {
        let mut d = demo();
        d.m.phase = Phase::Active;
        d.m.paused = false;
        let mut st = ScreenFxState::default();
        for i in 0..300 {
            if i == 30 {
                push(&mut d, MatchEvent::Countdown { n: 8 });
            }
            st.on_events(&d.m, Some(d.player_slot), None);
            d.m.events.clear();
            st.frame(0.016, &d.m, Some(d.player_slot), None);
        }
        let u = &st.u;
        for v in [
            u.u_speed,
            u.u_stretch,
            u.u_punch,
            u.u_blast,
            u.u_chroma,
            u.u_hurt,
            u.u_focus,
            u.u_desat,
            u.u_sat,
            u.u_flood_drip,
            u.u_hole,
            u.u_blast_ring,
            u.u_time,
        ] {
            assert!(v.is_finite(), "nan in {v}");
        }
        assert!(u.u_edge_ink.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn decay_all_settles_to_idle() {
        let mut st = ScreenFxState::default();
        st.s.chroma = 1.0;
        st.s.flood = 1.0;
        st.s.flood_mode = Some(FloodMode::In);
        for _ in 0..200 {
            st.decay_all(0.016);
        }
        assert_eq!(st.s.chroma, 0.0);
        assert_eq!(st.s.flood, 0.0);
        assert_eq!(st.s.flood_mode, None);
    }

    #[test]
    fn superjump_polling_edges_drive_geyser() {
        let mut d = demo();
        d.m.phase = Phase::Active;
        d.m.paused = false;
        let li = d.player_slot;
        let mut st = ScreenFxState::default();
        // charge 开始
        d.m.actors[li].super_jump = Some(inkwave_sim::actor::SuperJump {
            charge: true,
            t: 0.0,
            from: Vec3::ZERO,
            to: Vec3::ZERO,
            dur: 1.2,
        });
        st.sim(0.016, &d.m, Some(li), None);
        assert_eq!(st.s.jump, Some(JumpPhase::Charge));
        assert!(st.s.jump_charge > 0.0);
        // 转 flight → geyser
        d.m.actors[li].super_jump = Some(inkwave_sim::actor::SuperJump {
            charge: false,
            t: 0.0,
            from: Vec3::ZERO,
            to: Vec3::new(0.0, 6.0, 0.0),
            dur: 1.2,
        });
        st.sim(0.016, &d.m, Some(li), None);
        assert_eq!(st.s.jump, Some(JumpPhase::Flight));
        // 帧内次序 ≡ JS：事件沿边先置值，同帧 sim 尾部 one-shot 衰减随后。
        assert!((st.s.flash - (0.5 - 0.016 * 2.6)).abs() < 1e-4);
        // 落地沿边 → land 序列
        d.m.actors[li].super_jump = None;
        st.sim(0.016, &d.m, Some(li), None);
        // land 置 0.75 后同帧衰减 dt*2.4（≡ JS 同序）。
        assert!((st.s.blast - (0.75 - 0.016 * 2.4)).abs() < 1e-4);
        assert_eq!(st.s.jump, None);
    }
}
