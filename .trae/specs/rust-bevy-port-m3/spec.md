# INKWAVE Rust/Bevy 移植 — M3 内容移植与跨端开黑产品需求文档（PRD）

> 前置：M1（`.trae/specs/rust-bevy-port/`，Task 1–16）与 M2（`.trae/specs/rust-bevy-port-m2/`，Task 17–27）已全部完成。本文档只描述 M3 增量；M1 架构/同步机制与 M2 网络栈全部继续有效。

## M3 决议（2026-10-30 与用户确认）
- **主线 = 内容移植 + 跨端开黑**：武器/模式/地图补齐到与 JS 玩法对等，兑现 M2 Non-Goal「同房间跨端实际开黑」；**视觉/音频完整度（角色弹簧绑定、后处理链、黄昏环境、过程化音效、小地图、showcase 大厅）整体顺延 M4**。
- **模式**：turf + Zone Control；Boss 战独立大块留后。
- **武器**：12 主武器（`WEAPON_ORDER`）+ 15 副 + 19 特殊；kit（`['k']` 事件、s[21] ks 槽）不做。
- **地图**：tidewater + 2 张新图（推荐 saltpan + crossmarket：layout 最小 160/151 行，且均有 zones-data）。

## Overview
- **Summary**: 把 Rust 版从「Spritzer + turf + tidewater」扩展到 JS 上游的完整玩法内容集——全部 12 主武器、15 副武器、19 特殊武器、Zone Control 模式、2 张新地图、normal/hard bot 档位——并在此内容基础上完成 **JS↔Rust 跨端同房完整对局**（turf 与 zones 各至少一局），wire 层继续与 PROTO v1 逐字节兼容。
- **Purpose**: M2 已证协议兼容边界（握手 + tick 交换）；跨端实际开黑被武器/模式内容缺口挡住（M2 spec Non-Goals L18）。M3 补齐内容缺口，使 Rust 客户端成为可与 JS 客户端同房间开黑的完整对等实现。
- **Target Users**: 仓库维护者及其开黑对象（JS 客户端 ↔ Rust 客户端同房对战）。

## Goals
- **G-1 武器内容对等**：12 主武器行为逐字段对齐 `weapons.js`（kind 分发 L118-124）+ `Projectiles`（L776）；15 副对齐 `SubSystem`（subs.js L54/L71/L81/L107）；19 特殊对齐 `SpecialSystem`（specials.js L217，生命周期 start L254 / netGhost L267 / tick L336 / body L344 / weapon L359 / end L363）。数值全部走 tuning 提取管线，禁止手抄。
- **G-2 网络语义扩展**：`['b']` 副武器实体记录（netmatch.js recBomb L124-127）、`'dh'` 设备伤害直发（L159/L210）、`['z']` zone 事件 host 广播（recZone L148 / guest `match.zones.netEvent` L495）、tick 槽 `sp`(s[13])/`spx`(s[22])/`spst`(s[23]) 从 M1 恒 0 变为实值——全部并入金标准 fixture。
- **G-3 Zone Control 完整可玩**：`ZoneControl`（zones.js L105）逐行平移——zoneCells 多边形取格（L77）、占领/争夺阈值（L169-249 `_tick`）、轮换/终局锁定/加时/penalty（L265-424 `update`/`state`）、`netEvent`（L372）；Rust↔Rust 与 JS↔Rust zone 状态逐 tick 一致、结算判定一致。
- **G-4 多图为数据主路**：saltpan + crossmarket 走既有 `tools/extract/` 管线提取（layout.js 160/151 行），碰撞/出生点/zones-data（maps.js L118-121 合并逻辑）与 JS 侧一致；渲染为 M1 简化档体块。
- **G-5 bot 档位**：normal/hard 六参数（config.js L503-508：reaction/aimError/fireDiscipline/awareness/aimOmega/aimTurn）+ 武器使用行为差异平移；bot 可持随机 sub/special（session.js L285-286）。
- **G-6 跨端开黑达成**：JS 客户端与 Rust 客户端（原生/WASM）同房间跑完整局（turf 与 zones），武器选择互通（roster `{weapon,sub,special,style}`，session.js L279/L287），涂地覆盖/zone 状态/结算一致。

## Non-Goals（M3 明确不做）
- **视觉/音频完整度**：角色弹簧绑定（character.js 4062 行）、后处理链（GTAO/bloom/grade/SMAA/tilt-shift）、ink 湿润/光泽、黄昏/海况环境、过程化音效（audio.js 2505 行）、音乐、FX 完整移植、小地图、showcase 大厅——全部顺延 M4。新武器/副/特殊的**呈现**用 M1 风格体块/占位档（仅保证可辨识：弹道、zone 边界、特殊外形轮廓）。
- **kit 系统**：`['k']` 事件（recKit L134-137 / case L486）、s[21] `ks` 槽维持恒 0；Rust 收帧**解码容忍**（忽略不报错），不产生。**Task 30 修订（待批准）**：brolly 为 12 主武器（G-1/AC-M3-1 范围）且其伞面是挡弹实体，跨端不对等会直接破坏对局——例外放开两条：① s[21] `ks` 对 `kind==='brolly'` 实值化（`kits/brolly.js` netState L636-641 位包 / netApply L642-649 解包，其余 kind 仍恒 0）；② `['k', nid, 'brolly', [x,y,z,dx,dz,hp,h0]]` 发射伞记录（netRec brolly.js L266 / ghost L650-653）Rust 产生并消费；`['k']` 其余 kind（`sp`/`subs`/`waddle` 等）维持解码容忍不产生。fixture 金标准扩展并入 Task 36。
- **Boss 战**：`['bm']`/`['bc']`（L494/L496）解码容忍不处理；`mode:'boss'` 仍拒绝。
- 不移植 retired 武器 dualies/slosher/splatling（config.js L265-305：已从 roster 退役，仅存档兼容；Rust 无存档包袱）。
- 不改 `server/`、不改 `src/`、`tools/`、`docs/` 上游树（fixture 扩展脚本仍放 `rust/tools/`）。
- 不做手柄、移动端、发布部署。

## Background & Context（M3 调研结论，均已核实）
- **主武器集**：`WEAPON_ORDER = [shooter, twins, brolly, blaster, spinner, charger, bow, roller, brush, blade, mitts, bucket]`（config.js L303，12 把）。kind 分发在 `WeaponRunner.update`（weapons.js L118-124：shooter/blaster 自动射击、charger 蓄力、roller 滚压、bucket 泼洒、spinner 旋流、twins 双枪、brush 拖墨；brolly/bow/blade/mitts 分支同文件后段）。移速修正 L67-77。`Projectiles`（L776）统一弹丸物理/命中/拖尾喷墨。
- **副武器集**：`SUBS` 15 key（config.js L318 起：bomb/sticky/burst/seeker/scan/curtain/sprinkler/mine/beacon/mist/shaker/waddle/torpedo/tracer/boomerang）。`SubSystem.use/_throw/_plant`（subs.js L71/L81/L107）创建实体；实体可被子弹击毁 → 击毁方判 `['dh']`（kind,id,dmg）直发 owner 应用（netmatch L159/L210）。
- **特殊集**：`SPECIAL_ORDER` 19 key（config.js L469：slam/storm/barrage×5/bubbler/sonar/strike/zooka/wail/kraken/blower/jetpack/stamp/booyah/zipcaster/crab）。`SpecialSystem` 生命周期（specials.js L217-363）；充能值 = 每武器 `specialCost`（config WEAPONS 各条目）；网络态走 tick `sp`(s[13] 计量)/`spx`(s[22] SPECIAL_ORDER.indexOf+1)/`spx`(s[23] specialNetState) + `['ev'] special:use/slam`（net.rs 已占位 L411-451）。
- **Zone Control**：`opts.mode==='zones'`（match.js L14-23）；`ZONES` 参数 16 项（config.js L485-501：duration 300、count 100、rotate 30-60s、finalCentre 30、control .80/contest .40、rateCenter 1/rateHome .5/rateAway 2、penaltyK .75、gaugeHeld 4.5/gaugeNeutral 1.5、overtimeGrace 10/overtimeMax 300、sampleHz 5）。区域几何来自 `zones-data.js`（8 图全有 center/sideA/sideB 多边形）经 maps.js L118-121 合入 `layout.zones`。host 权威：`recZone(['z', e])` 仅 host 广播（netmatch L148），guest `zones.netEvent(e)` 应用（L495）；事件种类 `zz/zo/zp/za/zf/zt/zs/ze`（zones.js L265-424 段）。session cfg `mode` 三值 turf/zones/boss（session.js L292），zones 强制 `ZONES.duration`。
- **Rust 现状**：`inkwave_sim::weapon`（weapon.rs，仅 Spritzer runner）、`bot.rs`（仅 easy）、`match_.rs`（仅 turf）；`net.rs` 已占位 `KitRecord['k']`/`ZoneRecord['z']`/`DirectMsg::DeviceHit`/`BombRecord['b']` 与 sp/spx/spst 恒 0 语义（L140-205/L474-500/L598-600/L634）；`session.rs` turf-only 守卫 L37-40/L1270/L1306。`rust/tools/extract/`（extract-layout.mjs/extract-tuning.mjs/stable-json.mjs）管线已通用。
- **地图规模**（layout.js 行数）：tidewater 240、saltpan 160、crossmarket 151、lockgate 159、kelpline 250、cargo 250、terraces 718、halyard 内联于 maps.js L30。props.js 各图 ~1.8-2.1k 行（M3 不做装饰 props，仅提取碰撞体块，同 tidewater M1 口径）。cargo 为 onlineOnly/noBots（maps.js L524），不选。
- **跨端环境**：wrangler dev 本地 DO 沙箱可达（Task 27 探针实证 `HTTP/101` + welcome）；JS headless 客户端走 `tools/net-test.mjs` 形态（只读）。

## Functional Requirements

### 数值与管线
- **FR-M3-1 tuning 全量提取**：`extract-tuning.mjs` 扩至 `WEAPONS`（12 把逐字段）/`SUBS`（15）/`SPECIALS`（19）/`ZONES`/`DIFFICULTY` 进 `assets/tuning.json`；schema 文档更新；stable-json 保证可 diff；Rust `tuning.rs` 类型扩展 + 逐字段单测锚定上游字面量。
- **FR-M3-2 fixture 扩展**：`rust/tools/net-fixtures/gen.mjs` 增捕获：`['b']` 副武器记录（9 字段）、`'dh'` 直发、`['z']` zone 事件（8 种）、含 sp/spx/spst 非零的 tick 样本；Rust wire 编解码逐字节断言。

### 武器
- **FR-M3-3 武器 runner 架构**：`inkwave_sim::weapon` 按 kind 分发重构（对齐 weapons.js L32/L118-124 结构），弹丸系统泛化（对齐 Projectiles L776：速度/重力/散布/命中半径/拖尾喷墨/impactRadius），移速修正表（L67-77）。Spritzer 行为回归不破。
- **FR-M3-4 主武器 12 把**：分四批——(a) 射击类 twins/blaster/brolly；(b) 蓄力流类 charger/spinner/bucket；(c) 近战扫类 roller/brush/blade；(d) 特殊机制 bow/mitts（mitts 的 leap/wall cling 需 actor 状态机扩展：空中冲量、贴墙耗墨、落地溅射、guard 减伤弧）。每把：tuning 字段驱动、无头行为测试（射速/墨耗/伤害/散布逐值）、`tools/measure-handling.mjs` 口径对照。
- **FR-M3-5 副武器 15 个**：`SubSystem` 平移（throw/plant 物理、fuse/触发条件、区域效果：减速/驱雾/追踪/墙幕/跳跃垫等逐 key 对齐）；设备实体注册表（id 分配、击毁判定→`'dh'` 直发 + owner 应用）；联网下 sub 冷却走 tick 槽（对照 packActor 相应槽）。
- **FR-M3-6 特殊 19 个**：`SpecialSystem` 生命周期平移（start/tick/body/weapon/end + netGhost 远端呈现）；充能计量（gauge 0-100、specialCost、zone gaugeHeld/Neutral 加成）；`spx/spst/sp` 槽实值化；`special:use/slam` 事件联网重放。

