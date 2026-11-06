//! Task 51 Batch E — Zone Control ground markings, ported from
//! `src/fx/zoneMarks.js` (517 lines).
//!
//! JS draws every zone of a Zone Control match on the floor it covers: one
//! mesh + one transparent ShaderMaterial (single draw call), per-zone state in
//! small uniform arrays, rebuilt on `match:state` and cleared on `results`
//! (≡ zoneMarks.js L1-24 架构描述).
//!
//! # 批次 E 落地面（D-T51-7⑥，如实登记）
//! - **数据层全量平移**：flood 进度 / contested 警示 / active 淡入淡出 /
//!   sweep 跑马揭示 / flash 捕获闪光 / vis 终局淡出 ≡ `_update` L417-456；
//!   订阅面 ≡ L205-221（match:state → build/clear、zones:active →
//!   `_syncActive(true)`、zones:zone → flash、zones:contest → 仅音频）。
//!   uniform 数组（uCol/uSt/uCen/uFl/uTeamC/uTime/uWide/uFarK）在
//!   [`ZoneMarksCore`] 里逐帧算好，getter 暴露供未来 shader 接线。
//! - **简化边界带 mesh 占位档**：JS 把轮廓逐面裁剪到可涂地面
//!   （`_faceList`/`_facesOf`/`_buildZone` L288-404，依赖 paintFaces 面几何 +
//!   逐面高度）——Rust 呈现层无该面几何面（ink.wgsl 无 zoneMarks uniform，
//!   任务书既定事实），改按 render.rs 批处理先例建**单一带状 mesh**：每 zone
//!   每多边形每条边一个内向梯形带（宽 `RING`），平铺在 zone 中心高度 +
//!   `LIFT`，逐帧按数据层状态重算顶点色（act/flash/sweep/flood 波峰/
//!   contest 脉冲/中性跑马条纹占位）。裁剪/双面/幕帘（CURTAIN）省略=占位。
//! - **ZoneSounds 音频面顺延**：JS L211/L459+ 的 ZoneSounds（逐 zone 位置
//!   hum、flood/wipe surge、contest warn）不在本批——Task 50 SFX 表已烘
//!   zone_hum 等音源，驱动面（loop/pan/rate 状态机）登记缺口顺延。
//! - **uWide（TAB 地图视图）缺口**：`G.rig.mapK` 无 Rust 对应（CamRig 无
//!   map 视图系数）→ 恒 0（≡ JS `?? 0` 分支）。
//! - **capture flood 重建**：JS 读 `z.flood`（zones.js L161-166 挂在 sim 对象
//!   上的呈现字段，Rust sim 刻意不存——zones.rs 注释「spec §7」）→ Rust 在
//!   `ZoneEvent::Zone{owner ≥ 0}` 时按同公式重建 {dur=FILL_T, reach+0.4,
//!   ease out}；wipe（team<0）在 `ZoneEvent::Active{moved:true}`（轮换）时按
//!   zones.js L298 `_flood(z,-1)` 同公式重建 {dur=WIPE_T, reach+3.5, linear}
//!   ——记录本身不被 `_update` 绘制（L433 `f.team >= 0` 门），但会**覆盖**
//!   进行中的 fill 记录使波峰立即熄灭。t0 用呈现帧钟（JS 用 `G.time`，一致）。
//! - **NEUTRAL 色选择偏差**：JS L241 按 `G.teamHex` 是否「黄味」切
//!   yellow/cream；Rust 无 teamHex 呈现面（调色板固定 tangerine 30°/blue）→
//!   恒 NEUTRAL_YELLOW（yellowish 恒 false，登记偏差）。
//! - **本地时钟**：JS `_update` 用 `G.time`（全局帧钟）——Rust 用 core 自持
//!   `time`（≡ fx_hooks `runtime.time += dt` 先例）。

use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use inkwave_sim::match_::zones::ZoneState;
use inkwave_sim::match_::{MatchEvent, Phase, ZoneEvent};

use super::hooks::team_lin;

