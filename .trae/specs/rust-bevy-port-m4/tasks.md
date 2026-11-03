# INKWAVE Rust/Bevy 移植 — M4 任务分解（Task 41–57）

> 前置：M1 Task 1–16、M2 Task 17–27、M3 Task 28–40 全部 completed（基线 554 passed）。任务编号延续；格式与门禁口径沿用 M3。**M4 主线 = 玩法优先（spec.md「M4 决议」）**：Phase A Boss 战（41–46）→ Phase B 玩法收尾（47–48）→ Phase C 表现完整度（49–56）→ 收尾（57）。

## Task 41: Boss tuning 提取 + Boss wire 金标准
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 40
- **Description**:
  - `extract-tuning.mjs` 扩抓 `BOSS`（boss.js L16-27 逐字段）/`BOSS_MODE`（bossMode.js L8-13）/moves 时序与 hazard 参数（bossBrain/bossHazards 实读取字段集）/hitShapes 球表（socket/r/weak/active）；schema v3 + `tuning.rs` 类型 + 锚定单测。
  - fixture 扩展：`B` 快照（full/delta）、`'bhit'`、`['bm']`、`['bc']`、res boss 形态；`wire.rs` tick 消息 `B` 字段 + 编解码逐字节断言。
- **Acceptance Criteria Addressed**: AC-M4-1（基础）/AC-M4-2（基础）
- **Test Requirements**:
  - `rule` TR-41.1: 提取产物 stable-json 连跑逐字节一致；锚定单测逐值对 boss.js/bossMode.js/bossHazards.js 行号实读断言。
  - `rule` TR-41.2: fixture 新条目 Rust 编解码逐字节回环；既有 554 基线不破；`['bm']/['bc']` 容忍测试升级为产生+消费（保留未知 tag 容忍）。
- **Notes**: 纯管线+wire 任务，不动 sim 逻辑。boss 数值禁手抄（纪律）。

## Task 42: Boss sim 核心——Boss 实体/BossNav/hitShapes/快照
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 41
- **Description**:
  - `inkwave_sim::boss::{Boss, BossNav}`：状态机（pos/yaw/vel/hp/phase/move/moveT/movePhase/target/stunned/dead/seed）、boss 时钟 bt、phase 转换（roar 无敌拍 never mid-move）、hitShapes sim 侧球表查询（weak 先测/dead-intro 全灭规则）、伤害倍率链（weak 2.5/stunned 1.25/weapon 面积表×dmgDiff）、`remoteHit`/`adopt`/`handoff`/`pack`/`unpack`（boss.js L422/L432 逐字段）。
  - BossNav（bossNav 260 行）：level 导航限制（不入水/不越台/不穿墙）、出生点 no-go、charge 撞墙→stun。
- **Acceptance Criteria Addressed**: AC-M4-1
- **Test Requirements**:
  - `rule` TR-42.1: 固定种子状态机单测——HP 缩放（units×hpDiff）、phase 阈值 [.66,.33]、hitShapes 命中判定逐球、pack→unpack 往返逐字段一致。
  - `rule` TR-42.2: BossNav 三图冒烟（boss 出生/巡逻不出界、charge 撞墙 stun）。
- **Notes**: hitShapes 几何从 JS 渲染模型剥离进 sim 是本任务核心设计点（spec Background）。

## Task 43: BossBrain + BossHazards——六招与危害求值
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 42
- **Description**:
  - `BossBrain`（376）：move 选择（paceDiff/phase 门/目标选取）、`{id,t0,seed,params}` 确定性 record。
  - `BossHazards`（600）：slam 环/barrage 桶+splat/sweep 光束/charge/crablets/frenzy——几何由 record+时间求值（全端一致）；host-only splat（team1 墨）；victim-side 伤害；crablet 实体生命周期（`['bc']` 语义、pop 1.2s）；`onMove` guest 重放；`hz.threat()` 查询（Task 47 bot 消费）。
- **Acceptance Criteria Addressed**: AC-M4-1
- **Test Requirements**:
  - `rule` TR-43.1: 六招逐招固定种子断言（telegraph ≥0.6s、时序、hazard 几何逐帧对参考求值、伤害归属 victim-side）。
  - `rule` TR-43.2: 同 record 双端（sim=host/guest）hazard 位置逐 tick 一致；crablets 数量/追踪/pop 链。
- **Notes**: 本任务是 boss 局可玩性的逻辑核心；渲染无关。

