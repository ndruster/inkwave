# INKWAVE Rust/Bevy 移植 — M3 任务分解（Task 28–40）

> 前置：M1 Task 1–16、M2 Task 17–27 全部 completed。任务编号延续；格式与门禁口径沿用 M2。
> **M3 决议（2026-10-30 与用户确认）**：主线 = 内容移植 + 跨端开黑；模式 turf + Zone Control（boss 留后）；武器 12 主 + 15 副 + 19 特殊（kit 留后）；地图 tidewater + 2 张（推荐 saltpan/crossmarket，OQ-1）；视觉/音频完整度整体顺延 M4。

## Task 28: tuning 全量提取——武器/副/特殊/zone/bot 参数进管线
- **Status**: `completed`
- **Priority**: high
- **Depends On**: Task 27
- **Description**:
  - `extract-tuning.mjs` 扩抓 `WEAPONS`（12 把逐字段，config.js L94-315）、`SUBS`（15，L318 起）、`SPECIALS`（19，L411 起）+ `SPECIAL_ORDER`（L469）、`ZONES`（L485-501）、`DIFFICULTY`（L503-508）、`WEAPON_ORDER`/`weaponRange`（L303/L307-315）；`tuning.schema.md` 更新；`tuning.rs` 类型扩展 + 上游字面量锚定单测（每武器至少 damage/rate/ink 关键值逐条 assert）。
- **Acceptance Criteria Addressed**: AC-M3-1（基础）
- **Test Requirements**:
  - `rule` TR-28.1: 提取产物 stable-json 可 diff；锚定单测全绿（断言值 grep 自 config.js 行号注释）。证据：测试输出 + tuning.json diff。
  - `rule` TR-28.2: retired 武器（dualies/slosher/splatling）不入产物；`WEAPON_ORDER` 12 项一致。
- **Notes**: 纯管线任务，不动 sim 逻辑；JS 树零改动门禁本任务起持续生效。

### Task 28 Completion Evidence（2026-10-30）
- **TR-28.1 ✅**：`extract-tuning.mjs` 扩抓 PLAYER + `WEAPONS`(12，按 `WEAPON_ORDER` L303 投影) + `weaponRange`(L307-315 提取时预计算成映射表) + `SUBS`(15)/`SPECIALS`(19)/`ZONES`(L485-501)/`MATCH`/`TEAM_PALETTES`/`DIFFICULTY`(三档)，schema `inkwave.tuning.v1→v2`。确定性：连跑两次逐字节一致 + 评审员独立重提取 diff 一致。锚定单测全绿：`cargo test -p inkwave_sim` **216 passed**（189→216，+27：order 锁定×3、spritzer 别名等值、weaponRange 表、11 武器逐值、sub 分组×5、special 分组×6、zones、difficulty），断言值逐条对 config.js 行号注释实核。全量门禁日志 `rust/tools/verify/logs/task28-gates-20261030T085619Z.log`（fmt/clippy/test/wasm32/cargo tree/trunk release/headless 120 帧/JS 树零改动，全 exit 0；workspace **300 passed** > 273 基线，基线 273→300 单调递增）。首轮 fmt 失败（rustfmt 展开数组字面量）→ `cargo fmt --all` 修复后全绿。
- **TR-28.2 ✅**：retired `dualies/slosher/splatling`（L266-299）不入产物——提取器按 `WEAPON_ORDER` 投影（`pick()` 缺 id 即 throw），测试 `weapon_order_locked_and_retired_excluded` 以 `TUNING_JSON.contains` 反向断言 0 命中；`WEAPON_ORDER` 12 项与 L303 逐字一致。
- **类型扩展**：`tuning.rs` 顶层新增 `weapon_order/weapons(12 struct)/weapon_range(HashMap)/sub_order/subs(15)/special_order/specials(19，barrage_* 蛇形键显式 rename)/zones(17 字段)/difficulty(三档)`；u32=整数语义（pellets/blobs/drops/max/uses/hp/canopyHp/count/sampleHz/specialCost 等）、f32=其余、`[f32;2]`=bow 二元数组、`Option`=anim 等 JS 可选字段。`spritzer` 保留为 `weapons.shooter` M1 别名（Task 29 迁移消费端后移除），既有消费端零改动（workspace 全绿证明不破）。
- **文档**：`tuning.schema.md` 重写为 v2（顶层表、逐武器/副/特殊字段分组+行号锚点、weaponRange 表、zones 17 字段表、difficulty 三档表）。

