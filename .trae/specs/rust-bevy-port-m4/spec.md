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

---

## Task 41 实施设计（2026-11-03，实读锚点后定稿）

### §0 目标
tuning v3（boss 全数据表入产物）+ wire 金标准（B/bhit/bm/bc/res-boss 编解码）。不动 sim 逻辑。

### §1 JS 锚点（本窗口实读）
- **boss.js L15-25** `export const BOSS`：hpPerUnit 18000、hpDiff{easy .75,normal 1,hard 1.3}、dmgDiff{.7,1,1.2}、paceDiff{1.25,1,.85}、weak 2.5/stunned 1.25、weapon 倍率表{roller .3,slam .6,bomb/sticky/seeker/burst/shaker/waddle/torpedo .75}（L22 实读）、phases [.66,.33]、intro 1.8/introLen 3.4/roarLen 1.9。
- **boss.js L422-431 pack(full)**：`[r3 bt, r2 x, r2 y, r2 z, r3 yaw, r2 vx, r2 vz, round(hp*10)/10, phase, f, move?t0:-1, anim, r2 turn, flashN, weakN, killer, crabs, round(maxHp)]`；f 位：1 stunned/2 dead/4 invuln/8 visible（L424）；crabs 每只 `[id, r2 x, r2 y, r2 z, r2 yaw, round(hp)]`（L426）；full&&move 时 s[18]=move。
- **boss.js L432-434 static unpack(s, ts)**：`{t:ts, bt..maxHp: s[0..17], move: s[18]||null}`。
- **bossMode.js L8-13** `export const BOSS_MODE`：duration 240/intro 7.2/finishWin 5.2/finishLose 3.2/squad 8；L15 squadUnits{human 1.0, bot 0.6}。
- **bossHazards.js L17-24** `export const MOVES`（6 招 tele/act/rec，L18-23 实读：slam{1.15,1.9,1.0}/barrage{.85,0,.8}/sweep{1.15,2.1,.9}/charge{1.15,0,.9}/crablets{.9,.8,.6}/frenzy{1.2,3.0,1.7}——**招名是 sweep/frenzy，非 beam/spin**；HZ 内 beam* 参数为 sweep 的光束几何）；L25 PACE [1,1,.88,.78]；L26 MIN_TELE .7；L27-34 HZ（slamR 3.3/slamDmg 48/ringSpeed 9.5/ringR0 2.4/ringMax 19/ringW .55/ringH .5/ringDmg 32/barrelR 2.4/barrelDmg 52/barrelFlight 1.3/barrelGap .26/beamLen 17/beamW .95/beamDps 170/chargeSpeed[0,13,14.5,16]/chargeRamp .35/chargeDmg 55/chargeMin 6/stun[0,4.2,3.7,3.3]/**frenzyR 8.8/frenzyDps 52**/crabR 2.1/crabDmg 40/crabHp 30/crabLife 12/crabSpeed[0,4.4,4.4,5.2]）；L35-36 SAFE_PAD .6/REACH_N 96（private，切源；tuning 节内命名 reachStep/reachN）。
- **bossModel.js L14-18** `const HIT`（未导出，需切源）：10 球 `['shellF',1.8,false],['shellR',1.8,false],['body',1.05,false],['clawL',.85,false],['clawR',.55,false],['eyeL',.4,true],['eyeR',.4,true],['belly',.8,true],['crackL',.7,true],['crackR',.7,true]`。
- **bossBrain.js L12-15**（private，切源）：SPEED[0,2.9,3.3,3.8]/TURN[0,1.25,1.45,1.75]/COOL[0,1.05,.8,.6]/COOL_RND[0,.8,.6,.45]。
- **bossNav.js L10-11**（exported）：WALL_BODY[{z:2.3,r:2.5},{z:-1.8,r:2.6}]/FLOOR_BODY[{z:2.2,r:1.45},{z:0,r:1.5},{z:-2.3,r:1.45}]；PAD_MARGIN [3.5,1.0]（L15，private，切源；tuning 节内命名 padr）。
- **netmatch.js**：L196 `msg.B = boss.pack(this.bossN % 10 === 0)`（每 10 tick full 一次，其余 delta 无 s[18]）；L150-153 sendBossHit `{k:'bhit', a:nid, d:r2(dmg), weak:0|1, w, c:crab}`——**c 缺省 -1 哨兵（`crab = -1` 形参默认值），非 null**；L215 case 'bhit' → `remoteHit(d)`；L494 `['bm', ...]` 中（`boss.onMove(e[2])`，e[2]=record `{id,t0,s,d,p}`，s=seed，d=[tele,act,rec]，p=per-move 参数对象）；L496 `['bc', id, x, y, z, killed?1:0]`（`_crabBurst`）；L652-654 res：`{k:'res', cov, win, mode, bo:result.boss, st:[[nid, round(turf), splats, deaths, round(bossDmg)||0, weakHits||0]×8]}`——**st 行恒 6 宽（JS 对所有模式写 bossDmg||0）**；L659 res 应用。

### §2 设计
**tuning v3**：`schema: 'inkwave.tuning.v3'`，新增顶层 `boss` 节：`{boss:{...}, mode:{...}, squadUnits:{human,bot}, moves:{6×{tele,act,rec}}, pace:[4], minTele, hz:{...}, brain:{speed,turn,cool,coolRnd}, nav:{wallBody,floorBody,padr}, hit:[10×{socket,r,weak}]}`。提取方式：**不用 ESM import**（boss.js 拉 THREE/ctx 浏览器全局），沿用 gen.mjs 的**切源+eval**模式（`const BOSS = {`→`};`、`const MOVES = {`→`};`、`const HZ = {`→`};`、`const HIT = [`→`];`、四个 brain 数组、nav 两数组+PAD_MARGIN），每块 `assertContains` 钉住上下文行（防上游漂移）。`tuning.rs` 加 `Boss` 类型（f32/u32 语义同 M3；socket 名 String；phases/pace/chargeSpeed/stun/crabSpeed 为 `[f32;4]` 或 Vec）；`tuning.schema.md` 更新 v3；锚定单测逐值对行号。

**wire.rs**：
- `BossSnap` 结构体（bt/x/y/z/yaw/vx/vz/hp/phase/f/mv/anim/turn/flash/weak/killer/crabs/maxHp/move:Option<JVal>），`encode_boss_snap(full)`/`decode_boss_snap` 手写（异质数组用 JVal 保序，数值 r2/r3 语义对齐 pack）。
- `Tick` 增 `b: Option<BossSnap>`（None 时不写 `B` 键 → 非 boss 局字节不变，PROTO v1 兼容）。
- `'bhit'`：`BhitMsg{a,d,weak,w,c:Option<i32>}` 编解码。
- `['bm', rec]`/`['bc', id,x,y,z,killed]`：record passthrough（rec 存 JVal 保序；bc typed）。
- res：`bo: Option<JVal>` + `st` 行 6 元素（bossDmg/weakHits 追加，缺省 0 容忍）。
- **fixture**（gen.mjs 扩展）：切 `pack`/`unpack` 方法体 eval，用**固定假 boss 状态**（无 Math.random）产 B full+delta 两态；bhit/bm/bc/res-boss 各产金样；Rust 侧逐字节回环断言。`['bm']/['bc']` 容忍测试升级为产生+消费双向。

### §3 实施批次
- **批一**：extract-tuning.mjs + tuning.rs + schema.md + 锚定 单测（TR-41.1）。
- **批二**：wire.rs（BossSnap/bhit/bm/bc/res）+ gen.mjs fixture + wire 回环 单测（TR-41.2）。

### §4 测试
- TR-41.1：连跑两次逐字节一致；boss 节逐值锚定（BOSS/BOSS_MODE/MOVES/HZ/HIT/brain/nav）。
- TR-41.2：B full/delta、bhit、bm、bc、res-boss fixture 逐字节回环；非 boss 局 tick 字节不变；554 基线不破。

### §5 风险
- 切源 eval 的边界匹配（`};` vs `};;`）——沿用 gen.mjsn.mjs 已验证的 slice() 模式
- pack 数值精度：hp `round(*10)/10`（1 位小数）vs maxHp `round()`（整数）——JVal 序列化须区分。
- flags 位语义（1/2/4/8）与 mv=-1 哨兵——写入 schema.md 注释。
- 不新增依赖；sim 依赖面不变。

### §6 验收
九段门禁全绿（task41-gates-*.log）；测试数 >554；JS 树零改动。

### §7 定案（实施后回填 §8）

### §8 Task 41 定案（2026-11-03 实施后）
- **D-T41-1 提取**：切源+eval（非 ESM import，boss.js 拉 THREE 浏览器全局），每块 `assertContains` 钉源行、找不到 marker 即 throw（无软回退）；schema v3；`source.path` 如实记 `src/config.js + src/boss/*.js`；tuning.json 重生成，连跑两次逐字节一致（sha256 `ed71ae47…`）。
- **D-T41-2 BossSnap**：19 槽位与 boss.js L422-434 逐位对齐（hp 一位小数 vs maxHp 整数；f 位标志 1/2/4/8；mv=-1 哨兵；crabs 仅存活只；s[18] 仅 full&&move）；decode `len<18 → None`。
- **D-T41-3 tick B 键**：插入 e 与 c 之间（JS 对象字面量键序）；`b: None` 不写键 → 非 boss 局 tick 字节与改动前逐字节一致，PROTO v1 兼容面不破。
- **D-T41-4 bm/bc**：`['bm', rec]` arity 2、rec 以 JVal passthrough 保序；`['bc', id,x,y,z,killed]` arity 6 typed；wrong arity → None（Task 40 容忍合同保留）；right-arity 非数值槽经 `g` 钳 0（JS packer 恒写数字，仅防坏 relay）。
- **D-T41-5 bhit/res**：bhit `c` 为 **-1 哨兵**（netmatch L150 `crab = -1` 形参默认，非 null）；res `st` 行恒 6 宽（JS 对所有模式写 `bossDmg||0`/`weakHits||0`），Rust 解码保留短行容忍；`bo: Option<JVal>`。
- **D-T41-6 §1 勘误（实读修正，已改正文）**：招名 **sweep/frenzy**（非 beam/spin；HZ 的 beam* 参数属 sweep）；slam tele **1.15**；HZ 含 **frenzyR 8.8/frenzyDps 52**；bossHazards L35 **SAFE_PAD=.6**（tuning 节命名 reachStep）/L36 REACH_N 96；bossNav L15 **PAD_MARGIN=[3.5,1.0]**（tuning 节命名 padr）；BOSS.weapon 键集 roller/slam/bomb/sticky/seeker/burst/shaker/waddle/torpedo（boss.js L22）。
- **D-T41-7 评审**：独立只读评审 **APPROVE**（无 P0/P1）。P2 五项：①§1 锚点过期→已按 D-T41-6 修正文；②source.path 未含 boss 源→已修并重生成；③s[18] 仅 Null 视为无 move（JS `||null` 对 falsy 亦 null）→接受（move 恒为对象或缺省，无行为差）；④wire bc 注释失准→已修注释；⑤reachStep 与 SAFE_PAD 不同名→接受（schema.md 已注对应）。
- 测试：inkwave_sim 470 passed（+16 vs M3 基线 454 sim 侧），workspace 567（+13 vs 554）。

## Task 42 实施设计（2026-11-03，实读锚点后定稿）

### §0 目标
`inkwave_sim::boss::{Boss, BossNav}`：Boss 实体状态机 + 导航 + hitShapes 查询 + 伤害链 + pack/unpack/follow/adopt/handoff。不含 Brain 选招/Hazards（Task 43）、match 分支（Task 44）、联网驱动（Task 45）。附带提取扩展：hitPos rest 坐标 + nav 三常量。