### 模式与地图
- **FR-M3-7 Zone Control**：`inkwave_sim::match::zones` 平移 ZoneControl（多边形取格/占领 contest 阈值/倒计时速率/轮换/终局中心/加时/penalty/state 快照）；host 权威 + `['z']` 广播 + guest netEvent 应用；`res` 消息 zone 形态（count 剩余/penalty）对齐；session 守卫放开 `mode:'zones'`（仍拒 boss）。
- **FR-M3-8 新地图 ×2**：saltpan + crossmarket 提取（layout + 碰撞体块 + 出生点 + zones 多边形）；`assets/maps/` 索引扩展；无头碰撞冒烟（角色可行走区域/出生点落地）。

### bot 与大厅
- **FR-M3-9 bot 档位**：`DIFFICULTY` 六参数接入 + 行为差异平移（反应门限、瞄准弹簧 aimOmega/aimTurn、开火纪律、感知半径）；bot 武器/副/特殊使用（对照 bots.js L858 特殊驾驶段）；zone 模式补位涂区 tactic（L1176 needy patch 段）。
- **FR-M3-10 大厅 loadout**：极简 bevy_ui 增武器/副/特殊选择（12×15×19 列表滚动选择即可，无图标美术）；roster 字段实值化（weapon/sub/special/style 透传）；mode 选择 turf/zones。

### 跨端
- **FR-M3-11 跨端开黑验证**：JS headless（tools/net-test.mjs 形态，只读）↔ Rust（原生/WASM）经 wrangler dev 或 shim 跑完整局：turf 一把（双方不同主武器 + 副 + 特殊各触发 ≥1 次）、zones 一把；断言涂地覆盖差、zone 状态序列、结算一致、无 err 帧。

## Non-Functional Requirements
- **NFR-M3-1 sim 纯净性延续**：武器/副/特殊/zone 逻辑全在 `inkwave_sim`；`cargo tree -p inkwave_sim` 仍仅 glam/serde/serde_json。
- **NFR-M3-2 带宽**：满员 8 人全武器局 host tick 频率/大小仍远低于 relay RATE 90/s 与 MSG_MAX 64KB；事件峰值（特殊齐放）压测断言。
- **NFR-M3-3 确定性**：全部新武器/zone 逻辑无头可测（固定种子）；播放层指标门限（Task 25 定标）在武器事件增多后仍达标。
- **NFR-M3-4 门禁延续**：M1/M2 全部门禁每任务全绿；测试总数 > 273 单调递增；JS 树/`server/` 零改动。

## Acceptance Criteria

### AC-M3-1: 武器数值与行为对等
- **Type**: `rule`
- **Given**: tuning.json 全量提取 + 各武器无头行为测试
- **When**: 对 12 主武器逐把断言射速/伤害/墨耗/散布/弹道参数与上游字面量一致；handling 测量口径对照 JS 基线
- **Then**: 全部断言绿；tuning 提取 diff 可复核（stable-json）
- **Evidence**: `cargo test -p inkwave_sim` 输出 + measure 报告

### AC-M3-2: 扩展 wire 逐字节兼容
- **Type**: `rule`
- **Given**: fixture 扩展捕获（`['b']`/`'dh'`/`['z']` 8 种/sp 槽非零 tick）
- **When**: Rust codec 编解码
- **Then**: 与 JS 权威帧逐字节相等；`['k']`/`['bm']`/`['bc']` 解码容忍不 panic
- **Evidence**: 金标准测试全绿 + fixture 文件

### AC-M3-3: Rust↔Rust Zone Control 完整局
- **Type**: `rule`
- **Given**: shim + ≥3 无头客户端，mode zones，tidewater，固定种子
- **When**: 跑完整局（含轮换/终局中心/加时触发路径）
- **Then**: 各端 zone 状态序列逐 tick 一致、结算一致、`['z']` 无丢弃；turf 全武器局同口径复跑
- **Evidence**: harness 测试输出

### AC-M3-4: JS↔Rust 跨端完整对局
- **Type**: `rule`
- **Given**: wrangler dev（或 shim）+ JS headless 客户端 + Rust 客户端，双方各选不同武器
- **When**: turf 一局 + zones 一局完整打完
- **Then**: roster 武器字段互认、tick/事件互收无 err、涂地覆盖差 < 0.5%（turf）、zone 状态一致（zones）、结算一致
- **Pass Condition**: 沙箱实跑日志为准；不可达部分列用户机器复核清单并如实记录
- **Evidence**: 双端日志 + relay 记录

### AC-M3-5: 新地图提取与碰撞一致
- **Type**: `rule`
- **Given**: saltpan/crossmarket 提取产物
- **When**: 无头碰撞冒烟（出生点落地、可行走区域采样、zones 多边形内取格）
- **Then**: 与 JS 侧 layout/zones-data 数值一致；Rust↔Rust 各图 turf 完整局通过
- **Evidence**: 提取 diff + 冒烟测试输出

### AC-M3-6: bot 档位行为
- **Type**: `rubric`
- **Dimension**: normal/hard bot 在完整武器集下的对战行为（对照 JS 同参数 bot 的量化指标：命中间隔/存活时长/涂地速率）
- **Scale**: 1-5
- **Anchors**: 1=与 easy 无差别；3=六参数生效、行为单调（hard>normal>easy 于存活与命中指标）；5=同 seed 统计与 JS botlab 口径同量级
- **Pass Threshold**: ≥3
- **Evidence**: headless autopilot 统计报告

### AC-M3-7: 门禁与架构纪律
- **Type**: `rule`
- **Given**: M3 全部任务收尾
- **When**: 全量门禁矩阵（M2 Task 27 口径）
- **Then**: 全绿；测试数 > 273；sim 依赖面不破；JS 树/`server/` 零改动
- **Evidence**: 门禁日志

## Constraints
- 同 M1/M2（Rust stable、Bevy 0.19.1、trunk、不引入 GPL、**M3 不新增依赖**——武器/zone 全用现有栈）。
- relay/`server/`/上游树零改动硬约束。
- JS 行为对齐以源码行号注释为准（沿用 M1/M2 风格）。
- 数值一律走提取管线，禁止手抄（M1 纪律）。

## Assumptions
- 新武器/副/特殊视觉为体块/占位档即可满足玩法与网络验证（美术完整度 M4）；mitts/bow 等动画差异不阻塞判定。
- retired 武器（dualies/slosher/splatling）无跨端出现可能（JS 客户端 roster 仅发 `WEAPON_ORDER`，session.js L277-279），Rust 收到时按未知武器回退 shooter 并记录。
- 跨端 JS 客户端以 headless（net-test.mjs 形态）参与即满足验证；真人双端开黑列入用户机器复核清单。

## Open Questions
- [ ] 新地图最终选图：推荐 saltpan + crossmarket（layout 最小、均有 zones-data、无水面特殊机制）——若用户想验水面/高低差可换 kelpline（250 行 + 水面）。Task 37 实施前确认。
- [ ] mitts 的 leap/wall cling 对 actor 状态机（`inkwave_sim::actor`）的扩展幅度——Task 33 实施时定，若需改物理则记 PORT_MAP 偏差。
- [ ] 特殊武器 netGhost 远端呈现的占位档粒度（轮廓体块 vs 完全省略仅特效）——Task 35 实施时定。

---

# Task 30 实施设计（决策完备，2026-10-30）

> 上游事实源（全部已实读核线）：`src/game/weapons.js`（`_auto` L165-182、`_twins` L489-517、`_twinDodge` L529-548、`dodgeVelocity` L550-555、`tryDodge` L522-527、`fireBlaster` L1173-1192、`fireTwin` L1356-1392、`fireCustom` L1198-1213、`_step` L1583-1648、`_blastBurst` L1689-1709）；`src/game/kits/brolly.js`（fireBlast L79-119、update L122-159、shields L166-255、launch/addLaunched/updateLaunched L258-367、blockActors L369-386、tick L390-485、blockShot/blockRay L493-520、netState/netApply/ghost L635-653、钩子表 L657-670）；`src/game/kits/brolly-model.js` L20-26（`RIM_R=0.7`、`TH=52°`、`DEPTH=RS(1−cos TH)≈0.3414`）；`src/game/actor.js` L305-313（dodgeVelocity 覆盖 + tryDodge 吞 jump）；`src/config.js` L126-135/L158-167/L180-198；`src/net/netmatch.js` packActor L723-751 / recKit L134-137。

## 0. 决策记录
- **D-1 spec 修订（见 Non-Goals L26）**：brolly 的 `ks` 槽与 `['k','brolly',…]` 发射伞记录实值化 + 产生/消费；其余 kit 语义不变。
- **D-2 per-actor loadout**：`Match` 增 `pub kinds: Vec<WeaponKind>`（与 actors/runners 平行）；`Match::new` 默认全 `Shooter`；`build_net_match` 按 `RosterEntry.weapon` 经 `WeaponKind::from_kind_str` 映射（未知/retired 回退 `Shooter`，对齐 spec Assumptions L131）；`match_.rs` L502 硬编码 Shooter 改为 `self.kinds[i]`。bot 驾驶仍 `&Spritzer`（Task 39 接档位武器）。
- **D-3 散布默认**：blaster 维持 `1.2/4.0` 硬编码（config L126-135 确无 spread 字段，Task 29 P2-5 注记闭环）。
- **D-4 twins 的 `bloom += 0.25`**：JS L511 硬编码（非 `bloomPerShot`），twins 分支用 0.25。
- **D-5 twins 闪避不出现在 F 位**：JS packActor 的 `F.dodge` 读 `wr.dodge`（dualies 对象），twins 只动 `dodgeT/rollLock`，均不入 s[20]（`lockT` 是 dualies 字段）——Rust 逐字保持：twins 局 `lock` 槽恒 0、`F.dodge` 恒 0。
- **D-6 合并窗口**：brolly 弹丸 (blast,victim) 合并用 `ProjectileSim` 新增 `time`（`step` 累加 dt）+ `pending` 表，`MERGE=1/30`（L389）；跨端 hit 判定仍走既有 `Hit/HitSend` 路由。
- **D-7 呈现字段**：`flash/shake/vis/tail*/wob/nose/sats` 等 GPU 字段继续丢弃；`brolly:break/launch` 的 FX emit 不产生 sim 事件。