## Task 44: BossMode 规则 + 离线完整局 + boss bots
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 43
- **Description**:
  - `BossMode`（55）：squad 8 建队/`squadUnits`/胜负/finishWin 5.2/finishLose 3.2/result boss 形态；`match` boss 分支（match.js L20-21/L55-62/L100/L193/L216/L229/L262 实读平移）；session 放开 `mode:'boss'`（Task 36 守卫改造）+ MAPS 旗标（`mapBossOk/bossFallbackMap/noBotsStartBlock`，config L524-539）。
  - 离线入口：Play 菜单模式选择（turf/zones/boss）；lobby MODE 三值 cycle。
  - bot boss 段：`_bossTick`（bots.js L1874-2050 实读：think 门/hz.threat/evade/_bossGoal/stunned rush/crablet pop/清墨）。
- **Acceptance Criteria Addressed**: AC-M4-1
- **Test Requirements**:
  - `rule` TR-44.1: 离线无头完整局 win + lose 双分支（固定种子，autopilot 驱动）；结算形态字段逐项（bossDmg/weakHits 对账）。
  - `rule` TR-44.2: boss bots 行为断言（威胁响应/集火/crablet 清除）；turf/zones 既有测试不破；noBoss/noBots 旗标路径。
- **Notes**: 完成后离线 Boss 战端到端可玩（体块占位档）。

## Task 45: Boss 联网——快照播放/上行/迁移 + e2e + scenario E
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 44
- **Description**:
  - host：tick `msg.B` 发布（`bossN%10` full/delta）、`recBoss(['bm'|'bc'])` 事件时间线；guest：`_sampleBoss` 播放（Hermite/同 playback 时钟/>12m 断线滑移）、`'bhit'` 上行 + host `remoteHit` + log 对账、`onMove` 重放；host 迁移 `adopt/handoff`（netmatch L682-683）；res boss 形态收发（L652-661）。
  - net-harness：Rust↔Rust 完整 boss 局（含中途 host 迁移）；net-smoke scenario E（JS host boss ↔ Rust guest，沙箱 DEGRADED 口径）。
- **Acceptance Criteria Addressed**: AC-M4-2
- **Test Requirements**:
  - `rule` TR-45.1: harness e2e——guest boss 位置/phase/move 与 host 逐 tick 对账（播放层门限沿用 Task 25 定标）；`'bhit'` 累计伤害收敛；迁移后无缝。
  - `rule` TR-45.2: scenario E 实跑 dump（DEGRADED 如实记录 + 可断言子项：握手/roster/boss 快照到达）；带宽压测（NFR-M4-3）。
- **Notes**: 协议不改（JS 客户端已在用），Rust 对齐即可。

## Task 46: Boss 呈现占位档——体块剪影/telegraph 标记/HUD boss 条
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 44
- **Description**:
  - bevy 体块 boss 剪影（集装箱壳/双钳/眼柄/腿，`m.update` 状态驱动 ≡ st 契约）；telegraph 标记（环/目标圈/charge 线/光束扇形）；crablet 体块；intro title card + roar 震屏占位；HUD boss 条（name/HP/phase notches/stun OPEN/伤害 chip）；结算 VICTORY/DEFEAT + bossDmg/weakHits 行；bossAudio 占位（现有 M1 占位档风格）。
- **Acceptance Criteria Addressed**: AC-M4-4（boss 部分）
- **Test Requirements**:
  - `rule` TR-46.1: 无头冒烟（headless 600 帧 boss 局不 panic）+ wasm 加载冒烟；HUD 状态映射单测（纯函数档）。
  - `rule` TR-46.2: 复核项登记 REVIEW_CHECKLIST（GPU 环境 boss 局视觉）。
- **Notes**: 高保真模型 Non-Goal（spec）；本任务只求"可玩且可读"。

## Task 47: bot 深化——threat AI + 顺延战术项
- **Status**: `pending`
- **Priority**: medium
- **Depends On**: Task 44
- **Description**:
  - threat AI turf 段（bots.js L1288+ 实读）；stamp threat-steer（L460-470）；`_shieldFight`/`_canopyCtl`（brolly）；mitts/blade 深战术段；与 Task 38 档位表合并回归。
- **Acceptance Criteria Addressed**: AC-M4-3
- **Test Requirements**:
  - `rule` TR-47.1: 固定种子行为断言（threat 优先级/回避/掩体选择逐段）；难度单调性测试延续（tr38 ladder 不破）。
- **Notes**: 表现层无关；纯 sim。