### §1 JS 锚点（实读，上一窗口 boss.js/bossNav.js 全读 + 本窗口补核）
- boss.js：BOSS 常量 L15-25；构造 L31-72（st 契约 L62、log L48、crabs Map L47、_rain L67）；_spawnPoint L75-93（planIds 重心+评分 `min(Dw,6)*1.5+min(Dv,5)-dist*0.25`、yaw=atan2(pad.x-x,pad.z-z)、24 次 turnOk 修正）；update L100-116；_modelUpdate L119-140（st；movePhaseAt/moveTimes/beamAngle 属 Task 43，本任务 move 恒 None 路径先行）；segHit L205-231（weak 优先 te-0.02、_hs 复用对象）；_segCrabs L232-250（crab r=0.55、cy+0.35）；rollHit L252-269（drum 低层球过滤 h.pos.y-h.r>cy+0.7）；rayDist L272-284；hit L288-302（倍率链 weak×stunned×BOSS.weapon[wid]??1、invuln/!visible 挡、guest sendBossHit、min 无上限）；splash L304-321（先 crab d<R+0.4 全额 dmax；body 最近非 weak 球 bd<R + LOS 才 hit 线性衰减）；rain/_flushRain L323-334（0.3s 批、wid='storm'）；applyDamage L337-350（dead/invuln/!visible/非 playing 挡、d=min(d,hp)、bossDmg/weakHits stats、flashN/weakN、d>=60→hurt=clamp(d/160,.3,1)、hp<=0.001→_defeat 否则 brain.checkPhase）；remoteHit L352-359（log.recv/recvDmg、d.c>=0→_hitCrab(fromNet)、否则 applyDamage(min(d,2000))）；_defeat L361-368；_hitCrab L371-381；crabPop L383-388（recBoss ['bc',id,r2x,r2y,r2z,killed?1:0]）；_crabBurst L390-402（host paint splat、inPad 0.3 挡）；_crabVisuals L407-418（dead 1.2s 后删）；pack L422-431（18 槽+full&&move 第 19；r2/r3 精度 L496-497；flags bit1 stun/2 dead/4 invuln/8 visible；mv=move.t0 或 -1；crabs 活者 [id,r2x,r2y,r2z,r2yaw,round hp]；hp=round(hp*10)/10、maxHp=round）；unpack L432-434；onMove L436；_follow L439-472（pos=S+err、vel 差分 dt>0、len^2>400 回退 S.vx/vz、bt=max、f 位解、S.f&2→dead、anim 变→animT0=bt、S.move→onMove、mv>=0→hz.moves find t0、fl/wk 计数 flash、crabSeen/missT>0.6 掉、speed 平滑 min(sp,8) lerp dt*8）；adopt L475-480；handoff L482（buf 清+handoff 标记）。
- bossNav.js：WALL_BODY L10、FLOOR_BODY L11、PLAN_WALL 2.9 L12、PLAN_FLOOR 1.8 L13、STRIDE 1.35 L14、PAD_MARGIN L15；构造 L19-30（step 0.5、x0=minX+0.25、kind 0floor/1wall/2drop/3pad、pads=spawnPads r=spawnBarrier）；_build L32-91（floorY=0.25m 直方图众数（g>-5 计 Math.round(g*4)）、g>=best-0.55 否则 kind2、4 高度点 [STRIDE+.1,2.6,3.8,5.2] pointInside→kind1、pad margin 按 team 索引→kind3、非 kind1 时 floor=best、Dw=_field(kind==1)、Dv=_field(kind>=2,edge=true)、ok=Dw>=2.9&&Dv>=1.8、连通域 8 邻域 stack flood、score=size*(0.4+dist_centroid_to_pad0/span)、planIds=最佳域）；_field L94-113（R=14 暴力、offs 预计算排序、out?edge:match、d=max(0,dd-st*0.5)）；xz/cell/wallClear/floorClear/kindAt(越界=2)/floorAt(0.45 吸附)/inPad/isPlan L115-126；poseOk L129-134；turnOk L137-141；cast L144-159（stepLen 0.25、停止后 claw 前 9 扇形 a=-0.9+k/8*1.8 距 f.r+0.6 kindAt==1→wall）；_cellOk/lineOk L161-167（slack 0.3、采样 n=ceil(d/(step*0.8))）；nearestPlan L170-186（r<60 ring 搜索）；path L189-230（A* 8 邻域代价 hypot(dx,dz)*step*(1+max(0,5-Dw)*0.1+max(0,3-Dv)*0.15)、iter<40000、stamp 数组复用、回溯>8000 截断、string-pulling）；randomPoint L233；MinHeap L236-260。
- bossBrain.js（本任务仅 checkPhase 纯函数）：L41-43 `checkPhase()`：`f=hpFrac(); want = f<=phases[1]?3 : f<=phases[0]?2 : 1; if want>phase && want>pendingPhase → pendingPhase=want`；onDefeat L45（move/path/plan 清）。roar 触发在 brain.update=Task 43。
- **hitPos 静态求值路线（本窗口实读定案）**：bossModelGeo.js `buildRig` L318-367 的 `add()` 只设 position（L321-325，无 quaternion）⇒ rest 骨链 matrixWorld ≡ 纯平移 T(rest[bone])；`getSocket` L151-154 = `off.applyMatrix4(bone.matrixWorld)`，`off = p - rest[bone]`（L56）⇒ **rest 世界坐标 ≡ sockets 表模型空间字面点 p**。sockets 表 L822-830（依赖真实导出 CONT L274/ARMS L303-306/HEAD L309-313/cont L276/V3）。特例（bossModel.js L57-59 覆盖，同样无需 rig）：shellF=RotX(CONT.pitch)·(0,0.05,1.5)+CONT.pos；shellR=RotX(CONT.pitch)·(0,0.1,-1.55)+CONT.pos；body=(0,2.2,2.3)。HIT 十球名=shellF/shellR/body/clawL/clawR/eyeL/eyeR/belly/crackL/crackR（bossModel.js L14-18，Task 41 已提取 socket/r/weak）。
- Rust 接口面（本窗口实读）：`BossSnap`/`BossCrab` net.rs L367/L387（19 槽，Task 41 wire.rs encode_boss_snap L496/decode L545）；`collision.rs`：`point_inside(p,pad,exclude,scratch)` L250、`ground_height(x,z,y_max,skip_grates)` L268、`los(a,b)` L394；`SimWorld` actor.rs L80-98（collision+spawn_pads[Vec3;2]+spawn_barrier）；`tuning.rs`：`Boss` L1393-1418、`BossNavTuning{wall_body,floor_body,padr}` L1556、`BossHitSocket{socket,r,weak}` L1571；lib.rs 模块表 L13-29（无 boss）。

### §2 设计
- **提取扩展**（extract-tuning.mjs，沿用切块 eval+assertContains）：①`boss.nav` 增 `planWall/planFloor/stride`（bossNav.js L12-14 字面切片）；②`boss.hit` 各项增 `pos:[x,y,z]`：sockets 表 L822-830 切片 eval（context 注入真实 CONT/ARMS/HEAD/cont + THREE shim——node_modules/three 存在，直接 import 真 THREE 更稳）+ 特例三式（锚 L57-59）；③schema 版本 `inkwave.tuning.v3→v4`；④交叉校验脚本 `rust/tools/verify/boss-hitpos-crosscheck.mjs`：import 真 three+buildRig，按 getSocket 语义（bone rest 平移复合）重算 10 球世界坐标，与 tuning.json hitPos 逐值容差 1e-4 断言——静态路线的独立证据链。tuning.rs：`BossHitSocket` 增 `pos: [f32;3]`；`BossNavTuning` 增三字段。
- **`boss/nav.rs`**：BossNav 逐行平移（构造入参=&CollisionWorld+SimWorld pads/bounds；grid Vec2D<u8> kind + f32 floor + Dw/Dv 场；A* 用 Vec MinHeap，tie-breaking 对齐 JS 堆序以保证确定性）。
- **`boss/mod.rs`**：Boss 实体（状态字段对齐构造 L31-72；crabs=Vec<BossCrabState>，id 单调；`hit_shapes()` 每帧世界球表=rest pos 经 Ry(yaw)·+pos 变换（root 语义：model space→world，实读 bossModel.js root position/rotation 设置行后锚定）+ active 规则（belly=!dead&&!below&&(stunned||phase>=3)——P[C.belly] 动画项属 Task 43，本任务以 stunned||phase>=3 近似并注记；crack=!dead&&!below&&tear——Task 43 前恒 false；below=模型下沉项 P[C.by]，本任务 below 恒 false 注记）；segHit/rollHit/rayDist/hit/splash/rain/applyDamage/remoteHit/_defeat/_hitCrab/crabPop/_crabBurst/pack→BossSnap/unpack/_follow/adopt/handoff 全量；Brain 依赖以 `pending_phase` 字段+`check_phase()` 纯函数承载，选招/roar 留 Task 43 接口）。
- 数值全走 tuning（含 hitPos）；JS 锚注释 `// JS <file> L<line>`。

### §3 实施批次（委派）
- 批次 A：提取扩展+tuning.rs+schema.md+锚定单测+crosscheck 脚本（含重生成 tuning.json，DETERM 双跑）。
- 批次 B：`boss/nav.rs` + 单测（Tidewater 构建、poseOk/turnOk/cast/path 行为）。
- 批次 C：`boss/mod.rs` + 单测（TR-42.1 全项）。
- 主上下文实读核验 → 独立评审 → 修复 → 九段门禁。

### §4 测试
- TR-42.1：HP 缩放（units×hpDiff 逐档）、phase 阈值 [.66,.33]（checkPhase 边界值）、hitShapes 逐球命中判定（weak 优先、te-0.02 提前、rollHit 低层过滤、splash LOS）、伤害倍率链、rain 批（0.3s/storm）、remoteHit crab 分支、pack→BossSnap→unpack→_follow 往返逐字段。
- TR-42.2：BossNav 三图冒烟（出生点 inPlan、path 不出界、kindAt 越界=2、nearestPlan）；hitPos crosscheck 脚本入门禁（node 段）。
- 基线 567 单调递增；JS 树零改动。

### §5 风险
- sockets 表 eval 依赖 ARMS/HEAD 的 `.normalize()/clone()` 等 THREE 方法——用真 three import（node_modules 已存在），不用 shim。
- A* tie-breaking 与 JS 堆不一致 → 路径漂移：单测钉参考路径（JS 侧 node 跑 bossNav.path 导出参考序列，Rust 断言逐点一致）。
- active 规则的动画项（P[C.belly]/tear/below）Task 43 前不可求值：以保守近似+JS 锚注记，评审确认。
- trunk 段沙箱 OOM 持续存在 → 沿用 DEGRADED 口径。

### §6 验收
九段门禁全绿（task42-gates-*.log，TRUNK 允许 DEGRADED 注记）；测试数 >567；JS 树零改动；crosscheck 脚本 PASS。

### §7 定案（实施后回填 §8）

### §8 Task 42 定案（2026-11-03，实施+评审后）
- **D-T42-1 hitPos 静态求值路线**：rest 骨链纯平移（buildRig add() 只设 position，bossModelGeo.js L321-325）⇒ socket rest 世界坐标 ≡ sockets 表模型空间字面点 p；shellF/shellR/body 特例（bossModel.js L57-59）经 off+T(rest) 复合消去 rest ⇒ 纯字面式。双证据链：提取脚本 slice-eval（静态）+ `boss-hitpos-crosscheck.mjs` 真 THREE Group.updateMatrixWorld 动态骨链复算（10/10 容差 1e-4）。schema v4。
- **D-T42-2 BossNav 精度**：内部运算 f64、存储 f32（对齐 JS Float32Array 舍入语义）；A* 两条固定路径以 node 参考 harness（真 bossNav.js+level.js+Tidewater）逐点钉定；`Math.round` 复刻 `floor(x+0.5)`；MinHeap 弹出序单测钉定。
- **D-T42-3 任务描述 vs 源码**：`_build` 的 floor 赋值 JS L57 实为 `if (k)`（kind≠0 含 wall 均设 floor=best），非「kind≠1」；按源码移植，构建逐位测试证实与 JS 一致。
- **D-T42-4 root 变换与近似**：world = pos + Ry(yaw)·rest（THREE Ry 约定 x'=cos·x+sin·z）。动画项近似（归属 Task 43，代码注释注明）：belly=`!dead&&(stunned||phase>=3)`（丢 `P[C.belly]>0.5` 姿态项）、crack=false（依赖 `A.sp.tear.x`）、below=false（依赖 `P[C.by]`）。
- **D-T42-5 评审修复**：P1-1 `BossHit.c` i8→i32（crabSeq 无上限，255 别名 -1 本体哨兵；bossBrain.js L336 实读）+ 大 id 往返测试；P1-2 guest 上行 `d` 走 `r2(dec(d))`（netmatch.js L153 实读）+ `log.sent/sentDmg` 计数补全（原被误归 Task 43）+ 测试期望 60.000004→60.0；P2-1 blocked 事件加 `is_local` 门控（boss.js L294 实读）；P2-3 lerp 注释更正（THREE f64）。
- **D-T42-6 主上下文核验补漏**：`BossAttacker.has_nid` 门控（netmatch.js L152 `attacker.nid === undefined` 不发上行但仍本地预测）——评审未列，核验 JS 实读后补，含专项测试。
- **D-T42-7 接受项**：P2-2 f32 状态 + `mv` f32 量化 ⇒ Rust-host↔JS-guest 混部不受支持（PROTO v1 同语言部署口径不变）；P2-3 splash/rain 嵌套 hypot vs JS 三参 Math.hypot（~1ulp，仅 LOS 边界翻转理论风险，改动收益低于风险）。
- 测试：inkwave_sim 506 passed（470→506，+36），workspace 基线 567 单调递增（门禁日志为准）。

## Task 43 实施设计（2026-11-03，实读锚点后定稿）

### §0 目标
`BossBrain`（bossBrain.js 376 全读）+ `BossHazards` sim 段（bossHazards.js L1-352 全读；L357-600 `HazardFX` 渲染段归 Task 46）逐行平移进 `inkwave_sim::boss`；接入 `Boss::update`（boss.js L100-116 时序），实化 Task 42 留桩：`_modelUpdate` move 分支（L128-133 `movePhaseAt/moveTimes`）+ `_moveAim`（L142-149）+ target aim（L135-138）+ crack 近似实化（phase≥3）+ `hz.fizzle/paint/update/crabBurst` + `brain.noteDamage/checkPhase/onDefeat` 接线 + `adopt` 建 brain。

### §1 JS 锚点（本窗口全读实核）
- bossBrain.js：SPEED/TURN/COOL/COOL_RND L12-15（已提取）、**PREF L16 `[0,10,9,8]` 未提取→本任务扩抓**；ctor L21-37（adopt 语义：cool 1.0、checkPhase、`_spawned` 判 crablets 已过 tele、crabSeq=maxId+1 且补 age/tgt/retgt/speed 缺省 L31）；pace getter L39；noteDamage L40；checkPhase L41-44；onDefeat L45；update L48-72（intro/roar 时钟、dmgBy exp 衰减、`stunned = move.p.stun && movePhaseAt==='rec'` L67、floorAt 缓升 L69-70、vel/turn 回算 L71）；_roar L74-81；_idle L84-105（face 判定 0.16/0.6/1.2/2.4）；_pickTarget L107-120（评分式 L114、attn×0.8 衰减 L117）；_walk L122-143（waypoint 1.1、vmax clamp(1−|dy|/0.9)、speed ±4/3·dt、stuck 0.8）；_turn L146-164（侧移 8 向 room 场）；_pickGoal L166-183（16 采样、pad 18m −12）；_choose L186-213（权重表、last×0.3、force 审计钩）；_lane L215-227（13 偏置、cast 34、min 6）；**_start L230-296（record 构造：tele=max(MIN_TELE,M.tele·pace)、六招 p 形态、barrage 候选排序 L247 含比较器内 rnd() 抽取、pts 空回退、`recBoss(['bm'])` L294）**；_runMove L299-327（charge 位移、sweep 身姿 lean、frenzy spin、crablets `_spawned`）；_spawnCrabs L330-340；_crabs L342-375（crabLife 超时、retgt 0.4、bd 30、接触 1.05/1.3、7 偏置绕行、inPad 0.8、waterY+0.3、落差 −1.4、pointInside、speed 平滑 dt·8）。
- bossHazards.js：MOVES L17-24、PACE L25、MIN_TELE L26、HZ L27-34、SAFE_PAD L35、REACH_N L36（均已提取）；moveTimes/movePhaseAt L38-46；chargeDist/chargeTime L48-55；barrelPos L57-60；beamAngle L61（smoothstep）；ringStart/ringRadius L62-63；BossHazards L69-352：add（t0 幂等+log.moves）L80-85、current L86、fizzle L88-98（fx burst 渲染段省略）、_live L101-105（slam extra）、update L107-116（end+4 剪枝+hits 清理、live→_hurtLocal、prevBt）、_hurtLocal L119-187（victim-side：`!remote && alive && team!==1`、SAFE_PAD 免疫、once 键 `${t0}|${kind}|${i}|${nid}`、slam impact 跨界带、ring 扫带+reach 剪、barrel land、sweep dps·dt+submerged 免+LOS、charge WALL_BODY 双球+击飞速度式 L170、frenzy dps·dt+LOS）、crabBurst L189-194、_damage L195（`a.damage(dmg, boss.attacker, 'boss')`）、_los L196、_c/reach/reachAt L201-217（kindAt===0 步进 0.35、1.2 起步）、beamLen L220-230（raycast skipGrates=true、角度桶 round(ang·90)）、**paint L233-289（host-only：splat 射线 DOWN 4m skipGrates=true、点+法线·0.08、pad 0.3 免疫、waterY 拒；六招泼墨节律含 steps() 离散化）**、threat L294-349（level/ax/az/ringIn/cover/beam/lane、horizon 1.2、push 归一）。
- API 对应（实核）：`G.level.groundHeight(x,z,yMax)` ≡ `collision.ground_height(x,z,y_max,false)`（level.js L389 默认 skipGrates=false）；`G.physics.raycast(o,d,max,out,true)` ≡ `collision.raycast(...,true)`（physics.js L34）；`los` ≡ `collision.los`；`pointInside(p,0)` ≡ `point_inside(p,0.0,-1,scratch)`；`PLAYER.waterY/radius` ≡ tuning.player；`rng()` ≡ `Rng::next_f32`。
- 既有 Rust 桩：`Boss::update` L629-642（_bt0/_live 占位）、`model_update` L648-677（move 分支缺）、`emit_events` L690-706（per-phase 注）、`defeat` L1278（hz.fizzle/brain.onDefeat）、`apply_damage` L1207（noteDamage）、`crab_burst` L1396（hz.crabBurst）、`adopt` L1663（建 brain）、`follow` L1533（fizzle）、`shape_active` L593 crack=false、`BossLog` 无 moves、`CrabState` 缺 age/tgt/retgt。