## 1. weapon.rs（核心改动）
- `RoundSpec` 增 `trail_start: f32`（JS 三式并存：shooter `-(2.5−every)` L1034、twins `-(2.4−every)` L1381、blaster 固定 `-1.5` L1178）；`fire_round` 改读 `spec.trail_start`。
- `Projectile` 增（serde default，向后兼容旧 fixture）：`blast: bool`、`pellet: Option<PelletMark>`；`PelletMark { blast: u32, falloff_start: f32, falloff_end: f32, dmg_near: f32, dmg_far: f32 }`（命中时按 `start.distanceTo(hit)` 走 `pelletDamage` L64-68，damage=0 的 JS 语义由合并器负责，不再依赖 kit 侧计分）。
- `WeaponKind::is_wired` → `Shooter | Twins | Blaster | Brolly`。
- `update` 分发臂：`Blaster => self.auto_fire(...)`（L118 与 shooter 同路）；`Twins => self.twins(...)`（L123）；`Brolly => brolly::update(self, ...)`（L128 `MAIN_KITS[kind].update`）。
- `auto_fire`：`(interval, ink)` 匹配臂加 `Blaster => (w.blaster.fire_interval, w.blaster.ink_per_shot)`；`fire_kind_round` 加 `Blaster => projectiles.fire_blaster(actor, &w.blaster, self.spread, aim_point)`。
- `fire_blaster`：`RoundSpec{ proj_speed 23, straight_time 99, range 10.5, grav 0, drag 0, size 0.26, life = range/proj_speed, impact_radius 1.5, damage = direct_damage, trail_start -1.5, trail_every 2.2, trail_radius 0.45 }` + `blast=true`（config L126-135 驱动，禁手抄）。
- `twins` 分支（L489-517 逐行）：`roll_lock/since_roll/rolls_used/roll_queued/twin_side/since_hand` 新字段（`since_hand` 初值 `[99,99]`、`since_roll` 初值 9、`twin_side` 初值 1，JS reset L48-51）；`since_hand += dt` 移入公共段（JS L116）；turret 取消（`!fire || |move|>0.3`，move 来自 `Actor` 本帧 intent——`update` 增参 `move_h: f32`）；dodgeT 递减/链式 rollQueued；开火循环 guard<3、`bloom+0.25`、`twinSide=-twinSide`、`fire_twin(side = turret?0:twinSide)`、`cooldown += turret?turret_interval:fire_interval`。
- `try_dodge(&mut self, kind, a, move_dir, fire) -> bool`（L529-548）+ `dodge_velocity(&self, kind) -> Option<Vec3>`（L550-555）：`v0=roll_dist/(0.7·roll_time)`、`v=v0(1−0.6u)`。
- `fire_twin`（L1356-1392 的 fallback muzzle 分支 L1360-1373，sim 无 rig）：镜像 muzzle `_v4` 计算 + `los` 回退；`speed/straight/range = side ? (proj_speed, straight_time, range) : (turret_proj_speed, turret_straight, turret_range)`；`size 0.13, life 1.2, grav 28, drag 0.8, trail_start -(2.4−trail_every)`。
- `step_one`：① 伞面挡弹钩子置于 body 扫描前——`shields: &[Shield]` 参数（`Shield{ team, c, n, r, cp, np, hp_mut_key }`），`predict`（L249-255：drag 仅 straight 后）+ `seg_shield` 扫掠（L195-200），命中 → canopy 扣 `proj_damage`（L242-247：pellet→falloff(行程)；`CANOPY_MUL{drop:.25}` 其余 ×1；sim 暂无 drop/slosh 弹种）→ 弹丸 `dead` 且不 emit Impact；② `blast` 弹：body/world/life 三类死亡都走 `_blastBurst`（L1689-1709：burst 落点下 raycast 3.5 喷墨 `impact_radius`、splash 敌 `lerp(splash_damage_max, min, d/splash_radius)` LOS 检查、direct 命中者跳过；emit `Impact{kind blast, radius burst_radius}`）；③ `pellet` body 命中 → 写 `pending[(blast, victim_slot)]` 合并（1/30 s），到期一次性 `apply_hit` + `Hit/HitSend`，弹丸即死。
- `move_speed`：`Brolly` 臂接 kit 语义（L664：`open>0.2 → move_speed_shield`，否则落尾链 `firing_t>0 ? move_speed_firing : run`）。
- `spread_deg`：`Brolly => grounded ? w.brolly.spread_deg : w.brolly.spread_air`（L665）。
- 访问器（TR-30.2）：`charge()`、`lock_t()`（dualies 字段，保持 scaffold 语义）、`streaming()/burst_frac()`（Task 31 前恒 false/0，字段补齐）、`brolly_net_state() -> u32`（L636-641 位包）与 `net_apply_brolly(bits)`（L642-649：hp 降幅→flash 语义丢弃，仅 state/hp/open/grow/regrowK 重建）。

## 2. weapon/brolly.rs（新文件）
- `BrollyKit`：`hp/hp_max/state(Ready|Broken|Launched)/regrow_t/regrow_k/grow/open/opening/want/press_t/open_t/launch_k/shot/prev_fire/lock_release/last_hit`（L52-57，呈现字段删）；`WeaponRunner.brolly: Option<Box<BrollyKit>>` 惰性 `kit_of`。
- `update`（L122-159）：`botHold` 分支删（bot 不持 brolly，Task 39）；tap-blast 条件 `!shot && cooldown<=0 && open<0.05 && press_t<=open_delay+0.06`；`want = press_t>=open_delay`；开合 `openTime/closeTime`；`open>=1 && want → open_t += dt ≥ launch_hold → launch`。
- `fire_blast`（L79-119）：空墨 `_empty + cooldown=max(cd,0.2)`；`PATTERN` 11 点（1 心 + 4×0.42@i/4·2π+0.4 + 6×0.9@i/6·2π）；`cone=tan(spread)`、`turn=rng·2π`、抖动 `±0.09`、`oy×0.72`；每丸 `fire_round`（`speed·(0.985+rng·0.03)`、`radius=pellet_paint·(0.85+rng·0.3)`、`trail_every 0`、`pellet=Some(PelletMark{blast=SEQ++…})`、damage 0）。
- `launch`（L258-273）：落点 raycast（`pos+dir·0.9, +1.2` 下 3.5）；`hp=max(k.hp, canopy_hp·0.5)`；`h0=eye+0.9−pos.y`；产出 `SimEvent::Kit{nid, 'brolly', [x,y,z,dx,dz,hp,h0]}`（r2/r3 量化同 JS）；`state=Launched, regrow_t=regrow_time, lock_release=true`。
- `LaunchedCanopy`（L276-367）：`Match.launched: Vec<…>`；slide `launch_speed·(shoving?0.6:1)·min(1,0.35+t·3)`、双高 raycast 挡墙（0.3/1.0，`0.12+step`）、地面跟随/重力 24/入海 `waterY−0.4` 死亡、每 0.32 m `splat(launch_paint, stretch=dir)` + `Turf` 事件、`launch_life` 后 fade 0.45/0.18（hp≤0）移除；`block_actos`（L369-386：敌队推挤 `PLAYER.radius+0.18`、`|across|<rr` 推出、法向速度清零、`shoving` 标记）；盾盘 `C=pos+(LAUNCH_H+(h0−LAUNCH_H)·max(0,1−t/0.22))·ŷ + dir·(−DEPTH/2)`、`N=dir`、`R=0.8`。
- 伞维护（tick L393-412）：idle 折叠（`!alive || form!=kid`）；`state!=ready` → regrow 递减→`Ready, hp=hp_max, grow=0.001`；`ready` → `grow += dt/0.32`、`open<=0 && time−last_hit>canopy_regen_delay → hp += canopy_regen·dt`；`hit_shield`（L203-222：hp 扣减、`last_hit=time`、hp≤0→`break_shield` L223-239：`state=Broken, regrow_t, lock_release=true`）。
- `held_shield`（L172-177/L184）：`alive && kid && Ready && open>=0.55 && grow>=0.9` 才成盾；`N=(sin yaw·cos p, sin p, cos yaw·cos p)`、`p=clamp(aim_pitch,−0.5,0.45)`、`C=visual_pos+0.9ŷ+N·0.62`、`R=0.8`；`Cp/Np` 帧间快照（`prev/now` 扫掠 L193-200）。
- `block_shot(prev,pos,team,dmg) -> bool`（L493-503，供 Task 31 charger 激光与副武器复用；本任务由 step_one 内联扫掠覆盖）。

## 3. actor.rs（最小扩展）
- `Actor` 增 `pub dodge_vel: Option<Vec3>`（`integrate` 的水平段后覆盖 `vel.xz`，镜像 actor.js L305-306）与 `pub jump_swallowed: bool`（step 顶部消费并清）；`pub fn jump_edge(&self, input) -> bool`（`input.jump && !self.prev.jump`）。
- `Match::step` 在 `a.step` 前：`if a.jump_edge && !squid && r.try_dodge(kind, a, move, gate.fire) { a.jump_swallowed = true }`；每帧 `a.dodge_vel = r.dodge_velocity(kind)`（仅 Twins kind；brolly/其余 None）。既有直接 `Actor::step` 测试零回归（字段默认 None/false）。

## 4. net 层（TR-30.2）
- `ActorSample::from_actor` 增 `kind: WeaponKind` 参：`ch = wr.charge()`（`streaming` 前等价 JS L748）、`lock = wr.lock_t()`、`ks = (kind==Brolly).then(wr.brolly_net_state(), 0)`；调用点 replication.rs L889/L2203/L2214 与 wire 测试同步。
- 播放侧：proxy actor 的 tick 应用时 `kind==Brolly → wr.net_apply_brolly(ks)`（JS netApply L642-649），使远端伞在本机 `refreshShields` 等价物中成盾挡本地弹；`['k'] brolly` 事件 → `addLaunched(ghost=true)`（JS ghost L650-653：ghost 伞滑行挡弹挡人、不喷墨——`mute` 语义即 `ghost` 弹丸既有规则）。
- `SimEvent::Kit { nid, kind: String, data: Vec<f32> }` 新变体 + `wire.rs` `['k']` 编解码（recKit L136 形态 `['k',nid,kind,data]`；既有解码容忍路径保留，未知 kind 忽略）。

## 5. 测试（workspace 总数 > 303）
- **TR-30.1**（`weapon/tests.rs` + `weapon/brolly_tests.rs`，固定种子）：
  - twins：射速 0.085/发（guard<3 补发）、`ink 0.85/发`、damage 30、散布 ground 6°/air 11°/bloom 系数 `lerp(0.5,1,bloom)`、bloom +0.25/发、turret 态（roll 结束仍按火→`turretInterval 0.058/turretSpread 1.6/turretProjSpeed 36/turretStraight 0.16/turretRange 13.5`）、roll 链（`rollInk 7/rollTime 0.3/rollDist 3.4/rollCharges 2/rollLockout 2.6/rollReset 1.4`，`dodgeVelocity` 端点值）、`move_speed` 5.2。
  - blaster：0.78/发、ink 9、direct 125、blast 弹字段（straight 99/grav 0/drag 0/size 0.26/life 10.5/23）、burst：world 命中喷墨 + splash `lerp(70,30,d/2.6)` 逐值、寿终空中爆。
  - brolly：0.45/发、ink 5.5、11 丸锥（PATTERN 半径 0/0.42/0.9、y×0.72）、`pelletDamage 8→2.5@2.8→6.5`、canopy 时序（openDelay 0.12/openTime 0.14/closeTime 0.1/launchHold 0.9/regrowTime 5/regen 60@2s）、伞挡弹扣血/破伞/重长、发射伞滑行 0.32 m 喷墨条 + 挡人、合并一击（同 blast 同 victim ≤1 个 `Hit`）。
- **TR-30.2**：三武器 loadout 无头联网 tick：`from_actor` 的 ch/lock/ks 逐槽对 JS packActor 语义断言（twins/blaster ks=0、brolly ks 位包=手算期望、lock 恒 0、F 位无 dodge）；JS→Rust 收 `['k'] brolly` 生 ghost 伞且挡本地弹；Rust→Rust 伞状态经 ks 收敛一致。
- 回归：TR-29.2 四组 + M1 武器测试不破（`trail_start` 重构后 shooter 逐值同前）。

## 6. 门禁与落盘
- 全量九段矩阵（`rust/tools/verify/logs/task30-gates-<UTC>.log`）；`cargo tree` 依赖面不变；JS 树/`server/` 零改动；测试总数 > 303。
- `tasks.md` Task 30 段：Status completed + Completion Evidence + Review 附录；`PORT_MAP.md` 行更新；ndruster 身份提交（不 push）。

# Task 31 实施设计（决策完备，2026-10-31）

