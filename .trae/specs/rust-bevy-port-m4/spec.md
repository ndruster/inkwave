# INKWAVE Rust/Bevy 移植 — M4 Boss 战与表现完整度产品需求文档（PRD）

> 前置：M1（`.trae/specs/rust-bevy-port/`，Task 1–16）、M2（`rust-bevy-port-m2/`，Task 17–27）、M3（`rust-bevy-port-m3/`，Task 28–40）已全部完成并 push origin/main（基线 workspace **554 passed**）。本文档只描述 M4 增量；既有架构/同步机制/网络栈/内容集全部继续有效。

## M4 决议（2026-11-03 与用户确认：玩法优先）
- **主线 = Boss 战全量玩法 + bot 深化 + 全图 + 表现完整度**，按段推进：
  - **Phase A（Tasks 41–46）Boss 战**：`src/boss/` sim 段（boss/bossBrain/bossNav/bossHazards/bossMode ≈ 1800 行）+ 联网语义（`B` 快照 / `'bhit'` / `['bm']` / `['bc']` / res boss 形态 / host 迁移 adopt/handoff）+ 离线完整局（you+7 bots）+ Rust↔Rust e2e + JS↔Rust scenario E + 呈现占位档（体块剪影/telegraph 标记/HUD boss 条）。
  - **Phase B（Tasks 47–48）玩法收尾**：bot threat AI 与深战术顺延项；剩余地图提取（halyard/kelpline/lockgate/terraces/cargo）+ MAPS 旗标语义（onlineOnly/noBots/noBoss）。
  - **Phase C（Tasks 49–56）表现完整度**：小地图、音频过程化、fx 系统、ink 着色与地表材质、props/murals 装饰、角色完整绑定、环境与后处理、showcase 大厅。**沙箱无 GPU：Phase C 验收 = 无头可测部分（数据/几何/加载不 panic）全绿 + 视觉/听感验收登记 REVIEW_CHECKLIST 待用户机器复核**。
- **Boss 战 = 第四模式**：session/match/lobby 放开 `mode:'boss'`（M3 起 `['bm']/['bc']` 解码容忍已备，Task 40 显式测试锁形）。
- **Task 57 = M4 收尾**（台账/PORT_MAP/README/门禁矩阵/总评审/提交 push）。

## Overview
- **Summary**: 把 Rust 版补齐到与 JS 上游功能对等：HULLBREAKER Boss 战（离线+联网+跨端）、bot threat AI、全部 8 张图，以及 M1 起整体顺延的表现完整度（角色弹簧姿态绑定、fx 粒子系统、过程化音频+音乐、ink 湿润/光泽、黄昏环境、后处理链、props/murals 装饰、小地图、showcase 大厅）。
- **Purpose**: M3 后玩法内容已对等（12/15/19 武器 + turf/zones + 3 图 + 三档 bot），剩余缺口 = Boss 模式 + threat AI + 5 图 + 表现层。表现层是"移植完成"的最后一大块，但其验收依赖真实 GPU——故玩法优先收口硬风险，表现层逐项落地并如实登记复核项。
- **Target Users**: 仓库维护者；最终验收口径 = Rust 客户端可玩 JS 上游的全部内容（含 Boss 战），画面/声音达到可玩完整度。

