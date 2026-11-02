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
- **Status**: `completed`
- **Priority**: high
- **Depends On**: Task 28
- **Description**:
  - `inkwave_sim::weapon` 重构为按 kind 分发（对齐 weapons.js WeaponRunner L32/L118-124 结构）；`Projectiles` 泛化（L776 起：弹速/重力/散布/straightTime/命中半径/impactRadius/trail 喷墨/上限计数）；移速修正表（L67-77）；墨耗/过热通用路径。Spritzer 迁移到新架构，回归测试全绿。
- **Acceptance Criteria Addressed**: AC-M3-1
- **Test Requirements**:
  - `rule` TR-29.1: Spritzer 行为回归（M1 既有武器测试逐条不破）。
  - `rule` TR-29.2: 弹丸系统单测：直射/抛物/散布/拖尾喷墨各路径固定种子断言。
- **Notes**: 架构先行，避免每把武器重复脚手架；kind 枚举含全部 12 类占位。

### Task 29 Completion Evidence（2026-10-30）
- **TR-29.1 ✅**：M1 既有武器测试逐条不破——消费端迁移后 `cargo test -p inkwave_sim` 回归 215 passed 全绿（TR-7.1~7.4 等 Spritzer 行为断言原样保留，仅签名/字段访问迁移）；最终 workspace **303 passed**（基线 300：+4 TR-29.2 新测试，−1 `spritzer_alias_matches_weapons_shooter`——该测试断言的别名本体已按本任务要求删除，属自然移除，总数仍净增）。全量门禁日志 `rust/tools/verify/logs/task29-gates-20261030T103218Z.log`（fmt/clippy/test/wasm32/cargo tree/trunk release/headless 600 帧/JS 树零改动，全 exit 0）。
- **TR-29.2 ✅**：`weapon/tests.rs` 尾部四组固定种子单测全绿——① `fire_round` 直射段→重力+阻力抛物段（自定义 RoundSpec speed 20/straight 0.15/range 10.4/grav 20，逐帧对参考积分器 max_err<1e-2，life 截止 `gone_at=31`：life=(10.4/20).min(1.0)=0.52，32×DT=0.533>0.52>31×DT，无 trail 时 coverage=0）；② 散布锥（seed 42 发 8 弹 6°：全部锥内、max 半角>0.5°、同 seed 逐值复现、异 seed 不同）；③ 拖尾喷墨（seed 777 平射：首滴 2.5 m 后每 0.8 m raycast 滴落，drips≥5、turf≥3、z 严格递增、radius∈[0.24,0.36]、同 seed 序列相等）；④ kind 表与分发骨架（12 kinds 对齐 `WEAPON_ORDER` L303、retired→None、`move_speed`/`spread_deg` 全分支、`busy()`、MAX_ROUNDS=700、blaster 30 帧 no-op 不耗墨不发射）。
- **架构落地**：`WeaponKind` 12 类枚举 + `update` 按 kind 分发（JS L108-156）；`move_speed` 全表（L63-92 含 brush/bucket/spinner/twins switch、roller 尾链 lerp、kit 分支）与 `spread_deg`（L94-107，blaster `?? 1.2`/`?? 4` 默认）；`auto_fire` 墨耗通用路径（`_auto` L165-182 含空墨 lockout）；`ProjectileSim::fire_round` 经 `RoundSpec` 泛化（弹速/grav/drag/straightTime/size/impactRadius/trail*/life），shooter 为薄包装（JS L1034 硬编码 grav 28/drag 0.8/size 0.15/life 1.2）；`step_one` 平移 `_step` L1583-1648（积分→身体胶囊→世界段→trail drip→life/海水 expiry→impact splat）。仅 Shooter 接线实弹，其余 11 kind update 臂为刻意骨架（Task 30-33 填充）。
- **别名移除与消费端迁移**：`tuning.rs` `spritzer` 字段、`extract-tuning.mjs` spritzer 提取、`tuning.schema.md` 别名表述全部移除（顶层 `spritzer` 键 0 命中，重提取连跑两次逐字节一致）；18 文件消费端改读 `weapons.shooter`/`weapons`（`Replication::update` 签名 `&Spritzer`→`&WeaponSet`，为 Task 30 per-actor loadout 铺路；`drive_bots`/`record_fire` 保持 `&Spritzer`）。顺带修复 replication ghost `Projectile` 缺 `size` 字段（JS ghostProjectile L868 从 record e[15] 解构，wire 已编解码，直接透传）。`PORT_MAP.md` weapons.js 行更新为 M1/Task 29 状态。

### Task 29 Review 附录（独立评审：1 P0 / 0 P1 / 5 P2 → P0+4 P2 当场修复，1 P2 为 Task 30 前瞻注记 → 门禁复验全绿）
- 评审员实核通过：move_speed/spread_deg 全表逐字对齐（含 0.75/0.7/0.5/0.45/0.6/0.18 各系数与 lerp 边界）；fireShooter L1034 常量与 trail `-(2.5−trailEvery)` ✓；`_step` 顺序、drag 仅 straight 后生效（L1588）、drip `0.8+rand·0.4`/impact `0.85+rand·0.3` ✓；MAX_BLOBS=700 仅 `_draw` L1880 截断、sim 不 cull（决策成立）；gone_at=31 与锥角上界算术复核成立；消费端 spritzer 残留 grep 0；ghost size 对齐 L868 e[15]；JS 树零改动、sim 依赖仅 glam/serde/serde_json、tuning.json 无顶层 spritzer。
- **P0 ✅** `weapon.rs:791` `fire_kind_round` 的 `match kind { Shooter=>…, _=>{} }` 触发 `clippy::single_match`（首轮门禁 CLIPPY_EXIT=101，日志 `task29-gates-20261030T101331Z.log` 留痕）→ 改 `if kind == WeaponKind::Shooter`。
- **P2-1 ✅** `weapon.rs` spread_deg 文档锚点整体偏 1 行：blaster L101→L102、twins L102→L103、spinner L103→L104、return 0 L104→L105（实读 JS L102-105 复核）。
- **P2-2 ✅** `weapon/tests.rs` roller flick 注释 JS L82→L86（L82 是 splatling charging）。
- **P2-3 ✅** `weapon.rs` switch cases 锚点 L67-77→L67-78；kit kinds 注记补齐 JS 尾链真实语义（无 kit `moveSpeed` 时落 L89-90 `firingT>0 ? moveSpeedFiring : runSpeed`，Task 30+/32/33 接线时注意）。
- **P2-4 ✅** 模块头「draw sequence matches JS」措辞修正：JS fireShooter 每发另有一次呈现专用 `vis` 抽取（L1035），sim 删该字段不消耗，改为「所列各次抽取顺序一致」。
- **P2-5（注记，不改）** blaster 1.2/4.0 为硬编码默认（JS `??` 语义）；若 Task 30 tuning 出现 `spread`/`spreadAir` 字段需改读 Option。
- 修复复验：全量门禁矩阵重跑全绿（303 passed），日志 `task29-gates-20261030T103218Z.log`。

## Task 30: 主武器批一——射击类（twins/blaster/brolly）
- **Status**: `completed`
- **Priority**: high
- **Depends On**: Task 29
- **Description**:
  - twins（Pistols：左右手交替 fireInterval、bloom/spreadLock、moveSpeedFiring）；blaster（弹药段 pump 节奏、扩散弹）；brolly（开合伞：deploy 挡弹/行进喷墨，weapons.js 对应分支逐行对齐）。每把：tuning 驱动、无头行为测试、handling 口径测量。
- **Acceptance Criteria Addressed**: AC-M3-1
- **Test Requirements**:
  - `rule` TR-30.1: 每武器射速/伤害/墨耗/散布逐值断言（锚 config 行号）。
  - `rule` TR-30.2: 联网 tick 中三武器 runner 状态槽（ch/lock 等）与 JS packer 语义一致（对照 packActor L723-757）。