### §2 设计
- **record 形态 = JVal 端到端**（`{id,t0,s,d:[tele,act,rec],p}`，值经 r3 f64）：与 `['bm']` 逐字节同形、guest `on_move` 已收 JVal、pack s[18] 已带 record——不建平行 typed 结构；新增 `MoveId` enum（slam/barrage/sweep/charge/crablets/frenzy，from_str/as_str）+ 访问器族（`rec_d(m,i)`、`rec_p_f64(m,"x")`、`rec_rings(m)->Vec<f64>`、`rec_pts(m)->Vec<[f64;4]>`、`rec_n`、`rec_spin` 等）置于 `boss/hazards.rs`。
- **hz 状态上移实体**（沿用 Task 42 hoisting 先例）：`Boss` 新增 `hz_hits: HashSet<String>`、`hz_cache: HashMap<f64, HzCache>`（t0→{beam: HashMap<i32,f32>, reach: Option<Vec<f32>>}）、`hz_prev_bt: f32`、`hz_dead: f32`（−1 哨兵）；`BossLog` 增 `moves: Vec<(f64,String)>`（去 Copy 保 Clone）。hazards 逻辑写成 `impl Boss`（文件 `boss/hazards.rs`），公开方法：`hz_add(m)->bool`、`hz_current(bt)`、`hz_fizzle(bt)`、`hz_live(m,bt)`、`hz_update(dt,bt,live,squad)`、`hz_hurt_local(bt0,bt1,dt,squad)`、`hz_crab_burst(x,y,z,squad)`、`hz_reach/hz_reach_at/hz_beam_len`、`hz_paint(bt0,bt1)`（产 `BossEvent::Splat{team:1}`，seed 走 boss.rng）、`threat(x,y,z,horizon)->Threat`（pub struct，Task 47 消费）；几何自由函数 move_times/move_phase_at/charge_dist/charge_time/barrel_pos/beam_angle/ring_start/ring_radius/h_dist（smoothstep 复刻 ctx.js）。
- **brain 独立结构** `boss/brain.rs`：`BossBrain{rnd:Rng, cool, path:Option<Vec<[f32;2]>>, pi, goal, goal_t, stuck_t, attn:HashMap<i32,f64>, dmg_by:HashMap<i32,f64>, last:Option<MoveId>, cd:{crablets,frenzy}, retarget, plan:Option<Plan>, speed, force:Option<MoveId>, spawned}`；`Plan{id, face:Option<f64>, face_fn:bool(aimAt), lane:Option<Lane>, t}`；方法 `update(&mut self, dt, boss:&mut Boss, squad:&mut BossSquad)` 等（JS `this.b` 显式参数化）。`Boss` 增 `brain: Option<BossBrain>`，调用点 take/put 规避双借。`Boss::new`：sim 时建 brain（rnd 种子 = seed 派生 `seed ^ 0x9E37_79B9_7F4A_7C15`，与 boss.rng 分流，D-T43）；`adopt()` 建 adopt brain（cool 1.0、check_phase、spawned 判定、crab_seq=max+1 并补齐 crab age/tgt/retgt/speed）。
- **actor 视图** `boss/mod.rs`：`pub struct BossSquad<'s>{ pub actors:&'s mut [Actor], pub ids:&'s [i32], pub remote:&'s [bool], pub player:&'s tuning::Player }`（ids=JS `a.nid ?? a.slot`，remote=JS `a.remote`；Task 44 match 层持有真数据，测试合成）。`Boss::update(dt)` 签名改 `update(&mut self, dt:f32, squad:&mut BossSquad)`，内部序 ≡ boss.js L100-116：brain.update|follow → fizzle 条件（L106，`Phase::End` 并 finish/judge/results）→ paint（sim&&live&&bt>bt0，`[max(bt0,paint_bt),bt]`）→ paint_bt=bt → hz_update(dt,bt,live) → emit_events → crab_visuals → model_update(dt,squad) → flush_rain。
- **实化清单**：`model_update` move 分支 L128-133（ph/start/moveT/phaseDur）+ `_moveAim` L142-149（六招 aim 求值，barrage find 首个未落地桶）+ L135-138 target aim（squad 查 ids==target）；`shape_active` crack→`phase>=3`（D-T42-4 实化，姿态项 tear→phase 代理声明）；`BossEvent::Stunned` 增 `dur: f32`（=moveTimes(m).rec，JS L173）；`emit_events` move 事件保持每 move 一次（per-phase 拆分不做，Task 46 若需再议）；`defeat` 接 hz_fizzle+brain.onDefeat；`apply_damage` 接 brain.note_damage；`crab_burst` 接 hz_crab_burst（!killed&&live 门 ≡ JS L396）；`follow` 死亡分支接 hz_fizzle；`CrabState` 增 `age:f32/tgt:i32/retgt:f32`（host 字段，guest 侧恒初值）。
- **提取扩抓**：`extract-tuning.mjs` brain 段加 `PREF`（slice `const PREF = [`…`];` + assertContains 钉 L16）→ `brain.pref:[f32;4]`；schema `inkwave.tuning.v4→v5`；tuning.rs 类型+锚定测试；重生成 tuning.json 连跑逐字节一致。
- **伤害归属**：victim-side `_damage` → `actor.damage(dmg, Some((1,0)), Some(boss_pos), player_t)`（boss 伪攻击手 team1/slot0——boss 局无 team1 实体，D-T43）；charge 击飞直改 `a.vel/a.grounded`（JS L170 逐式）。

### §3 实施批次（委派）
- A：提取 PREF + schema v5 + tuning.rs 类型/锚测（独立，先行）。
- B：`boss/hazards.rs`（record 访问器+几何+hz 状态方法+threat+paint）——依赖 A 否（不依赖 pref）。
- C：`boss/brain.rs` + Boss 接线（update 签名、model_update/defeat/adopt/crab 字段等实化）——依赖 B。
- D：测试（TR-43.1/43.2）+ 既有测试适配（update 签名波及面）——依赖 C。

### §4 测试
- TR-43.1 六招逐招：固定种子 brain 序列确定（同 seed 两次逐 record 一致）；tele≥MIN_TELE；paceDiff 缩放；`force` 钩逐招钉 record 形态（slam rings 按 phase、barrage n=[0,4,5,7][ph]、sweep span、charge v/stun、crablets n、frenzy spin/rot0）；hazard 几何对参考值逐帧（ring 半径线性、barrelPos 抛物、beamAngle smoothstep 端点、chargeDist ramp、reach 剪枝 Tidewater 实测）；伤害 once 键不重、扫带防穿隧、SAFE_PAD、submerged 免束、LOS、dps·dt、victim-side（remote 不伤）。
- TR-43.2 双端：同 record host/guest `hz_current`/几何求值逐 tick 一致（guest 经 on_move+hz_update(live=false) 不产伤害）；crablets 生命周期（n 只、追踪重选 0.4s、life 超时 pop、接触 1.05 pop、`Bc` 事件、pop 1.2s 清除）；threat() 逐招输出钉值。
- 基线：inkwave_sim 506→≥560 目标（单调），workspace 603 递增；`cargo tree -p inkwave_sim` 依赖面不变。

### §5 风险
- barrage 排序比较器含 `rnd()` 抽取（JS L247 非纯比较器）：抽取次数依赖排序算法比较序——仅 host 产 record，guest 重放不重算；Rust↔Rust 确定、JS↔Rust 混部 host 不支持（沿 D-T42-7），测试钉 Rust 自身确定性即可。
- `update` 签名变更波及 Task 42 测试与调用点（headless 工具）——统一适配，空 squad 兜底。
- paint 节律 `steps()` 离散化 + splat 序列 host-only：逐字节钉 seed 流（boss.rng 消耗序）风险高——测试钉计数/半径/坐标量化值，不钉 seed 值本身（JS 用 Math.random 本就不可钉）。
- JS `for…of Map` 迭代序 = 插入序：crabs/attn/dmgBy 用 Vec/HashMap 需保证遍历序（attn/dmgBy 仅取值不依赖序；crabs 保 Vec ✓）。

### §6 验收
- TR-43.1/TR-43.2 全绿；九段门禁全绿（TRUNK DEGRADED 口径同 D-T40-1）；JS 树零改动；基线递增。

### §7 定案（实施后回填 §8）

### §8 Task 43 定案（2026-11-03，实施+评审后）
- **D-T43-1 brain RNG 分流**：JS `new rng((Math.random()*2**31)>>>0)`（bossBrain.js L23）不可钉种 ⇒ Rust 以 boss seed 派生（`seed ^ 0x9E37_79B9_7F4A_7C15`）保确定性、与 boss.rng 互不干扰；paint 的 splat seed 流仍走 boss.rng（JS L240 `Math.random()` 独立源）。
- **D-T43-2 barrage 排序偏差**：JS L247 比较器每次比较抽 `rnd()`（非纯、结果依赖排序算法）⇒ Rust 每候选预抽 jitter `(rnd()-0.5)*6`、按纯 key `dist+jitter` 降距排序（brain.rs DEVIATION 注释在案）；record 仅 host 产生（D-T42-7），钉 Rust 自身确定性（同 seed 双跑逐 record 一致测试）。
- **D-T43-3 hz_cache 键**：f64 不满足 `Hash/Eq` ⇒ `HashMap<u64>` 键 `t0.to_bits()`；`hz_update` 剪枝时同步 `remove` ≡ JS WeakMap 随记录回收（hazards.rs L419）。
- **D-T43-4 hz_paint 加 player 参**：JS L239 读全局 `PLAYER.waterY`，Rust Boss 不持有 player tuning ⇒ 签名 `hz_paint(bt0,bt1,player)`，接线传 `squad.player`。
- **D-T43-5 victim-side 伪攻击手**：JS `_damage` 传 `boss.attacker`（team1、无 nid ⇒ 不上行）⇒ Rust `Some((1,0))`+boss 位置；boss 局无 team1 实体，伤害归属仅审计面。
- **D-T43-6 crack 实化**：D-T42-4 近似落地——`A.sp.tear.x` 依赖 ⇒ `phase>=3` 代理（姿态 tear 项声明性丢弃）；belly 维持 `stunned||phase>=3`。
- **D-T43-7 Stunned.dur**：= `move_times(m).rec`（JS boss.js L173 `remaining` 在 tele→rec 边沿触发时刻 ≈ rec）。
- **D-T43-8 渲染段省略**：`hz_fizzle` 的 barrel fx burst 循环、`crabBurst(x,y,z,id)` 未用 id 参、HazardFX 全段（bossHazards.js L357-600）⇒ Task 46。
- **D-T43-9 nav.path 内部可变性**：`&mut self`→`&self`（RefCell/Cell scratch），brain 经实体共享引用规划所需，行为保持（评审 P2-2 补记）。
- **D-T43-10 crab_burst pending 队列**：host 正常路径带 squad 内联 victim-side；无 squad 调用点入队、下次 update 排空（测试覆盖）。
- **评审**：独立评审 PASS（0 P0 / 0 P1 / 5 P2）。处置：P2-1 `reachStep` 命名系 spec 既有命名（值=SAFE_PAD 已锚测+注释澄清）不改；P2-2→D-T43-9 补记；P2-3 hz_cache 不清理——**评审误报**，剪枝已 remove（L419 实核）；P2-4 Move 事件每 move 一次——§2 已声明（Task 46 若需再议）；P2-5 follow fizzle 顺序——`hz_fizzle` 不读 dead/killer，无观测差异。
- 测试：inkwave_sim 506→560 passed（+54：批次 A 锚测 1、批次 B smoke 6、批次 D 47），workspace 基线递增（门禁日志为准）；TR-43.1 逐招 record 钉形/几何参考值/伤害语义 27 项、TR-43.2 双端一致/crablets 生命周期/threat 钉值/paint host-only 20 项。

## Task 44 实施设计（2026-11-04，实读锚点后定稿）

### §0 目标
`BossMode`（bossMode.js 55 全读）+ `Match` boss 分支（match.js 全读实核）+ session 放开 `mode:'boss'`（session.js L244-292 实读）+ MAPS 旗标（config.js L515-545 实读）+ bot `_bossTick` 段（bots.js L345/L1874-2088 全读）平移进 Rust。**联网语义（B 快照发布/播放、bhit 上行、bm/bc 时间线、迁移、res 收发、e2e/scenario E）全部归 Task 45**——本任务 guest 收到 boss start cfg 不再拒绝即可。完成后离线 Boss 战端到端可玩（体块占位档，呈现归 Task 46）。