## Goals
- **G-1 Boss 战 sim 对等**：`Boss`（boss.js L497：状态 pos/yaw/vel/hp/phase/move/moveT/movePhase/target/stunned/dead/seed、boss 时钟 bt、pack/unpack L422/L432）、`BossBrain`（376：move 选择与 paceDiff 节奏、phases [0.66,0.33]）、`BossNav`（260：nav 限制/出生点 no-go/charge 撞墙→stun）、`BossHazards`（600：slam 环/barrage 桶/sweep 光束/charge/ crablets/frenzy，几何由 move record `{id,t0,seed,params}`+时间求值，全端一致）、`BossMode`（55：squad 8/HP 缩放/胜负/finishWin 5.2/finishLose 3.2）逐行平移；数值全走 tuning 提取。
- **G-2 Boss 联网对等**：host tick `msg.B`（netmatch L195-196，`bossN%10===0` full）、`recBoss(['bm'|'bc'])` 事件时间线（L146/L494/L496）、guest `'bhit'` shooter-authoritative（L150-152/L215）、guest 播放 `_sampleBoss`（L504：与远端 actor 同一 playback 时钟 Hermite + 关键 k/boss 段）、res boss 形态（L652-661：`bo`/st 行 `bossDmg`/`weakHits`/boss 胜停钟）、host 迁移 `adopt()/handoff()`（L682-683）——全部并入金标准 fixture 与 Rust↔Rust e2e。
- **G-3 bot 深化**：threat AI（bots.js L1288+ turf 段）+ `_bossTick` 段（L1874-2050：`hz.threat()` 感知/evade/goal/crablet pop/清 boss 墨）+ PORT_MAP 顺延项（stamp threat-steer L460-470、`_shieldFight`/`_canopyCtl`、mitts/blade 深战术）；难度单调性延续。
- **G-4 全图**：halyard（maps.js L30 内联）+ kelpline/lockgate/terraces/cargo 走既有 stage CLI 提取；MAPS 旗标（config.js L524-539）进 tuning，lobby 按旗标过滤（offline 不列 onlineOnly；boss 房 noBoss 自动换图 `bossFallbackMap`；noBots 强制关补位）。
- **G-5 表现完整度**：小地图（minimap.js 585）、音频（audio.js 2505 + music.js 1490 + bossAudio.js 177）、fx（fx.js 2267 + screenfx 1068 + fxHooks 712 + swimWake 107 + zoneMarks 517，含 M3 顺延的 flood 呈现层）、ink 着色（inkShading 314 GLSL→wgpu）+ 地表材质（levelMaterial 719 + texlib 1424）、props/murals/dressing（tidewater props 2093 等）、角色弹簧姿态绑定（character.js 4062 + character-geo/hair/face/mats/lod/style/weapons；locker 更衣室除外）、环境与后处理（environment 3116 黄昏/海况 + renderer 218 GTAO/bloom/grade/SMAA/tilt-shift）、showcase 大厅（showcase 3109 + lobbySet*）。
- **G-6 门禁与纪律延续**：测试基线自 554 单调递增；JS 树/`server/` 零改动；`cargo tree -p inkwave_sim` 依赖面不变；九段门禁每任务全绿（TRUNK 段含 SIGKILL 假绿断言，D-T40-1）。

## Non-Goals（M4 明确不做）
- **Boss 高保真模型**：bossModelGeo/Anim/Mats/Fx（≈2400 行，100-bone 蒙皮/≤150k tri 预算/5 材质+stencil atlas）与 locker 更衣室（character-outfit 相关）——Boss 呈现为体块剪影 + 程序化几何简化档（轮廓可辨识：集装箱壳/双钳/眼柄；telegraph 环/束/圈标记）；完整模型顺延 M5。
- **手柄/移动端/发布部署**：延续不做。
- **retired 武器、存档兼容**：无包袱。
- 不改 `server/`、不改 `src/`、`tools/`、`docs/` 上游树（fixture 扩展脚本仍放 `rust/tools/`）。
- `_zoneJump`（依赖 superJump，上游亦裁剪）、climb 边（M1 已声明偏差）维持 follow-up。