- **Notes**: brolly 的伞面挡弹是弹丸系统新交互，落 `Projectiles` 钩子。

### Task 30 Completion Evidence（2026-10-31）
- **TR-30.1 ✅**：15 个固定种子逐值测试全绿（`weapon/tests.rs` 8 个 + 新建 `weapon/brolly_tests.rs` 7 个）。twins：射速 0.085/发（guard<3 补发 12 发计数）、ink 0.85/发、damage 30 击杀、散布 ground 6°/air 11°、bloom +0.25/发（D-4 硬编码）、roll 种植 turret 组（0.058/1.6°/36/0.16/13.5）、roll 链（charges 2→lockout 2.6→refill 1.4、rollInk 7、dodgeVelocity 端点 v0=3.4/(0.7·0.3) 与 (1−0.6u) 衰减）、move_speed 5.2。blaster：0.78/发、ink 9、direct 125、blast 弹字段（straight 99/grav 0/drag 0/size 0.26/life 10.5/23）、world 命中下射 raycast 3.5 喷墨 + splash `lerp(70,30,d/2.6)`+LOS+direct 排除、寿终空中爆。brolly：0.45/发、ink 5.5、11 丸锥（PATTERN 半径 0/0.42/0.9、y×0.72、rng 消耗序）、pelletDamage 8→2.5@2.8→6.5（端点/中点/钳制）、canopy 时序（openDelay 0.12/openTime 0.14/closeTime 0.1/launchHold 0.9/regrowTime 5/regen 60@2s）、held 伞挡弹扣血→破伞→重长、发射伞 0.32 m 喷墨条 + blockActors 推挤（shoving 0.6 减速）+ 双高挡墙 + 入海死亡 + 重力 24、同 blast 同 victim 合并 ≤1 个 `Hit`、kit 惰性初始化。workspace **321 passed**（303→321：+15 TR-30.1、+3 TR-30.2）。
- **TR-30.2 ✅**：`net/replication.rs` 三测试全绿——① `tr30_2_pack_slots_three_loadouts`：twins/blaster/brolly 三 loadout 的 `ch`(s[14] streaming?burstFrac:charge)/`lock`(s[20] lockT，D-5 恒 0)/`ks`(s[21] brolly 位包、其余 0) 逐槽对 JS packActor L723-751 手算断言（位包期望值 16380/16320/15868/131008 评审复核正确）；② `tr30_2_kit_record_ghost_blocks_local_shots`：JS→Rust 收 `['k',nid,'brolly',[x,y,z,dx,dz,hp,h0]]` 生 ghost 伞（`addLaunched(ghost=true)`：挡本地弹、不喷墨，JS mute L490 语义）；③ `tr30_2_brolly_pose_converges_rust_to_rust`：伞状态经 ks 槽 pack→unpack→`net_apply_brolly`（JS netApply L642-649/applyRemote L398）逐 tick 收敛一致。
- **架构落地（spec §1–§4）**：`weapon.rs`——`RoundSpec.trail_start` 三式并存（shooter `-(2.5−every)`/twins `-(2.4−every)`/blaster −1.5）、`Projectile.blast/pellet(PelletMark)`（serde default 向后兼容）、`is_wired`=Shooter|Twins|Blaster|Brolly、update 分发臂（Blaster→auto_fire、Twins→twins、Brolly→brolly::update）、`fire_blaster`/`fire_twin`（fallback muzzle 镜像+LOS 回退）、`try_dodge`/`dodge_velocity`、`step_one` 伞面 shield 扫掠（predict L249-255 drag 仅 straight 后）+ `_blastBurst` + pellet pending 合并（BTreeMap 确定性、MERGE=1/30 D-6）、`move_speed`/`spread_deg` Brolly 臂、访问器 `charge/lock_t/streaming/burst_frac/brolly_net_state/net_apply_brolly`。新建 `weapon/brolly.rs`（686 行）——BrollyKit 全字段、update 开合/tap-blast/launch 触发、fire_blast 11 丸锥、LaunchedCanopy 滑行/挡人/喷墨/fade、伞维护（idle 折叠/regrow/grow/regen）、held_shield 成盾条件与帧间快照。`actor.rs`——`dodge_vel`（integrate 水平段后覆盖，actor.js L305-306）+`jump_swallowed`+`jump_edge`。`match_.rs`——`kinds`/`launched`/`kit_out` 平行注册表、dodge 拦截帧序、shield 装配、runner 分发读 `kinds[i]`。`net/matchbuild.rs`——roster.weapon→`WeaponKind::from_kind_str`（未知/retired 回退 Shooter）。`net.rs`——`ActorSample::from_actor` 增 kind 参（ch/lock/ks 实值化）、`SimEvent::Kit`；`net/replication.rs`——`kit_out`→`['k']` 记录（owner 且非 remote 守卫）、播放侧 `net_apply_brolly` + ghost 伞；wire `['k']` 编解码沿用 M1 占位形态（recKit L134-137 逐字节）。
- **门禁**：全量九段矩阵全 exit 0，日志 `rust/tools/verify/logs/task30-gates-20261030T171102Z.log`（fmt/clippy/test/wasm32/cargo tree/trunk release/headless 600 帧/JS 树零改动；workspace 321 passed > 303 基线单调递增；`cargo tree -p inkwave_sim` 仅 glam/serde/serde_json）。

### Task 30 Review 附录（独立评审：0 P0 / 0 P1 / 6 P2 → 全部为 spec 批准偏差或不可实际触发边界，无需修复 → 结论 PASS）
- 评审员实核通过：twins 交替手/roll 链/turret 组/dodgeVelocity 逐值；blaster `_auto` 同路/blast 字段/下射 raycast/splash 公式/burstRadius 1.9；brolly 11 丸锥 rng 消耗序/pellet falloff/canopy 时序/破伞 regrow/launch 0.32 m 条+shoving 0.6+双高挡墙+水死+重力 24；ks 位包与 netState L636-641 逐位一致、`['k']` 形态与 recKit 一致、TR-30.2 手算值复核正确、旧 fixture serde default 兼容；twins `move_h`=`intent.move` 原始世界输入（JS L493 ✓）、dodge 用原始 fire/twins 用 gated fire 两路区分（L493/L532 ✓）、dodge_velocity→try_dodge→a.step 时序对齐 actor.js L305-310 ✓；BTreeMap 键序确定 ✓；ghost 伞不喷墨但消耗 rng、挡弹扣伞血与 JS mute/pre-scan 一致 ✓；tuning 逐值对 config 一致、生产代码无手抄（blaster 1.2/4.0 为 D-3 批准例外）；JS 树/server/ 零改动、sim 依赖面不变。
- **P2-1（注记，不改）** `flush_pending` 置于 step_masked 顶部 vs JS tick 侧累积：同 blast+victim 第二丸恰在到期帧到达时 Rust 拆两击——同 blast 丸到达差 <1 帧而窗口 2 帧，几乎不可触发。
- **P2-2（注记，不改）** `blockActors` 在 step_masked 后 vs JS 核心弹步前：被推玩家该帧受弹用推前位置，1 帧差。
- **P2-3（批准）** `jump_edge` 精确按下帧 vs JS jumpBuffer 窗口——spec §3 明文批准且代码注释自报。
- **P2-4（注记，不改）** `seg_shield` 多 `a==0.0` 分支：弹恰在旧帧盾面时 JS 挡/Rust 穿（浮点相等概率极低）。
- **P2-5（Task 33 后移除）** `net-harness/match_e2e.rs` test-only shim 强制 bot kinds 回 Shooter（seed 77 抽到未实现武器致 e2e 零喷墨）——与 D-2「bot 驾驶仍 Spritzer、Task 39 接档位武器」一致，不掩盖生产 bug（生产 matchbuild 对未 wired kind 保留 kind、update 臂 no-op）。Task 31 后余 roller/brush/bow/blade/mitts 五把未接线，shim 保留、注释已改「Task 33 后移除」。
- **P2-6（注记，不改）** `CANOPY_MUL` drop/slosh 弹种未建模（本任务无此弹种，代码注释已标）。