## Task 48: 全图提取——halyard/kelpline/lockgate/terraces/cargo + 旗标语义
- **Status**: `pending`
- **Priority**: medium
- **Depends On**: Task 41
- **Description**:
  - stage CLI 扩五图（halyard 内联 maps.js L30 特殊处理）；MAPS 旗标（onlineOnly/noBots/noBoss/theme/times）入 tuning；lobby stage 选择按旗标过滤（离线不列 onlineOnly、boss 房自动换图、noBots 关补位）；run-all.sh 扩全图。
- **Acceptance Criteria Addressed**: AC-M4-3
- **Test Requirements**:
  - `rule` TR-48.1: 五图 stable-json 连跑逐字节一致 + 碰撞冒烟（出生点落地/可行走）+ zones 合并（有 zones-data 者）。
  - `rule` TR-48.2: 旗标路径测试（cargo 离线不可见/boss 换图/noBots 开局阻断 noBotsStartBlock）。
- **Notes**: cargo 为 onlineOnly+noBots+noBoss——仅数据对等，实际开黑价值有限（如实注记）。

## Task 49: 小地图
- **Status**: `pending`
- **Priority**: medium
- **Depends On**: Task 48
- **Description**: `minimap.js`（585）平移——paint 网格采样→纹理、队友/敌方点、zone 标记、boss 位置；HUD 布局集成；采样率/清晰度档位。
- **Acceptance Criteria Addressed**: AC-M4-4
- **Test Requirements**:
  - `rule` TR-49.1: 采样纯函数单测（覆盖率→像素映射固定输入断言）；无头冒烟。
  - `rule` TR-49.2: REVIEW_CHECKLIST 登记视觉复核。
- **Notes**: Phase C 开始；双轨验收口径（NFR-M4-5）。

## Task 50: 音频过程化合成 + 音乐 + bossAudio
- **Status**: `pending`
- **Priority**: medium
- **Depends On**: Task 46
- **Description**: `audio.js`（2505 WebAudio 合成）平移选型（程序化样本烘焙 vs 实时合成——spec 设计段定）；`music.js`（1490）曲目状态机；`bossAudio.js`（177）；SFX 触发 ≡ JS 事件总线订阅点全表对账。
- **Acceptance Criteria Addressed**: AC-M4-4
- **Test Requirements**:
  - `rule` TR-50.1: 事件→音效映射表逐项单测（触发计数）；合成参数锚定测试。
  - `rule` TR-50.2: REVIEW_CHECKLIST 登记听感复核（含静音开关回归）。
- **Notes**: 不新增依赖前提下选型（sim 依赖面不变；inkwave crate 可用 bevy_audio/kira——以现有依赖为准，实读后定）。

## Task 51: fx 系统 + flood 呈现层
- **Status**: `pending`
- **Priority**: medium
- **Depends On**: Task 50
- **Description**: `fx.js`（2267 粒子/墨团/命中）+ `screenfx`（1068 受击方向/击杀闪）+ `fxHooks`（712）+ `swimWake`（107）+ `zoneMarks`（517，M3 顺延 zone/flood 呈现）→ bevy 实体/粒子；sim 事件流唯一驱动。
- **Acceptance Criteria Addressed**: AC-M4-4
- **Test Requirements**:
  - `rule` TR-51.1: 事件→fx 触发对账单测；粒子预算上限（性能门限）。
  - `rule` TR-51.2: 无头+wasm 冒烟不破帧；REVIEW_CHECKLIST 视觉登记。
- **Notes**: 体量大，执行期拆批次委派；先 hook 面后特效精度。

## Task 52: ink 着色 + 地表材质全量
- **Status**: `pending`
- **Priority**: medium
- **Depends On**: Task 51
- **Description**: `inkShading`（314 GLSL→wgpu：高度/光泽/湿润）；`levelMaterial`（719 pattern 全量）+ `texlib`（1424 程序化纹理）；材质参数走提取管线。
- **Acceptance Criteria Addressed**: AC-M4-4
- **Test Requirements**:
  - `rule` TR-52.1: 纹理生成确定性（同参数逐字节）；shader uniform 表锚定单测。
  - `rule` TR-52.2: wasm 冒烟 + REVIEW_CHECKLIST 视觉登记（湿润/光泽对比档）。
- **Notes**: 与 Task 55 后处理链有交互，质量档位表在本任务起建。