## Background & Context（M4 调研结论，2026-11-03 实核）
- **Boss 模块体量**：boss.js 497 / bossBrain 376 / bossNav 260 / bossHazards 600 / bossMode 55（sim 段 ≈1800）；bossModel 242 / bossModelGeo 875 / bossAnim 859 / bossMats 567 / bossModelFx 143（模型段，Non-Goal）；bossAudio 177（Phase C Task 50 消费）。
- **BOSS 常量**（boss.js L16-27，实读）：`hpPerUnit 18000`（×squad units×hpDiff）、`hpDiff{easy .75,normal 1,hard 1.3}`、`dmgDiff{.7,1,1.2}`、`paceDiff{1.25,1,.85}`、`weak 2.5`/`stunned 1.25` 倍率、`weapon` 面积武器倍率表（roller .3/slam .6/bomb/sticky/seeker/burst/shaker/waddle/torpedo .75）、`phases [.66,.33]`、`introAt 1.8/introLen 3.4/roarLen 1.9`。BOSS_MODE（bossMode.js L8-13）：`duration 240/intro 7.2/finishWin 5.2/finishLose 3.2/squad 8`；`squadUnits`（L15：human 1.0/bot 0.6）。
- **六招 moves**（docs/BOSS.md 表）：slam（claw 升→冲击环）/barrage（3-6 桶，目标圈先行）/sweep（眼炮蓄→扇形光束）/charge（锁向→直线冲→撞墙 stun）/crablets（phase≥2，3-5 小蟹追人）/frenzy（phase3 旋喷 3s）；telegraph ≥0.6s 可读；phase 切换 roar+短无敌（never mid-move）。
- **hitShapes 契约**（docs/BOSS.md L99-102）：body `shellF/shellR`（r 1.8）/`body`/`clawL`/`clawR`；weak `eyeL/eyeR`（always）、`belly`（stunned/phase3/发光）、`crackL/crackR`（phase3）；weak 先于 body 测；dead/入地 intro 期全 inactive。**JS 侧 hitShapes 挂在模型上（`m.hitShapes`）——Rust sim 必须把球表几何脱离渲染模型进 sim（数值走提取/锚定，禁手抄）**。
- **联网路径**（netmatch.js 实核行号）：`recBoss` L146（host only）、`sendBossHit` L150（guest→host，`boss.log.sent/sentDmg` 对账）、tick `msg.B` L195-196、`case 'bhit'` L215（host `remoteHit(d)`）、guest 收 `d.B` L258-261（`Boss.unpack(d.B,d.ts)`→`boss.net.buf`）、`case 'bm'` L494（`boss.onMove(e[2])`）、`case 'bc'` L496（`_crabBurst`）、`_sampleBoss` L504+（Hermite 播放、>12m 视为断线滑移 L552 注）、res L652-661、迁移 L682-683。session：`mode==='boss'` 放行 L244、`mapBossOk/bossFallbackMap` L245-248、squad 建队 L274-292（team 0 全员、8 槽）。match：boss 分支 L20-21/L55-62/L100/L193/L216/L229/L262。
- **bots boss 段**：`G.boss` 短路 L345 → `_bossTick` L1874+（think 0.12-0.22s、`boss.hz.threat()`、evade 0.45s、`_bossGoal`/stunned rush）。
- **MAPS 旗标**（config.js L524-539）：cargo=`onlineOnly+noBots+noBoss`；`mapNoBots/mapBossOk/mapOfflineOk/bossFallbackMap/noBotsStartBlock` 五函数；`OFFLINE_MAPS` 过滤。
- **新图体量**（4 文件合计）：kelpline 2528 / lockgate 2519 / terraces 3017 / cargo 2530；halyard 内联 maps.js L30。提取管线已泛化（Task 37 stage CLI）。
- **Rust 现状**：`['bm']/['bc']` 解码容忍 + 显式测试（Task 40 `boss_and_unknown_record_tags_decode_to_none_without_panic`）；session `mode` 三值已建模但 boss 拒（Task 36 守卫）；tick 槽无 `B` 字段（wire 需扩）；res 无 boss 形态；HUD 无 boss 条；audio/fx/character 为 M1 占位。
- **沙箱约束**：无 GPU（SwiftShader PBR 崩溃，M3 实证）；`wasm-bindgen` 峰值 ~3.9GB vs avail ~4.0GB（trunk 需重试 + SIGKILL 断言）。Phase C 的视觉/听感项一律"无头可测部分 + 用户机器复核"双轨验收。

## Functional Requirements