## Task 31: 主武器批二——蓄力与流类（charger/spinner/bucket）
- **Status**: `completed`
- **Priority**: high
- **Depends On**: Task 29
- **Description**:
  - charger（蓄力曲线→激光/弹段、rangeMax、满充反馈；`['ev']` charge 相关记录对齐）；spinner（spin-up 蓄力→长流、burstMin/Max）；bucket（抛洒弧：drops/grav/落点泼溅 paint 形状）。
- **Acceptance Criteria Addressed**: AC-M3-1
- **Test Requirements**:
  - `rule` TR-31.1: 蓄力/释放时序固定种子逐值断言（chargeTime/burst 区间）。
  - `rule` TR-31.2: bucket 泼洒弧采样与 JS 口径对照（落点数/半径）。
- **Notes**: charger 激光为 hitscan 段 + 弹段混合，注意与 Hermite 播放层无冲突（激光是本地判定）。

### Task 31 Completion Evidence（2026-10-31）
- **TR-31.1 ✅**：3 个固定种子逐值测试全绿（`weapon/tests.rs`）。charger：60 帧 S 曲线逐帧断言（1.25/0.9375 系数）、满充 ding、释放 `c=max(0.12,charge)`/ink−=18·c/cooldown 0.28、tap 0.12 地板、半充 0.53125、20% 墨门槛 LowInk；hitscan 半充 dmg=lerp(40,99.2,0.5)=69.6、len≈5.7（求解器残差 ±0.05）、Impact radius 0.96、victim_slot、满充 160 击杀、空束 len=19+沿线 coverage>0。spinner：逐帧线性充、`burstT=lerp(0.35,1.8,c)`、65 帧 22 发（`while streamCd≤0 && guard<4`、每帧 ≤1 发）、尾 cooldown 0.2、流弹 speed 32.5/straight 0.165/size 0.12/life cap/trail `−(2.2−1.25)`。workspace **327 passed**（321→327：+6 TR-31 测试；tr29_2 骨架断言同步更新为 wired 集合）。
- **TR-31.2 ✅**：3 个测试全绿。bucket windup 帧 9 出 5 blob、逐 blob 速度 `15.5·(1−i·0.075)`/size `0.22−i·0.02`/radius `1−i·0.08`/start 回推 `−i·0.12`/volley 组号/belowFalloff/life 2.2/grav 20/drag 0.25/trail 1.6 逐值；pitch∈[0.28,0.32]、yaw≤0.025 抖动界；同 seed 逐值复现+异 seed 分歧。`lob_aim`：hd=6 收敛、重放积分（同款交点插值）落点 ±0.05、超程/过近返回 false 且 dir 不变。volley：双 blob 同 victim 单 `Hit` 55、第二丸穿组、200 帧后组记账清理、belowFalloff 抬高 start 后 dmg=55·(1−1.7·0.12)=43.78。
- **架构落地（spec §1–§5）**：`weapon.rs`——`_charger`/`_spinner`/`_bucket` runner 臂（S 曲线/线性充/flick windup，D-T31-1 `charger_release` 延迟一拍）、`fire_charger` hitscan（raycast+`segment_capsule_dist` 最近 victim+沿线 splat+终点 splat+`['ev']` charge/len）、`fire_stream`、`fire_slosh`+`lob_aim`（步长 SIM_DT、sweep+二分）、`Projectile` 新字段 `below_falloff/below_falloff_max/volley`（serde default）、`volley_next/volley_hits/volley_alive` 记账与 despawn 递减清组、`step_one` volley continue+belowFalloff 减伤、`is_wired += Charger|Spinner|Bucket`、`charging_flag()/flick_flag()/id_str()`。`match_.rs`——`MatchEvent::Fire` 透传 charge/len、charger drain。`net.rs`——`F.CHARGING`/`F.FLICK` 实值化。`net/replication.rs`——`record_fire` charger 跳过 `['p']`、payload 追加 charge/len（r3、键序 actor→weapon→muzzle→dir→charge→len）、`['ev']` weapon id 改真实 kind id（修 Task 30 遗留恒记 spritzer）。**f32 偏差**：蓄力时钟 `>1−1e-6` 吸附 1.0（对齐 JS double 溢出钳位，charger/spinner 同法）。
- **门禁**：全量九段矩阵全 exit 0，日志 `rust/tools/verify/logs/task31-gates-20261030T184700Z.log`（fmt/clippy/test/wasm32/cargo tree/trunk release/headless 600 帧/JS 树零改动；workspace 327 > 321 单调递增）。

### Task 31 Review 附录（独立评审：2 P0 / 3 P1 / 2 P2 → P0+P1 当场修复，P2 注记 → 门禁复验全绿）
- 评审员实核通过：数值全走 tuning（S 曲线/burst lerp/blob taper/belowFalloff/伤害公式逐字对齐 JS）；RNG 消耗序（slosh side→pitch→seed ×5、stream spread→seed、charger line→terminal seeds、uSeed 呈现删法已记录）；charge/len 仅 `['ev']` r3、charger 无 `['p']` 守卫、serde default 旧回放兼容；volley 记账无泄漏、continue 语义对 L1599；spinner 不置 streaming、ch 槽=charge；6 测试锁 JS 逐值、MM=1e-3；JS 树零改动。
- **P0-1 ✅** `['ev']` payload weapon id 恒记 Spritzer 规格（Task 30 遗留、Task 31 charger ghost beam 链路直接依赖）→ `record_fire` 改 `m.kinds[i].id_str()`（config.js id==kind 字符串），远端 `WEAPONS[e.weapon]` 分发恢复正确。
- **P0-2 ✅** 首轮门禁 FMT/CLIPPY 未绿（vec3 换行、collapsible_if、manual_range_contains）→ 修复后复验九段全 exit 0。
- **P1-3 ✅** bucket windup 未置 `F.flick`（JS netmatch.js L740）→ 新增 `flick_flag()`、net.rs 置 `flags::FLICK`。
- **P1-4 ✅** D-T31-1 等价性声明过宽 → 注释/spec 补记残余：JS 逐 actor 移动+武器串行，sim 两遍循环全体先移动，近失束 victim 扫描位差最多一帧（~0.13m@8m/s）。
- **P1-5 ✅** spinner/bucket `['p']` 按 Spritzer 规格重建的延后偏差 → 落盘 spec §5 偏差清单（ghost 仅表现层，随 Task 36 wire fixture 修）。
- **P2-1（注记，不改）** f32 蓄力时钟吸附：注释+spec §1 已记录；更干净的 f64 累加方案留后续。
- **P2-2（注记，不改）** `lob_aim` 当前 sim 内不可达（aim_point 恒 None，spec §4 已注明「仅 Some 时启用」，Task 36 播放侧/瞄准点接入后生效）。

## Task 32: 主武器批三——近战扫类（roller/brush/blade）
- **Status**: `completed`
- **Priority**: high
- **Depends On**: Task 29
- **Description**:
  - roller（charge 带墨滚动/flick 抛墨/撞击判定，weapons.js L218-290 段对齐）；brush（拖墨细线/侧扫泼溅）；blade（冲刺斩：dash 位移、斩击弧喷墨、blade-fx 语义简化档）。