/// Zones per match (≡ zoneMarks.js L29 `MAXZ = 8`).
pub const MAXZ: usize = 8;
/// Metres of edge band geometry (≡ zoneMarks.js L30 `RING = 2.2`).
pub const RING: f32 = 2.2;
/// Floor overlay offset, m (≡ zoneMarks.js L32 `LIFT = 0.02`).
pub const LIFT: f32 = 0.02;
/// Reveal sweep duration, s (≡ zoneMarks.js L46 `SWEEP_T = 0.7`).
pub const SWEEP_T: f32 = 0.7;
/// Neutral band colour, linear (≡ zoneMarks.js L35 `NEUTRAL_YELLOW`).
pub const NEUTRAL_YELLOW: [f32; 3] = [1.0, 0.8, 0.3];
/// Warm-cream fallback when a team's ink is itself yellow
/// (≡ zoneMarks.js L35 `NEUTRAL_CREAM`; Rust 无 teamHex 面 → 未启用).
pub const NEUTRAL_CREAM: [f32; 3] = [1.0, 0.93, 0.78];
/// Capture flood fill duration, s (≡ zones.js `FILL_T`, zones.rs L83 已锚).
pub const FILL_T: f32 = 0.6;
/// New-objective wipe duration, s (≡ zones.js `WIPE_T` L53).
pub const WIPE_T: f32 = 0.7;

/// Flood front record (≡ zones.js L161-166 `z.flood`). `team < 0` is the
/// rotation wipe: `_update` L433 skips its draw, but storing it cancels an
/// in-flight fill peak exactly like JS's overwrite.
#[derive(Clone, Copy, Debug)]
pub struct FloodRec {
    pub team: i8,
    pub t0: f32,
    pub dur: f32,
    pub reach: f32,
    /// JS `ease === 'linear'` — fill 恒 false（out ease）。
    pub linear: bool,
}

/// Per-zone animation state (≡ zoneMarks.js L282 state entries + L441 `warn`).
#[derive(Clone, Copy, Debug)]
pub struct ZoneMarkState {
    pub col: [f32; 3],
    pub act: f32,
    pub sweep: f32,
    pub reach: f32,
    pub flash: f32,
    pub vis: f32,
    pub held: f32,
    pub warn: f32,
    pub active: bool,
}

/// The zone-marks data layer: animation state + the uniform arrays the JS
/// ShaderMaterial would receive (≡ zoneMarks.js L250-257).
#[derive(Debug)]
pub struct ZoneMarksCore {
    /// ≡ `this.match !== null`（build/clear 生命周期）。
    pub built: bool,
    /// Presentation clock, s (≡ `G.time`，system 每帧 `+= dt`)。
    pub time: f32,
    /// ≡ `this._t`（上一帧时间戳，dt 钳制源 L421）。
    last_t: Option<f32>,
    state: Vec<ZoneMarkState>,
    floods: Vec<Option<FloodRec>>,
    /// 当前 active objective 的 zone id 集（≡ `Z.active.zones`）。
    active_ids: Vec<usize>,
    // ── uniform 面（getter 暴露，未来 shader 接线）────────────────────────
    u_col: [[f32; 3]; MAXZ],
    u_st: [[f32; 4]; MAXZ],
    u_cen: [[f32; 4]; MAXZ],
    u_fl: [[f32; 4]; MAXZ],
    u_team_c: [[f32; 3]; 2],
    u_time: f32,
    /// TAB 地图系数——`G.rig.mapK` 无对应 → 恒 0（模块头缺口登记）。
    u_wide: f32,
    u_far_k: f32,
}

impl Default for ZoneMarksCore {
    fn default() -> Self {
        Self {
            built: false,
            time: 0.0,
            last_t: None,
            state: Vec::new(),
            floods: Vec::new(),
            active_ids: Vec::new(),
            u_col: [[0.0; 3]; MAXZ],
            u_st: [[0.0; 4]; MAXZ],
            u_cen: [[0.0; 4]; MAXZ],
            u_fl: [[0.0; 4]; MAXZ],
            u_team_c: [[1.0, 0.5, 0.0], [0.0, 0.3, 1.0]], // ≡ L255 默认值
            u_time: 0.0,
            u_wide: 0.0,
            u_far_k: 1.0,
        }
    }
}

impl ZoneMarksCore {
    /// ≡ zoneMarks.js L236-286 `build(match)`（mesh 部分由资源/系统承担）。
    /// `active` = 当前 objective 的 zone id 列表（≡ `Z.active.zones`）；
    /// `warn`/`contest` ≡ `ZONES.warn`/`ZONES.contest`（build 尾部的首次
    /// `_update` L285 用）。
    pub fn build(&mut self, zones: &[ZoneState], active: &[usize], warn: f32, contest: f32) {
        self.clear();
        self.built = true;
        self.active_ids = active.to_vec();
        let n = zones.len().min(MAXZ);
        self.state = Vec::with_capacity(n);
        self.u_cen = [[0.0; 4]; MAXZ];
        for (i, z) in zones.iter().enumerate().take(n) {
            // JS L279-280 逐 part 顶点重算 reach ≡ ZoneState.reach（同公式，
            // sim 已预计算——直接取用，登记等价依据）。
            let reach = z.reach;
            self.u_cen[i] = [z.center[0], z.center[2], reach + 1.5, 0.0];
            self.state.push(ZoneMarkState {
                col: NEUTRAL_YELLOW, // ≡ L282 col: NEUTRAL（teamHex 偏差见模块头）
                act: 0.0,
                sweep: reach + 3.0,
                reach: reach + 1.5,
                flash: 0.0,
                vis: 0.0,
                held: 0.0,
                warn: 0.0,
                active: false,
            });
        }
        self.floods = vec![None; n];
        self.sync_active(false, active);
        // JS L285 建完即 _update()（_t null → dt 0）。
        self.update(zones, false, warn, contest);
    }

