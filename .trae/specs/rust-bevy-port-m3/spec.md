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
- **kit 系统**：`['k']` 事件（recKit L134-137 / case L486）、s[21] `ks` 槽维持恒 0；Rust 收帧**解码容忍**（忽略不报错），不产生。
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