### §1 JS 锚点（本窗口全读实核）
- bossMode.js（55 行全读）：`BOSS_MODE{duration 240, intro 7.2, finishWin 5.2, finishLose 3.2, squad 8}` L8-13；`squadUnits = Σ(isBot && !isLocal ? 0.6 : 1)` L15；ctor L19-29（units=team0 求和、`maxHp=round(hpPerUnit×units×hpDiff[diff]??1)`、`sim:!match.follower`、`G.boss=`、won/killT=-1、emit boss:spawn）；update L31-36（b.update(dt) → `!follower && b.dead && state==='playing' → setState('finish')`）；onDefeat L38-42（won=true、`killT=duration-time`、`by.stats.bossKill=1`）；result L44-52（`{mode:'boss', winner: win?0:1, coverage, boss:{name, win, time: round1(win&&killT>=0?killT:duration-time), hp: round(b.hp), maxHp, phase}}`）；dispose L53（G.boss=null）。
- match.js：mode 判定 L20（attract/practice 强制 turf）；duration L21（`opts.duration || boss?BOSS_MODE.duration`）；setup boss 建队 L55-62（单队 team0、`BOSS_MODE.squad` 8 槽、weapons 双份 pickTeam 拼 8 武器）；L63-77（slot0=local、bot 全员、sub/special 随机半默认）；摆放 L85-91（`ang=slot/(boss?squad:4)·τ+0.6`、`rr=boss?1.7:1.2`）；L100 `if(mode==='boss'){this.bossMode=new BossMode(this); this.boss=this.bossMode.boss}`；roster 路径 L104-133（同 L132 boss 建 BossMode）；intro 时长 L193（`bossMode?BOSS_MODE.intro:4.2`）；finish 时长 L216（`bossMode?(boss.dead?finishWin:finishLose):2.6`）；L229 `bossMode?.update(dt)` 在 actors 步进后、soft push 前；judge L262-267（bossMode 先于 turf：result=bossMode.result()）；dispose L159。
- session.js：setSettings L244（mode 三值放行）、L248（`boss && !mapBossOk(map) → map=bossFallbackMap(wasMap)`）、L249（`bots = mapNoBots ? false : botsPref ?? bots`）；start L274-292（`boss = mode==='boss' && mapBossOk(map)`；单队循环 team<1；humans 全进、bots 补到 `TEAM*2=8`、武器去重同 turf 规则；cfg.duration `!boss&&zones?ZONES: l.duration`；cfg.mode `boss?'boss':…`）；L258 startBlock=noBotsStartBlock。
- config.js L515-545：MAPS 8 项旗标（仅 cargo `onlineOnly+noBots+noBoss`）；`mapById/mapNoBots/mapBossOk/mapOfflineOk/OFFLINE_MAPS/bossFallbackMap/noBotsStartBlock(2+人且两队各一)`。
- bots.js：L345 `if(G.boss){this._bossTick(dt);return}`（在 superJump/not-playing 早退 L341-342 与计时递减 L343-344 之后、perception 之前）；`_bossTick` L1874-1979（think 0.12+rnd·0.1、`_th` 复用 threat(x,y,z,1.4)、reaction 门 thSeen/hopMiss（0.45−fireDiscipline·0.4）、refill 门（inkFrac<0.1 且非 crablet 贴脸 4m/0.03）、evading=`th.level>0.2 && !(beam && groundTeam===1)`、evadeT 0.45、`_bossGoal`（stunned 或 phase3·30% 抢 belly）、aim/fire（range+rad·0.6、charger 1.0 其余 1.05、circle-strafe（非 charger、无 threat、dist<4 后撤 0.6）、tol=`max(0.05,atan2(rad,max(dist,0.5)))×(firing?2.4:1.5)`、六 kind fire 分支（含 bow `max(0.5,chargeRelease)`、blade 0.98、brolly pump、roller dist<5.5/rolling<8、splatling——Rust 无此 kind，声明偏差）、blink 保充、sub bomb（非 crab、ink>cost+10、5<dist<13、p=0.025、cd 6+rnd·6）、special slam<5.5/storm<13+LOS）；不射击清墨段 L1959-1967（regionStats 前探 3m r2.5、groundTeam===2||enemy>0.2、sweep 2.1·dt、pitch 表）；refill 段 L1968-1971；dodge L1973-1976（ringIn<0.2 hop、dive=squid、own-ink swim）；`_bossPerceive` L1982-2022（crab 优先 d≤9+LOS、rad0.5；else hitShapes active 过滤、belly 强制、shapeT 粘性 1.2+rnd·1.5、eyes 概率 `0.25+fireDiscipline·0.3`（charger 恒真）、评分 `(belly?-100:weak&&eyes?-50:weak?10:0)+dist`、LOS 首选否则首个非 weak）；新目标 react=`reaction×(0.6+rnd·0.5)`+acq 重抽；`_bossGoal` L2026-2067（goalTimer 2.4+rnd·2、roller 清墨点 12 采样（threat 拒、enemy−r·0.02、p<0.7）、reach 表 charger15/roller3.2/`clamp(range·0.7,4.5,11)`、R=3.4+(stunned?min(reach,6):reach)、12 采样方位（stunned 正面±0.65 否则侧后 ±(0.95..2.85)）、nav.nearest 2.5、threat 拒 −6、LOS boss+2.5 +3、mate goal<4m −2.5、评分 `−dist·0.08+rnd`）；`_bossEvade` L2070-2087（12 采样沿逃逸向 ±1.2rad 3-8m、nav.nearest 1.2、`−level·8−dist·0.25+dot·1.5+rnd·0.5`、goalTimer 0.8）。
- Rust 现状实核：`Match`（match_.rs L299-407）无 Boss；`MatchMode`（zones.rs L251）两值；`with_roster_mode` L457-546（zones 降级先例 D-T36-1、`place_on_pad` L551-566 常量环）；phase 机 L751-848（Intro 4.2/Finish 2.6 常量、judge L1911-1966 zones 分支先例）；`MatchResult` L132-147（zones: Option<ZoneOutcome> 折叠先例）；`Boss<'a>`（boss/mod.rs L389-483）**借用 `nav/tb/world`（L468-471）⇒ Match 不能持有 Boss（自引用）**；`BossSquad<'s>` L355-361；`Boss::update(dt,squad)` L699；`stats_by_nid: HashMap<i32,AttackerStats{boss_dmg,weak_hits,splats}>` L452、`killer:i32` L412、`crabs:Vec<CrabState>` L445、`threat()` 已 pub（Task 43）；`BotCtx`（bot.rs L209-237）无 boss 字段、`Bot` 字段面 L319-463（evadeT/thSeen/hopMiss/wasEvading/rushing/pump/shapeT/bTgt 缺，其余 think/react/strafe/sweep/bomb_cd/acq 系全有）；`Bot::step` L603（JS L345 插入点=计时递减后、perceive 前）；`bot/zones.rs` 扩展先例；`autopilot::run`（autopilot.rs L88-200）Match::new 硬 turf、BotCtx 组装 L138-153；session.rs L37-43/L1282/L1794-1800 boss 拒（`guest_refuses_a_boss_start` L2859、L2454 断言待翻转）；tuning：`boss.mode: BossModeRules{duration,intro,finish_win,finish_lose,squad}` L1403/L1475、`boss.squad_units: BossSquadUnits{human,bot}` L1405、**无 maps 旗标段**；UI MODE 两值 cycle（ui/mod.rs L1579-1585）。

### §2 设计
- **Boss 所有权 = 驱动层**（关键决策）：`Boss<'a>` 借用 nav/tb/world，Match 无法持有 ⇒ 新增 `pub struct BossMode<'a>{ pub boss: Boss<'a>, won: bool, kill_t: f32 }`（`boss/mode.rs` ≡ bossMode.js）由 autopilot/game 层/nettest 构造持有；`Match` 只持 `boss_rules: Option<BossModeRules>`（≡ zones_tuning 模式，Copy 数据）与 `mode: MatchMode::Boss`。`Match::step/step_masked` 增参 `boss: Option<&mut BossMode<'_>>`（None=非 boss 局；既有调用点全 None 零波及）：内部序 ≡ match.js L220-229——actors 步进 → `boss.update(dt,&mut squad)` → **返回 `bool`（b.dead 且 state==Active 且 !follower ⇒ 调用侧 set_phase(Finish)，≡ bossMode.js L35）** → soft push。BossSquad 每帧由 step 现场构造（ids=roster 下标 nid 语义、remote=掩码、player=t）。`BossMode::update` 内 ≡ bossMode.js：b.update → dead 沿检测（won/kill_t=elapsed、killer 经 squad ids 找 actor 置 `Actor.boss_kill=1` 新字段 serde default）。`result(&self, coverage, elapsed)` ≡ L44-52。
- **Match 接入**：`MatchMode::Boss`（zones.rs enum 扩展）；`with_roster_mode` boss 分支——单队 team0×8（`boss_rules.squad`）、武器表双份拼接（JS L61 语义：8 槽全武器 kinds 覆盖）、`duration<=0 → boss_rules.duration(240)`、无 boss_rules 降级 Turf（D-T36-1 先例）；摆放 `place_on_pad` 参数化 `(ring_n, rr)`：boss=(8, 1.7) 否则 (4, 1.2)；phase 机 Intro `boss_rules.intro(7.2)` 否则 4.2、Finish `boss.dead?5.2:3.2` 否则 2.6（dead 经 boss 参读取）；judge boss 分支先于 turf：`MatchResult.boss: Option<BossOutcome{name,win,time,hp,max_hp,phase}>`（≡ zones: Option<ZoneOutcome> 折叠先例；points [0,0]、winner=win?0:1）；restart 不动 BossMode（驱动层负责重建，声明）。
- **提取扩抓（schema v6）**：`extract-tuning.mjs` 加 `maps` 段——config.js `export const MAPS = [`…`];` 字面块切片 eval，仅取 `{id, onlineOnly, noBots, noBoss}`（theme/times 归 Task 48）；`tuning.rs` `MapFlags` + `Tuning.maps` + 锚测（8 项、cargo 三真、其余全假）；纯函数 `map_boss_ok/map_no_bots/map_offline_ok/boss_fallback_map(prefer)`（≡ config L533-537，找不到表时回 `MAPS[0].id` 语义）置于 tuning.rs；`no_bots_start_block(lobby)` 置 session.rs（≡ L539-545）。
- **session 放开 boss**：`set_settings` mode 三值（L244）+ L248-249 旗标联动（boss 拒 noBoss 图→fallback、noBots 强制 bots=false——Rust 现有 `_bots_pref` 语义核对）；`start()` boss 建 roster ≡ L274-292（单队、humans 先、bots 补至 8、武器去重、cfg.mode "boss"、duration `!boss&&zones?ZONES:l.duration`）；guest `_begin` 收 boss cfg **不再拒绝**（L1794-1800 移除、模块 docs L37-43 改写、L2454/L2859 两测试翻转）；`matchbuild` boss：roster→单队 Match + `MatchMode::Boss`（follower=true，BossMode 由上层持——联网播放归 Task 45，本任务 guest 局 boss 实体不步进亦可（无 sim 权））。
- **bot boss 段**：新文件 `bot/boss.rs`（`impl Bot` 方法族，≡ bot/zones.rs 模式）：`boss_tick(dt,a,ctx)`/`boss_perceive`/`boss_goal`/`boss_evade` ≡ L1874-2088 逐行；`BotCtx` 增 `boss: Option<&'a Boss<'a>>`（threat/crabs/hit_sphere/shape_active/stunned/phase/yaw/pos/dead/visible 全 &self 可读）；`Bot` 增字段 `b_tgt: Option<BBotTgt>`（enum Shape{idx,rad,weak,dist,los}|Crab{id,pos,dist,los}）、`th: Threat` 复用槽、`th_seen:Option<f32>/hop_miss/evade_t/was_evading/rushing/pump/shape_t`；hitShapes 遍历=tuning `boss.hit` 表 × `boss.shape_active(i)`/`hit_sphere(i)`；`w.kind==='splatling'` 分支省略（Rust WeaponKind 无此档，沿 Task 38 既例，声明偏差）；插入点=Bot::step 计时递减后、perceive 前（≡ L345）。
- **UI**：MODE 三值 cycle（ui/mod.rs L1579-1585 turf→zones→boss→turf）；离线 Play 模式选择（客户端找既 offline 开局路径，加 boss 项并接 autopilot 式驱动：构造 BossMode+NavGraph+每帧 step 传 boss 参）；结算 UI boss 形态行（VICTORY/DEFEAT+bossDmg/weakHits chip）归 Task 46，本任务仅不 panic。

### §3 实施批次（委派）
- A：提取 maps 旗标（schema v6）+ tuning.rs 类型/锚测/纯函数（独立，先行）。
- B：`boss/mode.rs` + Match 接入（enum/with_roster_mode/phase/judge/step 增参/Actor.boss_kill）+ autopilot boss 驱动（run 增 mode 形参或新 `run_boss`，含 BossMode 构造：maxHp=round(hp_per_unit×units×hp_diff)、units=Σ(local?human:bot)）——依赖 A。
- C：`bot/boss.rs` 四方法 + BotCtx/Bot 字段 + step 插入——依赖 B（类型面），可与 B 尾并行。
- D：session/matchbuild/UI——依赖 B。
- E：测试 TR-44.1/44.2 + 既有适配（guest boss 两测试翻转、step 签名波及面）——依赖 B/C/D。

### §4 测试
- TR-44.1 离线完整局双分支：固定种子 autopilot 式驱动（8 队 bot+BossMode，Tidewater）——**lose**：240s 钟尽（boss 存活）→ finish 3.2s → judge → `result.boss{win:false, winner:1, time:240, hp>0, maxHp=round(18000×units×hpDiff), phase}` 逐项；**win**：t≈100s 注入 `apply_damage` 致死 → finish 5.2s → `win:true, winner:0, time=kill 时刻 round1、hp:0`；bossDmg/weakHits 对账=Σ`stats_by_nid` 与注入/射击伤害一致；同 seed 双跑 result 逐字节一致。
- TR-44.2 bot 行为：钉种子场景（合成 Boss+BotCtx.boss）——threat 响应（slam tele 下 evade 路径/jump hop 意图、reaction 门 thSeen 前不反应、hopMiss 概率面）；集火（belly stunned 强制选形、eyes 概率面、crab≤9m 优先、shapeT 粘性）；crablet 清除（spawn_crab→bTgt.crab 选中→射击意图）；refill 门（ink<0.1 转 refill、crab 贴脸 4m 例外）。旗标路径：map_boss_ok/boss_fallback_map/no_bots_start_block 钉 config 值；session：host setSettings boss 放行+fallback 换图、start() 单队 8 roster、guest 收 boss cfg 不再离开。turf/zones 既有测试全不破（step 增参 None）。
- 基线：inkwave_sim 560→≥600、workspace 657 单调递增；依赖面不变。