- **Acceptance Criteria Addressed**: AC-M3-1
- **Test Requirements**:
  - `rule` TR-32.1: 三类扫击的墨迹覆盖带（固定路径下覆盖格数断言）。
  - `rule` TR-32.2: flick/侧扫的抛射物与近距判定边界用例。
- **Notes**: 近战判定走 owner 本地 + splat 广播（同 Spritzer 命中模型），无新 wire 类型。

### Task 32 Completion Evidence（2026-10-31）
- **TR-32.1 ✅**：固定种子逐值测试全绿（`weapon/blade_tests.rs`，18 测试）。roller 拖行：≥0.28 m 一组 3 splat、ink=1.1×moved、Impact radius=rollWidth/2、40 帧 9 组、rng 消耗 3×组数；brush 拖行：≥0.22 m 一条细线、ink=0.32×moved、splat radius=brushWidth×0.45、30 帧 14 线、rng 1×组数；brush swipe：0.04 s wind-up 帧 4 释放、5 滴逐值（RNG 序 speed→radius→seed→cu、`swipeSide` 抛时翻转故首扫 side=−1、cooldown 0.16/firing_t 0.25）；blade tap 新月逐值（drop y=0.98+(0.5−t)·0.12、muzzle y=0.98）；heavy 波：`blade_wave` 交接记录、甲板条纹 14+1 组=29 draws、wall burst Impact radius=waveEndRadius 1.05/dying=0.14；void 掠过（评审后新增）：越界 0 draws 0 Turf、end spatter raycast miss 不抽 seed、paint_acc 仍复位。
- **TR-32.2 ✅**：边界/时序测试全绿。flick：pressed 边沿、0.22 s wind-up（14 帧）、9 滴逐值、cooldown 0.40、低墨 click empty 0.45 锁且保持滚动；brush moving boundary；drop dmg_far lerp 四段长采样（容差 0.5——`segment_capsule_dist` 6 步+三分 8 轮在 t=0 边界收敛偏差 ~0.003·L，已注记）；delay 冻结首帧；roller melee 窗（140 一击必杀→每帧复活 victim）；brush melee box/CD；blade melee 弧（75°±ε、face-hug 豁免 `d<hr+0.4 && fwd>−0.15`、每 swing 一次）；lunge 安全采样；ghost 波静默（0 事件 0 draws）；busy/swing 窗（swing 于 press 帧末 t=0 创建）。
- **架构落地（spec §1–§5）**：`weapon.rs`——`_roller`/`_brush` runner 臂、`fire_flick`/`fire_swipe`+`push_drop`（`DropSpec` 字面量带 `// JS L…` 锚点注释）、melee 弧、`is_wired += Roller|Brush|Blade`、`blade_wave.take()` 交接。`weapon/blade.rs`（新增 955 行）——`BladeKit` 状态机（hold/charge/store/cd/queued/swing/lunge，take/store 模式）+ `BladeWave` 世界对象（条纹 raycast/敌体穿刺/墙爆/end spatter/ghost），`update` 对齐 JS L128 `MAIN_KITS.blade.update`。`match_.rs`——`SimEvent::Kit`→`kit_out`、`blade_wave`→`m.waves` 统一步进。`net/replication.rs`——local `TickEvent::Kit` 发送、`"blade"` 接收分支推 ghost 波、ghost round `dmg_far=None`/`delay=0`。`actor.rs`——`FireGate` 扩展。无新 wire 类型（复用 `['k']` Kit 记录）。
- **门禁**：全量九段矩阵全 exit 0，日志 `rust/tools/verify/logs/task32-gates-20261030T224938Z.log`（fmt/clippy/test/wasm32/cargo tree/trunk release/headless 600 帧/JS 树零改动；workspace **345 passed** > 327 单调递增）。

### Task 32 Review 附录（独立评审：1 P0 / 1 P1 / 2 P2 → P0 修复、P1 核实驳回、P2 全部落地 → 门禁复验全绿）
- **P0 ✅** stripe rng 抽取未门控 `g.hit`（JS blade.js L353-359 `Math.random` 是 splat 实参，raycast 命中后才求值）→ `blade.rs` 两次抽取移入 `g.hit` 门内，ghost 仍抽取（mute 不抑制实参求值）；新增 void 掠过测试锁死「miss 时 0 抽取」。
- **P1 ❌（驳回）** blade `empty_click` 缺 `a.isLocal` 门控——核实：`match_.rs` L562-566 对 remote actor 直接 `continue`，runner pass 根本不执行，JS `isLocal` 语义已由调用点覆盖，无需再门控。
- **P2-1 ✅** `fire_flick`/`fire_swipe` `DropSpec` 字面量缺 JS 锚 → 补 `// JS L1226-1227`/`// JS L1231`/`// JS L1404-1405`/`// JS L1409`。
- **P2-2 ✅** void 用例缺失 → `tr32_1_blade_wave_over_void_draws_nothing`。
- **门禁首跑暴露 clippy 6 错**（`ensure_blade` 死代码删除、`BladeKit` 补 `Default`（连带 `unwrap_or_else`→`unwrap_or_default`）、negated comparison 改写、冗余括号×2、测试 helper `too_many_arguments` allow）→ 修复后九段复验全绿。
- **裁决/偏差记录**：`ground_stroke`→`Turf` 经 `SimEvent::Turf` 记账（同 Spritzer 命中模型）；D-T32-2/3 偏差（blade-fx 简化、远端波单记录）落 spec §5；`swipe_side` 抛时翻转（首扫 side=−1）；`spawn_at` 将 pos.y 吸附地面（甲板顶 y=0，全部 y 断言基准 0.0）；三分搜索边界偏差以容差 0.5+内点采样处置。

## Task 33: 主武器批四——特殊机制类（bow/mitts）
- **Status**: `completed`
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