> 锚点：weapons.js `_charger` L184-216 / `fireCharger` L1238-1303 / `_spinner` L450-486 / `fireStream` L1335-1353 / `_bucket` L434-447 / `fireSlosh` L1307-1332 / `_lobAim` L997-1025 / `_step` L1592-1614（volley/belowFalloff）/ `_impact` L1667-1687；config.js charger L117-125、bucket L137-147、spinner L148-157。数值全走 `tuning.rs`（Task 28 已提取 `Charger`/`Spinner`/`Bucket`）。

## 1. charger = 纯 hitscan（无弹丸实体）
- `ProjectileSim::fire_charger(a, w, charge, world, actors, paint, t, remote)` 平移 JS L1238-1303：muzzle/aim 同 `fire_round` 前段；`range = lerp(rangeMin, rangeMax, charge)`；`dmg = charge>=0.999 ? damageMax : lerp(damageMin, damageMax*0.62, charge)`。
- 世界段：`world.raycast(m, dir, range, true)`；敌人胶囊扫描：`segment_capsule_dist(m, m+dir·len, hit_base, PLAYER.radius+0.12, h)`，阈值 `PLAYER.radius+0.14`，`d = t·len` 取最近 victim。伤害路由与 `step_one` body-hit 完全同构（local `apply_hit`+`Hit` / proxy `HitSend`）。
- 沿线喷墨：`s = 1.2; s < len−0.3; s += lineSplatEvery`，每步 down-raycast 3.5、`splat(point+normal·0.1, lineRadius·(0.8+0.4·charge), stretch=dir, stretchAmt=1.2)`，seed 逐次新抽；终点 splat 仅当世界命中且无 victim：`impactRadius·(0.6+0.4·charge)`、`stretchAmt=0.6`。
- RNG 消耗序：line seeds（逐 splat）→ 终点 splat seed。beam `uSeed`（L1293）纯呈现不抽（同 Task 29 `vis` 删法）。`subs.blockRay`/`specials.rayHit` 截断（L1245-1246）为 Task 34/36。
- runner `_charger` 1:1：起蓄门槛 `ink ≥ inkFull·0.2` 否则 `_empty`；S 曲线 `t<0.2 ? t·1.25 : 0.25+(t−0.2)·0.9375`，`charge = min(clamp(ink/inkFull,0,1), curve)`；释放帧 `c = max(0.12, charge)`、`ink −= inkFull·c`、`lastFire=0`、`firingT=0.35`、`cooldown=0.28`、`charge/chargeT` 清零。新增字段 `charge_t`（JS `chargeT`）。**f32 偏差**：JS double 的 `chargeT` 累加在满充帧溢出后钳到恰 1.0（ding 触发、释放 c=1.0）；f32 累加落在 ~0.9999997，故时钟在 `>1−1e-6` 时吸附 1.0（每帧增量 ≥0.014，无误触；spinner 线性时钟同法）。

## 2. SimEvent::Fire 携带 charger 的 charge/len
- JS L1281 `weapon:fire` 仅 charger 带 `charge`/`len`（远端 `_ghostBeam` L898 消费）。`SimEvent::Fire` 增 `charge: Option<f32>`、`len: Option<f32>`（serde default，其余 kind `None`）；`MatchEvent::Fire` 透传。`['ev']` payload 的 `weapon` id 取 `m.kinds[i].id_str()`（JS `packEvent` 透传真实 `w.id`，远端 `WEAPONS[e.weapon]` 分发；此前恒记 Spritzer 为 Task 30 遗留，Task 31 修正）。wire 逐字节 fixture 对照留 Task 36。
- **D-T31-1（有意偏差）**：charger 释放不在 runner 内联执行 hitscan（runner 无 actor 列表做 victim 扫描），而是 runner 置 `charger_release: Option<f32>`、match 层同迭代 drain 调 `fire_charger`。帧位与 JS 内联一致（actor 姿态/aim 与 RNG 序不变）。残余：JS 为逐 actor「移动+武器」串行（match.js L228），actor i 放束时 victim j>i 尚未移动；sim 两遍循环全体先移动，近失束 victim 扫描位差最多一帧（~0.13m@8m/s）。

## 3. spinner：蓄力→流（burst，非 streaming）
- runner `_spinner` 1:1（L450-486）：起蓄门槛 `inkFull·0.15`；`charge = min(clamp(ink/inkFull), chargeT)`（线性，无 S 曲线）；释放 `c = max(0.15, charge)`、`ink −= inkFull·c`、`burstPower=c`、`burstT = lerp(burstMin, burstMax, c)`、`burstDur=burstT`、`streamCd=0`。burst 期：每帧 `burstT−=dt`、`streamCd−=dt`、`firingT=0.2`、`fireFacing=0.5`、`lastFire=0`，`while streamCd<=0 && guard<4` 发流并 `streamCd += fireInterval`；burst 结束 `cooldown=0.2`。新增字段 `charge_t/burst_power/stream_cd/burst_dur`。
- **不置 `streaming`/`burstFrac`**：JS 仅 retired splatling 置（L381-426）；spinner 的 `ch` 槽 = `charge`（burst 期为 0）、`F.charging` 仅蓄力期为 1。
- `fire_stream(a, w, power, spread_deg, aim_point)` = `fire_round` 包装：`speed=lerp(speedMin,speedMax,p)`、`straight=lerp(straightMin,straightMax,p)`、`range=lerp(rangeMin,rangeMax,p)`、`grav=28`、`drag=0.8`、`size=0.12`、`life=1.3`、`trail_start=−(2.2−trailEvery)`、trail/impact/damage 走 tuning。RNG 序：spread(2) → seed(1)。range→life 上限沿用 Task 29 偏差。

## 4. bucket：flick windup + 抛洒弧 volley
- runner `_bucket` 1:1（L434-447）：`flick≥0` 期间 `flick+=dt`、`fireFacing=0.45`，`flick≥windup` → `fireSlosh`、`cooldown=fireInterval−windup`、`firingT=0.35`；起手可 `firePressed`（JS L441 `inp.fire || inp.firePressed`）；墨门槛 `inkPerShot`。
- `fire_slosh`（L1307-1332）：默认仰角 `atan2(dir.y,hypot)+lob` 重设 dir；`_lobAim`（L997-1025）原样平移（无 RNG、步长 SIM_DT=1/60（weapon.rs 既有常量）、`hd<1 || hd>reach` 直接放弃），仅 `aim_point=Some` 时启用。逐 blob（i=0..blobs）：`sp=throwSpeed·(1−i·0.075)`、`side=(rng−0.5)·0.05`、`pi=pitch+(rng−0.5)·0.04`、`size=0.22−i·0.02`、`radius=impactRadius·(1−i·0.08)`、`pos=m−dir·i·0.12`、`life=2.2`、`straight=0`、`grav/drag` 走 tuning、`trailEvery=1.6`、`trailRadius=0.42`。RNG 序：每 blob side→pi→seed，共 3×blobs。
- `Projectile` 新字段（serde default 向后兼容）：`below_falloff`/`below_falloff_max`（命中时 `dmg ·= 1 − clamp((start.y−(e.pos.y+0.8)−0.5)·bf, 0, bfMax)`，JS L1605）、`volley: Option<u32>`（同抛组）。`ProjectileSim` 记账：`volley_next` 计数 + `volley_hits: BTreeMap<(u32,usize),()>` + `volley_alive: BTreeMap<u32,usize>`（despawn 时递减、归零清组）。`step_one` body-hit：已命中同组 → `continue` 扫下一 actor（JS L1599），否则记录。bucket 为 type 'slosh' 无 `vol/head`：`_impact` 走普通弹道喷墨分支（JS L1671 注释 "our bucket's blobs paint as before"），无需新分支。

## 5. net 接线与其余
- `from_actor` 置 `F.CHARGING`（JS L732；net.rs 常量已在位）——新访问器 `WeaponRunner::charging_flag()`；bucket windup 置 `F.FLICK`（JS L740 `wr.flick >= 0`）——新访问器 `flick_flag()`；`ch` 槽经既有 `charge()` 自动生效。播放侧 remote proxy 不驾驶 runner，flag/ch 存 tick 样本即可（fixture Task 36）。
- **延后偏差（Task 36 fixture 前记录）**：spinner/bucket 的 `['p']` Proj 记录仍按 Spritzer 规格重建（`record_fire` 的 Proj 块取 `w.shooter` 字段），JS `recProj` 对 stream 逐弹记 spinner 规格、对 slosh 记 3 条抛体（速度/尺寸/life 各不同）。ghost 仅表现层，不影响 sim 权威态；真实 per-kind `['p']` 规格随 Task 36 wire fixture 一并修。
- `is_wired += Charger|Spinner|Bucket`；update 分发臂从 no-op 骨架改为调 `_charger`/`_spinner`/`_bucket`。`move_speed`/`spread_deg` 三 kind 臂 Task 29 已平移，不动。
- Task 30 P2-5 的 `net-harness/match_e2e.rs` bot-kinds shim：bow/roller/brush/blade/mitts 仍未接线（Task 32/33），seed 77 仍可抽到未 wired kind → **保留 shim**，注释改「Task 33 后移除」。

## 6. 测试与门禁
- TR-31.1（固定种子逐值）：charger 蓄力曲线逐帧（S 段 1.25/0.9375 系数、maxCharge 钳制、ink 不足门槛）、释放时序（c≥0.12 下限、ink 扣减、cooldown 0.28、半充 damage=lerp(40,99.2,0.5)=69.6、range=19）、满充 160/27、line splat 计数与终点 splat 半径；spinner 线性充、burst 时长 lerp(0.35,1.8,c)、流节奏 0.05（guard<4 补发）、流弹 speed/straight/range lerp、同 seed 复现；bucket windup 0.12→fireSlosh、cooldown 0.48。
- TR-31.2（bucket 弧采样口径）：5 blob 的 radius/size/sp 逐值、pos 回推 −i·0.12、side/pi 抖动界（±0.025 rad / ±0.02 rad）、同 seed 弹道逐值复现、异 seed 不同；`_lobAim` 在 reach 内命中目标（数值收敛）、超 reach 保持默认弧；volley 同组第二丸穿已命中 victim、belowFalloff 下打减伤（0.12/m、上限 0.35）。
- 门禁：`/tmp/task30-gates.sh` → `/tmp/task31-gates.sh`，日志 `task31-gates-<UTC>.log` 九段全 exit 0；workspace 测试 > 321；JS 树零改动。


---

# Task 32 实施设计（决策完备，2026-10-31）

## 0. 范围与模式
主武器批三：roller（weapons.js `_roller` L218-290 + `fireFlick` L1215-1236）、brush（`_brush` L571-626 + `fireSwipe` L1396-1417）、blade（kits/blade.js `MAIN_KITS.blade` L622-743，仿 Task 30 brolly 的 kit 模式：`weapon/blade.rs` + `weapon/blade_tests.rs`）。近战判定 owner 本地 `applyHit` + splat 广播，无新 wire 类型；`['k','blade']` 复用 Task 30 的 `TickEvent::Kit` 通道。