### Boss 战（Phase A）
- **FR-M4-1 Boss tuning 提取**：`extract-tuning.mjs` 扩抓 `BOSS`（boss.js L16-27 全字段）/`BOSS_MODE`（bossMode.js L8-13）/moves 时序与 hazard 参数（bossBrain/bossHazards 实读后定字段集）/hitShapes 球表（socket/r/weak/active 条件）入 `tuning.json` schema v3；`tuning.rs` 类型 + 锚定单测锁值。
- **FR-M4-2 Boss wire**：fixture 扩展捕获 `B` 快照（full/delta 两态）、`'bhit'`、`['bm']` move record、`['bc']` burst、res boss 形态；`wire.rs` 编解码逐字节断言；tick 消息 `B` 字段并入 PROTO v1 兼容面（JS 客户端已在用，Rust 对齐即可，不改协议）。
- **FR-M4-3 Boss sim 核心**：`inkwave_sim::boss::{Boss, BossNav}`——状态机/boss 时钟 bt/phase 转换（含 roar 无敌拍）/hitShapes 查询（sim 侧几何）/`damageTaken`（weak/stunned/weapon 倍率）/`remoteHit`/`adopt`/`handoff`/`pack`/`unpack`。
- **FR-M4-4 BossBrain + Hazards**：move 选择（paceDiff/phase 门）、`{id,t0,seed,params}` 确定性 record、tele/act/rec 时间线、六招 hazard 几何求值（环/桶/束/圈/蟹/frenzy）、host-only splat（team1 墨）、victim-side 伤害应用、crablet 实体（`['bc']` 语义）；`onMove` guest 重放。
- **FR-M4-5 BossMode 规则 + 离线集成**：squad 建队（8 units/HP 缩放）、胜负判定（hp→0 / 时钟尽）、finishWin/finishLose、result boss 形态、`match` boss 分支、session 放开 `mode:'boss'`（含 mapBossOk/bossFallbackMap/noBots 旗标）、离线 Play 模式选择、lobby MODE 三值（turf/zones/boss）。
- **FR-M4-6 Boss 联网**：host tick `B` 采样发布、guest `_sampleBoss` 播放（Hermite + 同 playback 时钟）、`'bhit'` 上行 + host 应用 + log 对账、`['bm']/['bc']` 时间线重放、host 迁移路径；net-harness Rust↔Rust 完整 boss 局；net-smoke scenario E（JS↔Rust boss 局，沙箱 DEGRADED 口径同 AC-M3-4）。
- **FR-M4-7 Boss 呈现占位档**：体块剪影（壳/双钳/眼柄/步行腿）+ move telegraph 标记（环/圈/线/束）+ crablet 体块 + HUD boss 条（name/HP/phase notches/stun OPEN 提示）+ 结算 boss 形态（VICTORY/DEFEAT/bossDmg/weakHits）+ intro title card；bossAudio 占位。

### 玩法收尾（Phase B）
- **FR-M4-8 bot 深化**：threat AI（L1288+）+ `_bossTick` 段平移（hz.threat 消费/evade/goal/crablet/清墨）+ stamp threat-steer/`_shieldFight`/`_canopyCtl`/mitts·blade 深战术；固定种子行为测试 + 难度单调性延续。
- **FR-M4-9 全图**：halyard/kelpline/lockgate/terraces/cargo 提取（layout+碰撞+出生点+zones 多边形+surfaces 槽位）；MAPS 旗标入 tuning；lobby/stage 选择按旗标过滤；无头碰撞冒烟（可行走/落地/zones 合并）。

### 表现完整度（Phase C）
- **FR-M4-10 小地图**：`inkwave::ui::minimap`——turf 覆盖率网格渲染（对 paint 快照采样）、队友/敌方点、zone 标记、boss 位置；HUD 集成。
- **FR-M4-11 音频**：`audio.js` 过程化合成平移（WebAudio 节点图→bevy 程序化样本/实时合成，选型 spec 阶段定）；`music.js` 曲目状态机；`bossAudio.js` boss 音效；SFX 触发事件面 ≡ JS 事件总线订阅点。
- **FR-M4-12 fx 系统**：`fx.js` 粒子/喷墨团/命中特效 + `screenfx`（受击方向/击杀闪）+ `swimWake` + `zoneMarks`（M3 顺延 flood 呈现层）→ bevy 渲染实体；sim 事件流（SimEvent/TickEvent）为唯一驱动源。
- **FR-M4-13 ink 与地表**：`inkShading` GLSL→wgpu（高度/光泽/湿润）；`levelMaterial` pattern 全量 + `texlib` 程序化纹理；材质参数走提取。
- **FR-M4-14 props/murals/dressing**：tidewater props 2093 + decor/murals/dressing/mapThumb 平移为 bevy 几何（碰撞无关装饰档）；新图 props 同批。
- **FR-M4-15 角色绑定**：`character.js` 弹簧姿态系统平移（squid/kid 形态切换、发/脸/装体块→程序化绑定）；`character-lod/mats/style/weapons` 消费端；locker 更衣室除外。
- **FR-M4-16 环境与后处理**：`environment.js` 黄昏主题/海况/灯光；`renderer.js` 后处理链（GTAO/bloom/grade/SMAA/tilt-shift diorama）；质量档位系统（M1 仅 K 阴影开关→多档）。
- **FR-M4-17 showcase 大厅**：`showcase.js`+`lobbySet*` 3D 大厅场景（角色展示台/loadout 联动）；依赖 G-5 角色绑定与 props。