### §5 风险
- step 增参波及面广（headless 工具/nettest/replication 调用点）——统一 None 适配，机械改。
- BossMode 借用 world/nav/tuning 生命周期：驱动层结构需统一 lifetime（autopilot run 内构造 SimWorld/NavGraph 后建 BossMode，借用顺序注意 m.step 与 boss.update 的 actors 双借——squad 在 step 内部现场构造，BossMode 只经 &mut 传入，无冲突）。
- bot 段 RNG 抽取序（think 0.12+rnd、hopMiss、shapeT、goal 12 采样 rnd、react/acq 重抽）与 JS 逐式对齐——钉 Rust 自身确定性即可（JS 本用 Math.random）。
- boss 局 paint coverage 语义：JS boss 局 result 仍带 coverage（turf 次要显示）——沿用 paint.coverage()，无特判。
- guest boss 局（Task 44 范围内）BossMode sim=false 且无快照消费——boss 不步进不产事件，客户端不 panic 即可（联网收口 Task 45，如实注记）。

### §6 验收
- TR-44.1/TR-44.2 全绿；九段门禁全绿（TRUNK DEGRADED 口径同 D-T40-1）；JS 树零改动；基线递增；wire.rs/replication 零改动（联网归 Task 45）。

### §7 定案（实施后回填 §8）

### §8 Task 44 定案（2026-11-04，实施后+评审后）

- **D-T44-1 BossMode 落驱动层（Match 不持有 Boss）**：`Boss<'a>` 借用 `nav/tb/world`，`Match` 若同时持有 `world`/`nav`/`Boss` 即自引用，Rust 不可能；故 `BossMode` 由 `autopilot::run_boss` 驱动层构造并每帧 `step(..., Some(&mut bm))` 传入，`Match` 只经 `boss_outcome: Option<BossOutcome>` 收结算。与 §2 设计一致。
- **D-T44-2 onDefeat 边沿检测**：JS `onDefeat` 由 boss 实体死亡回调驱动；Rust `BossMode::update` 在 `dead` 由 false→true 的边沿捕获 killer（`Boss::killer`），置 `finishT = now + finishWin` 并 `set_phase(FIGHT)`，与 JS L30-38 等价。
- **D-T44-3 splatling 分支省略**：`WeaponKind` 无 Splatling 档（Task 38 既有裁剪先例），`bot/boss.rs` 开火门缺 splatling 蓄力臂，file-head 注释声明偏差。
- **D-T44-4 squadUnits 驱动聚合**：JS `squadUnits = Σ(isBot && !isLocal ? 0.6 : 1)`；sim `Actor` 无 `isBot/isLocal` 旗标，`run_boss` 按名册 humans/bots 计数加权，`hp_scale = squad_units`（JS `hpScale` 语义），与 §1 ctor 公式一致。
- **D-T44-5 离线 Play 菜单模式选择推迟 Task 46**：spec §2 承诺"离线 Play 模式选择（turf/zones/boss）"，本任务仅完成在线 lobby MODE 三值 cycle（`ui/mod.rs` L1580-1588）；离线 demo 仍为 turf-only（`DemoSim` 自引用借用 BossMode 需 unsafe 或 BossMode 无借用重构，超出本任务"仅不 panic"边界），经用户确认接线归 Task 46 呈现占位档。
- **D-T44-6 matchbuild boss 单队构造**：`cfg.mode == "boss"` 走 `Match::with_roster(..., MatchMode::Boss, Some(embedded_boss_rules()))` 单队（squad=8，spawn 环除数 `boss ? squad : 4`，JS match.js L87）；`BossMode` wrapper 留驱动层，联网 boss 实体步进待 Task 45。
- **D-T44-7 武器表双份拼接**：JS bots.js L61 全武器 kinds 覆盖；Rust 保留 Spritzer 默认 + per-kind 开火门（Charger/Spinner/Bow/Blade/Brolly/Roller），splatling 臂缺（D-T44-3）。
- **D-T44-8 guest boss cfg 放行不步进**：`session.rs` guest 接受 `mode:"boss"` 开局（不再 reject），但 boss 实体不联网步进（Task 45 联网播放路径），与 §2 一致。
- **D-T44-9 Report boss_dmg/weak_hits 对账（评审 P1-1）**：`Report` 增 `boss_dmg: f32`/`weak_hits: u32`（Σ `stats_by_nid`），TR-44.1 双分支断言 `boss_dmg + hp ≈ max_hp`（lose）/ `boss_dmg ≈ max_hp`（win），容差 0.51 = result `Math.round`。
- **D-T44-10 MAPS schema v6 旗标**：`extract-tuning.mjs` 提 `mapBossOk/bossFallbackMap/noBotsStartBlock` 入 `tuning.maps[]`；`tuning.rs` `map_boss_ok`（noBoss∨onlineOnly 皆假）/`boss_fallback_map`（prefer 合格→首个合格→MAPS[0].id）/`no_bots_start_block` 逐字 JS config.js L515-545 + session.js L244-249 语义；`embedded_maps()` 当前 3 项（tidewater/saltpan/cargo），JS 8 项，Task 48 提取后对齐。
- **评审处置**：P0=0；P1-1 已修（D-T44-9）；P1-2 误报（`bot/boss.rs` L673 `mod tests` 4 测试覆盖 TR-44.2 四组：crablet 感知/refill 门/slam 反应门/集火选形）；P1-3/4/5 由本 §8 覆盖（§7 为占位）；P2-1/2-6 误报（`update` 返回 bool；`self.th` 被 L1021 测试读取）；P2-3 不可达边缘（嵌入 tuning 恒非空，注释声明）；P2-4 已注释（Task 45）；P2-5/7/8 留档不修。
- **门禁**：`task44-gates-*.log`（raw-file 捕获形态）——FMT/CLIPPY/TEST/WASM32/TREE/确定性/hitPos 交叉验证/HEADLESS 全 0；TRUNK rc=1×3 尝试（wasm-bindgen OOM SIGKILL，D-T40-1 先例 DEGRADED 非阻塞）；JS 树干净。
- **基线**：`inkwave_sim` 560→572 passed（+12：TR-44.1×2 + TR-44.2×4 + session boss×3 + tuning 锚点×2 + mode hp×1）；`inkwave` 69 passed；全 workspace 零失败。

## Task 45 实施设计（2026-11-04，实读锚点后定稿）

### §0 目标
Boss 联网语义全量接线（协议不改，Rust 对齐 JS）：host tick `msg.B` 发布（`bossN%10` full/delta）、`recBoss(['bm'|'bc'])` 事件时间线、guest `d.B` 快照缓冲 + `_sampleBoss` 播放（Hermite/同 playback 时钟/>12m 断线滑移/阻尼修正）、`'bhit'` 上行 + host `remoteHit` + log 对账、`['bm']/['bc']` 时间线重放、host 迁移 `adopt/handoff`、res boss 形态收发（`bo`/mode/st 六列/boss 胜停钟）。net-harness Rust↔Rust 完整 boss 局（含中途迁移）+ scenario E（JS host boss ↔ Rust guest，DEGRADED 口径）+ 带宽压测（NFR-M4-3）。

### §1 JS 锚点（本窗口实读）
- netmatch.js：`recBoss` L146（`isHost && G.netm===this → _rec(e)`）；`sendBossHit` L150-154（guest-only、`attacker.nid===undefined` 挡、`log.sent++/sentDmg+=d`（raw d）、上线 `r2(d)`、`c` 默认 -1）；`update` L172-188 时序（advance→sample actors→**`_sampleBoss(dt)` L178**→`_playEvents(t)` L179→tick 定时器 `_sendTick`）；`_sendTick` L190-203（`boss=this.match?.boss; if(isHost&&boss&&boss.sim){bossN=(bossN||0)+1; msg.B=boss.pack(bossN%10===0)}`——guest 有 boss 但 !sim 不发布）；`case 'bhit'` L215（host `match?.boss?.remoteHit(d)`）；`_tick` L258-263（`d.B && from===hostId && boss && !boss.sim` → `Boss.unpack(d.B, d.ts)` → `net.buf` push（单调 t 门 + 40 上限 trim））；`case 'bm'` L494（`boss?.onMove(e[2])`——**无 !sim 门**）；`case 'bc'` L496（`b && !b.sim → _crabBurst(e[2..6], !!e[6])`——有 !sim 门）；`_sampleBoss` L504-564（guard `!b||b.sim` return；`peer=peers.get(hostId)`，`!buf.length||!peer||peer.tr===undefined` return；trim `while(buf.length>2 && buf[1].t<tr-0.3) shift`；`at(t,O)`：段查找 s0/s1，Hermite h00=2u³−3u²+1/h10=u³−2u²+u/h01=−2u³+3u²/h11=u³−u²，x/z 用 `h·s + h'·d·v`，y/bt/vx/vz 线性、yaw `angDiff`，返回 0；无 s1：`e=t>last.t ? min(t−last.t,0.15):0`、`s=t>last.t?last:s0`、`O.bt=s.bt+e`、返回 e>0?2:1；离散态 `for k of [hp,phase,f,mv,anim,turn,fl,wk,killer,maxHp,move] S[k]=s0[k]`、`s1&&s1.move→S.move=s1.move`；crabs 按 id 配对插值（x/y/z 线性·u、yaw angDiff·u、hp 取 c1）；修正：handoff 支（`err=prev−S`，`len²>144`→双清零）、prev 跳变支（`at(prevT,P)`、`j=prevRaw−P`（x/z）、`1e-6<j2<144`→`err.x+=jx;err.z+=jz`、`j2>=144`→双清零）；`prevRaw=(S.x,S.y,S.z)`、`prevT=tr`；阻尼 `w=13,k=exp(−w·dt)`，逐分量 `q=v0+w·x0; x'=(x0+q·dt)k; v'=(v0−w·q·dt)k`，`|err|²<1e-6&&|ev|²<1e-4`→清零；`prev=S+err`、`ready=true`）；`sendResult` L650-654（`bo:result.boss`、mode、st 六列 `round(bossDmg||0)/weakHits||0`）；`_result` L656-664（st 行 `bossDmg!==undefined` 才覆盖、`d.mode!=='boss'→m.time=0`（boss 胜停钟）、`result={mode:'boss',coverage,winner,boss:d.bo}`、setState judge）；`onLeave` L681-683（`hostChanged && boss`：新 host `adopt()`、其余 `handoff()`）。
- boss.js：`net` L64（buf/err/errV/prev/ready/handoff——`prevT/prevRaw/_P/fl/wk/crabSeen` 为 _sampleBoss/_follow 惰性挂的 scratch）；`pack` L422-431、`unpack` L432-434（Rust BossSnap 已对齐，Task 41/42）；`onMove` L436（t0 幂等 hz.add）；`_follow` L439-472（Rust `Boss::follow` 已在）；`adopt` L475-480（`!sim→sim=true`、err/errV 清零、`new BossBrain(this,{adopt:true})`）；`handoff` L482（buf 清 + handoff=true）；`remoteHit` L352-359（log.recv/recvDmg、c≥0→_hitCrab(fromNet) 否则 applyDamage(min(d,2000))）；`crabPop` L383-388（recBoss ['bc']）；brain `_start` L294（recBoss ['bm']）。
- bots.js L345：`G.boss` 短路——host 的 bot 经 `BotCtx.boss` 感知（Task 44 已平移 `_bossTick`，联网 host 需把 boss 传入 `drive_bots`）。