### Task 28 Review 附录（独立评审：0 blocker / 0 major / 5 P2 → 全部当场修复 → 结论可闭环）
- 评审员实核通过：重提取逐字节一致；bow/mitts/brolly/blade 全字段 + torpedo/waddle/shaker/boomerang/mine/beacon + stamp/crab/jetpack/blower/zipcaster/barrage×5 逐值对 config.js 一致；serde 边界（`barrage_*` 显式 rename、`rMin/rMax/launchSpeed0/finalCentre/ring1/deflectR` camelCase 转换）正确；u32/f32 选型正确；`Spritzer`/`Difficulty` struct 与 HEAD 逐字节相同（消费端无漂移）；JS 树零改动。
- **P2-1 ✅** schema.md PLAYER 锚点 L21-91→L21-90（L91 空行，实核）。
- **P2-2 ✅** schema.md blaster L126-136→L126-135（L136 为 bucket 注释行）。
- **P2-3 ✅** schema.md「解析前校验前缀」→「单测 `schema_and_provenance` 精确断言」（`embedded_tuning()` 无运行时校验，措辞对齐实现）。
- **P2-4 ✅** tuning.rs L207/L1460 retired 锚点 L265-305→L266-299（与 schema.md 统一）。
- **P2-5 ✅** tuning.rs 头注释单位说明补齐：散布/弧判定/扇形/跃角字段为度（原仅提散布）。
- 修复复验：fmt/clippy/test 重跑全绿（300 passed），追加至同一门禁日志。

## Task 29: 武器 runner 架构——kind 分发与弹丸系统泛化
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 28
- **Description**:
  - `inkwave_sim::weapon` 重构为按 kind 分发（对齐 weapons.js WeaponRunner L32/L118-124 结构）；`Projectiles` 泛化（L776 起：弹速/重力/散布/straightTime/命中半径/impactRadius/trail 喷墨/上限计数）；移速修正表（L67-77）；墨耗/过热通用路径。Spritzer 迁移到新架构，回归测试全绿。
- **Acceptance Criteria Addressed**: AC-M3-1
- **Test Requirements**:
  - `rule` TR-29.1: Spritzer 行为回归（M1 既有武器测试逐条不破）。
  - `rule` TR-29.2: 弹丸系统单测：直射/抛物/散布/拖尾喷墨各路径固定种子断言。
- **Notes**: 架构先行，避免每把武器重复脚手架；kind 枚举含全部 12 类占位。

## Task 30: 主武器批一——射击类（twins/blaster/brolly）
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 29
- **Description**:
  - twins（Pistols：左右手交替 fireInterval、bloom/spreadLock、moveSpeedFiring）；blaster（弹药段 pump 节奏、扩散弹）；brolly（开合伞：deploy 挡弹/行进喷墨，weapons.js 对应分支逐行对齐）。每把：tuning 驱动、无头行为测试、handling 口径测量。
- **Acceptance Criteria Addressed**: AC-M3-1
- **Test Requirements**:
  - `rule` TR-30.1: 每武器射速/伤害/墨耗/散布逐值断言（锚 config 行号）。
  - `rule` TR-30.2: 联网 tick 中三武器 runner 状态槽（ch/lock 等）与 JS packer 语义一致（对照 packActor L723-757）。
- **Notes**: brolly 的伞面挡弹是弹丸系统新交互，落 `Projectiles` 钩子。

## Task 31: 主武器批二——蓄力与流类（charger/spinner/bucket）
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 29
- **Description**:
  - charger（蓄力曲线→激光/弹段、rangeMax、满充反馈；`['ev']` charge 相关记录对齐）；spinner（spin-up 蓄力→长流、burstMin/Max）；bucket（抛洒弧：drops/grav/落点泼溅 paint 形状）。
- **Acceptance Criteria Addressed**: AC-M3-1
- **Test Requirements**:
  - `rule` TR-31.1: 蓄力/释放时序固定种子逐值断言（chargeTime/burst 区间）。
  - `rule` TR-31.2: bucket 泼洒弧采样与 JS 口径对照（落点数/半径）。
- **Notes**: charger 激光为 hitscan 段 + 弹段混合，注意与 Hermite 播放层无冲突（激光是本地判定）。

## Task 32: 主武器批三——近战扫类（roller/brush/blade）
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 29
- **Description**:
  - roller（charge 带墨滚动/flick 抛墨/撞击判定，weapons.js L218-290 段对齐）；brush（拖墨细线/侧扫泼溅）；blade（冲刺斩：dash 位移、斩击弧喷墨、blade-fx 语义简化档）。
- **Acceptance Criteria Addressed**: AC-M3-1
- **Test Requirements**:
  - `rule` TR-32.1: 三类扫击的墨迹覆盖带（固定路径下覆盖格数断言）。
  - `rule` TR-32.2: flick/侧扫的抛射物与近距判定边界用例。
- **Notes**: 近战判定走 owner 本地 + splat 广播（同 Spritzer 命中模型），无新 wire 类型。