    /// ≡ zoneMarks.js L224-232 `clear()`（mesh dispose 由系统承担；
    /// `sounds.stop()` 顺延——模块头 ZoneSounds 缺口）。
    pub fn clear(&mut self) {
        self.built = false;
        self.state.clear();
        self.floods.clear();
        self.active_ids.clear();
        self.last_t = None;
        self.u_col = [[0.0; 3]; MAXZ];
        self.u_st = [[0.0; 4]; MAXZ];
        self.u_cen = [[0.0; 4]; MAXZ];
        self.u_fl = [[0.0; 4]; MAXZ];
    }

    /// ≡ zoneMarks.js L406-415 `_syncActive(animate)`.
    pub fn sync_active(&mut self, animate: bool, active: &[usize]) {
        self.active_ids = active.to_vec();
        for (i, s) in self.state.iter_mut().enumerate() {
            let on = active.contains(&i);
            if on && !s.active {
                if animate {
                    s.sweep = 0.0;
                    s.flash = 0.6;
                } else {
                    s.sweep = s.reach + 2.0;
                }
            }
            s.active = on;
        }
    }

    /// `zones:zone` 事件面 ≡ zoneMarks.js L219（flash）+ zones.js L161-166
    /// （flood 记录重建，仅 fill）。`reach` = 该 zone 的 `ZoneState.reach`。
    pub fn on_zone_flip(&mut self, zone_id: usize, owner: i8, reach: f32) {
        if let Some(s) = self.state.get_mut(zone_id) {
            s.flash = 1.0; // ≡ L219 `if (s) s.flash = 1`
        }
        if (owner == 0 || owner == 1) && zone_id < self.floods.len() {
            // ≡ zones.js L162-164 fill 分支：dur FILL_T, reach+0.4, ease out。
            self.floods[zone_id] = Some(FloodRec {
                team: owner,
                t0: self.time,
                dur: FILL_T,
                reach: reach + 0.4,
                linear: false,
            });
        }
    }