### §2 设计
- **Boss 句柄线程化**（沿 D-T44-1 驱动层模式）：`Replication` 不持有 Boss（借用面冲突），`update/send_tick/on_tick/play_events/on_message/on_leave/drive_bots/send_result` 增 `boss: Option<&mut Boss<'_>>` 形参（非 boss 局恒 None，既有调用点机械适配零波及）。`Boss<'a>` 借 `nav/tb/world`（皆 &），与 update 的 `sim:&SimWorld`（&）无冲突；`&mut Boss` 与 `&mut Match` 不同对象。
- **时序对齐**（评审 P1-1 修正定稿）：实驱动序（harness `boss_frame` 与 app `match_link`）= `m.step(...)` → `rep.update(...)`。JS 中 `['bm']/['bc']` 在 Match.update 期间 recBoss 入 `out`、ride **下一** tick（_sendTick 已跑）；Rust 中 boss 事件在 `m.step` 产生，`rep.update` 于 **send_tick 之后** drain `boss.drain_events()`（zone drain/`record_ink` 同位先例）：`Move(rec)→rec(TickEvent::BossMove)`、`Bc→rec(TickEvent::BossCrabBurst)`——本帧记录 ride 下一 tick，逐 JS 等价；FX 事件（Hit/Phase/Splat）**回推** `boss.events` 供 app 层消费者（Task 46），本 drain 只取时间线对（评审 P1-2）。guest `guest_hits`（sendBossHit 队列，点对点直发非时间线）update 开头 drain → `tr.send_to(host_id, encode_direct)`（≤1 帧延迟，D-T35-3 先例注记）。
- **`boss_n: u32`** 入 Replication（JS `this.bossN` L196）；`send_tick`：`is_host && boss.sim → boss_n+=1; b=Some(boss.pack(boss_n%10==0))`。
- **`on_tick` B 消费** ≡ L258-263：`tick.b && from==host_id && boss && !boss.sim` → `net.buf` push `(tick.ts as f64, snap)`（单调 t 门 + 40 trim）。
- **`sample_boss(&mut self, boss)`** ≡ `_sampleBoss`：`BossNet` 扩 `prev_t: Option<f64>`、`prev_raw: Option<Vec3>`（JS 惰性 scratch 显式化）；`peer_tr = self.playback.peer_tr(&self.host_id)`；Hermite `at(t)` 闭包（段查找/三次基函数/angDiff——复用 playback 的 `ang_diff` helper，实读后定）；离散态 + crab 插值填 `net.cur`（BossSnap 形）；修正三段（handoff/prev 跳变/阻尼 w=13）；`follow(dt)` 由驱动层在 `m.step` 内执行（Task 43 接线 `Boss::update` sim=false 分支已调 follow——联网 guest 的 boss 步进即 `m.step(Some(&mut bm))`，bm.update→b.update→follow，无需 replication 直调）。
- **play_events**：`BossMove → boss.on_move(rec)`（JS L494 无 !sim 门，逐字）；`BossCrabBurst → !boss.sim 时 crab_burst(...)`（JS L496 门）。
- **on_message 'bhit'** ≡ L215：`is_host && boss → decode DirectMsg::BossHit → RemoteHit{a,d,weak,c} → boss.remote_hit(&h, attacker_exists)`（attacker_exists=`idx_of_nid(a).is_some()`——JS remoteHit 按 nid 找 actor 传 attacker，实读 Rust `remote_hit` 形参语义后接线）。
- **on_leave** ≡ L683：`host_changed && boss` → `is_host ? boss.adopt() : boss.handoff()`；新 host 的 BossMode 由 app 层已持有（sim=false 构造），adopt 翻 sim 建 brain。
- **drive_bots**：`BotCtx.boss = boss.map(|b| &**b)`（JS L345 G.boss；host 的 bot 打 boss）。
- **res boss 形态**：`send_result`——`r.boss: Option<BossOutcome>` → `mode:"boss"` + `bo` JVal（键序 ≡ bossMode.js L44-52 `{name,win,time,hp,maxHp,phase}`）+ st 行 `boss_dmg/weak_hits` 从 `boss.stats_by_nid`（JS `a.stats.bossDmg||0`——Rust 该统计挂 boss 侧 per-nid，D-T42 先例，取数路径换算注记）；`host_result`——`bo` JVal 解回 `BossOutcome`、`mode!="boss"` 才 `m.time=0`（JS L660 boss 胜停钟）、points [0,0]（JS boss result 无 points，zones 折叠先例）、st 行 boss 列 `bossDmg!==undefined` 才覆盖。
- **app 层（match_link.rs）**：`cfg.mode=="boss"` 时构造 `BossNav::new(&sim,&tuning.boss.nav)` + `BossMode::new(..., sim=!follower, seed)`；每帧 `rep.update(..., boss)`、`m.step(..., Some(&mut bm))`、`drive_bots/on_leave/send_result` 传 `Some(&mut bm.boss)`；host `maxHp` 按 squad_units 公式（JS bossMode L21-22；roster humans/bots 计数 D-T44-4）。
- **测试**：TR-45.1 e2e（replication.rs tests，FakeRelay pair 先例）——①host↔guest boss 局逐 tick：guest `boss.pos/hp/phase/mv` 对 host 真值播放门限内（沿用 Task 25 定标口径：`nfr_m2_2` 的 overlay 门限先例，boss 位差钉 <0.5m 稳态）；②`'bhit'` 收敛：guest 射击 boss → host `log.recv/recv_dmg` 与 guest `log.sent/sent_dmg` 对账（r2 量化容差）；③迁移：host 离开→guest adopt 后 boss 继续步进、hp/pos 无缝（err 清零 + brain 建立断言）；④`['bm']` 重放：guest `hz.moves` 收到 record、hazard 几何与 host 同 t0；⑤res boss：host judge → guest `m.result.boss` 逐项 + 停钟断言。NFR-M4-3 带宽：boss 局 full tick 字节 < 64KB（MSG_MAX）断言。TR-45.2 scenario E：net-smoke.mjs 扩 `--scenario E`（JS host boss ↔ Rust guest wasm，C/D 先例复用 serve.py :8490）；沙箱 DEGRADED 口径如实记录，可断言子项=握手/roster/boss 快照到达（console 计数）。

### §3 实施批次（委派）
- A：sim 接线（BossNet 扩 + Replication 八处 + res 形态）+ 单元/集成测试（TR-45.1 ①-⑤ + 带宽）——核心批次。
- B：app 层 match_link.rs boss 驱动 + inkwave crate 适配——依赖 A 签名。
- C：net-smoke.mjs scenario E——独立（仅工具层）。
- 主上下文核验 → 独立评审 → 修复 → 九段门禁。

### §4 测试
- TR-45.1 harness e2e 全绿（①-⑤）；既有 572 基线不破（boss 参 None 机械适配）。
- TR-45.2 scenario E 实跑 dump（DEGRADED 如实）+ 带宽压测断言。
- 基线：inkwave_sim 572→≥600、workspace 递增；`cargo tree -p inkwave_sim` 依赖面不变；wire.rs 协议零改动（仅消费面）。

### §5 风险
- `_sampleBoss` 的 `at()` 借用冲突（Rust 闭包读 buf 又写 O）——以本地函数 + 输出结构体实现，行为逐式。
- guest 局 `m.step` 内 boss.update→follow 消费 `net.cur`——实驱动序 step→update 使 follow 消费**上一帧** update 填入的 cur（JS 同帧 sample→follow）：16ms 滞后在播放缓冲语义内良性（评审核过），注记即可。
- res `bo` JVal 键序与 bossMode.js L44-52 逐字（fixture 已钉 wire 形，Task 41）。
- trunk 段沙箱 OOM 持续 → DEGRADED 口径沿用。

### §6 验收
九段门禁全绿（TRUNK DEGRADED 口径同 D-T40-1）；TR-45.1/45.2 全绿；JS 树零改动；基线递增；wire.rs/协议零改动。

### §7 定案（实施后回填 §8）

### §8 定案（Task 45，2026-11-04）
- **D-T45-1 Boss 句柄线程化**：`Replication` 不持 Boss（借用面冲突，沿 D-T44-1 驱动层模式）；`update/play_events/send_tick/on_tick/on_message/on_leave` 增 `boss: Option<&mut Boss<'_>>`、`drive_bots/send_result` 增 `Option<&Boss>`（只读即可）、`host_result` 增 `bo: Option<&JVal>`。非 boss 局恒 None——572 基线逐字节不变（inkwave_sim tests 103 处 + special/tests.rs 1 处 + match_link 22 处 + net-harness 7 处机械补 None）。
- **D-T45-2 时序定稿（评审 P1-1 修复）**：实驱动序 = `m.step` → `rep.update`（harness `boss_frame` 与 app `match_link` 两路一致）。host `recBoss(['bm'|'bc'])` drain 置于 **send_tick 之后**（zone drain/`record_ink` 同位先例）——本帧 step 产生的事件 ride 下一 tick，逐 JS 等价（JS recBoss 在 Match.update 期间入 out、ride 下一 _sendTick）；首版误置 update 开头（早乘一 tick），评审发现后当场修正，§2 论述同步更正。guest `guest_hits` 直发 drain 留 update 开头（点对点非时间线，≤1 帧延迟，D-T35-3 先例）。
- **D-T45-3 FX 事件回推（评审 P1-2 修复）**：host drain 只取时间线对（Move/Bc），`other => b.events.push(other)` 回推 Hit/Phase/Splat 供 app 层消费者（Task 46 fx/UI 不饿死）。
- **D-T45-4 BossNet 形态**：`prev: Option<BossSnap>`→`Option<Vec3>`（JS `n.prev` 是 Vector3，`_sampleBoss` 只读 x/y/z L546-547；评核实核 follow() 仅消费 cur/err，无破坏）；新增 `prev_t: Option<f64>`、`prev_raw: Option<Vec3>`（JS 惰性 scratch n.prevT/n.prevRaw 显式化）。
- **D-T45-5 `sample_boss` ≡ `_sampleBoss` L504-564**：Hermite 基函数（h00/h10/h01/h11、`d=max(1e-3,Δt)`）、y/bt/vx/vz 线性、yaw `ang_diff`（复用 playback helper ≡ JS L38）、外推 `e=min(t−last.t,0.15)`、离散 11 键自 s0 + move 覆盖、crab 按 id 配对插值（hp 取 c1）、三段修正（handoff `>144` 归零 / prev 跳变 `1e-6<j2<144` 仅 x/z / 阻尼 w=13 临界式）、`prev=S+err`。评审逐式全对齐（0 P0）。
- **D-T45-6 发布/消费门**：`send_tick` `boss_n.is_multiple_of(10)` full（≡L196；guest !sim 不发布不计数）；`on_tick` B 消费 `from==host_id && !b.sim` + 单调 t 门 + 40 trim（≡L260-262）；`play_events` 'bm' 无 !sim 门（≡L494）、'bc' 有门（≡L496）；`on_message` 'bhit' is_host + `remote_hit(&h, attacker_exists)`（attacker_exists=`idx_of_nid(a).is_some()` ≡ JS 按 nid 查 actor）。
- **D-T45-7 迁移**：`on_leave` host_changed 时 `is_host ? adopt() : handoff()`（≡L683）；adopt 翻 sim 建 brain 清零 err（Task 42 实体面），handoff 清 buf 置旗标。
- **D-T45-8 res boss 形态**：`send_result`——`bo` JVal 键序 ≡ bossMode.js L44-52 `{name,win,time,hp,maxHp,phase}`、mode 推 "boss"、st 行 `boss_dmg/weak_hits` 自 `stats_by_nid`（JS `a.stats.bossDmg` 挂 actor——Rust 该统计存于 boss 侧 per-nid，D-T42-4 先例，取数路径换算）；`host_result`——`bo` 解回 `BossOutcome`、`mode!="boss"` 才 `m.time=0`（JS L660 boss 胜停钟）、points [0,0]（JS boss result 无 points，zones 折叠先例）、st 行 `bossDmg!==undefined` 才覆盖（JS L659）、不回写本地 stats（审计留 boss 侧）。
- **D-T45-9 app 层 BossMode 跨帧持有不可行（批次 B 实读借用面）**：`Boss<'a>` 持 `nav/tb/world` 三借用（mod.rs L478-481）；`SimWorld<'a>` 不含 tb（actor.rs L80-86 仅 collision+Copy 字段）；Bevy `Resource: 'static` 且 `NetMatchState`/`DemoSim` 互借在系统签名下无法表达；每帧重构造丢 Boss 全状态且 sim 无 save/restore API。未做 unsafe、未动 sim——**app 层 boss 实体驱动（含 host maxHp squad 公式接线、drive_bots/on_leave/send_result 传 Some）留 Task 46**；本任务 inkwave crate 全 None 适配、非 boss 局行为逐字节不变。
- **D-T45-10 nettest boss 放行**：`netmode` 门放开 `turf|zones|boss`（经 `SessionCmd::SetMode`→`set_settings`，session.rs 已接受三模式 + noBoss 图回退）；`tr39_parse_loadout_knobs` 改断言 `netmode=boss` 透传、`netmode=duel` 废值（无新门洞）。
- **D-T45-11 scenario E（批次 C）**：net-smoke.mjs `bossScenario`（C/D 先例复刻）——JS host `setSettings({mode:'boss'})` ↔ Rust guest wasm；断言面=握手（`[nettest] loadout` 行含 `mode=boss` echo）+ roster 跨认 + **host 侧 `__G.netm.bossN>0`**（sim 层已消费 B，但 wasm seam 传 None 无 guest 探针——评审 P2-2 后注释更正为 D-T45-9 口径；guest B 到达留 REVIEW_CHECKLIST）。沙箱实跑：Rust guest tab 被 SwiftShader wgpu 杀死 → **DEGRADED ok=true**（B 场景同历史表现，未伪造 PASS），dump `rust/tools/verify/logs/net-smoke-2026-11-04T08-19-23-*.json`。
- **D-T45-12 测试定标**：TR-45.1 ① 路径收敛取「近 120 帧最优差 <0.5m + 末帧 <6m」（播放延迟 85-300ms × boss 速度，12m 断线滑移准下 6m 可接受，评审 P2-4 在案）；② bhit 对账（log.recv/sent、recv_dmg/sent_dmg、host hp 差、stats_by_nid 归属）；③ 迁移无缝（sim/err/brain/follower/pos 连续/bt 推进六断言）；④ ['bm'] 重放逐条 (t0,id) 相等；⑤ res 结算逐项 + 时钟不钉零；NFR-M4-3 最坏满快照（8 小蟹+move_rec）tick <65536B。harness 注：guest actor 需传送至 boss 旁 3m + invuln（bossBrain L112 pad 排除会跳过蹲出生点的待机 actor，否则 boss 永不索敌）。
- **D-T45-13 评审处置**：0 P0；P1-1 时序（drain 移 send_tick 后 + spec §2/§5 更正 + 注释重写）、P1-2 FX 回推——当场修复并复验；P2-1 本节回填、P2-2 net-smoke 注释更正、P2-3 注释行号 L544→L546-547、P2-4 门限在案（不改）。
- **基线**：`inkwave_sim` 572→**578** passed（+6：tr45_1_1~5 + nfr_m4_3）；`inkwave` 69 passed；workspace 675 全绿；`cargo tree -p inkwave_sim` 依赖面不变；wire.rs/JS 树/server/ 零改动。

## Task 46 实施设计（2026-11-04，实读锚点后定稿）

### §0 目标
Boss 呈现占位档（FR-M4-7）：bevy 体块剪影（集装箱壳/双钳/眼柄/步行腿，st 契约驱动）+ telegraph 标记（slam 环/barrage 目标圈+桶/sweep 扇形+光束/charge 车道/frenzy 圈）+ crablet 体块 + HUD boss 条（name/HP/phase notches/stun OPEN/伤害 chip）+ intro title card + 结算 boss 形态（VICTORY/DEFEAT/bossDmg/weakHits）+ bossAudio 占位。同时收口移交项：**D-T45-9**（app 层 boss 实体驱动——Boss owned 重构使 Bevy Resource 可跨帧持有）、**D-T44-5**（离线 Play 菜单 boss 选择）、**D-T43-8**（HazardFX 呈现档）、**D-T45-3**（boss FX 事件 app 消费）、**D-T45-11**（wasm seam guest 侧 boss 实体驱动探针）。只求「可玩且可读」；高保真模型 Non-Goal。