## 1. 共享基建改动（weapon.rs）
- `Projectile` 新增 `#[serde(default)] pub dmg_far: Option<f32>`：JS `type:'drop'` 伤害衰减（`_step` L1603 `dmg = lerp(damage, dmgFar, clamp(start.distanceTo(hit)/7,0,1))`），仅 `Some` 时生效，在 below_falloff 之前乘算（JS 顺序 L1603→L1605）。
- `step_one` 延迟门（JS update L1571 `if(p.delay>0){p.delay-=dt; if(p.delay>0) continue}`）：`Projectile` 新增 `#[serde(default)] pub delay: f32`，在积分之前递减/跳过（age/prev/pos 全不动）。
- brolly 盾预扫跳过延迟丸（brolly.js L428 `if(!p.owner || (p.delay>0 && p.delay-dt>0)) continue`）：`step_masked` 盾循环内同条件跳过。
- `WeaponRunner` 新增字段（JS `reset()` L37-61 对齐）：`swipe_side: i32`（brush，初值 1）、`last_roll_pos: Vec3`、`roll_hits: BTreeMap<usize,f32>`（per-victim 近战冷却倒计时；JS `G.time` 绝对时刻（L264/L608）在 sim 无全局时钟下等价改写：每帧 `retain(|_,v| *v-=dt >0)` 后判 `<=0`、命中置 `cd`，窗口语义逐值一致）。blade kit 字段 `blade: Option<Box<BladeKit>>`、波次交接 `blade_wave: Option<BladeWave>`、`kid_t()` 只读访问器（Actor `kid_t` 私有，blade `canAct` 需要）。
- `update` 签名扩展：追加 `actors: &mut [Actor]`、`paint: &mut PaintGrid`、`t: &PlayerTuning`、`remote: Option<&[bool]>`、`ps_time: f32`（`ps.time`，blade 空闲判定/波次时钟）。roller/brush 敌扫描与 blade melee 需要 actor 列表与画布；proxy 路由复用既有范式（`remote[i]` → `HitSend`，否则 `apply_hit`+`Hit`）。
- dispatch（JS L117-129）：`Roller => self.roller(...)`（L120）、`Brush => self.brush(...)`（L124）、`Blade => blade::update(...)`（L128 default→MAIN_KITS）；Bow/Mitts 保持空分支至 Task 33。`is_wired` 增 Roller/Brush/Blade。
- `move_speed`：Blade 分支替换 `run`（JS blade.js L683-689：`swing.heavy && t<0.3 → moveSpeedFiring*0.55`；`charging → moveSpeedCharging`；`firingT>0 → moveSpeedFiring`；否则尾链 `run`）。Roller/Brush 分支 Task 29 已就位，本批接线后生效。
- 注记（同 Task 31 P2-2）：sim `aim_point` 恒 None → `cutYaw` 直接落 `a.yaw`（blade.js L65-71 aimPoint 分支不可达）。

## 2. roller 分支（JS `_roller` L218-290 逐值）
- flick 风上（L221-232）：`flick>=0 → flick+=dt; flick>=flick_windup → flick=-1; ps.fire_flick(a, roller); cooldown=flick_interval-flick_windup; firing_t=0.25`（JS 字面量 0.25，非 `FIRING_T`）。
- 出手（L233-242）：`gate.pressed && cooldown<=0`：`ink<flick_ink → _empty`（`empty_cd<=0 → EMPTY_CD+LowInk`），否则 `ink-=flick_ink; note_fired; flick=0; return`。
- 滚动（L243-253）：`can_roll = gate.fire && grounded && ink>0.5 && cooldown<=0.25`；`roll_t = can_roll ? +dt : 0`；边沿进入 → `rolling=true; last_roll_pos=a.pos`；`gate.fire && ink<=0.5 → _empty`；`!rolling → return`；`note_fired`。
- 位移扣墨+涂墨（L254-289）：`fx=sin(yaw), fz=cos(yaw)`；`moved=|a.pos-last_roll_pos|`；敌扫描（见下）先于位移门；`moved<0.28 → return`；`last_roll_pos=a.pos`；`ink=max(0, ink-roll_ink_per_meter*moved)`；3 个 stripe splat：`i=-1..1`，`off=i*roll_width*0.33`，点 `(a.pos + f*0.75 + rx*off, y+0.35)`（`rx=(fz,0,-fx)`），`radius=0.62`（JS 字面量），`SplatOpts{seed:rng, kind:Some(SplatKind::Roll), stretch:Some((fx,0,fz))}`，每 splat 一次 rng 抽值；`Impact{pos:(a.pos+f*0.75, y+0.02), normal:Y, radius:roll_width/2, owner_slot, team, area=Σ, victim_slot:None}`。
- 敌扫描（L258-267）：`hs=hypot(vel.x,vel.z)`；敌 `e`：`fwd=dx*fx+dz*fz ∈ (-0.2,1.35)`、`lat=|dx*fz-dz*fx| < roll_width/2+0.35`、`|dy|<1.2`、`hs>1.0` → 冷却 `0.5`（JS 字面量）→ 伤害 `roll_damage=140`。
- boss 分支（L269-273）与 audio/rumble/character 不移植（既有范围）。

## 3. brush 分支（JS `_brush` L571-626 逐值）
- flick（L573-577）：`flick+=dt; flick>=0.04 → flick=-1; ps.fire_swipe(a, brush, swipe_side); cooldown=swipe_interval; firing_t=0.25`（JS 硬编码 0.04，非 config 字段）。
- `moving = grounded && hs>2.0 && |intent.move|>0.3`（`move_dir` 参数，JS 读 raw intent）。
- 出手（L580-590）：`(pressed || (fire && !moving)) && cooldown<=0`：`ink<swipe_ink → _empty`，否则 `ink-=swipe_ink; note_fired; flick=0; swipe_side=-swipe_side; rolling → rolling=false; return`。
- 拖墨（L591-599）：`brushing = fire && moving && ink>0.3`；边沿进入 → `last_roll_pos=a.pos; roll_hits.clear()`；`!rolling → return`；`note_fired`。
- 敌扫描（L602-611）：`fwd∈(-0.2,1.2)`、`lat<brush_width/2+0.3`、`|dy|<1.2`（无 hs 门槛）→ 冷却 `brush_hit_cd=0.35` → `brush_damage=30`。
- 位移（L618-624）：`moved<0.22 → return`；`ink=max(0, ink-0.32*moved 字段)`；1 splat：`(a.pos+f*0.65, y+0.35)`，`radius=brush_width*0.45`，`stretch=(fx,0,fz)`，`stretch_amt=0.5`，`kind=None`（JS L623 未传 kind），seed 1 抽；`Impact{pos:(x+fx*0.65, y+0.02, ...), radius:brush_width/2, area}`。
- `move_speed` Brush 分支 Task 29 已就位（rolling lerp(moveSpeedFiring→brushSpeed, smoothstep(0,0.35,rollT))；firingT>0→moveSpeedFiring）。

## 4. fireFlick / fireSwipe（ProjectileSim 新方法，drop 弹种）
- `fire_flick`（L1215-1236）：`m=(pos.x, pos.y+1.0, pos.z)`；`up=clamp(aim_pitch,-0.2,0.5)+0.32`；`t=(i/(drops-1))*2-1`；每 drop RNG 序 = [角度抖动, 速度, 半径, seed, cu]（`vis` 为 presentation-only 不抽，同 Task 29 规则）：`ang=yaw+t*flick_spread_deg*DEG*0.5+(r-0.5)*0.05`；`sp=flick_speed*(0.82+0.28*(1-|t|)+r*0.08)`；`radius=0.85+r*0.3`；`seed`；`cu=cos(up+(r-0.5)*0.12)`。drop 字段：`life 1.4, straight 0, size 0.15, dmg=near, dmg_far=Some(far), trail 0, trail_every 1.8, trail_radius 0.45, grav 26, drag 0.4`；`pos=(m.x+fx*0.6, m.y+0.3, m.z+fz*0.6)`（prev=start=pos）；`vel=(sin(ang)*cu*sp, sin(up)*sp, cos(ang)*cu*sp)`。发射后 `Fire{muzzle:(a.pos+f*0.6, y+1.3, z+fz*0.6), dir:normalize(fx, sin(up), fz)}`。
- `fire_swipe`（L1396-1417）：`up=clamp(aim_pitch,-0.3,0.3)*0.5+0.08`；`arc=swipe_spread_deg*DEG*1.6`；`t=i/(drops-1)`；`ang=yaw-side*(t-0.5)*arc`；`ox=sin,oz=cos`；`tx=-oz*side,tz=ox*side`；RNG 序 = [sp, radius, seed, cu]：`sp=swipe_speed*(0.8+r*0.3)`；`radius=0.55+r*0.2`；`cu=cos(up+(r-0.5)*0.08)`；drop：`life 1.0, size 0.1, trail_every 2.2, trail_radius 0.32, grav 26, drag 0.6, dmg=near/far`；`pos=(x+ox*0.85, y+0.45, z+oz*0.85)`；`hx=ox*0.8+tx*0.45+fx*0.25, hz 同, hl=hypot`；`vel=(hx/hl*cu*sp, sin(up)*sp, hz/hl*cu*sp)`。`Fire{muzzle:(x+fx*0.85, y+0.45, z+fz*0.85), dir:normalize(fx, sin(up), fz)}`。