    /// ≡ zoneMarks.js L417-456 `_update()`（uWide/uTeamC/sounds 差异见模块头）。
    /// `over` ≡ `Z.winner != null && match.state !== 'playing'`（L428）。
    pub fn update(&mut self, zones: &[ZoneState], over: bool, warn: f32, contest: f32) {
        if !self.built {
            return;
        }
        let t = self.time;
        let dt = match self.last_t {
            None => 0.0,
            Some(p) => (t - p).clamp(0.0, 0.25), // ≡ L421
        };
        self.last_t = Some(t);
        self.u_time = t; // ≡ L424
        let tc = [team_lin(0), team_lin(1)]; // ≡ L426-427（G.teamColors linear ≡ team_lin）
        self.u_team_c = [[tc[0].x, tc[0].y, tc[0].z], [tc[1].x, tc[1].y, tc[1].z]];
        let warn_s = warn - 0.05; // ≡ L439 `(ZONES.warn ?? 0.3) - 0.05`
        for i in 0..self.state.len() {
            let s = &mut self.state[i];
            let z = match zones.get(i) {
                Some(z) => z,
                None => break,
            };
            // capture flood front ≡ L432-437
            let f = self.floods[i];
            if let Some(f) = f {
                if f.team >= 0 && t - f.t0 < f.dur {
                    let x = ((t - f.t0) / f.dur).max(0.0);
                    self.u_fl[i][0] = f.reach
                        * if f.linear {
                            x
                        } else {
                            1.0 - (1.0 - x) * (1.0 - x)
                        };
                    self.u_fl[i][1] = if x < 0.85 { 1.0 } else { (1.0 - x) / 0.15 };
                } else {
                    self.u_fl[i][1] = 0.0;
                }
            } else {
                self.u_fl[i][1] = 0.0;
            }
            // contested: held, and the other team is past the warning line ≡ L438-442
            let cz = if s.active && z.owner >= 0 {
                let other = z.share[(1 - z.owner) as usize];
                ((other - warn_s) / (contest - warn_s)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            s.warn += (cz - s.warn) * (1.0 - (-6.0 * dt).exp());
            self.u_fl[i][2] = s.warn;
            if z.owner >= 0 {
                self.u_fl[i][3] = (1 - z.owner) as f32;
            }
            // colour lerp toward the holder's ink ≡ L443-445
            let target = if z.owner >= 0 {
                tc[z.owner as usize]
            } else {
                glam::Vec3::from_array(NEUTRAL_YELLOW)
            };
            let k = 1.0 - (-9.0 * dt).exp();
            for ch in 0..3 {
                s.col[ch] += (target[ch] - s.col[ch]) * k;
            }
            // active fade / sweep / flash / held / vis ≡ L446-451
            s.act += ((if s.active { 1.0 } else { 0.0 }) - s.act)
                * (1.0 - (-if s.active { 14.0 } else { 6.0 } * dt).exp());
            if s.active && s.sweep < s.reach + 2.0 {
                s.sweep = (s.sweep + dt * (s.reach + 2.0) / SWEEP_T).min(s.reach + 2.0);
            }
            s.flash = (s.flash - dt * 1.6).max(0.0);
            s.held += ((if z.owner >= 0 { 1.0 } else { 0.0 }) - s.held) * k;
            self.u_cen[i][3] = s.held;
            s.vis += ((if over { 0.55 } else { 1.0 }) - s.vis) * (1.0 - (-4.0 * dt).exp());
            self.u_col[i] = s.col; // ≡ L452
            self.u_st[i] = [s.act, s.sweep, s.flash, s.vis]; // ≡ L453
        }
        // ≡ L455 this.sounds.update(...) —— ZoneSounds 顺延（模块头缺口）。
    }

    /// 消费一帧的呈现层事件（≡ L213-220 订阅面）。`has_zones` = `match.zones.is_some()`。
    pub fn on_events(
        &mut self,
        events: &[MatchEvent],
        has_zones: bool,
        zones: &[ZoneState],
        active: &[usize],
        warn: f32,
        contest: f32,
    ) {
        for e in events {
            match e {
                MatchEvent::Phase(p) => {
                    // ≡ L213-216：非 zones 局（或 attract）→ clear；zones 局 →
                    // 新局 build；results(≡ End) → clear。Rust 无 match 对象身份，
                    // 以 built 标志 + 每次进 Active 重建表达「新局」（登记偏差：
                    // JS 在 intro→playing 建，Rust 在首个 Phase 事件即建）。
                    if !has_zones {
                        self.clear();
                    } else {
                        if !self.built {
                            self.build(zones, active, warn, contest);
                        }
                        if *p == Phase::End {
                            self.clear();
                        }
                    }
                }
                MatchEvent::Zone(ze) => {
                    if !self.built {
                        continue;
                    }
                    match ze {
                        ZoneEvent::Active {
                            zones: ids, moved, ..
                        } => {
                            // ≡ zones.js L298: a rotation wipes each new zone
                            // (`_flood(z, -1)`) — the record overwrites an
                            // in-flight fill and kills its peak.
                            if *moved {
                                for &zid in ids {
                                    if zid < self.floods.len() {
                                        let reach = zones.get(zid).map(|z| z.reach).unwrap_or(0.0);
                                        self.floods[zid] = Some(FloodRec {
                                            team: -1,
                                            t0: self.time,
                                            dur: WIPE_T,
                                            reach: reach + 3.5,
                                            linear: true,
                                        });
                                    }
                                }
                            }
                            // ≡ L218（sounds.shifted 顺延）
                            self.sync_active(true, ids);
                        }
                        ZoneEvent::Zone { zone, owner } => {
                            // ≡ L219（sounds.flipped 顺延）
                            let reach = zones.get(*zone).map(|z| z.reach).unwrap_or(0.0);
                            self.on_zone_flip(*zone, *owner, reach);
                        }
                        ZoneEvent::Contest { .. } => {
                            // ≡ L220：仅 sounds.contested——数据层无动作（顺延）。
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    // ── getters：未来 zoneMarks shader 接线面 ──────────────────────────────

    pub fn built(&self) -> bool {
        self.built
    }
    pub fn state(&self) -> &[ZoneMarkState] {
        &self.state
    }
    /// ≡ `u.uCol.value`。
    pub fn u_col(&self) -> &[[f32; 3]; MAXZ] {
        &self.u_col
    }
    /// ≡ `u.uSt.value`（act, sweep, flash, vis）。
    pub fn u_st(&self) -> &[[f32; 4]; MAXZ] {
        &self.u_st
    }
    /// ≡ `u.uCen.value`（cx, cz, reach+1.5, held）。
    pub fn u_cen(&self) -> &[[f32; 4]; MAXZ] {
        &self.u_cen
    }
    /// ≡ `u.uFl.value`（front r, front y, warn, contest holder）。
    pub fn u_fl(&self) -> &[[f32; 4]; MAXZ] {
        &self.u_fl
    }
    /// ≡ `u.uTeamC.value`。
    pub fn u_team_c(&self) -> &[[f32; 3]; 2] {
        &self.u_team_c
    }
    /// ≡ `u.uTime.value`。
    pub fn u_time(&self) -> f32 {
        self.u_time
    }
    /// ≡ `u.uWide.value`（mapK 缺口 → 恒 0）。
    pub fn u_wide(&self) -> f32 {
        self.u_wide
    }
    /// ≡ `u.uFarK.value`（A/B knob，JS 默认 1）。
    pub fn u_far_k(&self) -> f32 {
        self.u_far_k
    }
    /// 每区 flood 前沿记录（Task 52 批次 E2：ripple 接线在 `t0` 变化沿发一次
    /// 涌浪，≡ paint.js L729）。
    pub fn floods(&self) -> &[Option<FloodRec>] {
        &self.floods
    }
}

/// Resource: the data core + the placeholder band-mesh handles.
#[derive(Resource, Default)]
pub struct ZoneMarksState {
    pub core: ZoneMarksCore,
    entity: Option<Entity>,
    mesh: Option<Handle<Mesh>>,
    material: Option<Handle<StandardMaterial>>,
}

/// One band vertex: `(position, colour)` — flat quad strip along each polygon
/// edge, inset by [`RING`] (占位档形态，模块头登记)。
fn band_vertex_color(
    st: &ZoneMarkState,
    fl: &[f32; 4],
    r: f32,
    s_arc: f32,
    time: f32,
) -> ([f32; 3], f32) {
    // reveal：active 区从中心向外扫出（≡ shader `1 - smoothstep(sweep-0.9, sweep, r)`）
    let reveal = smoothstep_rev((st.sweep - 0.9).max(0.0), st.sweep, r);
    let mut col = Vec3::from_array(st.col);
    let mut a;
    if st.active {
        a = 0.55 * st.act * st.vis * reveal;
        // capture flash ≡ shader flash 项（L448 数据 + shader 亮度）
        let flash = st.flash;
        col = col.lerp(Vec3::ONE, (0.16 + 0.5 * flash).min(0.85) * flash.max(0.2));
        // flood crest：波峰亮点（≡ zoneMarks.js L15 描述 + uFl.x/y）
        if fl[1] > 0.0 {
            let x = r - fl[0];
            let crest = (-x * x * 4.0).exp() * fl[1];
            col = col.lerp(Vec3::ONE, (crest * 0.8).min(0.8));
            a += crest * 0.35 * st.vis;
        }
        // contested pulse：向 contest holder 色脉冲（≡ L17 描述 + uFl.z/w）
        if fl[2] > 0.001 {
            let pulse = fl[2] * (0.35 + 0.35 * (time * 10.0).sin());
            let other = if fl[3] > 0.5 {
                Vec3::from_array([0.0, 0.3, 1.0])
            } else {
                Vec3::from_array([1.0, 0.5, 0.0])
            };
            col = col.lerp(other, pulse.min(0.9));
        }
    } else {
        // inactive zones: a faint dashed ghost outline ≡ L10（dash 占位）
        let dash = (s_arc / 1.15).fract() > 0.42;
        a = if dash { 0.12 * st.vis } else { 0.0 };
    }
    // 中性跑马条纹占位（≡ shader stripe：(s - d)*1.35/w - uTime*0.45，
    // 简化为沿弧滚动暗纹；held<0.5 视为中性）
    if st.held < 0.5 {
        let q = (s_arc / 1.15 - time * 0.45).fract();
        let dark = 1.0 - 0.4 * smoothstep_rev(0.3, 0.5, q) * (1.0 - smoothstep_rev(0.5, 0.7, q));
        col *= dark;
    }
    (col.to_array(), a.clamp(0.0, 1.0))
}

/// `smoothstep(a, b, x)`（≡ GLSL，shader 内 reveal 用）。
fn smoothstep_rev(a: f32, b: f32, x: f32) -> f32 {
    if b <= a {
        return if x >= b { 1.0 } else { 0.0 };
    }
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// 从 zone 多边形生成内向带三角（占位档：平铺 center.y + LIFT，无逐面裁剪）。
fn push_band(
    zones: &[ZoneState],
    core: &ZoneMarksCore,
    pos: &mut Vec<[f32; 3]>,
    col: &mut Vec<[f32; 4]>,
    idx: &mut Vec<u32>,
) {
    for (zi, z) in zones.iter().take(MAXZ).enumerate() {
        let Some(st) = core.state().get(zi) else {
            break;
        };
        let fl = core.u_fl()[zi];
        let time = core.u_time();
        let y = z.center[1] + LIFT;
        let (cx, cz) = (z.center[0], z.center[2]);
        for part in z.def.parts() {
            if part.len() < 3 {
                continue;
            }
            // 归一 CCW（≡ JS `_buildZone` 的 area 符号翻转 L310+ 简化版）
            let mut poly: Vec<[f32; 2]> = part.to_vec();
            let mut area = 0.0f32;
            for i in 0..poly.len() {
                let [x0, z0] = poly[i];
                let [x1, z1] = poly[(i + 1) % poly.len()];
                area += x0 * z1 - x1 * z0;
            }
            if area < 0.0 {
                poly.reverse();
            }
            let mut s_arc = 0.0f32;
            let n = poly.len();
            for i in 0..n {
                let a_pt = poly[i];
                let b_pt = poly[(i + 1) % n];
                let ex = b_pt[0] - a_pt[0];
                let ez = b_pt[1] - a_pt[1];
                let el = (ex * ex + ez * ez).sqrt();
                if el < 1e-4 {
                    continue;
                }
                // CCW 内法线（左侧）
                let nx = -ez / el;
                let nz = ex / el;
                let base = pos.len() as u32;
                let ra = ((a_pt[0] - cx).powi(2) + (a_pt[1] - cz).powi(2)).sqrt();
                let rb = ((b_pt[0] - cx).powi(2) + (b_pt[1] - cz).powi(2)).sqrt();
                let (ca, aa) = band_vertex_color(st, &fl, ra, s_arc, time);
                let (cb, ab) = band_vertex_color(st, &fl, rb, s_arc + el, time);
                // 外 A、B → 内 A'、B'（宽 RING，无 safeInset 折叠检查=占位简化）
                pos.push([a_pt[0], y, a_pt[1]]);
                col.push([ca[0], ca[1], ca[2], aa]);
                pos.push([b_pt[0], y, b_pt[1]]);
                col.push([cb[0], cb[1], cb[2], ab]);
                pos.push([a_pt[0] + nx * RING, y, a_pt[1] + nz * RING]);
                col.push([ca[0], ca[1], ca[2], aa * 0.75]);
                pos.push([b_pt[0] + nx * RING, y, b_pt[1] + nz * RING]);
                col.push([cb[0], cb[1], cb[2], ab * 0.75]);
                idx.extend_from_slice(&[base, base + 2, base + 1, base + 1, base + 2, base + 3]);
                s_arc += el;
            }
        }
    }
}

/// The per-frame system：事件消费（drain 前）+ 数据层 update + 占位带 mesh 重建。
pub fn zone_marks_update(
    time: Res<Time>,
    pc: Res<crate::input::PlayerControls>,
    demo: Res<crate::ink_render::DemoSim>,
    mut zms: ResMut<ZoneMarksState>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if pc.paused {
        return;
    }
    let dt = time.delta_secs();
    zms.core.time += dt;
    let m = &demo.m;
    let warn = demo.tuning.zones.warn;
    let contest = demo.tuning.zones.contest;
    let Some(zc) = &m.zones else {
        // 非 zones 局：≡ L214 `!match.zones → clear`（事件面兜底）。
        if zms.core.built {
            zms.core.clear();
        }
        teardown_mesh(&mut zms, &mut commands);
        return;
    };
    let active: Vec<usize> = zc.objectives[zc.active].zones.clone();
    zms.core
        .on_events(&m.events, true, &zc.zones, &active, warn, contest);
    if !zms.core.built {
        teardown_mesh(&mut zms, &mut commands);
        return;
    }
    let over = zc.winner.is_some() && m.phase != Phase::Active;
    zms.core.update(&zc.zones, over, warn, contest);

    // ── 占位带 mesh（模块头 D-T51-7⑥ 登记；render.rs 批处理先例）──────────
    if zms.entity.is_none() {
        let material = materials.add(StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            double_sided: true, // ≡ JS DoubleSide（L260）
            ..default()
        });
        let handle = meshes.add(empty_band_mesh());
        let e = commands
            .spawn((
                Name::new("fx:zone_marks"),
                Mesh3d(handle.clone()),
                MeshMaterial3d(material.clone()),
                Visibility::default(),
                Transform::default(),
            ))
            .id();
        zms.entity = Some(e);
        zms.mesh = Some(handle);
        zms.material = Some(material);
    }
    if let Some(handle) = zms.mesh.clone() {
        let mut pos = Vec::new();
        let mut col = Vec::new();
        let mut idx = Vec::new();
        push_band(&zc.zones, &zms.core, &mut pos, &mut col, &mut idx);
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, default());
        // one normal per vertex — the unlit material ignores it; the attribute
        // exists to satisfy the standard mesh layout (≡ render.rs L283-285 先例).
        let normals = vec![[0.0, 1.0, 0.0]; pos.len()];
        let uvs = vec![[0.0f32, 0.0]; pos.len()];
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
        mesh.insert_indices(Indices::U32(idx));
        if let Some(mut mm) = meshes.get_mut(&handle) {
            *mm = mesh;
        }
    }
}

fn empty_band_mesh() -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, Vec::<[f32; 3]>::new());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, Vec::<[f32; 3]>::new());
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, Vec::<[f32; 2]>::new());
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, Vec::<[f32; 4]>::new());
    mesh.insert_indices(Indices::U32(Vec::new()));
    mesh
}

fn teardown_mesh(zms: &mut ZoneMarksState, commands: &mut Commands) {
    if let Some(e) = zms.entity.take() {
        commands.entity(e).despawn();
    }
    zms.mesh = None;
    zms.material = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use inkwave_sim::geometry::ZoneDef;
    use inkwave_sim::match_::zones::ZoneKind;

    fn zone(cx: f32, cz: f32) -> ZoneState {
        let poly = vec![
            [cx - 3.0, cz - 3.0],
            [cx + 3.0, cz - 3.0],
            [cx + 3.0, cz + 3.0],
            [cx - 3.0, cz + 3.0],
        ];
        let def = ZoneDef::new(vec![poly], None, None);
        let reach = def
            .parts()
            .iter()
            .flatten()
            .map(|[x, z]| ((x - cx).powi(2) + (z - cz).powi(2)).sqrt())
            .fold(0.0f32, f32::max);
        ZoneState {
            id: 0,
            kind: ZoneKind::Center,
            home: -1,
            owner: -1,
            share: [0.0, 0.0],
            hold: 0.0,
            warn_armed: true,
            warn_t: -99.0,
            cells: Vec::new(),
            center: [cx, 0.5, cz],
            reach,
            def,
        }
    }

    #[test]
    fn build_seeds_centres_and_ghost_sweep() {
        let zones = [zone(0.0, 0.0), zone(10.0, 0.0)];
        let mut core = ZoneMarksCore::default();
        core.build(&zones, &[0], 0.3, 0.8);
        assert!(core.built());
        // ≡ L281-282：uCen=(cx, cz, reach+1.5, 0)、sweep=reach+3、reach=reach+1.5
        let reach0 = core.u_cen()[0][2];
        assert!((reach0 - (18.0f32.sqrt() + 1.5)).abs() < 1e-3);
        assert!(core.state()[0].active);
        assert!(!core.state()[1].active);
        // 非 animate build：sweep 直达 reach+2（≡ L412 animate false）
        assert!((core.state()[0].sweep - (18.0f32.sqrt() + 1.5 + 2.0)).abs() < 1e-3);
    }

    #[test]
    fn capture_event_sets_flash_and_flood_front() {
        let zones = [zone(0.0, 0.0)];
        let mut core = ZoneMarksCore::default();
        core.build(&zones, &[0], 0.3, 0.8);
        core.time = 5.0;
        core.on_zone_flip(0, 1, zones[0].reach);
        assert_eq!(core.state()[0].flash, 1.0);
        // flood 推进：t0=5，dur=0.6 → t=5.3 时 x=0.5，out ease
        core.update(&zones, false, 0.3, 0.8);
        core.time = 5.3;
        core.update(&zones, false, 0.3, 0.8);
        let x = 0.5f32;
        let want = (zones[0].reach + 0.4) * (1.0 - (1.0 - x) * (1.0 - x));
        assert!((core.u_fl()[0][0] - want).abs() < 1e-3);
        assert_eq!(core.u_fl()[0][1], 1.0); // x < 0.85 全亮
        // 过期后 front 灭
        core.time = 6.0;
        core.update(&zones, false, 0.3, 0.8);
        assert_eq!(core.u_fl()[0][1], 0.0);
    }

    #[test]
    fn contest_and_held_flags() {
        let mut zones = [zone(0.0, 0.0)];
        zones[0].owner = 0;
        zones[0].share = [0.5, 0.79]; // 对方冲过 warn 线
        let mut core = ZoneMarksCore::default();
        core.build(&zones, &[0], 0.3, 0.8);
        for _ in 0..300 {
            core.time += 0.016;
            core.update(&zones, false, 0.3, 0.8);
        }
        // warn 平滑趋近 clamp((0.79-0.25)/(0.8-0.25)) ≈ 0.98
        assert!(core.u_fl()[0][2] > 0.9);
        // uFl.w = 1 - owner ≡ L442
        assert_eq!(core.u_fl()[0][3], 1.0);
        // held 趋 1 → uCen.w ≡ L449-450
        assert!(core.u_cen()[0][3] > 0.99);
        // col 趋近 team 0 linear 色
        let t0 = team_lin(0);
        assert!((core.u_col()[0][0] - t0.x).abs() < 0.05);
    }

    #[test]
    fn over_dimms_vis_to_055() {
        let zones = [zone(0.0, 0.0)];
        let mut core = ZoneMarksCore::default();
        core.build(&zones, &[0], 0.3, 0.8);
        for _ in 0..200 {
            core.time += 0.016;
            core.update(&zones, true, 0.3, 0.8);
        }
        assert!((core.u_st()[0][3] - 0.55).abs() < 0.02);
    }

    #[test]
    fn events_drive_build_clear_and_active_sync() {
        let zones = [zone(0.0, 0.0), zone(10.0, 0.0)];
        let mut core = ZoneMarksCore::default();
        core.on_events(
            &[MatchEvent::Phase(Phase::Intro)],
            true,
            &zones,
            &[0],
            0.3,
            0.8,
        );
        assert!(core.built());
        // zones:active 切到 1 ≡ L218
        core.on_events(
            &[MatchEvent::Zone(ZoneEvent::Active {
                objective: "b".into(),
                zones: vec![1],
                final_lock: false,
                moved: true,
            })],
            true,
            &zones,
            &[1],
            0.3,
            0.8,
        );
        assert!(core.state()[1].active);
        assert!(!core.state()[0].active);
        assert_eq!(core.state()[1].flash, 0.6); // animate 切换 flash 0.6 ≡ L412
        // End ≡ results → clear ≡ L216
        core.on_events(
            &[MatchEvent::Phase(Phase::End)],
            true,
            &zones,
            &[1],
            0.3,
            0.8,
        );
        assert!(!core.built());
        // 非 zones 局 → clear
        core.build(&zones, &[0], 0.3, 0.8);
        core.on_events(
            &[MatchEvent::Phase(Phase::Intro)],
            false,
            &zones,
            &[],
            0.3,
            0.8,
        );
        assert!(!core.built());
    }

    #[test]
    fn rotation_wipe_cancels_inflight_fill() {
        // ≡ zones.js L298 + L161-166: `_flood(z, -1)` on rotation overwrites
        // the in-flight fill record (team -1 is never drawn ⇒ peak dies).
        let zones = [zone(0.0, 0.0), zone(10.0, 0.0)];
        let mut core = ZoneMarksCore::default();
        core.on_events(
            &[MatchEvent::Phase(Phase::Intro)],
            true,
            &zones,
            &[0],
            0.3,
            0.8,
        );
        core.on_events(
            &[MatchEvent::Zone(ZoneEvent::Zone { zone: 0, owner: 0 })],
            true,
            &zones,
            &[0],
            0.3,
            0.8,
        );
        let fill = core.floods[0].expect("capture fill recorded");
        assert_eq!(fill.team, 0);
        assert_eq!(fill.dur, FILL_T);
        core.on_events(
            &[MatchEvent::Zone(ZoneEvent::Active {
                objective: "b".into(),
                zones: vec![0],
                final_lock: false,
                moved: true,
            })],
            true,
            &zones,
            &[0],
            0.3,
            0.8,
        );
        let wipe = core.floods[0].expect("wipe overwrites the fill record");
        assert_eq!(wipe.team, -1);
        assert_eq!(wipe.dur, WIPE_T);
        assert!(wipe.linear);
        assert_eq!(wipe.reach, zones[0].reach + 3.5);
    }

    #[test]
    fn band_geometry_is_finite_and_indexed() {
        let zones = [zone(0.0, 0.0), zone(10.0, 0.0)];
        let mut core = ZoneMarksCore::default();
        core.build(&zones, &[0], 0.3, 0.8);
        core.time += 0.5;
        core.update(&zones, false, 0.3, 0.8);
        let mut pos = Vec::new();
        let mut col = Vec::new();
        let mut idx = Vec::new();
        push_band(&zones, &core, &mut pos, &mut col, &mut idx);
        assert!(!pos.is_empty());
        assert_eq!(pos.len(), col.len());
        assert!(idx.iter().all(|i| (*i as usize) < pos.len()));
        assert!(
            col.iter()
                .all(|c| c.iter().all(|v| v.is_finite()) && c[3] >= 0.0 && c[3] <= 1.0)
        );
    }
}