### §1 JS 锚点（本窗口实读）
- `bossModel.js` L71（st 契约初值形态：speed/turn/move/moveT/movePhase/phase/stunned/hurt/dead/aim）、L76-123（update：phase 变色/crack/enrage/dead 压平/hitShapes active 规则）、L175-242（Crablet：tumble 0.55s/三脚架腿/snap 钳/pop 0.14s 肿胀+闪+墨 burst）。
- `bossHazards.js` L357-600（HazardFX.draw：slam tele 目标圈 `fx.mark(HZ.slamR, k=u/tele)` + 冲击环 `ringRadius(m,k,u)`/`ringStart` 逐 ring、reach 裁边；barrage 目标圈（`min(tele*0.4, b[3]-0.5)` 起、land 脉冲）+ 桶下落 `barrelPos(p,b,k)`；sweep 扇形 fan（lo→hi、radius=min(beamLen,reach)）+ 光束 `beamAngle(m,u)` 圆柱；charge 车道 plane（`chargeDist` 前移、`p.L+4.2-s` 长、5.0 宽）；frenzy 全圆 fan（frenzyR）+ 旋转喷射（粒子，占位档可省））。
- `hud-boss.js` 全 524 行：S 状态机（`_fresh` L45-49：hp/shown/chip/chipHold/notch break/stunT/shieldT/introT 填充曲线 L300-314、chip 滞后排放 L311-313）、`_hp` L162-181（chip 新段条件 `chip<=shown+0.003`→hold 0.6s、notch 跨界 is-past/is-break）、`_writeBar` L336-346（量化 4000 档、pct ceil、low ≤0.2）、`_move` L183-195（tele/act/rec callout 生命周期）、`_stun` L232-241（openT=stunDur、OPEN!）、`_phase` L243-255（pips/shield 1.8s/phase3 cracked emblem/banner 文案 `PHASE 2!`/`SHELL CRACKED!`）、`_defeat` L257-266（SUNK! 1.3s 后 is-gone）、`_intro` L152-160（title card + shown 从 0 填充）、`_ending` L474-494（SUNK! / TIME'S UP!）、`prompt` L278-282、`squadTeams` L284-289。
- `boss-art.js` L19-24（BOSS_NAME/BOSS_EPITHET/MOVE_LABELS：SLAM!/INCOMING!/SWEEP!/CHARGE!/BROOD!/FRENZY!/OPEN!）、L136-157（MOVE_ICONS 六招+open）。
- `bossMode.js` L44-52（result boss 形态 `{name,win,time,hp,maxHp,phase}`——res 数据面 Task 45 已通）。
- `bossAudio.js` L84-113（move×phase→SFX 全表：slam:tele→boss_tele、slam:act→+0.12s boss_slam、barrage:tele→boss_tele pitch1.12、barrage:act→n=3+rand3 组 whistle/barrel 链、sweep:tele→charge 循环、sweep:act/rec→loop 开关、charge:tele→tele+0.2s roar、charge:act/rec→gallop loop、crablets:tele→pitch1.25、crablets:act→chitter+0.3s、frenzy:tele→roar、frenzy:act/rec→frenzy loop）、L114-176（hit/crit 去重门 0.06/0.07/0.3、stun→crash+0.35s dizzy loop、phase→boss_phase+roar、defeat→duck+sunk 1.35s、foot/impact/fx bonk/hatch、crablet spawn/pop pitch killed?1.1:0.85）、L9（TRACK 表 1/2/3→boss/boss_2/boss_3）。
- `weapons.js` L1650-1665（`_bossImpact`：volley 去重 key=crab|boss、drop 距离衰减 `lerp(dmg,dmgFar,d/7)`、roller flick 0.3s ×0.12、`G.boss.hit(owner,dmg,target,wid||type,at)`）+ L272/L616/L1262（roller/brush/charger 直连）+ splash 族调用点（weapons L1506/1708/1781、subs L566/582、specials L77、kits 各件）。

### §2 设计
- **D-1 Boss owned 重构（批次 A，解 D-T45-9）**：`Boss<'a>`→`Boss`：`nav: BossNav`/`tb: tuning::Boss`/`collision: CollisionWorld`+`layout: StageLayout`（方法内局部构 `SimWorld::new`）；`BossMode` 随之 'static。netmatch/autopilot/match_/bot.rs/tests 机械适配，行为零变化、578 基线钉死。app 层 `DemoSim`/`NetMatchState` 因此可持 `bm: Option<BossMode>`。
- **D-2 弹丸→boss 接线（批次 A）**：武器管线接 `boss.hit/splash/rain` ≡ JS `_bossImpact` 语义（volley 去重/drop 衰减/flick 窗口逐式）+ roller/brush/charger 直连；boss None 时逐字节不变。难等价件如实登记顺延。
- **D-3 app 层 boss 局驱动（批次 B）**：`DemoSim` 增 boss 模式——`replay_boss(opts)`：squad 8 建队（match 已支持 BossMode::with_roster_mode）+ `BossMode::new`（units=squad_units(1,7) ×hpDiff 公式 ≡ bossMode.js L21-22）；`step_once` 传 `Some(&mut bm)` + `BotCtx::boss Some`；boss 事件（`BossEvent` 流 + FX 回推 D-T45-3）导出到 `BossHudEvents` Resource 供 HUD/音频消费（每帧 drain）。离线 Play 菜单：mode cycle 第三档 boss → `replay_boss`（D-T44-5）。nettest wasm seam：host 侧 bm 驱动 + guest B 探针（D-T45-11 收口，沙箱 DEGRADED 口径不变）。
- **D-4 呈现实体（批次 C，world.rs/新 boss_render.rs）**：体块剪影=集装箱壳（box）+双钳（box 组）+眼柄（柱+球×2）+步行腿（4-6 柱），变换读 `BossSt`（speed/turn 步频摆动、move/movePhase 钳举/俯冲、stunned 倾斜、dead 压平 ≡ bossModel.js L86-101 简化档）；位置=boss.pos/yaw。telegraph 标记=每帧读 `hz.moves` 活跃集（`hz_current` 等价查询）→ 平面 mesh（圈=Circle 描边档、环=按 ringRadius 缩放 Circle、扇形=triangle fan、车道=Rectangle、桶=小圆柱 barrelPos）；颜色 boss ink（team1）。crablet=体块（壳+腿简档）+ pop 0.14s 肿胀缩放 ≡ Crablet L222-234。
- **D-5 HUD boss 条（批次 C）**：纯函数状态机 `BossHudState`（S 字段全表 ≡ hud-boss.js `_fresh`/`_hp`/update 缓动/chip 排放/notch/pips/stun/phase/intro 填充）+ 事件输入（BossEvent 流）→ 单测钉值（TR-46.1 纯函数档）；渲染=文本/矩形占位档（name/百分比/pips/stun OPEN!/callout 文案 MOVE_LABELS）；title card（BOSS_NAME 字卡 ~3.2s ≡ _titleCard）+ phase banner + SUNK!/TIME'S UP! ending；结算 boss 行（VICTORY/DEFEAT + bossDmg/weakHits chip，数据=res BossOutcome+stats）。
- **D-6 bossAudio 占位（批次 C）**：M1 占位档风格——事件→SFX 名映射表 ≡ bossAudio.js 全表（含去重门/延迟链/loop 开关的触发计数），无真合成（Phase C Task 50 替换）；触发计数单测。

### §3 实施批次（委派）
- **批次 A**（sim）：D-1 + D-2。基线 578/69/675 全绿钉死。
- **批次 B**（app 驱动）：D-3。依赖 A。headless 600 帧 boss 局不 panic。
- **批次 C**（app 呈现）：D-4/D-5/D-6。依赖 B（事件流+状态面）。
- 评审：独立 agent 对照 §1 锚全量核。门禁九段（TRUNK DEGRADED 口径）。

### §4 测试
- TR-46.1：无头冒烟（headless 600 帧 boss 局不 panic + 结算到达）+ wasm 加载冒烟；`BossHudState` 纯函数单测（hp 缓动/chip 排放/notch 跨界/pips/stun OPEN 时长/phase 门/intro 填充曲线逐值钉）；弹丸→boss 接线测试（批次 A 内）；bossAudio 事件→触发计数表测试。
- TR-46.2：REVIEW_CHECKLIST 登记 GPU 视觉项（剪影轮廓/telegraph 可读性/HUD boss 条布局/title card/结算行/crablet pop）。

### §5 风险
- owned 重构波及面大（netmatch/autopilot/tests）——机械适配+基线钉死兜底。
- 无 GPU：呈现实体只验「spawn 不 panic + 几何求值正确」，观感全走 REVIEW_CHECKLIST。
- 弹丸接线若与 Rust 武器管线形态冲突（kits 各件），如实登记顺延不硬凑。
- DemoSim boss 局确定性：boss 时钟 bt 与 match 时钟双轨（Task 45 res 停钟已处理），无头冒烟断言到达 finish。

### §6 验收
AC-M4-4（boss 呈现部分）：可玩（弹丸掉血/胜负可达）+ 可读（剪影/telegraph/HUD 条/结算）+ 双轨口径。

### §8 实施后定案（2026-11-04，批次 A/B/C + 独立评审 0P0/4P1/4P2 全处置）
- **D-T46-1 owned 重构（D-T45-9 解）**：`Boss`/`BossMode` 去生命周期——`nav: BossNav`/`tb: tuning::Boss`/`collision: CollisionWorld` 按值持有（`Boss::new(world,&nav,tb,cfg)` 构造时 clone），方法内局部构 `SimWorld::new`。适配 netmatch/autopilot/match_/bot.rs/tests/inkwave 全调用点，行为零变化（578 基线钉死）。Bevy Resource 可跨帧持有 BossMode。
- **D-T46-2 弹丸→boss 接线**：管线注入 `BossRef = Option<&mut Boss>`（None=非 boss 局逐字节不变，578 基线背书）；接线点 ≡ JS 锚：step_one `seg_hit` 优先序（weapons.js L1621-1624）、`boss_impact` drop 衰减 `lerp(dmg,dmgFar,d/7)` + flick 0.3s×0.12（新字段 `Actor::flick_boss_t`，L1651-1665）、blast_burst `direct!=='boss'` 才 splash（L1708）、bomb 弹开+explode splash wid='bomb'（L1719-1727/L1506）、charger 光束截断（L1258-1263）、roller/brush 直连（L268-273/L612-617）、rain（world.js L1776）。**wid 坍缩**：Rust 弹丸无 `p.wid` → "blaster"/"drop"/"shot"（BOSS.weapon 表 boss.js L22 无此三键，倍率全 ×1，行为等价）。**未接线（顺延注记）**：subs/specials 直伤链（waddle/torpedo/shaker/sticky/seeker/burst——specials.js 链非 weapons.js 弹丸管线）、kits bow/mitts/brolly 近战直伤；`p.vol` volley 去重与 slosh-head 经评审核实不可达（`p.vol`/`head` 仅退役 slosher 设置，JS `_bossImpact` 对 bucket 本不去重）。新增 `weapon_wiring_tests` 4（weak ×2.5=75 钉值/flick 比值/charger 全额/crab 分支）。
- **D-T46-3 DemoSim boss 面**：`bm: Mutex<Option<BossMode>>`（Mutex 非为竞争——`BossNav` 的 A* scratch `RefCell/Cell` 使 BossMode `Send` 非 `Sync`，Bevy Resource 需两者；恒单系统持锁）+ `boss_events: Vec<BossEvent>` 每帧 drain（≡ JS `emit('boss:*')` 总线，D-T45-3 收口）。`replay_boss`：roster 8 槽 + maxHp ≡ bossMode.js L21-22（`round(18000×(1+7×0.6)×hpDiff)`，三档钉 93,600/70,200/121,680）；`step_once` 锁内推进 + `BotCtx::boss Some`；turf 路径 bm None 逐帧不变（评审核实）。
- **D-T46-4 离线菜单**：新增 MODE 按钮两档 cycle `turf→boss`（`Action::ModeCycle`）+ 难度固定 normal（bossMode.js L23 默认）。**两档声明**：JS 三卡含 zones（menus.js L954-963，boss 卡 beta+NEW!），但离线 zones demo 为 M3 起缺口（DemoSim 无 zones 路径），zones 卡随离线 zones demo 落地再开。
- **D-T46-5 在线 boss 局**：`match_link::start_net_match` `cfg.mode=="boss"` 双端构造 BossMode（≡ netmatch.js L164-171；`sim: !m.follower` guest 纯跟随插值；units=`squad_units(humans,bots)`）；nettest guest 侧 `[nettest] boss B recv=<n>` 探针（**D-T45-11 收口**；JS smoke 未 grep 该行——tools/ 冻结，登记台账）。
- **D-T46-6 剪影**：壳 `Cuboid(2.44,2.6,6.06)` 长轴沿 z（CONT hl=3.03 沿 z，bossModelGeo.js L274/L436——评审 P1-1 修正横放错误）+ pitch 绕 X；双钳 ARMS W（±2.02,1.05,4.3）；眼柄 stalk 柱（0.11,0.42）+ 眼球 r0.28（L311）；6 腿柱（feet z 3.55/0.3/−3.45、x ±3.4，L281-283）；st 驱动简档：speed 腿摆/turn 倾/tele 钳举/slam act 俯冲/stunned 垂/dead 压平 0.35/hurt 闪/phase≥3 红移（bossModel.js L86-101）。
- **D-T46-7 telegraph 标记**：slam 圈（`slam_r`+tele 进度 k）/环带（`ring_start`/`ring_radius`/alpha 15-19 软边）；barrage 圈（`barrel_show`/`barrel_mark_k`）/桶（`barrel_pos`，Cylinder 0.42/0.475 ≡ L423）；sweep 扇（`sweep_span` fan 顶点，y=record `y`+0.05 地面——评审 P1-2 修正悬空；alpha `0.55+0.45·min(1,k·1.5)`——P1-4）/beam（`beam_angle`，长 3D hypot 含 `p.y+0.3−p.oy`——P1-3）；charge 车道（`charge_dist`、`max(0.5,L+4.2−s)`、宽 5.0）；frenzy 全圆（`frenzy_r`）。**占位简化声明**：环/扇无墙体裁边（`hz_reach`/`hz_beam_len` 需 `&mut` per-move 缓存，只读锁不可用；beamLen 常数 17）。
- **D-T46-8 crablet 体块**：crabs→小壳+4 腿；pop 肿胀 `1+0.45u²`/0.14s 后隐藏（bossModel.js L222-234）。非 boss 局零实体（bm None 一次锁检查返回）。
- **D-T46-9 BossHudState**：纯函数 S 机（ui/boss_hud.rs，零 Bevy）——JS `S` L45-49 全字段 1:1 + intro 填充/chip_hold 0.6s/notch [⅔,⅓] 下穿锁存/`_writeBar` 4000 档量化/OPEN! 倒计时/PHASE 2!·SHELL CRACKED! banner/title 3.2s/ending 5.2s/is-gone 1.3s/SUNK!。**Move callout 帧间边沿合成**（sim `Move` 每 move 一事件 ≠ JS per-phase——D-T43-5 预告落地）。未移植 DOM 效应：tele hover cue（L191）/`is-zero` 闪（L180），模块头声明。
- **D-T46-10 BossAudioDirector**：占位档事件→`BossCue` 记录表 ≡ bossAudio.js L84-176（move×phase 15 行 + intro/hit·crit 三门 0.06/0.07/0.3/stun→crash+dizzy `at:0.35`（评审 P2-3 修）/phase/defeat duck+sunk 1.35s/crablet spawn·pop pitch killed?1.1:0.85/TRACK 1/2/3 + playing/ended 边沿 Music 切换）；barrage 随机用确定性 LCG；**dead 门**（L85/L115/L124，评审 P2-3 补）。未移植：foot/impact/fx cue（sim 未发对应事件）；loop maxDur 用 JS fallback 常数 2.7/2.3/3.3。
- **D-T46-11 结算 boss 行**：results a 行加 PHASE；stats_rows boss 模式 per-player dmg/weak/splats 列 + `BOSS DMG Σ WEAK Σ` 总计行（≡ main.js L960-961 + menus.js L4031-4040；离线 nid==slot，match_.rs L1432）；非 boss 格式不变。VICTORY/DEFEAT ≡ bm.result()（bossMode.js L44-52）。
- **D-T46-12 评审与处置**：独立评审 0 P0/4 P1/4 P2——P1 全修（壳长轴/sweep 扇悬空/beam 长 y 分量/alpha ×1.5 斜坡）、P2 全修（Cylinder 参数序 stalk/leg/barrel、head z 1.55、腿 z 引脚值、dizzy 0.35 延迟、dead 门、hover/is-zero 声明）；评审自核 volley/slosh 不可达声明属实、maxHp 三档钉值、_step 优先序、HUD 全时序公式、bossAudio 全表。
- **D-T46-13 门禁复跑收口**：TRUNK 首跑 rc=1 实为真实编译错（震屏段 `elapsed_seconds`≠Bevy 0.19 `elapsed_secs`，E0599）——非 D-T40-1 的 wasm-bindgen SIGKILL，不适用 DEGRADED 口径；修复后 attempt 1 即成功。TEST 段暴露概率性 flake `run_boss_lose_branch`：`stats_by_nid` 为 `HashMap`，实例级随机种子使迭代序异 → f32 求和序异 → 同种子两跑 `boss_dmg` 差 0.004（逐玩家值确定，故 result JSON 比对先过）。修复=按 nid 排序后求和（autopilot.rs TR-44.1 对账段）；复验 30/30 + workspace 711 全绿。
- **headless 冒烟**：`--headless --mode boss` 600 帧不 panic（bt≈10、hp 满、events≥3 含 Intro/Move——无玩家输入不掉血，符合预期）；turf 回归 exit 0。
- **基线**：`inkwave_sim` 582（578+4 接线）/`inkwave` 101（74→+8 C1+19 C2）/workspace **711** passed 全绿；`cargo tree -p inkwave_sim` 依赖面不变；JS 树/wire.rs/PROTO/fixture 零改动。
- **九段门禁**：`rust/tools/verify/logs/task46-gates-20261104T174853Z.log` — FMT/CLIPPY/TEST(**workspace 711 passed**=675+36，inkwave 101/inkwave_sim 582)/WASM/TREE/DETERM/XCHK(10/10)/TRUNK(**attempt 1 成功 rc=0**，D-T46-13 的 `elapsed_secs` 编译错修复后不再适用 D-T40-1 SIGKILL 口径)/HEADLESS(turf 600 帧 + boss 600 帧 双跑)/JS_TREE_CLEAN **全绿**。

