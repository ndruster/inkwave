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