## Non-Functional Requirements
- **NFR-M4-1 sim 纯净性**：Boss/threat AI 逻辑全在 `inkwave_sim`；依赖面仍仅 glam/serde/serde_json；Phase C 全在 `inkwave` crate（渲染/音频/UI），sim 零改动。
- **NFR-M4-2 确定性**：Boss moves/hazards/crablets 固定种子无头可测；`B` 快照 pack→unpack 往返一致；boss 局 Rust↔Rust 逐 tick 对账。
- **NFR-M4-3 带宽**：boss 局 tick 增 `B` 字段后仍远低于 relay RATE 90/s 与 MSG_MAX 64KB（压测断言）。
- **NFR-M4-4 门禁**：九段矩阵每任务全绿；测试总数 >554 单调递增；JS 树/`server/` 零改动；TRUNK 段 SIGKILL 假绿断言（D-T40-1）。
- **NFR-M4-5 表现层双轨验收**：无头可测（几何求值/加载不 panic/wasm 冒烟/数据对账）沙箱闭环；视觉/听感人工验收登记 REVIEW_CHECKLIST 待用户 GPU 机器复核。

## Acceptance Criteria

### AC-M4-1: Boss 离线完整局
- **Type**: `rule`
- **Given**: 离线 `mode:'boss'`（you + 7 bots，tidewater）
- **When**: 无头固定种子跑完整局：win（hp→0）与 lose（时钟尽）两分支各可达；phase 转换/六招 moves/hazard 伤害/hitShapes/结算形态逐项断言
- **Then**: 全部断言绿；数值逐字段对 tuning v3 锚定
- **Evidence**: `cargo test -p inkwave_sim` + harness 日志

### AC-M4-2: Boss 联网与跨端
- **Type**: `rule`
- **Given**: fixture 金标准（B/bhit/bm/bc/res-boss）+ shim relay + net-harness
- **When**: Rust↔Rust 完整 boss 局（guest `'bhit'` 上行、`B` 播放、迁移 adopt/handoff）；JS↔Rust scenario E 实跑
- **Then**: wire 逐字节一致；boss 状态逐 tick 对账；host 迁移无缝；scenario E 沙箱按 DEGRADED 口径如实记录（Pass Condition 同 AC-M3-4，用户机器复核）
- **Evidence**: fixture 测试 + harness e2e + net-smoke JSON dump

### AC-M4-3: bot 深化与全图
- **Type**: `rule`
- **Given**: threat AI/boss bot 固定种子测试 + 五图提取产物
- **When**: 行为断言（threat 响应/evade/crablet pop/清墨）；五图 stable-json 连跑逐字节一致 + 碰撞冒烟 + 旗标过滤
- **Then**: 全绿；难度单调性不破
- **Evidence**: 测试输出 + assets/maps diff

### AC-M4-4: 表现完整度双轨
- **Type**: `joint`
- **Given**: Phase C 各任务的无头可测断言 + wasm 加载冒烟
- **When**: 几何/数据/事件面逐项对账；`trunk build --release` + wasm-smoke 无 panic
- **Then**: 沙箱可测部分全绿；视觉/听感项全部登记 REVIEW_CHECKLIST 复核条目（含具体复核命令与期望）
- **Evidence**: 测试输出 + 门禁日志 + REVIEW_CHECKLIST

### AC-M4-5: 门禁与纪律延续
- **Type**: `rule`
- **Given**: 九段门禁矩阵
- **When**: 每任务收尾全跑
- **Then**: 全绿；测试数 >554 单调；JS 树/`server/` 零改动；sim 依赖面不变
- **Evidence**: `rust/tools/verify/logs/taskNN-gates-*.log`