## Task 47 实施设计（2026-11-05，实读锚点后定稿）

**范围**（tasks.md L156-165，AC-M4-3，纯 sim）：threat AI 全家（bots.js L1260-1502）+ stamp threat-steer（L460-470）+ `_shieldFight`（L1507-1549，fight 内 L525 接线）+ `_canopyCtl`（L1554-1615，step L623-626 接线）+ tail snappy 参数（L630/L643-645）+ mitts/blade 深战术段（blade.js `botTactics`、mitts.js L762-831 fight/paint hooks）。boss 段仅回归确认（Task 44 已全量）。

- **D-T47-1 模块布局**：新建 `bot/threat.rs`（`impl Bot`：threat_scan/threat_score/threat_ctl/thr_end/thr_drift/thr_evade/thr_sidestep/pick_evade/melee_window/dev_trigger/shield_fight/canopy_ctl + `fat_los`/`dry_line` 探针）；`bot.rs` 加字段与接线。JS `_thrAim` 共享可变对象 → Rust `struct AimHold{yaw,pitch,dist}` 按值返回 `Option<AimHold>`。
- **D-T47-2 设备视图**：`sub.rs` 加 `pub struct ThreatView`（kind: ThreatKind{Waddle,Torpedo}、team、pos、aim_y（waddle=MID=0.11·1.8=0.198；torpedo=0）、radius、trigger（waddle=sb.trigger_dist；torpedo=HIT_R(0.3)+t.radius）、speed（sb.speed / sb.launch_speed）、sense_radius（sb.sense_radius / sb.lock_range 视 kind）、lock_range: Option<f32>、ground: bool、live、state: ThreatState{Fly,Sense,Wake,Walk,Unfold,Launch}、hp、shootable、locked、target: Option<usize>、vel（waddle: walk&&!air→heading·speed（sim 无 hop，D-T34-7 已删，恒按非 hop）；fly||air→vel；else 0。torpedo: launch→dir·speed；fly→vel；else 0）、left: Option<f32>（waddle: walk→min(life-walk_t,(max_travel-travel)/speed)；else life。torpedo None=JS undefined，`d.left!==undefined` 分支跳过））。`SubSystem::threats(&self) -> Vec<ThreatView>` 枚举 waddle+torpedo 列表（含 ghost，JS threats 不过滤）。Bot 跨帧持 `ThreatRef{kind, owner_idx, gid}`（gid 仅同 owner 唯一，键须含 owner_idx），每帧经 `find` 重解析；解析失败=JS `!d.live`。`_thrMem`：JS Map(对象) → Rust `HashMap<(u8 kind, usize owner_idx, u32 gid), f32>`（仅 get/contains/retain，无顺序敏感求和）。
- **D-T47-3 盾面视图**：`BotCtx` 加 `pub launched: &'a [LaunchedCanopy]`（7 个构造点同步：autopilot.rs L150/L333、bin/ink-atlas-check.rs L52、net/replication.rs L2395、bot/boss.rs L805、bot/tests.rs L131/L1426）。JS `this._shl` 别名活对象（pose 每帧刷新）→ Rust 存 `ShieldRef{Held(usize actor_idx)|Launched(usize)}` 于 scan 时重建（滤己队，JS L1277），`shield_fight`/`canopy_ctl` 每帧对 ctx 重解析出 `ShieldView{held, team, c, n, r, pos, half_w, plane_off, speed, hp, live, left, blocks_actors}`：held=`held_shield(kit,a)` gate + kit.hp + R=HELD_R + left=0 + speed=0 + blocks_actors=false；launched=`cur_c/cur_n` + pos(foot) + half_w=LAUNCH_R + plane_off=-canopy_depth()·0.5 + speed=weapons.brolly.launch_speed + left=max(0, life-t) + R=LAUNCH_R + blocks_actors=true；解析失败=not live。`canRef` 跨帧=ShieldRef 比较。
- **D-T47-4 THREAT_AI/STATS**：`bot/threat.rs` 内 `static THREAT_AI_ENABLED: AtomicBool=true` + `pub fn set_threat_ai(bool)`（测试 A/B 开关，JS L52）；`THREAT_STATS`（JS L53：noticed/shoot/evade/swim/sidestep/gone/steer/breakWall/flank/holdFire(f32 秒)/shieldSub）→ `thread_local! RefCell<ThreatStats>` + `threat_stats()/clear_threat_stats()`（不进快照/不进 JSON 基线，仅测试断言）。
- **D-T47-5 探针**：`dry_line`（JS L779-783，0.45m 采样 `wet`——Rust `wet` 已有 L3650）；`fat_los`（JS L1781-1793：中心+±0.34 肩线、y+0.45、`world.raycast(o, dir, len-0.05, walk=true)`——签名按 collision.rs 实测定，brolly 用法 `raycast(pos,dir,d,true)` 第 4 参即 walk 挡格）。`G.physics.los` → `collision.rs L394 pub fn los(a,b)`。`regionStats` → `paint.region_stats(world,x,y,z,1.3,team)`。
- **D-T47-6 接线**：step L918-923（special_ctl 后、tail 前）：`threat_ctl`→`canopy_ctl(free=thr_aim.is_none())`→合并→覆盖 want_yaw/pitch→`tail(..., thr_aim.is_some(), enemy_visible)`；tail `snappy = fighting || thr_aim`（JS L643-645，om/turn=diff.aim_omega/aim_turn vs 8/6）。fight 内 dodge 块后（JS L525 位）：`if enemy_visible { shield_fight } else { block_t = 0 }`。stamp threat-steer：fight move 段（twins planted 块后、归一化前，JS L460-470）——`t.special_active.id=="stamp"` 时 bodyYaw 取 `KindState::Stamp(st).body_yaw`（读法仿 bot.rs L2946），锥 `cos(1.25)`/dist<10 侧移、dist<4.5 非 melee 后退 0.4。
- **D-T47-7 threat_ctl 依赖核对**：`a.special_active` gate（kraken/stamp/crab/jetpack/zipcaster by id||kind；`a.climbing`——sim actor 无 climb 状态则恒 false；`a.super_jump.is_some()`）；`moveOnly=!!sp`；`wr` 需 charging/charge/burst_t/streaming/cooldown + bow ring1（`weapons.bow` 字段名实测）；`KB?.melee/charges` → `kind.is_melee()/is_charge()`（Task 38 家族表已覆盖 blade/mitts/bow）；duel 段 `weaponRange(T.weapon)` → `ctx.weapon_reach(target kind)`；`it.fire` 消费后 `thrFiring = fire && off<tol`；`noProg/bestD` 复位（Rust 同名字段）。JS `_climbAim` gate 恒 null（无 climb）。
- **D-T47-8 mitts/blade 深战术**：blade.js `botTactics`（pre-charge/flank/swim-in/strafe + `_blHold` + `botState{waveT,press}`）与 mitts.js L762-831（cling 下打/escape leap/pounce/wall leap + paint travel pounce + `botState{leapCd,wallCd,escapeCd,travelCd}`）。前提=sim 的 mitts/blade runner 已有 leap/cling/wave 原语（Task 32/38 移植面）——实现前实读 `weapon/mitts.rs`/`weapon/blade.rs` 表面 + blade.js `botTactics` 全文；scratch 字段按 bow 先例挂 brain（`bot_want/bot_jump` 模式）。若 runner 缺原语（如 `botLeap` 的 bot 驱动入口），按 JS 补 sim 侧函数。
- **D-T47-9 测试**：TR-47.1 固定种子行为断言（threat 优先级/回避/掩体选择逐段）——构造小场景直驱 `Bot::step`：waddle 追猎→noticed→shoot/evade 计数；torpedo launch→sidestep；canopy 夹道→flank/steer/breakWall；stamp 正面→侧移；tail snappy 参数生效（aim 收敛更快）。THREAT_AI.enabled=false 时 threat 段全零（A/B 对照）。tr38 难度 ladder 回归不破；`inkwave_sim` 基线 582 单调递增；workspace 711 → 新基线。RNG：所有 `Math.random()` 走 bot 自有 `rng` 流、use-site 抽取（既有偏差口径），新增 draw 改变 bot-run 录制基线→按 Task 38 batch-2 先例更新。
- **D-T47-10 工作流**：批次 A=视图/注册表/ctx 字段/STATS；B=threat AI 核心+接线+tail；C=shield/canopy/stamp；D=mitts/blade；E=测试+评审+门禁（仿 /tmp/gate-task46.sh → rust/tools/verify/gate-task47.sh，日志 logs/task47-gates-<TS>.log）+台账+提交 `M4 Task 47: ...`。JS 树/server//wire.rs 零改动；`cargo tree -p inkwave_sim` 依赖面不变。
- **D-T47-11 zones catch_up 时延补偿（JS 无对应，待用户评审）**：Task 47 批次 B-D 的 bot 行为改动使 `tr37_crossmarket_zones_three_clients` 回归：host 在帧 N 广播 zone 记录，follower 在帧 N+5 才收到并应用，而 follower 本地 `penalty` 预测按本地 paint 即时算，两侧对同一帧的 `pen` 差 0.083（=5/60，复现证据 frame 6913）。修复：`match_/zones.rs` 的 `'zo'` field2 / `'zs'` field10 随记录携带 host 时钟戳（`projectiles.time`），follower `catch_up` 用戳差把本地预测回放到收包时刻再比对。JS 侧 `zones.js` 无此补偿（浏览器同帧语义），属 sim 网络路径新逻辑；评审确认互操作安全（新增字段为可选、旧端忽略不炸），不改 JS/wire 生产路径。登记于 REVIEW_CHECKLIST 复核项。
- **D-T47-12 rebuild_shields 每帧重建（threat.rs 自认偏差，评审定级 P1 可接受）**：JS `refreshShields` 只在 `_threatScan`（~4 Hz，`thrScanT` 门）时刷新 `_shl` 的**成员**（位姿字段是 live getter 每帧读）；Rust `Bot::rebuild_shields` 每帧重建整个列表。行为影响：canopy 的出现/消失至多比 JS 早一个 scan 周期（≤0.25s）被反应；launched 索引在旧 canopy 先离开列表前被复用时会读成新 canopy（JS 身份是对象、Rust 是 `ShieldRef::Launched` 当前索引）。评审定级 P1：语义偏差有界、无互操作影响，不改代码。
- **D-T47-13 评审与处置**：独立评审 2 P0/2 P1/3 P2——P0-1 `BotCtx.time` 误接 `Match::time` 倒计时（应为 `projectiles.time` 累加钟，`_bl_hold` 救援窗前后半程语义失效）已修 6 构造点并复验（ladder 逐位无影响，无 blade 装具）；P0-2 = REVIEW_CHECKLIST 过期条目（B/C 后 ladder 端点门翻转，D 后 1602.4≥1599.1 恢复通过）已改最终事实；P1-1/P1-2 → D-T47-11/D-T47-12 登记；P2 日志清理完成、`pathDir`/hop 省项为在案偏差。threat.rs 全段 RNG 抽取序/evade 方向/swap-remove 序/dev_trigger 分支经评审逐段核对与 JS 一致。