## 5. blade kit（weapon/blade.rs，仿 brolly.rs）
`BladeKit{held,hold_t,charge,charging,full,store,cd,queued,side:i32(-1),swing:Option<Swing>,lunge,lx,lz,lv,last_t,since}`；`Swing{heavy,t,side,yaw,pitch,hits:BTreeSet<usize>,fired,dev,stroke}`。
- `update`（L624-678）：`ps_time-last_t>0.3 → cancel_charge; held=false; swing=None; lunge=0; queued=0`；`last_t=ps_time`；`cd-=dt; since+=dt; queued=max(0,queued-dt)`；`swing → run_swing`；`raw=gate 前的 intent.fire`（sim 用 `gate.fire||gate.pressed` 近似 raw——注记：JS `a.intent.fire` 是未门控原始输入，sim 的 gate 已含 emerge 抑制；`canAct` 独立判 form/kid_t/alive，raw 差异仅影响 squid/emerge 期的 held 保持，记为偏差 D-T32-2）；`canAct = form!=Squid && kid_t>=emerge_delay && alive`；`!canAct`：charging 时 `store+=dt; a.note_fired 侧 lastFire`（JS `a.lastFire=0` → `note_fired`）；`!raw || store>charge_store → cancel_charge`；`held = held&&raw`；`r.charge=k.charge; return`。`store=0`。
- `pressed || (auto&&fire&&cd<=0)`：`cd<=0 → tap; else queued=0.16`；`held=true; hold_t=0`。`else queued>0 && cd<=0 && !charging → queued=0; tap`。（`coreDriven` bot 逻辑不移植：`auto=false`；`botHolds` 同理省略，记 D-T32-3。）
- `fire && held`：`hold_t+=dt`；`!charging && hold_t>=charge_delay && !(swing&&heavy) → charging=true,charge=0,full=false`；`cap = ink>=heavy_ink ?1:0.94`；`charge=min(cap, charge+dt/charge_time)`；`charge>=1 && !full → full=true`；`cap<1 && charge>=cap && !low_warned → low_warned=true; _empty`；`a.fireFacing 侧` 不移植；`r.firing_t=max(firing_t,0.3)`；`note_fired`。
- `held && !fire`（释放）：`held=false; low_warned=false`；charging：`c=charge; cancel_charge; c>=1 → heavy; else c>=0.3 → (cd<=0.04 ? cd=0,tap : queued=0.16)`。末尾 `r.charge=k.charge`。
- `cancel_charge`（L49-53）：`charging=false; charge=0; full=false; store=0; r.charge=0`。
- `tap`（L72-86）：`ink<tap_ink → _empty; cd=0.18; return`；`ink-=tap_ink; note_fired; side=-side; cd=tap_interval; since=0; swing={heavy:false,t:0,side,yaw:a.yaw,pitch:a.aim_pitch,hits:{},fired:false,dev:false}; r.firing_t=0.4`。
- `heavy`（L87-102）：`ink=max(0,ink-heavy_ink); note_fired; cd=heavy_interval; since=0; swing={heavy:true,side:1,stroke:false,...}; r.firing_t=0.5`；`grounded && lunge_safe → lunge=lunge_time; lx/lz=(sin,cos)(swing.yaw); lv=lunge_dist/(lunge_time*0.75)`。
- `lunge_safe`（L103-111）：`k∈[0.5,1,1.4]`：`world.ground_height(pos+f*d*k, y+0.6) > water_y+0.25`（bot 悬崖分支不移植）。
- `run_swing`（L114-127）：`t+=dt`；`!fired && t>=(heavy?0.05:0.028) → fired=true; heavy?spawn_wave:fire_drops`；`heavy && !stroke && t>=0.075 → stroke=true; ground_stroke`；`t>=0.01 && t-dt<=（heavy?0.16:0.12） → melee`（本帧跨过窗口即判，同 JS）；lunge：`-=dt; >0 && alive → v=lv*(0.55+0.45*min(1,lunge/(lunge_time*0.5))); a.vel.x=lx*v; a.vel.z=lz*v`；`t>0.4 → swing=None`。
- `fire_drops`（L131-178）：`steep=pitch<-0.55`；`up=steep? pitch*0.85 : clamp(pitch,-0.4,0.45)*0.75+0.06`；`vol` 组复用 Task 31 volley 基建（`volley_next/volley_hits/volley_alive`，`volley=Some(g)`，`volley_alive.insert(g,N)`）；`t=i/(N-1)`；`off=side*(t-0.5)*tap_spread_deg*DEG`；`mid=1-|t-0.5|*2`；RNG 序 = [sp, radius, seed, cu]：`sp=tap_speed*(0.8+0.24*mid)*(0.95+r*0.1)*(steep?0.45:1)`；`radius=tap_drop_radius*(0.85+r*0.3)`；drop：`life 1.2, straight=steep?0:tap_straight, size 0.13, dmg=near, dmg_far=Some(far), trail -0.2, trail_every 1.25, trail_radius 0.3, grav 24, drag 0.5, delay=t*0.018`；`r0=steep?0.55:0.8`；`pos=(x+ox*r0, y+(steep?0.7:0.98)+(0.5-t)*0.12, z+oz*r0)`；`!los((x,pos.y,z),pos) → pos=(x,y,z)+(ox,0,oz)*0.12`；`cu=cos(up+(r-0.5)*0.05)`；`hx=ox+tx*0.12, hz=oz+tz*0.12`；`vel=(hx/hl*cu*sp, sin(up)*sp, hz/hl*cu*sp)`。发射后 `Fire{muzzle:(x+sin(yaw)*0.8, y+0.98, z+cos(yaw)*0.8), dir:normalize(sin(yaw), sin(up), cos(yaw))}`。
- `ground_stroke`（L181-194）：`d∈[0.75,1.35]`：`raycast((x+fx*d, y+0.8, z+fz*d), DOWN, 2.2)` → `splat(g.point+n*0.08, stroke_radius, seed, stretch=(fx,0,fz), stretch_amt=0.9)`；area 走 `Impact`（`pos=g.point, normal=g.normal, radius=stroke_radius, victim None`）。
- `melee`（L199-242）：`half=(heavy?heavy_arc:melee_arc)*DEG*0.5`；`dmg=heavy?heavy_melee:tap_melee`；`o=(x,y+1,z)`；敌 `ei`（跳过 `ei==self`/同队/死亡/`hits.contains`）：`d=hypot(dx,dz)<=reach+hr`；`ey=e.pos.y+e.smooth_y()-a.pos.y`；`ey <= (heavy?1.9:1.55) && ey+h >= (heavy?-0.9:-0.35)`；`ang=atan2(lat,fwd)<=half || (d<hr+0.4 && fwd>-0.15)`；`los(o → (e.x, e.y+smooth_y+min(h*0.6,0.85), e.z))`；`hits.insert`；`remote → HitSend else apply_hit+Hit`。`dev` 一次性（L237-241 `damageArea`）不移植（无 devices 层，记注）。boss（L232-235）不移植。
- `spawn_wave`（L247-258）：`pitch = |S.pitch|<0.3 ? S.pitch*0.4 : clamp(S.pitch,-0.45,0.35)`；`dir=(sin(yaw)*cos p, sin p, cos(yaw)*cos p)`；`pos=(x+sin(yaw)*0.55, y+wave_height*0.52, z+cos(yaw)*0.55)`；`!los((x,pos.y,z),pos) → pos=(x,pos.y,z)`；`r.blade_wave=Some(BladeWave{owner_slot,team,pos,prev=pos,dir,dist:0,t:0,hits:{},paint_acc:0.3,dying:-1,ghost:false})`；`Kit{kind:"blade", data:[r2 pos×3, r3 dir×3]}`（经 runner→match `kit_out`，同 brolly L609 模式）；`Fire{muzzle=pos, dir}`。
- `BladeWave` 步进（`step_wave` L304-372，Match 每帧反序）：`t+=dt`；`dying>=0 → dying-=dt; <=0 → remove`；`step=wave_speed*dt; prev=pos; pos+=dir*step; dist+=step`；`dmg=lerp(wave_damage, far, clamp(dist/wave_range,0,1))`；敌 slab：`along=ex*ux+ez*uz ∈ [-hr, step*hl+hr]`、`|ex*uz-ez*ux| <= wave_width*0.5+hr`、`cy=lerp(prev.y,pos.y,clamp(along/max(1e-4,step*hl),0,1))`、`y0=e.y+smooth_y`、`y0+h >= cy-wave_height*0.55 && y0 <= cy+wave_height*0.45`；`hits` 一次；`!ghost → apply_hit/HitSend`（weaponId 'blade'）。`blockShot/specials`（L347）不移植（无 subs/specials 层，记注）。`world.segment(prev,pos)` 命中 → `pos=hit.point; end_wave(hit.point, hit.normal)`；`paint_acc+=step`，`>=wave_paint_every && 未停` → `raycast(pos, DOWN, wave_height*0.52+1.4)` → `splat(g.point+n*0.08, wave_paint_radius*(0.9+r*0.2), seed, stretch=(ux,0,uz), stretch_amt=0.85)`（2 抽：半径抖动、seed）；`dist>=wave_range && 未停 → end_wave(None)`。
- `end_wave`（L272-295）：`dying=0.14`；`at=Some` → `splat(at+n*0.1, wave_end_radius, seed)` + `Impact`；`None` → `raycast(pos, DOWN, 3)` → `splat(g.point+n*0.1, wave_end_radius*0.85, seed, stretch=dirXZ, stretch_amt=0.6)` + `Impact`。ghost 不涂不伤（`mute` 语义 = `ghost` 守卫，同 LaunchedCanopy）。
- `busy`：blade swing（`WeaponRunner::busy` 追加 `blade.as_ref().is_some_and(|k| k.swing.is_some())`）。
- `move_speed` Blade：见 §1。

## 6. Match 接线（match_.rs）
- 新增 `pub waves: Vec<BladeWave>`（仿 `launched` L230）；runner 循环 drain `blade_wave.take()` → `waves.push`；reset/clear 同 `launched`。
- 步进位置：launched canopy 之后、projectile step 之前（JS `Projectiles.update` L1564 kit tick 环先于丸循环；brolly tick 在前，alphabet 序 brolly<blade）。
- `update` 调用点（L573）传新参数：`&mut self.actors`（扫描用切片 + 本 actor 已由 `a` 借用——实现取 `actors=&mut self.actors` 整片、runner 内以 `i` 索引自身，避免双借；或传 `&mut self.actors[..i]`+`&mut self.actors[i+1..]` 由实施定形，语义不变）、`&mut self.paint`、`t`、`remote`、`self.projectiles.time`。
- `SimEvent::Kit{kind:"blade"}` 已由 `kit_out` 通道落线（Task 30 §4），无新编码。

## 7. net 接线（replication.rs）
- `TickEvent::Kit` 回放（L877-917）：现 `k.kind != "brolly" → continue` 改为匹配 `"blade"`：`data.len()>=6`、owner 为 remote → `m.waves.push(BladeWave{ghost:true, dir=vec3(d[3],d[4],d[5]).normalize(), ...})`（JS `ghost` L267-270：不喷墨不判伤，`dying` 自行走完）。`KitRecord` 的 kind 为 String、data 为 Vec<f32>，无新 wire 类型。
- `F.flick` 旗标（Task 31 已接）自动覆盖 roller/brush 风上窗口；`ch` 槽位覆盖 blade 蓄力（`r.charge=k.charge` 镜像，HUD/姿态用）。

## 8. 测试与门禁
- TR-32.1（固定种子逐值，覆盖带断言）：
  - roller：60 帧滚动扫过固定路径 → 断言 3-splat 组数 = 位移/0.28 触发次数（逐值）、每帧 rng 抽值数 = 3、ink 扣减 `1.1×moved` 逐值、`moved<0.28` 帧不涂不扣、覆盖带新增格数下限断言（带内格被 `Roll` stripe 覆盖）。
  - brush：1-splat 计数与 `moved<0.22` 门、ink `0.32×moved`、覆盖格数下限；`swipe_side` 交替（+1→-1）与 fireSwipe 5 drop 逐值。
  - blade：tap 7 drops 的 delay/pos/vel/radius 逐值（固定种子）、heavy 满充释放 → `Kit` 事件 data 6 值 = r2/r3 量化、wave 步进 12 m 射程 → paint splat 次数 = floor((acc)/0.7) 逐值、覆盖带格数下限、`end_wave` 半径 1.05/0.8925。
- TR-32.2（flick/侧扫与近距边界）：
  - roller flick：windup 0.22 帧精确（flick 0→0.22 释放、cooldown=0.40、firing_t=0.25、flick_recover=0.18）；ink<9 → `_empty`+LowInk、`empty_cd` 0.45 节流；`pressed` 沿（非 held）。
  - brush flick：windup 0.04、`cooldown=swipe_interval`（0.16 全额）；`moving` 三条件边界（hs=2.0/2.01、intent 0.3/0.31）；`fire&&!moving` 出手沿。
  - drop 衰减：`dmg_far` 命中距 0/3.5/7/9 → 125/77.5/30/30（flick 值）逐值；`delay` 丸延迟帧（首帧不步进）。
  - 近距边界：roller `hs>1.0` 门槛（hs=0.9 不判伤）、`fwd∈(-0.2,1.35)` 两端、`lat` 恰界；brush `fwd<1.2`、per-victim 0.35 冷却逐帧；blade melee `d=reach+hr` 恰界、`ang=half` 恰界、贴脸 `d<hr+0.4 && fwd>-0.15` 豁免、LOS 遮挡不判、每 swing 一次；lungeSafe 三采样点（海面/平台）。
  - ghost wave：不涂不伤、12 m 后 `dying=0.14` 移除。
- 门禁：`/tmp/task31-gates.sh` → `/tmp/task32-gates.sh`，日志 `rust/tools/verify/logs/task32-gates-<UTC>.log` 九段全 exit 0；workspace 测试 > 327；JS 树零改动。

# Task 33 实施设计（决策完备，2026-10-31）

## 0. 范围与模式
主武器批四：bow（kits/bow.js `MAIN_KITS.bow` L496-524，526 行）、mitts（kits/mitts.js `MAIN_KITS.mitts` L873-931，934 行）。均为 kit 武器，仿 blade（`weapon/blade.rs`）落 `weapon/bow.rs` + `weapon/mitts.rs`（各配 `*_tests.rs`）。自有弹丸实体（箭/拳套）走 Match 级世界对象列表（仿 `waves`），`['k','bow']`/`['k','mitts']` 复用 `TickEvent::Kit` 通道，无新 wire 类型。bot 驾驶段（bow L451-491、mitts L659-833）与 `predictLeap`/本地预览/telegraph（纯呈现与 bot 规划）不移植 → Task 39（决策 D-T33-1，leap 落点以真实物理接触判定）。audio/fx/rumble/character.trigger 一律不移植（既有范围）。`blockShot`/`specials.shotHit`/`damageArea`/boss 分支不移植（无 subs/specials/boss 层，Task 34/35 接线，记注）。