## Task 53: props/murals/dressing 装饰
- **Status**: `pending`
- **Priority**: medium
- **Depends On**: Task 52
- **Description**: tidewater `props.js`（2093）+ 各图 props/murals（359）/decor（486）/dressing（129）/mapThumb（116）→ bevy 几何（装饰档，碰撞体外不变）；`special-props.js` 一并。
- **Acceptance Criteria Addressed**: AC-M4-4
- **Test Requirements**:
  - `rule` TR-53.1: props 清单提取对账（数量/位置/尺寸锚定）；无头冒烟。
  - `rule` TR-53.2: REVIEW_CHECKLIST 视觉登记。
- **Notes**: 纯呈现层；不改碰撞（提取口径已锁）。

## Task 54: 角色完整绑定（弹簧姿态）
- **Status**: `pending`
- **Priority**: medium
- **Depends On**: Task 53
- **Description**: `character.js`（4062 弹簧姿态）+ character-geo/hair/face/mats/style/weapons/lod 平移——squid/kid 形态、姿态通道、武器手持；体块→程序化绑定升级；locker 更衣室除外（Non-Goal）。
- **Acceptance Criteria Addressed**: AC-M4-4
- **Test Requirements**:
  - `rule` TR-54.1: 姿态求值纯函数单测（固定输入→关节位置断言）；形态切换状态机测试。
  - `rule` TR-54.2: 8+1 实体性能预算冒烟；REVIEW_CHECKLIST 视觉登记。
- **Notes**: Phase C 最大单体；执行期拆 2-3 批委派。

## Task 55: 环境（黄昏/海况）+ 后处理链 + 质量档位
- **Status**: `pending`
- **Priority**: medium
- **Depends On**: Task 54
- **Description**: `environment.js`（3116：黄昏主题/海况/灯光烘焙档）；`renderer.js`（218：GTAO/bloom/grade/SMAA/tilt-shift diorama）；质量档位系统（低/中/高/ultra + K 键并入）。
- **Acceptance Criteria Addressed**: AC-M4-4
- **Test Requirements**:
  - `rule` TR-55.1: 主题参数提取锚定；档位开关组合冒烟（headless/wasm 不 panic）。
  - `rule` TR-55.2: REVIEW_CHECKLIST 视觉登记（黄昏档/后处理逐档）。
- **Notes**: SwiftShader 环境后处理需可全关（保底可玩路径）。

## Task 56: showcase 大厅
- **Status**: `pending`
- **Priority**: medium
- **Depends On**: Task 54
- **Description**: `showcase.js`（3109）+ `lobbySet*`（geo/mats/tex）3D 大厅——角色展示台/loadout 实时联动/房间码 UI 场景化；极简大厅保留为低配档。
- **Acceptance Criteria Addressed**: AC-M4-4
- **Test Requirements**:
  - `rule` TR-56.1: loadout→展示状态联动单测（数据面）；wasm 冒烟。
  - `rule` TR-56.2: REVIEW_CHECKLIST 视觉登记。
- **Notes**: 依赖角色绑定与 props 完成；最后的表现项。

## Task 57: M4 收尾——台账/文档/门禁矩阵/总评审
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 41–56
- **Description**:
  - SYNC_LEDGER M4 段；PORT_MAP「M4 完成核对」+ 状态图例扩档；rust/README M4 章节（Boss 战运行/表现档位/复核命令）；九段门禁全矩阵；M4 总评审（独立只读评审员）；REVIEW_CHECKLIST 汇总复核清单；提交 + push origin/main。
- **Acceptance Criteria Addressed**: AC-M4-5
- **Test Requirements**:
  - `rule` TR-57.1: 文档内命令全部实跑 exit 0；门禁日志落盘；总评审 P0/P1 闭环。
- **Notes**: 沿用 Task 27/40 收尾流程。

---

## 执行顺序与并行
- Phase A 严格串行（41→42→43→44→45；46 依赖 44 可与 45 并行窗口）。
- Phase B：47 依赖 44、48 依赖 41（可提前穿插，但默认按序）。
- Phase C 默认串行（49→50→51→52→53→54→55→56）；50/49 相互独立可依窗口调度。
- 每任务固定流程：实读 JS 锚点 → spec.md 尾部「TaskNN 实施设计」§0-§7 → 分批委派 → 主上下文实读核验 → 独立评审 → P0/P1/P2 当场修复复验 → 九段门禁 → tasks.md 勾选 + Completion Evidence + Review 附录 → spec §8 定案 → PORT_MAP/台账 → commit → 中文汇报。