### Task 33 Completion Evidence（2026-10-31）
- **TR-33.1 ✅**：18 个固定种子逐值测试全绿（`weapon/bow_tests.rs`）。shot 档位表逐值（`bow_shot`：0.449→tier0 `lerp(14,20,u)`=19.9867/range 7.4944/speed_tap/fan_tap、不 lodging 无 fuse 无 burst 组；0.45→tier1 起 lodging+fuse 0.7+burst 组 0；0.998→tier1 上段 37.97/16.4855/47.9782；满充→tier2 damageFull 55/fuse 0.55/burst 组 1）；地面 volley 绕 UP 三箭扇（RNG 按 i=−1,0,+1 三抽、侧箭 side=dmg·side_mul、`['k','bow']` 记录）、空中 volley 绕 dir×UP；draw→release 全流程（tank caps charge、低墨 empty click、sweep stalled draw）；箭飞行（straight 段→重力段、dive past range、tap impact splat、直伤先掉后插、lodged fuse burst 溅射 `lerp(bd,be,d/br)`+LOS 必需、ghost 全静默、trail drip 两抽）；MAX_ARROWS=192 池顶（新箭逐出、swap_remove 语义）；move_speed 蓄力/射击段。
- **TR-33.2 ✅**：26 个 mitts 测试全绿（`weapon/mitts_tests.rs`）。leap 端点逐值（`leap_velocity`：charge=0→10.9·(sin40,cos40)、charge=1→18.6、角钳 60°上/18°下、wall exit guard）；punch（左右手交替 cadence+RNG 消耗序、backfill 上限 2、cling 中仅右手、低墨 empty click、fist 寿终空中爆、直伤 38、溅射 falloff+直伤排除、LOS 必需、wall burst 一抽）；leap 蓄力（release 推离 1 抽、ledge cancel、stale seen 复位、jump hook 吞/起）；cling（进入同帧起 drain 8/s、glancing/floor 拒绝、四种退出：jump/脱离/墨尽 'empty'（4 墨 8/s ≤31 帧）/失墙、hang 冻速——贴墙态在 `MittsKit`，`Actor::step` 消费 `k.hang` 实现 JS actor.js L502-507）；land splash（glove 砸地+伤害环 lerp+25 抽+LOS）；guard 弧（正面 70° 内 ×guard_armor 减伤（测试覆写 0.65 断言）、背后满伤、leap 甲+grace、move_speed 全臂）。
- **TR-33.3 ✅**：actor 状态机扩展回归——M1/M2 既有移动/游泳/重生测试逐条不破（lib 303→305、workspace 345→389 单调递增）。
- **架构落地（spec §1–§7）**：`weapon/bow.rs`（新增）——`BowKit` 蓄力/释放状态机 + `bow_shot` 档位表 + `BowArrow::step`（straight/重力/dive/lodge/fuse burst/直伤 `route_hit` 路由/trail drip/ghost）；`weapon/mitts.rs`（新增）——`MittsKit`（charge/leap/cling/hang）+ `MittsFist::step` + `leap_velocity`/`land_splash`/`start_cling`/`drop_cling` + `KitArmor` 每帧快照（JS `damageTaken` L883-903，spec §4）；D-T33-1：`predictLeap`/telegraph 不移植（落点由真实物理接触决定）。`actor.rs`——`kit_armor: Option<KitArmor>` + jump hook（JS actor.js L311-313）+ `k.hang` 冻速 + `busy()` 偏差声明。`weapon.rs`——`is_wired` 12 类全真、`ensure_mitts` 惰性初始化（bow 的 kit 槽由 `bow.rs` 内部懒建）、move_speed/spread_deg bow/mitts 臂。`match_.rs`——arrows/fists 世界对象步进+`swap_remove` 删除、Mitts actor 循环预接 runner。`net/replication.rs`——`['k','bow']`/`['k','mitts']` case 0 ghost 重放（mitts 分支 `d.len()<8||d[0]!=0.0` 守卫、ghost fist dir 原样拷贝）。**e2e shim 移除**：net-harness `match_e2e.rs` Task 30 bot-kind 强制 Shooter 块删除（12 武器全部 wired），tr25 5 测试 + acceptance 4 测试全绿。
- **门禁**：全量九段矩阵全 exit 0，日志 `rust/tools/verify/logs/task33-gates-20261031T091834Z.log`（fmt/clippy/test/wasm32/cargo tree/trunk release/headless 600 帧/JS 树零改动；workspace **389 passed** > 345 单调递增）。

### Task 33 Review 附录（独立评审：1 P0 / 2 P1 / 5 P2 → 全部处置 → 门禁复验全绿）
- **P0-1 ✅** mitts guard 弧位置源错误：`route_hit` 的 `a_pos` 改 `Option<Vec3>`，burst 溅射（JS mitts.js L169）与 step 直伤（L212）改从 `foes` 按 slot+team 查 owner **实时位置**（原用拳 spawn 位/命中点，弧判定错位）；`land_splash` 两处传 `Some(a_pos)`；补回归测试 `tr33_2_guard_arc_uses_owner_position`（owner 在 victim 背后满伤 38/挪至正面 ×0.65 对照）。
- **P1-1 ✅（声明偏差）** `busy()` 未接 squid dive（JS actor.js L282）/idle refill（L358）——`Actor::step` 无 runner 句柄、12 武器共性，落 actor.rs 模块注释「Declared deviation (Task 33 review P1-1)」，留 follow-up。
- **P1-2 ✅** replication mitts ghost 分支守卫调序 `d.len() < 8 || d[0] != 0.0`（先长度后值，杜绝越界面）。
- **P2-1 ✅** 池满删除统一 `swap_remove`（bow.rs volley `arrows.swap_remove(0)` 对 JS `kill(0)` L71；match_.rs arrows/fists 同步）；`tr33_1_max_arrows_cap` 断言改 swap 语义（1000/1191 被逐、老箭 190 存活、`arrows[0]` 为新箭）。
- **P2-2 ✅** `start_cling` 注释记 JS L466 `botHold` 未移植（Task 39 bot 驱动）。
- **P2-3 ✅** replication ghost fist dir 去 normalize（JS mitts.js L866 原样拷贝），锚点改 L863-870/L866。
- **P2-4 ✅** bow `SimEvent::Impact` 移出 `if !p.ghost`（JS bow.js L207 emit 无条件）——ghost 箭仅发 Impact 不发 Hit 不扣血；补回归测试 `tr33_1_ghost_arrow_impact`。
- **P2-5 ✅** `weapon.rs` 新增 `ensure_mitts`（ensure_brolly 旁，ink_min=`w.mitts.leap_ink_min`），`match_.rs` Mitts actor 循环 a.step 前调用（修 kit 状态槽首帧缺失）。
- **锚点 ✅** actor.rs guard 弧注释改「JS mitts.js L888-889」。
- 评审员实现侧疑点两条核实：① `SimEvent::Hit` 携减伤前原始伤害与 JS `applyHit` L1553 `emit('hit',{damage:dmg})` 一致（armor 在 `victim.damage()` 内应用，actor.js L184）；② `foes` 扫描不含 owner 仅测试直调退化路径，match 实路径 `FoeScan::all` 全体包含，无问题。
- 修复复验：重跑首段 FMT 绿、CLIPPY 抓到新回归测试 `unused_mut` → 修复后九段复跑全 exit 0（389 passed），日志同上。

## Task 34: 副武器 15 个 + 设备网络语义（['b']/'dh'）
- **Status**: `completed`
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