## 1. bow kit（weapon/bow.rs）
- `BowKit{charging, charge, charge_t, ring:u8, stall, last_c, seen}`（JS `kitOf` L40；`botYaw/botWant/botJump/botJumped/botPaint` 不移植）。`WeaponRunner` 新增 `bow: Option<Box<BowKit>>`，`reset` 置 None（同 brolly/blade）。
- `update`（L389-422）：`fire && cooldown<=0`：非 charging → `ink < ink_full*ink_min → _empty`，否则起抽（charge=0/charge_t=0/ring=0/stall=0/last_c=0）；`max_c=clamp(ink/ink_full,0,1)`；`charge_t=min(1, charge_t+dt/charge_time)`；`charge=min(max_c, charge_t)`；`ring` 阈值仅 emit（L405-416）→ 记 `ring` 字段不产事件；`stall = charge>last_c+1e-4 ? 0 : stall+dt; last_c=charge`；`is_bot && stall>0.45 && bot_mode!=fight → release`（bot 门控，现阶段恒假，保结构）。`else if charging → release`。`seen=ps_time` 每帧置。
- `release`（L423-435）：`charging=false; c=charge; ink=max(0, ink - ink_full*max(ink_min,c)); note_fired; loose_volley(a,c); charge=0; charge_t=0; ring=0; stall=0; firing_t=0.35; cooldown=w.cooldown`。
- `bow_shot(c)`（L46-61）纯函数 → `BowShot{tier,range,speed,dmg,side,fan,lodge,fuse,br,bd,be,bp}`：`c>=0.999`→tier2 全 full 字段（`side=side_full`）；`c>=ring1`→tier1 `u=(c-ring1)/(1-ring1)`，dmg/range/speed 走 `damageRing/flightRing/speedRing` lerp，`side=dmg*side_mul`，fuse/br/bd/be/bp 取 `[0]`；否则 tier0 `u=clamp(c/ring1,0,1)`，`damageTap/flightTap` lerp、speed=speed_tap、`lodge=false`、fuse/br/bd/be/bp=0。
- `loose_volley`（L74-93）：`m=ps.muzzle(a)`、`dir=ps.aim_from(a,m)`、弹道解算复用 Task 31 bucket 的 ballistic（`_ballistic(m,dir,aim_point,speed,straight,grav,0,range)`，JS L78；aim_point 恒 None 时保持原 dir，同 D-T32 注记）；`air=!a.grounded`；Kit 记录 `[r2 m×3, r3 dir×3, r3 c, air?1:0]`（L88，8 值）；`volley()`：轴 `air→normalize(dir×UP)`（lenSq<1e-4→(1,0,0)）否则 UP（L98）；`i=-1..1`：`len>=192 → kill(0)`（L101）；`d=dir 绕轴转 i*fan*DEG`；箭字段 `pos=prev=m; vel=d*speed; dir=fdir=d; st=0; age=0; dist=0; t=0; range/speed/tier/lodge/fuse/br/bd/be/bp 自 S; dmg = i!=0 ? side : dmg; trail=-1.8; seed=rng 抽 1; no_hit=false; center=(i==0); ghost`。
- 箭步进（`step_arrow` L175-264，Match 反序）：`st==1`：`t+=dt`；`t>=fuse → burst + kill`（ticks 仅音频，不移植）。飞行：`age+=dt; prev=pos; age>straight → vel.y-=grav*dt; dist>range → vel.y-=dive_grav*dt; vel*=1-dive_drag*dt; pos+=vel*dt; sp=|vel|; dist+=sp*dt; sp>0.01 → dir=vel/sp`。敌胶囊（`!no_hit`，L197-214）：粗筛 `|dx|>3+hr || |dz|>3+hr → skip`；`segment_capsule_dist(prev,pos,base(e),hr,h)`；`res.dist < hr*0.95+0.11`（ARROW_R L29）→ `at=prev.lerp(pos,res.t)`；`!ghost → apply_hit(owner,e,dmg,'bow')`；`!lodge → kill`；否则 `no_hit=true; fdir=dir; pos=prev=at; vel=(dir.x*1.5,-9,dir.z*1.5); dist=max(dist,range)`。devices/specials/boss（L216-231）不移植。世界 `segment(prev,pos)`（L233-249）：`lodge → stick_dir = no_hit ? normalize((fdir.x*0.55,-1,fdir.z*0.55)) : dir; lodge(箭, hit.point, hit.normal, stick_dir)`；否则 tap：`splat(hit.point+n*0.12, paint_tap*(0.85+0.3*seed), seed, stretch=dir, stretch_amt=0.7)`（0 新抽）+ `Impact{radius:paint_tap, victim:None}`。拖尾滴墨（`lodge && !no_hit`，L252-259）：`trail+=sp*dt; >trail_every → trail=0; raycast(pos,DOWN,4)` → `splat(g.point+n*0.1, trail_radius*(0.8+rng*0.4), seed=rng)`（2 抽 [半径,seed]）。寿终（L260-263）：`age>3 || pos.y < water_y-1.2 → kill`。
- `lodge`（L122-132）：`st=1;t=0;hitP=point;nrm=n;dir=normalize(d);pos=hitP+dir*0.12`；`splat(hitP+n*0.08, paint_stick, seed, stretch=dir, stretch_amt=0.5)`（0 抽）。
- `burst`（L135-162）：`c=hitP+n*0.18`；`area=splat(hitP+n*0.1, bp, seed)` + `i=0..1`：`a=seed*20+i*2.7; r=bp*0.55; pt=(c.x+cos a*r, c.y+0.1, c.z+sin a*r); splat(pt, bp*0.45, seed+i)`（全 0 新抽）；`ghost → 仅 Impact`；溅射：敌 `v=(e.x, clamp(c.y, by+0.25, by+max(0.3,h-0.25)), e.z)`；`d=max(0, dist(v,c)-hr*0.6)`；`d<=br && los(c,v)` → `dmg = d<=burst_inner ? bd : lerp(bd,be,(d-inner)/max(0.01,br-inner))` → `apply_hit('bow')`；`Impact{pos:c, normal:n, radius:br}`。
- 挂起抽弓清扫（tick L514-519）：runner `charging && kind==bow && ps_time-seen>0.15 → 静默取消`（charging=false/charge=0/charge_t=0/ring=0）。
- `move_speed`（L500-504）：`charging → lerp(run_speed*0.75, move_speed_drawing, min(1, charge*2.5))`；`firing_t>0 → move_speed_firing`；否则尾链 `run`。`spread_deg`（L505）→ 0。`busy` 无钩子 → false。
- 幽灵箭（`['k','bow']` 收帧）：`bow_shot(d[6])` 重建 S，`volley(ghost=true)`（dir 归一化 d[3..5]，air=d[7]）；ghost 全程不喷墨不判伤（mute 语义 = `!ghost` 守卫）。

## 2. mitts kit（weapon/mitts.rs）
- `MittsKit{seen, cd, side:i32(初值1), charging, charge, charge_t, charge_air, full, leaping, leap_t, leap_v:Vec3(水平分量), leap_h, slid, trail, trail_p, land_t(初值9), cling, hang, cling_t, cling_n, cling_pt, dry_warn}`（JS `K()` L48-67；`bot*` 与 `land/pred_t` 预测字段不移植 D-T33-1）。`WeaponRunner` 新增 `mitts: Option<Box<MittsKit>>`，reset 置 None。
- `update`（L546-646）顺序：`seen 过期>0.25 → cancel_charge/leaping=false/dropCling('reset')`（L549）；`seen=ps_time`；`cd-=dt; land_t+=dt`；`special_active && cling → dropCling('special')`；`form==squid → cling dropCling('squid')、charging cancel_charge`。
  - 蓄力（L556-579）：`charge_t+=dt`；`max_c=clamp((ink-leap_ink_min)/(leap_ink_max-leap_ink_min),0,1)`；`charge=min(max_c, charge_t/leap_charge_time)`；`r.charge=charge`；`note_fired`；`charge>=0.999 && !full → full=true`；离地：`!grounded && !cling → charge_air+=dt, >0.12 → cancel_charge` 否则清零；释放（本地）：`!intent.jump → release_leap`（bot 分支 `botWant/botReleaseOk` 不移植）。
  - 飞行（L582-612）：`leap_t+=dt`；`grounded → leaping=false, land_t=0, land_splash(a.pos, ground_n.y>0.5 ? ground_n : UP)`；`form!=kid || leap_t>4 → leaping=false`；否则贴墙判定（L591-594）：`!slid && contacts.wall && leap_t>0.03`：`into = -(leap_v·wall_n)`；`into>1.5 && clingable_at(pos, wall_n, wall_block) → start_cling` 否则 `slid=true`；仍 leaping：`!slid && hypot(vel.xz - leap_v.xz)>2.2 → slid`；`!slid → vel.x=leap_v.x; vel.z=leap_v.z`；`note_fired`；拖尾：`trail += dist(pos,trail_p); trail_p=pos; >1.6 → trail=0; raycast((x,y+0.3,z),DOWN,5)` → `splat(g.point+n*0.08, 0.42+rng*0.15, seed=rng)`（2 抽）。telegraph（L415-424）纯呈现不移植。
  - 贴墙（L615-629）：`cling_t+=dt; vel=0; note_fired; ink=max(0, ink-cling_drain*dt)`；`dry_warn` 仅 HUD 不移植；墙存续 `raycast((x,y+1,z), -cling_n, radius+0.5)`；`ink<=0 → dropCling('empty')`；`grounded || !wall_there || !alive → dropCling('lost')`；bot backstop 不移植。
  - 出拳（L632-645）：`!fire||charging||leaping → cd=max(cd,0)`；否则 `firing_t=0.35`；`while cd<=0 && guard<2`：`ink<ink_per_punch → _empty; cd+=punch_interval; break`；`ink-=ink_per_punch; note_fired; side = cling ? 0 : 1-side; fire_fist(side); since_hand[side]=0; cd+=punch_interval`。