## Task 33: 主武器批四——特殊机制类（bow/mitts）
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 29、Task 32
- **Description**:
  - bow（蓄力抛物箭、重力/穿透语义，bow.js 段对齐）；mitts（拳套：alternating fists 弹丸、guardArc 减伤、charged leap 冲量 + 落地溅射 + wall cling 贴墙耗墨——`inkwave_sim::actor` 状态机扩展，OQ-2 在此定案并记 PORT_MAP）。
- **Acceptance Criteria Addressed**: AC-M3-1
- **Test Requirements**:
  - `rule` TR-33.1: leap 弹道（leapSpeedMin/Max/角度钳制）与 land 溅射半径逐值断言。
  - `rule` TR-33.2: cling 状态进入/退出（jump/墨尽）与 drain 时序用例。
  - `rule` TR-33.3: actor 扩展回归——既有移动/游泳/重生测试不破。
- **Notes**: 本任务风险最高（actor 状态机），先写 spike 评审再落码。

## Task 34: 副武器 15 个 + 设备网络语义（['b']/'dh'）
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 29
- **Description**:
  - `SubSystem` 平移（subs.js L54/L71/L81/L107：throw/plant 物理、fuse/触发、15 效果逐 key：bomb/sticky/burst/seeker/scan/curtain/sprinkler/mine/beacon/mist/shaker/waddle/torpedo/tracer/boomerang）；设备实体注册表（gid 分配、击毁判定→`'dh'` 直发 owner 应用，netmatch L159/L210）；`['b']` recBomb 记录（L124-127 九字段）联网重放（owner 建实体、他人 ghost）；fixture 扩展捕获（并入 Task 36 金标准）。
- **Acceptance Criteria Addressed**: AC-M3-1、AC-M3-2
- **Test Requirements**:
  - `rule` TR-34.1: 15 sub 的墨耗/ fuse/伤害半径/效果时序逐值断言。
  - `rule` TR-34.2: `'dh'` 直发在 FakeTransport 下 owner 应用/非 owner 忽略；`['b']` ghost 不喷墨不判伤仅呈现。
  - `rule` TR-34.3: beacon 上限 3/mine 上限 2/torpedo 一发在途等数量守卫用例。
- **Notes**: 呈现占位档（体块 + 简单特效），美术 M4。

## Task 35: 特殊武器 19 个 + tick 槽实值化（sp/spx/spst）
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 34
- **Description**:
  - `SpecialSystem` 生命周期平移（specials.js L217/L254-363：start/netGhost/tick/body/weapon/end；19 key 对齐 SPECIAL_ORDER L469）；充能（gauge、specialCost、zone gaugeHeld/Neutral 加成联动 Task 37）；`sp`(s[13])/`spx`(s[22])/`spst`(s[23]) 从恒 0 实值化（packActor L747-750 口径）；`special:use/slam` 事件联网重放；netGhost 占位档（OQ-3 定案）。
- **Acceptance Criteria Addressed**: AC-M3-1、AC-M3-2
- **Test Requirements**:
  - `rule` TR-35.1: 每特殊的核心机制时序断言（slam 落点/storm 域/barrage 五变体/kraken 路径等，固定种子）。
  - `rule` TR-35.2: sp/spx/spst 槽 fixture 逐字节（含非零样本）；`['k']`/`['bm']`/`['bc']` 解码容忍测试。
  - `rule` TR-35.3: 充能联动：击杀/涂地/zone 反推 gauge 数值断言。
- **Notes**: 19 个是大头，按机制族分组实现（弹道族/领域族/位移族/召唤族）；barrage×5 复用同主体。

## Task 36: Zone Control 规则 + ['z'] 广播 + session 模式放开
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 35
- **Description**:
  - `inkwave_sim::match::zones` 平移 ZoneControl（zones.js L105：zoneCells L77/insidePoly L58/`_tick` L238/update L266/netEvent L372/state L416；16 项 ZONES 参数；轮换/终局中心/加时/penalty/contest 全路径）；match.js mode 分支（L14-23/L96-147）；host 权威 `recZone(['z',e])` 广播 + guest 应用（8 种事件 zz/zo/zp/za/zf/zt/zs/ze）；`res` zone 形态结算；session.rs turf-only 守卫放开至 zones（L37-40/L1270/L1306），仍拒 boss；fixture 扩展并入金标准。
- **Acceptance Criteria Addressed**: AC-M3-2、AC-M3-3
- **Test Requirements**:
  - `rule` TR-36.1: 规则单测——占领/争夺阈值、倒计时速率（center/home/away）、penaltyK 取整、加时 grace/max 逐值断言（锚 config.js L485-501）。
  - `rule` TR-36.2: FakeTransport 双端 zone 状态序列逐 tick 一致；`['z']` fixture 逐字节。
  - `rule` TR-36.3: guest 收 `mode:'boss'` 仍拒绝退出（回归）。