### Task 34 Completion Evidence（2026-10-31）
- **TR-34.1 ✅**：58 个固定种子逐值测试全绿（批一 `sub.rs` 16 + 网络 `replication.rs` 5 + 批二 `sub/*_tests.rs` 37）。批一非 kit 10 种：`tr34_1_ink_cost_matches_the_subs_table`（15 key 墨耗逐值对 SUBS 表）、throw 弹道记录（`throw_velocity`+r2 舍入+`['k',nid,'subs',[0,gid,kind,x,y,z,vx,vy,vz]]` 字段序）、mine/beacon 放置（placed 分流、井下载体、beacon 可射）、sticky fuse 起爆、burst 直击+两段溅射、scan 落地云 track、seeker 追踪爆、curtain 衰减淡出+hp 通道、sprinkler 定时滴墨节拍、mine 武装→触发→track、mist  linger+poison（落点直写 4.5 / 边缘 tick 通道 0.6 双敌场景）。批二 kit 5 种逐机制：shaker（墨耗/投掷/level 爆数/hold 蓄力档/fuse 连爆节拍/伤害半径/0.38m 拖滴）、waddle（投掷记录/fuse 原地爆/锁定行走/到达爆/步速与行程帽/被射爆）、torpedo（投掷+脚下墨/一发在途/锁定展开发射/burst 核心伤害与墨滴/triggered 十滴/无人争夺小爆/被射落）、tracer（发射几何记录/地面弹跳掠行水洼/直击+终结记录/拖尾触碰单次标记/射程耗尽无记录/最大弹跳终结）、boomerang（tuning 逐值/投掷计划记录/生命周期相位+终结爆/hover 削杀/接触武装大爆/轨道擦伤/一发在途/owner 被 splat 哑火）。
- **TR-34.2 ✅**：`tr34_2_dh_owner_applies_the_ghost_hit_non_owner_ignores`（FakeTransport 三端：'dh' 仅投递 owner、owner 应用伤害、非 owner 收不到）、`tr34_2_bomb_ghost_record_is_presentation_only`（`['b']` ghost 不喷墨不判伤）、`tr34_2_torpedo_ghost_locks_launches_but_decides_nothing`、`tr34_2_waddle_dh_kills_the_real_one_and_broadcasts_the_end`（owner 侧终结经 `['k']` op 2 广播）、`tr34_2_boomerang_ghost_never_bursts_on_a_local_foe`、`tr34_kit_ghost_round_trip`（出站 gid 重写 nid 基/入站反算回本地 gid 闭环）+ tracer/boomerang ghost 各一。
- **TR-34.3 ✅**：`tr34_3_beacon_cap_evicts_the_oldest`（上限 3 逐出最旧）、`tr34_3_mine_cap_detonates_the_oldest`（上限 2 引爆最旧）、`tr34_3_torpedo_one_out`/`tr34_3_boomerang_one_out`（一发在途）、`tr34_3_waddle_shoot_down_pops_it`。
- **架构落地（spec §0–§7）**：`sub.rs`（新增 2956 行）——`SubSystem`：`use_sub`（L71-79 throw/plant/placed 三分流）/`update`/`_step` 状态机（fly/stuck/run/cloud/mist/curtain/spray/mine/beacon/dead）、`net_ghost`/`net_hurt`、挡弹钩子 `block_shot`/`block_ray`/`damage_area`/`block_actor`（L609-672）、`Ctx::track`/`poison`/`blast`/`paint_at`/`hurt`；`sub/{shaker,waddle,torpedo,tracer,boomerang}.rs`（新增，共约 4.7k 行）——五 kit sub 实体状态机 + `['k',nid,<kit>,data]` op 表；`weapon.rs`——`shaker_hold`（L1318-1400）；`actor.rs`——`track`/`poison` 状态（减速/poisonTime）；`match_.rs`——runner sub block（JS weapons.js L130-155）+ splat bomb（weapons.js L1431-1507/L1711-1758）+ SubSpawn/SubEnd/DeviceHit/SubUse 事件通道；`net/replication.rs`——`['b']` BombRecord 九字段编解码+ghost 消费、`['k']` subs/kit 记录出站 gid 重写（`nid*100_000+off%100_000`）与终结记录 idx→nid 翻译、`'dh'` 直发路由（owner 应用/非 owner 忽略）。声明偏差 D-T34-1..10（storm 不建云/beacon 超跳交互/it.sp 恒 false/sub:cantuse 不 FORWARD/DEVICE_H 常量/kit 墨路径恒 ink/shaker waggle 仅 yaw 等，见 sub.rs 模块注释与 spec §6）。
- **门禁**：全量九段矩阵全 exit 0，日志 `rust/tools/verify/logs/task34-gates-20261031T222107Z.log`（fmt/clippy/test/wasm32/cargo tree/trunk release/headless 600 帧/JS 树零改动；workspace **447 passed** > 389 单调递增）。

### Task 34 Review 附录（独立评审：0 P0 / 0 P1 / 3 P2 → 全部处置 → 门禁复验全绿）
- **P2-1 ✅** `sub.rs` 文档锚点错位修正：`UP`/`DOWN` L22→L25、`GRAV` L23→L28、`r2` L18→L22（对齐 subs.js 实际行号）。
- **P2-2 ✅** gid `off==100_000` 边界：出站重写 `gid%100_000` 使 off=100_000 的设备上线为 0，入站反算失配会静默丢伤害——`replication.rs` "dh" 臂加对称回映射 `off==0→100_000`（D-T34-5 注释）。
- **P2-3 ✅** ghost 设备伤害 `dmg > 0` 守卫（JS registry.js L68 `netHurt` 发送侧）：落点选 `dh_out` 唯一发送汇聚点加 `hit.d > 0.0` 过滤（等价 JS 单一闸口，覆盖 subs/waddle/torpedo/torpedoArea 全部 5 个 push 点；curtain 本地淡出在事件入队前已应用，不受影响）。
- 评审已验证一致面：net_ghost op 字段序/r2 舍入/gid 去重、tracer 终结记录 idx→nid 翻译、hit_routed 三向路由、kit 常量全走 tuning、RNG 确定性、测试期望值对 config.js 无适配性修改。
- 修复复验：`cargo fmt --all` + clippy `-D warnings` + workspace 447 passed 全绿，九段门禁复跑全 exit 0（日志同上）。

## Task 35: 特殊武器 19 个 + tick 槽实值化（sp/spx/spst）
- **Status**: `completed`
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

### Task 35 Completion Evidence（2026-11-02）
- **TR-35.1 ✅**：19 个特殊（13 kind，barrage×5 共享主体）全部落地并逐值固定种子断言——批一（核心层：生命周期 start/netGhost/tick/body/move/weapon/end/remaining/filterDamage + gauge + 三槽 + `['k','sp']` 泛化混合载荷 + 13 桩，447→468）；批二（slam rise/hang/fall、storm 云雨+dps34 只打本地、bubbler 免伤击退钳 6.5+分享 1.5、sonar reveal 8 s、kraken hop/attack cd0.65/免伤击退，→475）；批三（strike aim7/flight2.2/龙卷 dps62 pull1.6、zooka interval1.0/speed34/伤 180 穿刺、wail charge1.3/blast3.2/dps260 穿墙、blower max3/inflate1.0/r0.9→2.1/pop55、booyah charge4.5/cheer0.12/fuse1.5/伤 220→60、barrage×5 gap 逐变体+无墨+主武器可用，→482）；批四（jetpack hover3.8/boost9/interval0.55/直伤 125/splash70→30/结束跳回、zipcaster range21/speed22/hang1.4/cd0.35/冲击 100/30、crab hp460/gun0.075×18 双抽风/cannon gap1.15/炮弹 150→40/roll 护甲 0.35/后上暴露 front<-0.45∨dy>2.4、stamp turn1.7/静止 6.0/strafe0.35/interval0.42/guard0.45/前弧 70°/flip 双段，含 determinism 双跑五分量全等，→487）；评审修复 +1（ghost blast attacker drop 回归，→488）。workspace **488 passed**（基线 447 单调递增）。全量九段门禁日志 `rust/tools/verify/logs/task35-gates-20261102T175845Z.log`（fmt/clippy/test/wasm32/cargo tree/trunk release/headless 600 帧/JS 树零改动，全 exit 0）。
- **TR-35.2 ✅**：sp/spx/spst 非零样本 packer 逐字节（对 packActor L747-750 手算 fixture：`sp=Math.round(special)` 原始点值、`spx=indexOf+1`、`spst=specialNetState`——crab 位包 `roll|firing<<1|round(hull/TAU·255)<<2`、booyah `round(clamp(charge)·63)`）；`['k','sp']` 各 op（0/1/2/3/4，含字符串混合载荷）编解码 round-trip；`['bm']/['bc']` 容忍回归；`['b']` storm 弹体 + `it.sp` 旗标。
- **TR-35.3 ✅**：涂地充能 `area×0.8` 钳 `special_cost`、特殊墨不充能（`add_turf_no_special` 全口径：slam 落点/风暴云雨/barrage 弹/jetpack 尾焰/crab 足迹/stamp 砸击均走 no-charge 路径并有 gauge 不变断言）、splat 保留 `active?cost·keep:min(cost,special)·keep`、ready 边沿事件、booyah cheer +12、cost 随武器 specialCost。zone 加成（zones.js L260 gaugeHeld/Neutral）留钩子，Task 36 接。
- **架构落地**：新建 `special.rs`（生命周期/gauge/net_ghost 分发/世界对象注册表/blast/filterDamage）+ `special/{slam 族批二文件,barrage,bubbler,sonar,strike,zooka,wail,kraken,blower,jetpack,stamp,booyah,zipcaster,crab,world}.rs`；`actor.rs` 扩展（`special/special_id/special_active`、`SpecialActive` flags+KindState、`ActorStatus.reveal/shield/slow`、body/move_hook/weapon 钩子/hang/jump_back/superJump）；`net.rs` 三槽实值化 + KitRecord 字符串泛化；`net/replication.rs` from_actor/applyRemote（spx→special_id、spst→net_apply、0.6 s ghost 宽限）；`sub.rs` `it.sp` 旗标 + storm 建云；`match_.rs` special 步进与各 drain 队列（zip slam/stamp slam/jet fire·paint/marker 动作/超级跳落地擦 marker）。数值全走 `tuning.specials`/`weapons[*].special_cost`/`player.special_charge_rate`，JS 硬编码常量以命名 const + `// JS L…` 锚 + D-T35-4 记录。
- **文档**：`spec.md` §7 偏差定案 D-T35-1..22 全部落档（见下 Review 附录判定）；`PORT_MAP.md` specials.js 行更新为 M3 Task 35 状态。