- `jump` 钩子（L650-660，见 §4 actor 扩展）：`charging||leaping → true`；`cling`：`fire && !is_bot && ink>=leap_ink_min → start_charge`，`else if !is_bot || !fire → dropCling('jump')`；恒 `true`；否则 `is_bot || !fire || !grounded || form!=kid → false`；`start_charge`。
- `start_charge`（L355-362）：`charging||leaping → true`；`ink<leap_ink_min → _empty, false`；置字段 true。`cancel_charge`（L363-366）：`charging=false; charge=0; charge_t=0; r.charge=0`。
- `release_leap`（L368-390）：`c=charge; charging=false; r.charge=0; ink=max(0, ink-lerp(leap_ink_min,leap_ink_max,c)); note_fired; v=leap_velocity(c); from_wall=cling; from_wall → cling=false/hang=false; leaping=true; leap_t=0; slid=false; leap_v=(v.x,0,v.z); leap_h=hypot(v.x,v.z); trail=0; trail_p=a.pos; a.vel=v; grounded=false; coyote=0; jump_buffer=0`；Kit 记录 `[2, r2 pos×3, r2 c, from_wall?r2 cling_n.x:0, from_wall?r2 cling_n.z:0, from_wall?1:0]`（L385，8 值）；`!from_wall → splat((x,y+0.25,z), 0.9, seed=rng)`（1 抽）。
- `leap_velocity`（L275-287）：`ang=clamp(leap_angle*DEG + aim_pitch*leap_pitch_k, leap_angle_min*DEG, leap_angle_max*DEG)`；`v=sqrt(lerp(speed_min², speed_max², clamp(c,0,1)))`；`vy=min(leap_vy_max, v*sin ang); vh=v*cos ang`；`dx=sin(aim_yaw), dz=cos(aim_yaw)`；`cling` 出墙守卫：`dn=dx·n.x+dz·n.z; dn<0.35 → (dx,dz)+=n*(0.35-dn) 归一`；`out=(dx*vh, vy, dz*vh)`。
- `land_splash`（L426-461）：`land_invuln>0 → a.invuln=max(a.invuln, land_invuln)`；手套砸（L430-436）：敌 `hypot(dx,dz) <= glove_radius+hr && |e.y+smooth_y+0.7-at.y|<1.6 → apply_hit(glove_damage,'mitts')`；`c=at+n*0.35`；`area=splat(c, land_paint, seed=rng)` + 6 环 splat：`ang=(i/6)*TAU+rng*0.5; rr=land_paint*(0.75+rng*0.35); 基向量 n.y>0.7 ? xa=(1,0,0) : xa=normalize((-n.z,0,n.x)); ya=n×xa; pt=c+xa*cos(ang)*rr+ya*sin(ang)*rr; splat(pt, 0.55+rng*0.35, seed=rng)`（每环 4 抽 [ang,rr,radius,seed]，共 25 抽/次）；溅射：敌 `v2=e.pos+smooth_y+(squid?0.3:0.7)`；`d=dist(v2,c)<=land_radius && los(c,v2)` → `dmg=lerp(land_damage_max, land_damage_min, clamp((d-0.6)/(land_radius-0.6),0,1))` → `apply_hit`；Kit 记录 `[1, r2 at×3, r2 n×3]`（7 值）；`Impact{pos:c, normal:n, radius:land_radius}`。
- `start_cling`（L463-477）：`cling=true; hang=true; leaping=false; cling_t=0; dry_warn=0; cling_n=flatten(n) 归一; cling_pt=a.pos - cling_n*PLAYER.radius; cling_pt.y=a.pos.y+1.0; a.vel=0; a.grounded=false; land_t=0; land_splash(cling_pt, cling_n)`。
- `drop_cling`（L479-490）：`!cling → return`；`cling=false; hang=false`；`why=="jump" → vel=(n.x*3.4, 4.6, n.z*3.4)`；`why!="squid" → vel=(n.x*1.2, 0.5, n.z*1.2)`；`grounded=false`。
- `clingable_at`（L345-353）：block `rail||grate → false`（Rust layout blocks 若无该旗标则视为可贴，记注）；`raycast((x,y+1.35,z), (-n.x,0,-n.z), radius+0.45)` 命中且 `|normal.y|<=0.5`；`raycast((x,y+0.2,z), DOWN, 0.5)` 不命中。
- 拳套（`MittsFist`，Match 级列表）：`fire_fist`（L137-146）：muzzle = `glove_muzzle`（L114-125 无 character → 直接 fallback：`base=(x,y+1.05,z)`；`out=base+aim_dir*0.35+(cos(yaw)*0.16*sx, 0, -sin(yaw)*0.16*sx)`，`sx=hand?1:-1`；`!los(base,out) → out=base`）；`dir=ps.aim_from(a,m); ps.spread(dir, punch_spread)`；`spawn_fist`：`life=fist_range/fist_speed; seed=rng 抽 1; trail=-0.8; ghost=false`（`spin` 纯呈现不抽，同 Task 29 vis 规则）；Kit 记录 `[0, r2 m×3, r3 dir×3, hand]`（8 值）。
- 步进（`step_fist` L197-246）：`age+=dt; prev=pos; pos+=vel*dt`（直射无重力）；敌胶囊阈值 `hr*0.95+fist_size` → `at=lerp`；`!ghost → apply_hit(punch_damage)`；`burst_fist(at, direct=e, normal=None)`；世界 `segment` → `burst_fist(hit.point, None, normal)`；`age>=life → burst_fist(pos, None, None)`；`pos.y<water_y-1.5 → 直接死（不爆）`；拖尾 `trail+=fist_speed*dt; >1.5 → 0; raycast(pos,DOWN,3)` → `splat(g.point+n*0.08, 0.28+rng*0.1, seed=rng)`（2 抽）。
- `burst_fist`（L152-174）：`ghost → return`（不涂不伤）；`normal → splat(at+n*0.12, fist_paint*(0.85+rng*0.3), seed, stretch=f.dir, stretch_amt=0.45)`（1 抽）；否则 `raycast((at.x,at.y+0.2,at.z), DOWN, 3.4)` → `splat(g.point+n*0.1, fist_paint*0.85, seed)`（0 抽）；溅射：敌 `e!=direct`：`v2=e.pos+smooth_y+(squid?0.3:0.7)`；`d<=splash_radius && los` → `lerp(splash_max, splash_min, clamp(d/splash_radius,0,1))` → `apply_hit('mitts')`；`Impact{pos:c, normal:normal.unwrap_or(UP), radius:splash_radius, victim:direct}`。
- 钩子：`busy`（L879）= `leaping`；`move_speed`（L905-912）：`leaping → max(leap_h, air_min_speed)`；`charging → is_bot ? 0.05 : move_speed_charging`；`land_t<0.25 → move_speed_charging*2`；`firing_t>0 → move_speed_firing`；否则 `run`；`spread_deg`（L913）= `punch_spread`；`firing_pose` 纯呈现不移植。
- `damage_taken`（L883-903，接 actor 伤害入口）：`leaping || land_t < leap_armor_grace → amount*leap_armor`；guard：`!cling && form==kid && attacker 有效 && firing_t > 0.35-guard_after`：`dot=(sin(aim_yaw)*dx+cos(aim_yaw)*dz)/l > cos(guard_arc*DEG) → amount*guard_armor`；否则原值。
- netState/netApply（L840-862）**不接 tick 槽**：M3 kit 非目标，`ks`(s[21]) 维持仅 brolly 实值（spec Non-Goals 修订只开了 brolly 例外）；远端 mitts 无 runner pass（match 跳 remote）。`['k','mitts']` 收帧：case 0 → ghost 拳套（飞+爆仅呈现，不涂不伤）；case 1/2 → 解码容忍（无 sim 态）。`['k','bow']` → ghost 三连箭。

## 3. 共享基建改动（weapon.rs）
- `WeaponRunner`：`bow: Option<Box<BowKit>>`、`mitts: Option<Box<MittsKit>>`；`update` dispatch `Bow => bow::update(...)`、`Mitts => mitts::update(...)`（JS L128 default→MAIN_KITS）；`is_wired` 增 Bow/Mitts；`move_speed`/`spread_deg`/`busy` 按 §1/§2 替换 L899/L1074 空臂；`reset` 清两 kit。
- rng 消耗序（rng_draws 锁）：bow  volley 3 抽（i=-1,0,1 各 seed）→ 箭步进各路径如上；mitts  拳套 1 抽/发、land_splash 25 抽/次、release_leap 推地 1 抽、飞行拖尾 2 抽、拳 burst 1 抽、拳拖尾 2 抽。

## 4. actor 扩展（OQ-2 定案，actor.rs）
- `Actor::step` 增参 `mitts: Option<&mut MittsKit>`（match_ 以 `actors[i]` 与 `runners[i].mitts` 分列借用传入；非 mitts 武器恒 None）。
- jump 声明（JS actor.js L311-313）：在 jump_buffer 更新/`jump_swallowed` 处理之后、跳跃块（L616）之前：`jump_buffer>0 && form!=squid && mitts 声明（MittsKit::jump 语义，读 intent.fire/grounded/coyote 等）→ jump_buffer=0`。顺序在 twins dodge（`jump_swallowed`）之后，与 JS L310→L313 一致。
- hang 冻结（JS L502-507）：对应 Rust 地面跟随段入口前：`!special_active && form==kid && mitts.hang → vel=0; grounded=false; air_time=0; 跳过地面跟随/roof/rail 段`。
- `release_leap`/`drop_cling` 直接改 `a.vel/grounded/coyote/jump_buffer/invuln`（经 step 参数或 match_ 侧回写，实施取 step 传 `&mut MittsKit` + kit 更新期 match 已持 actors 切片的既有范式）。
- 回归：既有移动/游泳/重生测试不破（TR-33.3）。

## 5. Match 接线（match_.rs）
- 新增 `pub arrows: Vec<BowArrow>`、`pub fists: Vec<MittsFist>`（仿 `waves`）：runner 循环 drain kit 产出；步进位置 = `waves` 之后（JS kit tick 序：brolly→blade→bow→mitts 按注册字母序，L1564 单环）；reset/clear 同 `waves`。
- runner pass 传参扩 §1/§2 所需（actors 切片、paint、tuning、remote、ps_time、kit_out）；remote actor 维持直接 continue 不跑 runner pass（Task 32 P1 结论）。
- `SimEvent::Kit{kind:"bow"/"mitts"}` → `kit_out` → `TickEvent::Kit`（local-only 通道，Task 30/32 既有），无新编码。

## 6. net 接线（replication.rs）
- `TickEvent::Kit` 回放路由：`"bow"` → `data.len()>=8` → ghost 三连箭入 `arrows`；`"mitts"` → `d[0]==0 && len>=8` → ghost 拳套入 `fists`；`d[0]==1/2` → 忽略；`"blade"/"brolly"` 维持。ghost 实体不涂不伤（`!ghost` 守卫）。
- `ks` 槽不扩（仅 brolly，M3 决议不变）；`ch` 槽（s[14]）经 `r.charge` 镜像自动覆盖 bow 抽弓/mitts 蓄力（`from_actor` 既有逻辑）。

## 7. 测试与门禁
- TR-33.1（leap 弹道与 land 溅射逐值）：`leap_velocity` c=0/1 端点（sqrt 线性于 v²）、角度钳制（pitch 极值→min/max 恰界）、`vy` 封顶 13、贴墙出墙守卫（dn=0.3/0.4 边界）；`land_splash` 伤害 `d=0.6→100 / d=2.8→35 / 恰界/LOS 遮挡` 逐值、glove 砸 `reach=1.1+hr` 恰界 + `|dy|<1.6`、`invuln` 置值；bow `bow_shot` 三档边界（c=0.449/0.45/0.998/0.999）全字段逐值、ink 扣减 `16*max(0.15,c)`、fan 几何（地面绕 UP / 空中绕 dir×UP，±fan 角恰值）、三连 rng 3 抽与 dmg center/side。
- TR-33.2（cling 进/出与 drain 时序）：贴墙进入（leap 撞可贴墙 `into>1.5` + clingable 三判据）→ `land_splash(cling_pt)` 时序、`hang` 冻结（vel 恒 0、不落地）；退出四因：jump（`vel=(n.x*3.4,4.6,n.z*3.4)`）、墨尽（`ink<=0` 帧精确、drain `8*dt` 逐帧）、squid（不变速直接落）、lost（墙消失）；drain 与出拳扣墨同帧序（先 cling 后 punch）；jump 钩子吞按（charging/leaping 中按 jump 不产生跳跃；地面 fire+jump 起蓄不跳；裸 jump 正常跳）。
- TR-33.3：actor 既有移动/游泳/重生测试全绿（签名扩展后调用点更新不破语义）。
- 补充固定种子用例：bow 抽弓→释放全流程（charge_t 线性、tank 封顶 max_c、ring1 边界、cooldown 0.22、firing_t 0.35）；箭 lodging→burst fuse 0.7/0.55、tap 不 lodge 直接 splat、dive 段（dist>range 后 diveGrav/diveDrag）；mitts 出拳节奏（0.12s/发、guard<2 补发、side 交替、cling 时恒 0 手）、拳射程 4.6/28 寿终空中爆、splash `lerp(22,10,d/1.4)`+LOS+direct 排除、guard 减伤弧（±70° 内 ×0.5、外原值、leap armor ×0.35 + grace 0.15）。
- 门禁：`/tmp/task32-gates.sh` → `/tmp/task33-gates.sh`，日志 `rust/tools/verify/logs/task33-gates-<UTC>.log` 九段全 exit 0；workspace 测试 > 345；JS 树零改动。