- **Notes**: zone 涂地采样 sampleHz=5 与 paint 网格联动是本任务性能关注点。

## Task 37: 新地图 ×2 提取（saltpan + crossmarket）+ 碰撞冒烟
- **Status**: `pending`
- **Priority**: medium
- **Depends On**: Task 28
- **Description**:
  - OQ-1 定案后：`extract-layout.mjs` 跑 saltpan/crossmarket（layout.js 160/151 行 + surfaces 色/pattern + zones-data 合并 maps.js L118-121 口径）；half 镜像展开沿用 tidewater 流程；碰撞体块 + 出生点 + bounds 入 `assets/maps/`；无头碰撞冒烟（出生点落地、可行走采样、zone 多边形取格非空）。
- **Acceptance Criteria Addressed**: AC-M3-5
- **Test Requirements**:
  - `rule` TR-37.1: 提取产物与 JS layout 数值 diff 复核（stable-json）。
  - `rule` TR-37.2: 两图 Rust↔Rust turf 完整局 + 一图 zones 完整局（harness）。
- **Notes**: 装饰 props 不提取（M1 口径）；渲染复用体块材质档。

## Task 38: bot normal/hard + 武器/副/特殊使用 + zone tactic
- **Status**: `pending`
- **Priority**: medium
- **Depends On**: Task 36
- **Description**:
  - `DIFFICULTY` 六参数接入（config.js L503-508；bots.js L271-276 setDifficulty）；行为差异平移（反应门限/瞄准弹簧 aimOmega/aimTurn/fireDiscipline/awareness）；bot 用武器全 kind（含近战扫/蓄力节奏）；sub 投掷与特殊驾驶（bots.js L858 段）；zone needy-patch 涂区 tactic（L1176 段）；roster bot 随机 sub/special 50% 规则（session.js L285-286）。
- **Acceptance Criteria Addressed**: AC-M3-6
- **Test Requirements**:
  - `rule` TR-38.1: 三档同 seed 统计单调性（存活/命中/涂速）报告。
  - `rule` TR-38.2: bot 持各 kind 武器无 panic 长跑（autopilot headless 120 帧 × 全武器矩阵）。
- **Notes**: 量化对照口径尽量贴 botlab（tools/botlab 只读）。

## Task 39: 大厅 loadout UI + 跨端完整开黑验证
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 36、Task 37、Task 38
- **Description**:
  - 极简 bevy_ui loadout：武器 12/副 15/特殊 19 列表选择 + mode turf/zones 开关（roster `{weapon,sub,special,style}` 实值化，session.js L279/L287 口径；style 仍透传忽略）；跨端验证：wrangler dev（Task 27 探针已证可达）+ JS headless（tools/net-test.mjs 形态只读）↔ Rust——turf 一局（双方不同主武器 + 副/特殊各触发 ≥1）+ zones 一局；断言覆盖差 <0.5%、zone 序列一致、结算一致、无 err 帧。
- **Acceptance Criteria Addressed**: AC-M3-4、AC-M2-6 回归
- **Test Requirements**:
  - `rule` TR-39.1: loadout 选择 → roster 字段 → 对局内武器生效全链路集成测试（harness）。
  - `rule` TR-39.2: 跨端两局实跑日志（net-smoke 扩展 scenario）；不可达项列 REVIEW_CHECKLIST 并如实记录。
- **Notes**: 沿用 Task 26 net-smoke 脚手架扩 scenario，不新建工具链。

## Task 40: M3 收尾——门禁、台账、文档与总评审
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 39
- **Description**:
  - 全量门禁矩阵（M2 Task 27 口径）；SYNC_LEDGER/PORT_MAP 更新（武器/zone/地图各行状态、M3 完成核对小节）；README「武器与 Zone Control」章节 + 联网章节跨端开黑更新；REVIEW_CHECKLIST 追加 M3 条目（跨端真机开黑、新图视觉、mitts leap 手感复核）；独立审查 agent 总评审 M3 全部产出并闭环。
- **Acceptance Criteria Addressed**: AC-M3-7
- **Test Requirements**:
  - `rule` TR-40.1: 门禁命令输出（测试总数 > 273、`cargo tree -p inkwave_sim` 仍仅 glam/serde/serde_json、trunk release、headless 冒烟、JS 树/`server/` 零改动）；证据为日志。
  - `rule` TR-40.2: 文档逐条命令实际执行验证。
  - `rule` TR-40.3: 独立审查无未闭环 Blocker/Major。
- **Notes**: 完成后提交 M3 里程碑并 push origin/main（沿用既有提交流程与身份环境变量）。

---

## Review 附录（逐任务追加）

（实施时按 M1/M2 格式：独立审查发现 → 修复 → 复审结论）