### Task 35 Review 附录（独立评审：1 P0 / 1 P1 / 5 P2 → P0+P2-2 当场修复，P1+其余 P2 定案记录 → 门禁复验全绿）
- 评审员实读 JS 复核通过：批四四机制数值全走 tuning、帧序（body 短路/weapon 钩子先于 runner/tick 次序）对齐 actor.js L274-277；crab hurt 暴露公式（L2017-2022）、spst 位包 Math.round 语义（L2127）、gatling 每轮 2 抽 RNG 序（L1970）、stamp 前弧 70°/guardSweep/flip 双段、zip hang 1.4+挂墙（actor.js L505）、jetpack 结束跳回链（jump_back→superJump→marker back）逐行一致；网络三槽/充能规则/ghost 不喷墨/锚点抽查/测试逐值性全部实核。
- **P0 ✅** `special.rs blast()` 玩家伤害循环缺 ghost 攻击者 drop 门控（JS netmatch.js L711-717 `shouldApplyHit`：`attacker.remote → 'drop'` 先于 victim 检查）——接收端 ghost ts/sh/or/bubble 爆炸对本地玩家双重伤害。修复：攻击者 remote 掩码命中则整体跳过玩家伤害采集（`damage_area` 维持 `muted` 门控），注释带 JS 锚；回归测试 `tr35_b4_review_ghost_blast_attacker_drop` 四场景（ghost ts 落地爆炸无 Hit、owner 侧 Hit+HitSend 正常、ghost bubble 'x' 路由无伤害、本地气泡伤害正常）。
- **P1（定案=D-T35-9 豁免）** zipcaster 系绳射线用 `aim_from(None)=aim_dir` 而非 JS L1795 原始 aimPoint——sim 不建模 aimPoint 输入，回退语义两端一致；spec §7 显式记录。
- **P2-1（定案记录）** zip slam/stamp slam 与 duration 到期同帧时 record 次序 `[1,'time']` 先于 `[4,'zi']/[4,'sl']`（JS body/weapon 先于 tick）——仅接收端 ghost 同帧重放次序，无值差。
- **P2-2 ✅** crab ghost footprint 不抽 seed 与 paint_splat「mute 下仍抽 seed」约定不一致：实读 JS（GHOST.crab.tick L2118 无条件调 IMPL.crab.tick、paint helper L53 seed 调用点求值、mute 只砍 splat 本体 paint.js L470）判定接收端确实消耗 RNG → `paint_footprint` 加 `muted` 参 seed 恒抽；`ghost_tick` crab 臂补 `crab::tick`；replication 层 muted drain。
- **P2-3（定案=顺延）** crab 自身 hitR/hitH 放大受击盒（JS L1914/L2031）未实现，骑乘态命中盒偏小——记 D-T35-21，M4 或后续补。
- **P2-4（定案可接受）** super-jump 途中 owner splat marker 不 finish（JS L710 视觉回调未接），30 s 安全帽兜底——D-T35-11。
- **P2-5（定案记录）** zooka/blower `_swapWeapon` runner reset 未映射（match_.rs 仅 kraken/jetpack/stamp）——end 后首发射 1 帧级时序差，批二三以「runner 每帧被跳过」论证无值差。
- 偏差候选判定：D-T35-6（torpedo blast None/None）**不成立**（torpedo 全走 `apply_hit_routed`，归因与 drop 均与 JS 一致）；D-T35-5/7/8/10/12/13/14/15/16（已修）/17/18/19/20/22 成立可接受，全部按 §7 递增编号落档 spec.md。
- 修复复验：fmt/clippy/test 三门禁 + 九段矩阵全 exit 0（workspace **488 passed** > 447 基线），日志同上。

## Task 36: Zone Control 规则 + ['z'] 广播 + session 模式放开
- **Status**: `completed`
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

### Task 36 Completion Evidence（2026-11-02）
- **TR-36.1 ✅**：`match_/zones/tests.rs` 十组固定种子测试全绿——①`tr36_01` config.js L485-501 十六值逐条（一律读 tuning.zones）；②`tr36_02` center/home/away 三档扣速 + penalty 先扣 + tie point；③`tr36_03` penalty 公式取整/钳 0/tie 封顶/`start==count` +1 边界 + `['zp']` 先于 `['zo']` 次序断言；④`tr36_04` 轮换区间 [rotateMin,rotateMax]/MIN_STINT/finalLock 双路径/加时冻结；⑤`tr36_05` knockout `['ze']` 十字段 r3 逐值；⑥`tr36_06` time_up 三分支 + 加时四出口（comeback/retake/neutralised/cap）+ sudden death；⑦`tr36_07` 占领 flood→share=1、wipe hold 门控；⑧`tr36_08` `['zs']` 十字段 r3/Infinity→-1/0.5s 节拍；⑨`tr36_09` follower 全流重放 `state()` 逐字段与 host 相等；⑩`tr36_10` placeholder 分支（10×10 中心/side 锚点/镜像）。flood 本体 5 测（paint.rs：前缀翻格/ease 半程 0.75 vs 0.5/同 region 替换/dead 排除/wipe 归零）+ geometry 2 测（真实 tidewater.json zones 解析、poly/polys 归一化）。
- **TR-36.2 ✅**：①net-harness `tr36_zones_match_three_clients`（FakeTransport 三客户端 zones 完整局 8811 帧，7 项检查：决策重放收敛/快照纠偏/结算一致等，全 PASS）；②`['z']` 金标准：`gen.mjs` node 侧真 `ZoneControl` + stub G 捕获 `fixtures.zone_seq` **257 条**（8 标签 zz9/zo9/zs229/zp5/za2/zf1/zt1/ze1），连跑两次逐字节稳定；Rust wire 回环测试 decode→encode 逐字节 + 标签覆盖断言。录制臂位置经评审实读 JS 证实与 `_sendTick` 帧序同构（replication.rs，zone 记录先于同 tick splat）。
- **TR-36.3 ✅**：guest 收 `mode:'boss'` 仍 fail("Unsupported game mode")（既有回归测试保持绿）；host `set_settings(boss)` 拒；新增 zones 被接受 + zones 局启动测试。
- **规则平移**：`match_/zones.rs`——zones.js 425 行全路径逐字平移（zoneCells/insidePoly/mirror/placeholder/`_sample` follower 分支次序/`_zone_owner`/`_contest` WARN−0.08 重装/`_set_owner`/`_penalise`/`_tick`/`_fill_specials`/`update`/`_swap`/`_final_lock`/`time_up`/`_overtime`/`_tiebreak`/`_end`/`net_event` 8 case/`state()`）；`['z']` payload 走既有 `ZoneRecord{JVal}` passthrough（wire 层 M1 占位本任务闭合消费端）。
- **管线与接线**：`extract-layout.mjs` 按 maps.js L118-119 口径合并 `ZONE_DEFS` → `assets/maps/tidewater.json` 顶层 `zones` 键（center=circle(0,0,8.55) 24 点 y0 0.8/y1 1.3、side=rect(10,17.5,-20,-10)）；`geometry.rs` `ZoneLayout/ZoneDef`（serde from 归一化 polys||[poly]）；`paint.rs` `flood/step_floods`（paint.js L717-842 CPU 半）；`match_.rs` mode/duration/zones 步进序（zones.update→knockout→加时停钟→timeUp host 判定）/`end_zones`/`_zone_turf`（aim_probe 替代 aimPoint，D-T36-3）/judge zones 形态/`MatchResult.zones`（serde default 向后兼容）；`session.rs` 模式放开（boss 仍拒）；HUD 结算面板 zones 文案（镜像 hud.js L1096-1140）。
- **门禁**：workspace **509 passed**（488→495→505→509 单调递增）；九段矩阵 `rust/tools/verify/logs/task36-gates-20261102T205010Z.log`（fmt/clippy/test/wasm32/cargo tree/trunk release/headless 600 帧/JS 树零改动，全 exit 0；trunk 段首轮 cwd 误用后从 rust 根重跑成功，日志尾部追加留痕）。

### Task 36 Review 附录（独立评审：0 blocker / 2 P1 / 5 P2 → P1 落档 + P2-1 当场加固 → 结论可闭环）
- 评审员实核通过：zones.rs 逐路径语义（follower 分支次序/zp→zo 次序/knockout 仅非 follower/zs 十字段/8 case 重放）全部忠实；flood prep 与 step 对齐 paint.js L717-842；match 接线对齐 match.js L197-212；session 锚（L244/L292/L364）实核；fixture 257 条 8 标签无退化；锚抽查 100+ 处无假锚；tuning 17 字段无手抄；JS 树零改动。
- **P1-1 ✅**：D-T36-1..4 代码引用但 spec 未落档 → spec.md §7.1 补 D-T36-1..9 全量定案。
- **P1-2 ✅**：TR-36.2「逐 tick」口径弱化未声明 → 落档 D-T36-5（JS 决策事件天然滞后 ≥1 tick，逐 tick 严格相等 JS 自身亦不成立；实现为 12 帧静默窗口收敛 + count±1/penalty±0.05）。
- **P2-1 ✅**：`contest()` 加 `debug_assert!(z.owner >= 0)` 固化 JS L178/L183 短路前提。
- **P2-2/3/4/5（定案记录）**：f32/f64 乘积域 1 ulp 差（r3 量化覆盖，D-T36-6）；flood 默认分支未移植（调用点恒显式，D-T36-7）；结算文案合并适配（D-T36-8）；静默窗口采样缝隙（rotateMin=30s 下不可能触发）。
- 修复复验：fmt/clippy/test 三门禁 + 九段矩阵全 exit 0（workspace **509 passed** > 488 基线），日志同上。

## Task 37: 新地图 ×2 提取（saltpan + crossmarket）+ 碰撞冒烟
- **Status**: `completed`
- **Priority**: medium
- **Depends On**: Task 28
- **Description**:
  - OQ-1 定案后：`extract-layout.mjs` 跑 saltpan/crossmarket（layout.js 160/151 行 + surfaces 色/pattern + zones-data 合并 maps.js L118-121 口径）；half 镜像展开沿用 tidewater 流程；碰撞体块 + 出生点 + bounds 入 `assets/maps/`；无头碰撞冒烟（出生点落地、可行走采样、zone 多边形取格非空）。
- **Acceptance Criteria Addressed**: AC-M3-5
- **Test Requirements**:
  - `rule` TR-37.1: 提取产物与 JS layout 数值 diff 复核（stable-json）。
  - `rule` TR-37.2: 两图 Rust↔Rust turf 完整局 + 一图 zones 完整局（harness）。
- **Notes**: 装饰 props 不提取（M1 口径）；渲染复用体块材质档。

### Completion Evidence（2026-11-02）
- **TR-37.1 提取与数值 diff**：`extract-layout.mjs` 泛化为 stage CLI（tidewater/saltpan/crossmarket，未知 stage 拒绝 exit 1）；三图 stable-json 双跑逐字节一致（批一报告 + 评审复跑 + P2-1 修复后复跑三图 BYTE-STABLE-OK）。产物：`saltpan.json` 118 件（68 box/14 obox/36 ramp；turf 110/zones 116）、`crossmarket.json` 112 件（60/34/18；turf 106/zones 112）；`tidewater.json` 重生成纯增键（onlyIn/notIn:null ×359 + variantCounts，唯一改动行 source.commit，D-T37-3）。zones-data.js L31-39 两图 zone 多边形逐字入 `zones` 键（评审手核 orect(X31) toFixed(3) 全等）；surfaces 槽位 34-39 入 meta。Rust 解析交叉断言 11 项（geometry.rs tests：stage info/counts/zones 字面量/for_mode 旗标过滤/tidewater no-op/出生点落地射线/可行走采样 saltpan 1205、crossmarket 798-820，floor >500）。
- **TR-37.2 harness 完整局**：`MatchOpts.stage` + `run_match` 按 `embedded_layout(stage).for_mode(mode)` 单源构建 world/sim/nav/zone_layout（block id 位置序一致，JS main.js L242 口径）；新 scenario `tr37_saltpan_turf_three_clients`（1311 帧，winner=1，覆盖差 <0.5pp）、`tr37_crossmarket_turf_three_clients`（1311 帧）、`tr37_crossmarket_zones_three_clients`（8811 帧，zones 全 7 项断言含 `['z']` 无丢弃 + D-T36-5 静默窗口收敛，复用 OT cap 短化缝）——e2e 套件 9/9。
- **规则平移**：variants.js L6 `in_mode` 逐字（geometry.rs L200-204）；L16-18 `layoutFor` → `StageLayout::for_mode`；level.js L27 先滤后镜像与 Rust union 过滤等价性经 mirrorDef `...d` 旗标携带证明（评审实核 saltpan 配对旗标 10 件=5×2）；`CollisionWorld::from_layout` 定案 turf 缺省（main.js L242）。
- **管线接线**：lib.rs `embedded_saltpan/embedded_crossmarket/embedded_layout(id)`；BrushCommon `only_in/not_in` serde default；15 处构造点 + nav/tests LayoutMeta 同步。
- **门禁**：九段矩阵全 exit 0（fmt/clippy/test/wasm32/cargo tree/trunk release/headless 600/JS 树零改动），workspace **525 passed**（>509 基线，+13 sim +3 harness），日志 `rust/tools/verify/logs/task37-gates-20261102T213942Z.log`。

### Review 附录
- 独立评审（批一+批二全量 diff）：**0 P0 / 0 P1 / 3 P2**，APPROVE。锚抽查 18 处全真（variants.js L2/L3/L6/L16-18、main.js L242、maps.js L118-119、zones-data L5/L11/L31-39、两图 layout.js/surfaces.js 各锚）；计数算术复核（118=2+58+58、112=4+54+54、旗标件 kind 增量吻合）；`cargo tree -p inkwave_sim` 仍仅 glam/serde/serde_json；JS 树/server/ 零改动。
- **P2-1 ✅**：`in_mode` 对 `Some("")` 与 JS falsy 分歧（不可达）→ 提取器 `d.onlyIn || null` 对齐 truthiness + 文档注记；三图重跑字节不变。
- **P2-2 ✅**：`new_stages_spawn_pads_land` 死脚手架 `expect_blocks` 删除。
- **P2-3 ✅**：`for_mode` 文档补注 meta.variant_counts 保持 union 计数。
- 偏差定案 D-T37-1..4 见 spec.md §7.1（含 crossmarket side-zone 取格 turf/zones 等值的前提修正——过滤证据改块数+旗标在场断言）。

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
