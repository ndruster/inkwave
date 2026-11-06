# INKWAVE Rust/Bevy 移植 — M4 任务分解（Task 41–57）

> 前置：M1 Task 1–16、M2 Task 17–27、M3 Task 28–40 全部 completed（基线 554 passed）。任务编号延续；格式与门禁口径沿用 M3。**M4 主线 = 玩法优先（spec.md「M4 决议」）**：Phase A Boss 战（41–46）→ Phase B 玩法收尾（47–48）→ Phase C 表现完整度（49–56）→ 收尾（57）。

## Task 41: Boss tuning 提取 + Boss wire 金标准
- **Status**: `completed`
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

### Task 41 Completion Evidence（2026-11-03）
- **TR-41.1 ✅**：`extract-tuning.mjs` 扩抓 `BOSS`（boss.js L15-25 逐字段，含 `weapon` 面积倍率表 9 键）/`BOSS_MODE`（bossMode.js L8-13）/moves 时序与 hazard 参数（bossBrain/bossHazards 实读取字段集，含 frenzyR/frenzyDps、slam tele 1.15）/hitShapes 球表（socket/r/weak，10 项）；schema `inkwave.tuning.v2→v3`，`source.path` 如实记 `src/config.js + src/boss/*.js`。提取形态=字面块切片 eval + `assertContains` 钉源行（marker 缺失即 throw，无软回退；不 ESM import boss 模块，避免拉 THREE）。确定性：tuning.json 连跑两次逐字节一致（sha256 `ed71ae47…`）。锚定单测逐值对 boss.js/bossMode.js/bossHazards.js/bossNav.js 行号实读断言。
- **TR-41.2 ✅**：`net-fixtures.json` 新条目 = `B` 快照（full/delta）、`'bhit'`、`['bm']`、`['bc']`、res boss 形态；Rust 编解码逐字节回环（重生成连跑一致 sha256 `4d92b861…`）。`['bm']/['bc']` 由 Task 40 的「容忍」升级为「产生+消费」，未知 tag 容忍合同保留（wrong arity → None）。workspace **567 passed**（554→567，+13，基线单调递增）。
- **wire.rs**：tick 消息 `B` 字段插入 `e`/`c` 之间（JS 对象字面量键序）；`b: None` 不写键 → 非 boss 局 tick 字节与改动前逐字节一致，PROTO v1 兼容面不破。`BossSnap` 19 槽位与 boss.js L422-434 逐位对齐（hp 一位小数 vs maxHp 整数；f 位标志 1/2/4/8；mv=-1 哨兵；crabs 仅存活；s[18] 仅 full&&move；decode `len<18 → None`）。`bhit.c`=-1 哨兵（netmatch L150 形参默认）；res `st` 行恒 6 宽、Rust 保留短行容忍。
- **九段门禁**：`rust/tools/verify/logs/task41-gates-20261103T120224Z.log` — FMT/CLIPPY/TEST(567)/WASM(cargo check wasm32)/TREE/DETERM/HEADLESS(600 帧)/JS_TREE_CLEAN 全绿；**TRUNK=DEGRADED**：wasm-bindgen 处理 116MB wasm 被 SIGKILL（沙箱 OOM），门禁 3 次重试 + 手动 detached 循环 + 直连复验共 5 次全部 `signal: 9`，按 M3 先例 D-T40-1 判环境性、非本任务代码回归，复验结论已追记同一日志。
- **门禁脚本修复**：TRUNK 段改为 raw-file 捕获退出码 + retry≤3 + `grep 'signal: 9'` 断言，杜绝管道假绿（`| tail; echo $?` 恒 0 教训，已记 project_memory.md）。

### Task 41 Review 附录（独立评审：0 P0 / 0 P1 / 5 P2 → 全部当场处置 → 结论可闭环 APPROVE）
- 评审员实核通过：提取重跑逐字节一致；BossSnap 19 槽与 boss.js pack 逐位对齐；tick 键序与 None 不写键的 PROTO 兼容断言成立；JS 树零改动。
- **P2-1 ✅** spec §1 锚点过期→按 D-T41-6 实读修正文（招名 sweep/frenzy、slam tele 1.15、frenzyR/frenzyDps、SAFE_PAD、PAD_MARGIN、BOSS.weapon 键集）。
- **P2-2 ✅** `source.path` 未含 boss 源→已修并重生成产物。
- **P2-3 ✅（接受）** s[18] 仅 `Null` 视为无 move（JS `||null` 对 falsy 亦 null）→move 恒为对象或缺省，无行为差。
- **P2-4 ✅** wire.rs `bc` 注释失准→已修注释。
- **P2-5 ✅（接受）** `reachStep` 与 JS `SAFE_PAD` 不同名→schema.md 已注对应关系。
- 处置后复验：fmt/clippy/test 重跑全绿（567 passed），TRUNK 按环境性 DEGRADED 记录，不阻塞提交。

## Task 42: Boss sim 核心——Boss 实体/BossNav/hitShapes/快照
- **Status**: `completed`
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

### Task 42 Completion Evidence（2026-11-03）
- **TR-42.1 ✅**：`boss/mod.rs` ≡ boss.js 497 行全量平移（194 条 JS 锚注释）——实体状态机/bt 时钟/_spawnPoint（planIds 重心+评分+24 turnOk 修正）/hitShapes 世界球查询（rest pos 经 `pos+Ry(yaw)·rest` 变换，weak 先测 te-0.02 提前、rollHit 低层过滤、_hs 复用）/伤害倍率链（weak×stunned×weapon 表逐键测试 75/93.75/312.5）/splash LOS/crab R+0.4/rain 批 0.3s storm/remoteHit crab 分支/applyDamage 挡链+hp 缩放/checkPhase [.66,.33]/pack→BossSnap→unpack→_follow 往返逐字段/adopt/handoff。
- **TR-42.2 ✅**：`boss/nav.rs` ≡ bossNav.js 260 行——_build 直方图/kind 场/Dw/Dv/连通域/score、poseOk/turnOk/cast/lineOk/nearestPlan/A* path/randomPoint/MinHeap；**path 以 node 参考 harness（真 bossNav.js+level.js+Tidewater）逐点钉定**（两条固定起终点路径逐点一致 + floorY/网格/planIds/area 逐位断言）；Tidewater 冒烟（出生 poseOk、kindAt 越界=2、floorAt 吸附、inPad）。
- **提取扩展 ✅**：schema v4——`hit[].pos` rest 模型空间坐标（纯平移骨链静态求值 + `boss-hitpos-crosscheck.mjs` 真 THREE 动态骨链交叉校验 10/10 容差 1e-4，双证据链）+ `nav.planWall/planFloor/stride`（bossNav.js L12-14）；tuning.json 连跑逐字节一致；`tuning.rs` 类型+锚定测试（boss_hit_pos_values 逐球 1e-6）。
- **九段门禁**：`rust/tools/verify/logs/task42-gates-20261103T155339Z.log` — FMT/CLIPPY/TEST(**workspace 603 passed**=567+36，inkwave_sim 506)/WASM(cargo check wasm32)/TREE/DETERM/XCHK(crosscheck ok)/HEADLESS(600 帧)/JS_TREE_CLEAN 全绿；**TRUNK=DEGRADED**：3 次重试 wasm-bindgen SIGKILL（与 Task 41 同沙箱 OOM，累计 8 次），按 M3 先例 D-T40-1 判环境性，判定已追记日志，不阻塞提交。

### Task 42 Review 附录（独立评审：0 P0 / 2 P1 / 3 P2 → 全部当场处置 → APPROVE 闭环）
- 评审员实核：194 锚抽查 ≥40 条一致；boss.js L288-482/bossNav.js L144-260/bossModel.js/bossBrain.js/netmatch.js 逐式对照；tuning v4 双证据链成立；测试非同义反复。
- **P1-1 ✅** `BossHit.c` i8→i32（`crabSeq++` 无上限，id 255 会别名 -1 本体哨兵；bossBrain.js L336 实读）+ wire 大 id 往返测试（128/200/255/1000 逐字节钉定）。
- **P1-2 ✅** guest 上行 `d` 补 `r2(dec(d))` 量化（netmatch.js L153 实读）+ `log.sent/sentDmg` 计数补全（原被误归 Task 43）+ 测试期望修正（60.000004→60.0，raw d 进审计、r2 值上线）。
- **P2-1 ✅** blocked 事件加 `is_local` 门控（boss.js L294 实读）+ 非 local 无事件断言。
- **P2-2 ✅（接受）** f32 状态+`mv` 量化 ⇒ Rust-host↔JS-guest 混部不支持——spec D-T42-7 声明（PROTO v1 同语言部署口径不变）。
- **P2-3 ✅** lerp 注释更正（THREE f64）；嵌套 hypot vs 三参 Math.hypot（~1ulp）接受并记录 D-T42-7。
- **主上下文核验补漏 ✅**（评审未列）：`BossAttacker.has_nid` 门控——JS netmatch.js L152 `attacker.nid === undefined` 不发上行但仍本地预测，含专项测试。
- 处置后复验：fmt/clippy/test 全绿（inkwave_sim 506 passed），门禁重跑九段如上。

## Task 43: BossBrain + BossHazards——六招与危害求值
- **Status**: `completed`
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

### Task 43 Completion Evidence（2026-11-04）
- **TR-43.1 ✅**：`boss/brain.rs` ≡ bossBrain.js 376 行全量平移（目标选取评分 L114/attn 衰减/walk·turn 8 向 room 场侧移/选招权重表+last×0.3+force 审计钩/六招 `_start` record 构造逐键 r3/`_runMove` charge 位移·sweep lean·frenzy spin/crablet 管理 `_spawnCrabs`·`_crabs` 全式）+ `boss/hazards.rs` ≡ bossHazards.js L1-352（MoveId+record JVal 访问器/几何自由函数 moveTimes·movePhaseAt·chargeDist·chargeTime·barrelPos·beamAngle·ringStart·ringRadius/hz 状态上移 Boss（hits·cache·prevBt·dead）/victim-side 六招伤害逐锚（once 键、跨界带防穿隧、SAFE_PAD、submerged 免束、LOS、dps·dt、charge 击飞 L170 逐式）/paint host-only Splat{team1}（steps 离散化、pad 0.3、waterY 拒）/threat 查询（Task 47 消费））。测试 27 项：逐招 record 钉形（slam rings 按 phase、barrage n/四元组/throw 时钟、sweep span 对称、charge v/stun、crablets n、frenzy spin/rot0）、tele≥MIN_TELE、pace 缩放、同 seed 双跑逐 record 确定性、几何参考值（ring 线性、barrelPos 端点、beamAngle smoothstep、chargeTime 往返、reach 剪枝 Tidewater 实测）、伤害语义（once 不重、防穿隧、SAFE_PAD、submerged、dps·dt、remote 不伤）。
- **TR-43.2 ✅**：host/guest 同 record 逐 tick `hz_current`/几何一致（guest `on_move`→`hz_add` 幂等、`hz_update(live=false)` 不产伤不产 Splat）；crablets 生命周期（n 只生成、retgt 0.4 重选、crabLife 超时 pop、接触 1.05 pop+victim-side burst 伤害、pop 1.2s 清除、无 squad 入 pending 队列）；threat 逐招钉值 3 项；paint 3 项（host-only+pad+water 拒）。测试 20 项。接线：`Boss::update(dt,squad)` ≡ boss.js L100-116 时序（brain→fizzle→paint→hz_update→emit→model_update→rain）、model_update move 分支+_moveAim+target aim 实化、shape_active crack→phase≥3（D-T42-4 落地）、Stunned.dur、defeat/apply_damage/crab_burst/follow/adopt 全接。
- **提取扩展 ✅**：schema v4→v5——`brain.pref`（bossBrain.js L16 `[0,10,9,8]`，slice+assertContains 钉定）；tuning.json 连跑逐字节一致；`tuning.rs` 类型+`boss_brain_pref_values` 锚测；`tuning.schema.md`/`boss-hitpos-crosscheck.mjs` 同步 v5。
- **九段门禁**：`rust/tools/verify/logs/task43-gates-20261104T001844Z.log` — FMT/CLIPPY/TEST(**workspace 657 passed**=603+54，inkwave_sim 560)/WASM/TREE/DETERM/XCHK/HEADLESS(600 帧)/JS_TREE_CLEAN 全绿；**TRUNK=DEGRADED**：3 次重试 wasm-bindgen SIGKILL（沙箱 OOM 同 D-T40-1 先例），不阻塞提交。首轮门禁 FMT/CLIPPY 失败（新文件未跑 fmt+22 条 lint），修复后复跑：机械修 20（collapsible_if/map 迭代/needless_range_loop/`from_str`→`parse_id`/`not_above` NaN 语义 helper 等，行为等价）+豁免 2（测试记录构造器 too_many_arguments，扁平参数面保 JS 逐式对应）。

### Task 43 Review 附录（独立评审：0 P0 / 0 P1 / 5 P2 → 全部核销 → PASS 闭环）
- 评审员实核：bossBrain.js 376/bossHazards.js L1-352 全文对照，≥20 锚抽查全一致（含 _hurtLocal 六招逐式、charge 击飞 L170、paint steps、threat、_pickTarget 评分、record 键序 wire 兼容）；接线 boss.js L100-116/L128-149/L396/adopt L21-37 一致；已声明偏差（barrage jitter 预抽、hz_cache u64 键、hz_paint player 参、伪攻击手、seed 派生、crack 代理、Stunned.dur）逐条与 JS 行为核对自洽；测试钉行为非重言式、无脆弱钉。
- **P2-1 ✅（不改）** `reachStep` 命名系 spec 既有命名（值=SAFE_PAD 0.6，tuning.rs 注释+锚测澄清）。
- **P2-2 ✅（补记）** nav.rs `path()` `&mut self`→`&self`（RefCell/Cell）未列简报清单——机械适配行为保持，D-T43-9 补记。
- **P2-3 ✅（误报核销）** hz_cache 剪枝不清理——实核 hazards.rs L419 已 `remove`，评审误报。
- **P2-4 ✅（在案）** Move 事件每 move 一次 vs JS per-phase——spec §2 已声明，Task 46 若需再议。
- **P2-5 ✅（无差异）** follow 死亡分支 fizzle 与 dead 赋值顺序相反——`hz_fizzle` 不读 dead/killer，无观测差异。

## Task 44: BossMode 规则 + 离线完整局 + boss bots
- **Status**: `completed`
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
- **Evidence**:
  - 新增文件：`boss/mode.rs`（212 行 BossMode 规则）、`bot/boss.rs`（1088 行 bot boss 段 + 4 测试）
  - 改动文件：`boss/hazards.rs`、`boss/mod.rs`、`bot.rs`、`autopilot.rs`（run_boss 驱动 + Report boss_dmg/weak_hits 对账）、`match_.rs`（boss 分支）、`net.rs`/`net/session.rs`/`net/matchbuild.rs`/`net/replication.rs`（session 放行 + matchbuild boss + step 增参）、`tuning.rs`（MAPS schema v6 旗标）、`ui/mod.rs`（lobby MODE 三值 cycle）、`ink_render.rs`（step 签名适配）、`tools/extract/extract-tuning.mjs`（MAPS 旗标提取）、`tools/net-harness/src/match_e2e.rs`、`tools/verify/boss-hitpos-crosscheck.mjs`、`assets/tuning.json`、`assets/tuning.schema.md`、`spec.md`
  - 测试：TR-44.1×2（autopilot/tests.rs win+lose 双分支 + bossDmg 对账）、TR-44.2×4（bot/boss.rs crablet 感知/refill 门/slam 反应/集火选形）、session boss×3（guest 放行/set_settings 回退/start 名册）、tuning 锚点×2、mode hp×1；inkwave_sim 560→572 passed（+12）
  - 门禁：`task44-gates-*.log` 全绿（TRUNK DEGRADED 同 D-T40-1 先例）
  - 偏差注记：离线 Play 菜单模式选择推迟 Task 46（D-T44-5）；splatling 分支省略（D-T44-3）；matchbuild boss 单队构造留驱动层（D-T44-6）；guest boss cfg 放行不步进（D-T44-8）
  - 评审：P0=0；P1-1 已修（D-T44-9）；P1-2/P2-1/P2-6 误报；P1-3/4/5 由 §8 覆盖；其余 P2 留档

## Task 45: Boss 联网——快照播放/上行/迁移 + e2e + scenario E
- **Status**: `completed`
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

### Task 45 Completion Evidence（2026-11-04）
- **TR-45.1 ✅**：`replication.rs` tests 六 e2e（FakeRelay pair 先例）——①`tr45_1_1` 双端 BossMode 600 帧路径收敛（近 120 帧最优差 <0.5m + 末帧 <6m、hp 一位小数量化精确、phase 相等）；②`tr45_1_2` guest `hit()`→'bhit'→host `remote_hit`：log.recv/sent、recv_dmg/sent_dmg、host hp 差、stats_by_nid 归属全对齐；③`tr45_1_3` 迁移：adopt 后 sim=true/err 归零/brain 生成/无位置跳变/bt 推进；④`tr45_1_4` ['bm'] 时间线 guest `log.moves` 与 host 逐条 (t0,id) 相等；⑤`tr45_1_5` 击杀→5.2s 胜利窗口→judge→res：`bo` 逐字段重建、st 含 bossDmg 列、guest 时钟不钉零。
- **TR-45.2 ✅**：scenario E（`net-smoke.mjs` `bossScenario`，C/D 先例复刻）沙箱实跑 **DEGRADED ok=true**（Rust guest tab 被 SwiftShader wgpu 杀死，B 场景同历史表现；dump `net-smoke-2026-11-04T08-19-23-252Z.json`）；可断言子项=握手（`[nettest] loadout` 行 `mode=boss` echo）+ roster 跨认 + host 侧 `__G.netm.bossN>0`（guest 侧 B 到达留 REVIEW_CHECKLIST——wasm seam 无 boss 实体驱动，D-T45-9）；`nfr_m4_3` 带宽压测：最坏满快照（8 小蟹+move_rec）tick <65536B。
- **接线 ✅**：`BossNet` 扩 `prev_t/prev_raw` + `prev` 改 `Vec3`（D-T45-4）；`sample_boss` ≡ `_sampleBoss` L504-564 逐式（Hermite/离散 11 键/crab 配对/三段修正/阻尼 w=13，评审逐式全对齐）；八处钩子（send_tick `%10` full、on_tick B 消费单调门+40 trim、play_events 'bm' 无门/'bc' 有门、on_message 'bhit' is_host、on_leave adopt/handoff、drive_bots ctx.boss、res 形态+停钟）；host drain 在 send_tick 后 ride 下一 tick ≡ JS（评审 P1-1 修正）+ FX 事件回推（P1-2）。
- **app 层 ✅/留档**：inkwave crate 22 处 + net-harness 7 处 None 适配（非 boss 局逐字节不变）；nettest `netmode` 放开 boss；**BossMode 跨帧持有不可行**（Bevy Resource:'static 自引用，D-T45-9 借用面实读证据）——app 层 boss 实体驱动留 Task 46。
- **基线**：`inkwave_sim` 572→578 passed（+6）；`inkwave` 69；workspace 675 全绿；`cargo tree -p inkwave_sim` 依赖面不变；wire.rs/JS 树/`server/` 零改动。
- **九段门禁**：`rust/tools/verify/logs/task45-gates-<TS>.log` — FMT/CLIPPY/TEST(578+69)/WASM/TREE/DETERM/XCHK/HEADLESS(600 帧)/JS_TREE_CLEAN 全绿；**TRUNK=DEGRADED**（沙箱 wasm-bindgen SIGKILL 同 D-T40-1 先例，不阻塞）。
- **评审**：独立评审 0 P0 / 2 P1 / 4 P2 → P1-1（时序）/P1-2（FX 回推）当场修复复验；P2-1 §8 回填、P2-2/P2-3 注释更正、P2-4 在案 → PASS 闭环。

## Task 46: Boss 呈现占位档——体块剪影/telegraph 标记/HUD boss 条
- **Status**: `completed`
- **Priority**: high
- **Depends On**: Task 44
- **Description**:
  - bevy 体块 boss 剪影（集装箱壳/双钳/眼柄/腿，`m.update` 状态驱动 ≡ st 契约）；telegraph 标记（环/目标圈/charge 线/光束扇形）；crablet 体块；intro title card + roar 震屏占位；HUD boss 条（name/HP/phase notches/stun OPEN/伤害 chip）；结算 VICTORY/DEFEAT + bossDmg/weakHits 行；bossAudio 占位（现有 M1 占位档风格）。
- **Acceptance Criteria Addressed**: AC-M4-4（boss 部分）
- **Test Requirements**:
  - `rule` TR-46.1: 无头冒烟（headless 600 帧 boss 局不 panic）+ wasm 加载冒烟；HUD 状态映射单测（纯函数档）。
  - `rule` TR-46.2: 复核项登记 REVIEW_CHECKLIST（GPU 环境 boss 局视觉）。
- **Notes**: 高保真模型 Non-Goal（spec）；本任务只求"可玩且可读"。

### Task 46 Completion Evidence（2026-11-04）
- **TR-46.1 ✅**：无头冒烟——`--headless --mode boss` 600 帧不 panic（bt≈10/hp 满/events≥3 含 Intro/Move）+ turf 回归 exit 0；wasm 冒烟=TRUNK attempt 1 成功 rc=0。`BossHudState` 纯函数单测 14 项（intro 填充曲线/chip hold 0.6s 滞后排放/notch [⅔,⅓] 下穿锁存/`_writeBar` 4000 档量化+low≤0.2/OPEN! 倒计时/PHASE 2!·SHELL CRACKED! banner/callout 生命周期/3.2s title/SUNK! 1.3s is-gone/TIME'S UP!）；`boss_render` 几何参数对 JS 钉值 7 项（slam/barrage/sweep/charge/frenzy/fan 顶点/非 boss 局零实体）；`BossAudioDirector` 触发计数表 5 项（move×phase/hit·crit 三门 0.06/0.07/0.3/stun dizzy 0.35s/crablet pitch）；弹丸接线 4 项（weak ×2.5=75/flick 比值/charger 全额/crab 分支）。
- **TR-46.2 ✅**：REVIEW_CHECKLIST 登记 GPU 视觉复核项（剪影轮廓/telegraph 可读性/HUD boss 条布局/title card/结算行/crablet pop）；wasm seam guest 侧 B 到达探针（`[nettest] boss B recv`）留用户机器复核（JS smoke 未 grep 该行——tools/ 冻结）。
- **批次 A（sim）✅**：D-T46-1 owned 重构（`Boss`/`BossMode` 去生命周期，nav/tb/collision 按值持有；578 基线钉死行为零变化）——**D-T45-9 解**；D-T46-2 弹丸→`boss.hit/splash/rain` 接线（`BossRef` Option 注入，None 逐字节不变；seg_hit 优先序/drop 衰减/flick/blast direct 门/bomb 弹开/charger 截断/roller·brush 直连/rain 全锚；wid 坍缩与 subs/specials/kits 直伤链未接线如实顺延）。
- **批次 B（app 驱动）✅**：D-T46-3 `DemoSim.bm: Mutex<Option<BossMode>>`（Mutex 因 BossNav RefCell 非 Sync）+ `boss_events` 每帧 drain（**D-T45-3 收口**）；`replay_boss` maxHp 三档钉 93,600/70,200/121,680；D-T46-4 离线菜单 MODE 两档 cycle（**D-T44-5 收口**，zones 卡随离线 zones demo 再开）；D-T46-5 在线 `start_net_match` boss 双端构造 + nettest 探针（**D-T45-11 收口**）；roar 震屏（BossShake+camera jitter）。
- **批次 C（呈现）✅**：D-T46-6 剪影（壳长轴沿 z/双钳/眼柄/6 腿，st 驱动简档 ≡ bossModel.js L86-101）；D-T46-7 telegraph 五招标记（圈/环带/桶/扇/beam/车道/frenzy 全圆；占位简化=无墙体裁边声明）；D-T46-8 crablet 体块+pop 肿胀；D-T46-9 `ui/boss_hud.rs` 纯函数 S 机（零 Bevy；Move callout 帧间边沿合成——D-T43-5 落地）；D-T46-10 bossAudio 占位表（**D-T43-8 呈现档收口**）；D-T46-11 结算 boss 行（PHASE/bossDmg Σ/weak Σ）。
- **评审**：独立评审 0 P0 / 4 P1 / 4 P2 → 全当场修复（P1 壳长轴/sweep 扇悬空/beam y 分量/alpha 斜坡；P2 Cylinder 参数序/head z/腿 z/dizzy/dead 门/hover·is-zero 声明）→ APPROVE 闭环（D-T46-12）。
- **基线**：`inkwave_sim` 582（578+4）/`inkwave` 101（74→+27）/workspace **711 passed** 全绿；依赖面不变；JS 树/wire.rs/PROTO/fixture 零改动。
- **九段门禁**：`rust/tools/verify/logs/task46-gates-20261104T174853Z.log` — FMT/CLIPPY/TEST(711)/WASM/TREE/DETERM/XCHK(10/10)/**TRUNK(attempt 1 成功 rc=0，不再适用 D-T40-1 DEGRADED 口径)**/HEADLESS(turf+boss 双跑)/JS_TREE_CLEAN **全绿**。
- **D-T46-13 收口**：TRUNK 首跑 rc=1 系真实编译错（Bevy 0.19 `elapsed_secs`）非 SIGKILL；TEST flake `run_boss_lose_branch`（HashMap 迭代序→f32 求和序）修复=按 nid 排序求和，复验 30/30。

## Task 47: bot 深化——threat AI + 顺延战术项
- **Status**: `completed`
- **Priority**: medium
- **Depends On**: Task 44
- **Description**:
  - threat AI turf 段（bots.js L1288+ 实读）；stamp threat-steer（L460-470）；`_shieldFight`/`_canopyCtl`（brolly）；mitts/blade 深战术段；与 Task 38 档位表合并回归。
- **Acceptance Criteria Addressed**: AC-M4-3
- **Test Requirements**:
  - `rule` TR-47.1: 固定种子行为断言（threat 优先级/回避/掩体选择逐段）；难度单调性测试延续（tr38 ladder 不破）。
- **Notes**: 表现层无关；纯 sim。

### Task 47 Completion Evidence（2026-11-05）
- **threat AI 核心 ✅**：`bot/threat.rs`（1331 行）≡ bots.js L1260-1620 逐行——`threat_scan`（0.2+rnd·0.1s 扫描/reaction 门/`_thrMem` 按 ThreatRef）/`threat_score`（walker `senseRadius||7`、flyer `lockRange||6.5`、trigger 爆点 `||1.2`、eta、left 加权）/`threat_ctl`（aim 误差+wander L59、dev_trigger、shoot 站定/kite/×0.25、evade/swim/sidestep）/`thr_end`/`thr_drift`/`thr_evade`/`thr_sidestep`/`pick_evade`（dry_line 采样 L779-783）/`melee_window`/`dev_trigger`（stream/charge/burst/melee 族分支 L1477-1502）；`AimHold` ≡ `_thrAim`；`THREAT_AI` AtomicBool A/B 开关 + thread_local `THREAT_STATS`（关闭态零 RNG 零行为）。
- **视图/接线 ✅**：`sub.rs` `ThreatView/ThreatRef/threats()/find_threat()` ≡ waddle.js L664-685/torpedo.js L709-726 threatOf（`left!==undefined`/state 门控语义保真）；`BotCtx.launched`（7 构造点）+ `BotCtx.time`（≡ `G.time` 累加钟 = `projectiles.time`，评审 P0-1 修正）；step L1012-1032（threat_ctl→canopy_ctl→aim 覆盖→tail）；tail `thr_aim` 参数（snappy L643-645/aimPoint dist L658/钳地门 L660）；fight dodge 后 `_shieldFight`（L525）+ stamp threat-steer（L460-470）。
- **盾面/canopy ✅**：`shield_fight` ≡ L1507-1549（hold_fire/flank 侧向/穿盾条件/lob sub）；`canopy_ctl` ≡ L1554-1615（edgeX/edgeZ steer/break 判定/`_canBroke`）；`ShieldView` 每帧重建 ≡ refreshShields L178-190 + shieldOf L604-627（held `held_shield` 门 + launched cur_c/cur_n；swap-remove 滤己队顺序复刻 L1277）。
- **mitts/blade 深战术 ✅**：`bot/blade_tactics.rs` ≡ blade.js L455-620 botTactics 全文（SKILL/DIVE 档位、pre-charge/连斩/掩护/flank、`_bl_hold` 记账→blade.rs L914 同钟比较）；mitts ≡ L755-835（leap 规划 `predict_leap` 恢复、fight/paint 全分支 escape/pounce/wall/cling/travel、release gate）；`BotCtx` 扩 `time`、`ActorInput.bl_hold/mitts_req` 等（serde default，快照兼容）。
- **TR-47.1 ✅**：`bot/threat_tests.rs` 8 项（score 分支/scan 过滤/pick_evade 干地/dev_trigger 6 分支/shield flank+压火/canopy steer+break/stamp 锥+A/B 开关逐位对照）+ `bot/tests.rs` mitts pounce/blade 连斩 2 项；`inkwave_sim` 582→**592**（+10），3 轮无 flaky。
- **tr38 ladder 延续 ✅**：easy 1599.1s/normal 1631.7s/hard 1602.4s——端点门 hard≥easy **通过**（+3.3s）、kills 单调 **通过**（59/71/77）；严格 survival 单调 false 与基线同态（spec §5 记录并评审口径，REVIEW_CHECKLIST 在案：B/C 曾翻转→D 后恢复，A/B 实验证明源自 threat 行为本身且逐分支保真）。
- **tr37 回归修复 ✅**：threat AI 改变捕获相位暴露 zones follower 预测时延（frame 6913 pen 差 0.083=5/60）——`match_/zones.rs` 'zo' field2/'zs' field10 host 时钟 + `catch_up` 补偿（Rust 内部 JSON 面，互操作安全经评审确认；**JS 无对应新逻辑**，登记 D-T47-11；裁定闭环见下行）。
- **catch_up 专项评审 + F-1 修复 ✅（2026-11-05）**：机制 6 项核对通过；F-1（catch_up 缺 `_tick` tie-point 边沿检测）已补同式置位并复验（sim 592/net-harness 20 全绿、fmt/clippy 净，spec.md D-T47-11a）；用户裁定=兼容超集定位+网络一致性强化，D-T47-11 闭环（PORT_MAP 在案）。
- **评审**：独立评审 2 P0（P0-1 时钟接线/P0-2 ladder 门红=过期登记条目）+ 2 P1（catch_up/rebuild_shields 偏差登记 D-T47-11/12）+ 3 P2（日志清理等）→ 全处置闭环。
- **基线**：`inkwave_sim` 592/`inkwave` 101/workspace **721 passed** 全绿；`cargo tree -p inkwave_sim` 依赖面不变；JS 树/`server/`/wire.rs/PROTO/fixture 零改动。
- **九段门禁**：`rust/tools/verify/logs/task47-gates-20261104T223313Z.log` — FMT/CLIPPY/TEST(592+101+…=721)/WASM/TREE/DETERM(重跑逐字节一致)/XCHK(10/10 sockets)/HEADLESS(turf 完成+boss 600 帧 bt=10/hp 满/events=3)/JS_TREE_CLEAN 全绿；**TRUNK attempt 1 成功 rc=0**（非 D-T40-1 DEGRADED 口径）。

## Task 48: 全图提取——halyard/kelpline/lockgate/terraces/cargo + 旗标语义
- **Status**: `completed`
- **Priority**: medium
- **Depends On**: Task 41
- **Description**:
  - stage CLI 扩五图（halyard 内联 maps.js L30 特殊处理）；MAPS 旗标（onlineOnly/noBots/noBoss/theme/times）入 tuning；lobby stage 选择按旗标过滤（离线不列 onlineOnly、boss 房自动换图、noBots 关补位）；run-all.sh 扩全图。
- **Acceptance Criteria Addressed**: AC-M4-3
- **Test Requirements**:
  - `rule` TR-48.1: 五图 stable-json 连跑逐字节一致 + 碰撞冒烟（出生点落地/可行走）+ zones 合并（有 zones-data 者）。
  - `rule` TR-48.2: 旗标路径测试（cargo 离线不可见/boss 换图/noBots 开局阻断 noBotsStartBlock）。
- **Notes**: cargo 为 onlineOnly+noBots+noBoss——仅数据对等，实际开黑价值有限（如实注记）。

### Task 48 Completion Evidence（2026-11-05）
- **批次 A 提取 ✅**：`extract-layout.mjs` STAGES 扩 8 图（surfaceSlots 逐值对 `src/world/stages/<id>/surfaces.js`：kelpline{tarmac:31,quay:32,chequer:33}/lockgate{setts:40,engbrick:41,hoofsteps:42}/terraces{calce:43,cotto:44,pebble:45}/cargo{tarmac:46,quay:47,chequer:48}）；halyard 走 `src/world/maps.js` L30-114 `HALYARD` 内联分支（`source.path`=maps.js，surfaceSlots={}，intro/decor/water 声明忽略=D-T48-6）；`run-all.sh` 8 图行齐；`rust/assets/maps/` 落盘 8 JSON（primitive：kelpline 185/halyard 72/lockgate 138/terraces 441/cargo 185）；旧 3 图 JSON 仅 `source.commit` 戳更新（重提取正常行为）。
- **TR-48.1 ✅**：DETERM 门禁=8 图+fixture+tuning 重跑 sha256 前后逐字节一致；TR-37 碰撞冒烟两测扩全 8 图（spawn pad 落地+bounds 网格可行走，500 地板全过）；zones 合并断言扩 halyard/kelpline/lockgate/terraces（zone 数对 ZONE_DEFS：halyard|saltpan=2 余=1，cargo 无 zones 不进）；geometry.rs 新 5 图 parse 锁值测。
- **批次 B sim 资产面 ✅**：`lib.rs` 5 新 `*_JSON` const + 5 `embedded_*()` + `embedded_layout` 8 项 match（未知 id panic 文案不变式保持）。
- **批次 C tuning v7 ✅**：maps 段加 `theme`/`times`（缺 times 不写键），schema v6→v7（`tuning.schema.md` 同步、`boss-hitpos-crosscheck.mjs` 钉值连带 v7）；`MapFlags` 加 `theme`+`times`（serde default 向后兼容）；diff 核实纯增量（8 项 theme/times+schema+commit 戳）。
- **批次 D 旗标收口 ✅（TR-48.2）**：`tuning.rs` 新纯函数 `map_theme`（≡ config.js L513 含 JS 真值回退链：times 空串→theme→'day'）+ `offline_map_ids`（≡ L535 七项无 cargo 上游序）；`map_offline_ok` Task 44 既有复用；锚测 4 项（`maps_theme_times_values`/`map_theme_function` 8×2 值锁/`map_theme_empty_string_fallbacks` 合成回退锁/`offline_map_list`）；boss 换图/noBots 三态 session.rs 既有测覆盖核实（`set_settings_accepts_boss_and_falls_back`/`solo_on_a_no_bots_stage_blocks_the_start`/`no_bots_stage_needs_one_player_per_team`），未堆重复测。
- **collision.rs 顺带修复 ✅**：`face_hidden` 薄面（terraces 栏杆 z=0.085m）使 `clamp(0.05, su-0.05)` lo>hi panic——su/sv≥0.1 时与原字面量严格同值（评审实证：既有 3 图最小基元 ≥0.140 不受影响），<0.1 时 0.4× 内缩；调用方唯一（collision.rs L958 face 枚举）。
- **评审**：独立实证评审 **PASS（0 P0/0 P1）**，1 P2（map_theme 空串回退链与注释不符）→ 已修（filter 前置）+ 合成测锁定；`map_theme` 三入参 `'a` 保留（edition-2024 不可省略），`offline_map_ids` 省略过 clippy。
- **基线**：`inkwave_sim` 592→**601**（+9）/`inkwave` 101/workspace **730 passed**；`cargo tree -p inkwave_sim` 依赖面不变；JS 树/`server/`/wire.rs/PROTO/fixture 零改动。
- **九段门禁**：`rust/tools/verify/gate-task48.sh`（stage-6 扩 8 图重跑 sha 对账）——`rust/tools/verify/logs/task48-gates-20261105T084151Z.log` FMT/CLIPPY/TEST(730)/WASM/TREE/DETERM(BYTE_IDENTICAL)/XCHK/TRUNK(attempt 1 rc=0)/HEADLESS(turf+boss)/JS_TREE_CLEAN **全绿**。

## Task 49: 小地图
- **Status**: `completed`
- **Priority**: medium
- **Depends On**: Task 48
- **Description**: `minimap.js`（585）平移——paint 网格采样→纹理、队友/敌方点、zone 标记、boss 位置；HUD 布局集成；采样率/清晰度档位。
- **Acceptance Criteria Addressed**: AC-M4-4
- **Test Requirements**:
  - `rule` TR-49.1: 采样纯函数单测（覆盖率→像素映射固定输入断言）；无头冒烟。
  - `rule` TR-49.2: REVIEW_CHECKLIST 登记视觉复核。
- **Notes**: Phase C 开始；双轨验收口径（NFR-M4-5）。

### Task 49 Completion Evidence（2026-11-05）
- **TR-49.1 ✅**：`inkwave::ui::minimap`（新 2144 行，纯函数无 Bevy，仿 boss_hud 先例）≡ minimap.js 全 585 行分层平移——`Projector`（L101-109 toCanvas/_worldX/_worldZ+flip）；`RasterTables`（L112-187 块顶面栅格 hgt/top/nrm+turf 双线性采样表 pixCell/Fx/Fy/Sx/Sy，跳过规则逐字）；`draw_base`（L189-279 全像素公式：sea/swell/泡沫/块色提亮/grate/height tint/hillshade/投影阴影/AO/rim/hatch/5m 网格+sunset 色板）；`draw_ink`（L290-338 经 `PaintGrid::grid()/face_meta()/cell_dead()/version` pub 访问器，**sim 零改动**）；`zone_paths`+`draw_zones`（L542-576 虚线幽灵/活动 fill+描边/中立跑马）；`compose`+`MinimapClock`（L341-371：timer 0.15/BANDS=3/version 轮询/theme·teamKey 边沿/flashT 衰减）；档位 `MinimapTier{Low=4,Med=7(JS 默认),High=10}` pxPerM。点数据层 `player_dots` ≡ main.js L1293-1304 全过滤链（swim 含 actor.js L994 `specialActive.body||noSquid` 覆盖，评审 P0 修复+锁定测；climb 分支 sim 无对应如实豁免）；`boss_dot`=Rust 扩展（JS 无，FR-M4-10）。29 测固定输入逐值断言（覆盖率→像素映射全 0/全 1/棋盘/单格边界/dead/fast path/emboss/flash 边沿；确定性 build 逐字节一致；期望值独立手算非实现复算）。
- **TR-49.2 ✅**：REVIEW_CHECKLIST「Task 49 小地图」登记（清晰度三档/采样率观感/base 层对照/点标占位档/zone 虚线/HUD 布局+TAB 弹簧/flip 时序差/live 层顺延注记）。
- **HUD 集成 ✅（批次 C）**：`minimap_ui.rs`（396 行）——`MinimapRes`+`minimap_update`（clock 分带→compose→整图重传沿 ink_render 先例；viewer flip≡setViewerTeam；theme 经 `map_theme(id,"day")`）+`minimap_ui`（角标 `fit(14.5u)` 左下、`KeyCode::Tab` 展开 `fit(min(H*0.78,W*0.6))`、弹簧 k=190/c=21 ≡ hud.js `_updMap` L1724-1744；dots=12px 方块色标占位（team_color/self 旋转/dead .7）、boss 点仅 boss 局存活）；系统挂 Screen::Hud，headless 冒烟路径不受影响（WorldPlugin 非 headless 分支既有门控）。
- **评审**：独立实证评审首跑 **FAIL（1 P0/1 P1/9 P2）**→修复复验：P0-1 swim 缺 specialActive 覆盖（水下 body/noSquid 特技敌方点错误隐藏）已修+新测；P1-1 dots alpha 注释捏造 CSS 锚已对齐真值（is-dead .7/tracked=ping 环顺延）；P2-2/P2-4 注释与测试期望值同修。留档 P2：zone 逐 part even-odd vs JS nonzero（当前 8 图无多 part 重叠）、flip 重建帧 ink 立即全量（≤0.15s 一次性时序差）、hex_rgb 短串切片（输入全编译期常量不可达）、zone 局每帧整图重传（≡JS 每帧 canvas 重绘）、flip 重建旧 Image handle 未释放（每局一次）。50+ 锚点抽查通过。
- **基线**：`inkwave` 101→**130**（+29）/workspace **759 passed**（730→759 单调递增）；`inkwave_sim` 601 不变、依赖面不变；JS 树/`server/`/wire/fixture 零改动。
- **九段门禁**：`rust/tools/verify/gate-task49.sh`——`rust/tools/verify/logs/task49-gates-20261105T111904Z.log` FMT/CLIPPY/TEST(759)/WASM/TREE/XCHK/HEADLESS(turf+boss 600 帧)/JS_TREE_CLEAN **全绿**；DETERM 首跑 MISMATCH=存量 9 产物 `source.commit` 戳刷新（Task 48 提交后重提取正常行为，diff 核实仅 commit 字段 5343cd2→476b967，产物随本任务提交）→复跑 **BYTE_IDENTICAL** 已追记；**TRUNK=DEGRADED**：3 次重试 wasm-bindgen SIGKILL（沙箱 OOM 同 D-T40-1 先例），不阻塞提交。

## Task 50: 音频过程化合成 + 音乐 + bossAudio
- **Status**: `completed`
- **Priority**: medium
- **Depends On**: Task 46
- **Description**: `audio.js`（2505 WebAudio 合成）平移选型（程序化样本烘焙 vs 实时合成——spec 设计段定）；`music.js`（1490）曲目状态机；`bossAudio.js`（177）；SFX 触发 ≡ JS 事件总线订阅点全表对账。
- **Acceptance Criteria Addressed**: AC-M4-4
- **Test Requirements**:
  - `rule` TR-50.1: 事件→音效映射表逐项单测（触发计数）；合成参数锚定测试。
  - `rule` TR-50.2: REVIEW_CHECKLIST 登记听感复核（含静音开关回归）。
- **Notes**: 不新增依赖前提下选型（sim 依赖面不变；inkwave crate 可用 bevy_audio/kira——以现有依赖为准，实读后定）。

### Task 50 Completion Evidence（2026-11-06）
- **选型定案（D-T50-1）**：**过程化样本烘焙**——WebAudio 图离线求值→44.1k mono f32 PCM→`PcmSound`/bevy_audio 播放；否决实时合成（不新增依赖+wasm 风险+无头不可测）。惰性 bake：首用烘焙+缓存 handle（`SfxAssets::cue`），`inkwave_sim` 依赖面零改动（NFR-M4-1）。
- **TR-50.1 合成核 ✅（批次 A）**：`audio/synth.rs`（新 1063 行，纯逻辑无 Bevy）≡ 逐式平移——`Rng`（mulberry32 ≡ music.js L30-38，固定种子替代 Math.random，按名 FNV 播种=声明偏差 D-T50-6）；`Param`（AudioParam 自动化 setValueAtTime/linear/exponential/setTarget，f64 逐样本求值）；`perc/ahr/adsr/pts/sweep`（≡ L174-215）；`Osc` 相位累加+`Wave` 波形+`pulse_coeffs/stroke_coeffs`（≡ PeriodicWave 谐波表 L139-170）；`Biquad`（RBJ cookbook ≡ createBiquadFilter）；`noise_buffer`（≡ L48-95 2s 无缝循环/种子 11/23/37/双遍 IIR/DC block/RMS 0.3，**Paul Kellet pink noise b6 每样本更新 ≡ L71**）；`drive_curve`（≡ L126-136）；pan/mix/delay 缓冲域链。`audio/graph.rs`（新 881 行）：WebAudio 图离线求值器（节点类型/连接/时间轴调度）。锚定测试逐值钉 JS 参考（含 node 现跑同种子 pink noise 前 16 样本）。
- **TR-50.1 SFX 全表 ✅（批次 B）**：`audio/sfx_table.rs`（新 9322 行）——148 SFX 全表 ≡ `SFX_GROUPS`（audio.js L2479-2501 逐组 1:1 对账，事件→音效映射表逐项单测）；`SfxGate` ≡ `play()`（L258-304：voice steal/MAX_VOICES=48 JS 实值 L26/冷却/全局音量）；`bake()` 返回 `Option<Vec<f32>>`（未知名不 spawn）。4 条 pink 相关 golden（splat_big/sonar_ping/zone_hum/boss_roar）随 b6 修复重生成。
- **TR-50.1 音乐 ✅（批次 C）**：`audio/music.rs`（新 3088 行）——9 曲（title/menu/battle/battle_final/results_win/results_lose/boss/boss_2/boss_3）状态机+`compile_song` ≡ music.js L601-777 grid-string 编译器逐语义；fade/remap/escalation/duck 全链；`set_volumes` ≡ audio.js L165-172（taper^1.5/τ=0.04）；file-backed 曲目本仓无资产恒 false（声明）；全曲烘焙冒烟 `#[ignore]`（性能门限）。
- **TR-50.1 bossAudio+设置面 ✅（批次 D）**：`audio/boss_audio.rs`（新 383 行）——BossCue 6 类真实播放替换 Task 46 记录表占位（≡ bossAudio.js 177 行订阅点）；`audio.rs`（1640→1716）cue 守卫放宽至全 148 表、`AudioVolumes` 三轨接线、mute 回归测 `mute_switch_stops_spawns_and_global_volume`（TR-50.2）；`world.rs` Update 链插 `music_scenes`/`music_update`/`boss_audio_playback`。
- **TR-50.2 ✅**：REVIEW_CHECKLIST「Task 50 音频」条目登记（音色对照 JS/混响缺失声明/音乐状态机/bossAudio/设置面/事件缺口 42 名 kit 系/烘焙性能/静音回归）。
- **评审**：独立实证评审 **FAIL（1 P0/0 P1/11 P2）**→当场修复复验：P0-1 pink noise IIR `b6` 恒零不可变（违反 ≡ 合同，synth.rs L453-484 改 `let mut b6`+y 后更新+跨双 pass 保留 ≡ music.js L71，补逐样本钉 JS 参考测，连带 4 golden 重生成）；P2-1 steal 后全局 voices 未剪除（同步移除+断言）；P2-8 bake 未知名烘空缓冲（改 Option+不 spawn+新测）；P2-9 loop 起播 60ms fade-in 未声明（boss_audio.rs 模块头 Deviations 补注释）。留档 P2（不修，已裁决）：P2-2 jitter 二次 clamp 不可达、P2-3 parse_chord 带 quality 转位和弦、P2-4 parse_note 双位八度、P2-5 短图案 cells[s] panic（现存 9 曲数据全合法）、P2-6 DuckState NaN clamp、P2-7 Biquad clamp 注释略夸大、P2-10 pulse/stroke f64 精度差、P2-11 根目录游离 task38-ladder-*.log 勿入库。偏差裁决表 9 项全部「可接受」（D-T50-6 五条+评审补充）。
- **基线**：`inkwave` 130→**180**（+50）/workspace **759→809 passed**（单调递增）；`inkwave_sim` 601 不变、`cargo tree -p inkwave_sim` 依赖面不变；JS 树/`server/`零改动。
- **九段门禁**：`rust/tools/verify/gate-task50.sh`——`rust/tools/verify/logs/task50-gates-20261105T172415Z.log` FMT/CLIPPY/TEST(809)/WASM/TREE/XCHK/**TRUNK(attempt rc=0，本轮成功)**/HEADLESS(turf+boss)/JS_TREE_CLEAN **全绿**；DETERM 首跑 MISMATCH=存量 9 产物 `source.commit` 戳刷新（476b967→2c9ae6c，Task 48/49 同态；diff 核实仅 commit 字段）→复跑 **BYTE_IDENTICAL (rerun)** 已追记，产物随本任务提交。

## Task 51: fx 系统 + flood 呈现层
- **Status**: `completed`（2026-11-06 完成，评审 PASS：0 P0/7 P1/8 P2 全部当场处置；证据见下方 Completion Evidence 与 spec.md「Task 51 收口」）
- **Priority**: medium
- **Depends On**: Task 50
- **Description**: `fx.js`（2267 粒子/墨团/命中）+ `screenfx`（1068 受击方向/击杀闪）+ `fxHooks`（712）+ `swimWake`（107）+ `zoneMarks`（517，M3 顺延 zone/flood 呈现）→ bevy 实体/粒子；sim 事件流唯一驱动。
- **Acceptance Criteria Addressed**: AC-M4-4
- **Test Requirements**:
  - `rule` TR-51.1: 事件→fx 触发对账单测；粒子预算上限（性能门限）。
  - `rule` TR-51.2: 无头+wasm 冒烟不破帧；REVIEW_CHECKLIST 视觉登记。
- **Notes**: 体量大，执行期拆批次委派；先 hook 面后特效精度。

### Task 51 Completion Evidence（2026-11-06）
- **批次 A（fx 数据核）✅**：`fx/particles.rs`（新 1815 行，纯逻辑无 Bevy）≡ fx.js L726-1017 逐式——六池结构体（Drop/Sprite/Ring/Shell/Beam/Mote 字段≡JS 属性）+池容量/回收（`_killDrop` swap-with-last L760-771 语义）+`_updateDrops/_updateSprites/_updateRings/_updateShells/_updateBeams`（重力/阻力/浮力/drag=2.2/easeOut/水面命中 waterY+ripple 回调）+`_after` 延迟调度器（OP 枚举≡L1163-1188）+`_near` 相机剔除（cam_pos 入参化无头可测）+固定种子 PRNG（≡`audio::synth::Rng` 口径，D-T51-7④）+预算上限（quality×particles 系数≡L663-707）。
- **批次 B（spawn API 面）✅**：`fx/api.rs`（新 3715 行）——≈60 公开方法逐一 ≡ 平移（burst/explosion/splatted/muzzle kind 分支/hitSplash amount=30·killed/storm·slam·superJump 系/默认值/旗标组合/`_color` 归一/内部调用链逐值锚行号）；`onSpeck/onRipple` 接 `PaintGrid` 既有 API（sim 零改动）；固定输入单测每 API 断言池条目+数量区间+关键字段，复杂 API 种子固定逐值钉。
- **批次 C（Bevy 呈现接线）✅**：`fx/mod.rs`（84 行）+`fx/render.rs`（411 行）——`FxState` Resource 持数据核；instanced billboard **占位档**（每池实例缓冲 pos/size/color/alpha→近似材质；impostor 拉伸=view 对齐 quad scale 近似，GLSL 精度顺延=D-T51-7②）；`fx_update` 挂 world.rs Update 链非 headless 分支；`main.rs` `mod fx`；clear/setLighting 对应面。
- **TR-51.1 触发对账 ✅（批次 D）**：`fx/hooks.rs`（新 1990 行）——SimEvent/TickEvent 消费 ≡ fxHooks 订阅表 22 项逐项（Fire→muzzle/Impact→hitSplash·bounce/LowInk→dryFire/Kit·splat 系…）+**actor:\* polling 推断**（Rust sim 无 actor 总线≡JS fallback，D-T51-7③：per-actor 30 字段状态机——grounded 边沿→land、form 边沿→formPop、vel.y 阈值→jumpOff/dive/emerge、stepAcc 步频累计→footstep…）+`seen` 旗标保留+连续 update 数据面（_projectiles/_bombs/_clouds/_ambient）+触发计数 `count` 逐项单测（注入事件序列→count+池增删对账）；缺口如实登记（bomb:explode/weapon:impact/damage 无 Rust 事件源→REVIEW_CHECKLIST）。**粒子预算上限 ✅**：满池 cap 单测（drops/sprites/rings 达上限 swap 回收不超发）。
- **批次 E（zoneMarks+swimWake+screenfx 状态机）✅**：`fx/zone_marks.rs`（872 行）——flood 进度/contested/active/sweep/flash 数据层≡L417-456+简化边界带 mesh 占位档（**M3 顺延 flood 呈现层落地**）；`fx/swim_wake.rs`（548 行）——swimmer 筛选（≤4/本地优先/form swim|climb/相机距离）+轨迹 ring-buffer（BREAK/LIFE/SPACING≡L50-107），ink.wgsl 无 `uWake` 组 uniform→数据层落地+上传点顺延 Task 52（D-T51-7⑦）；`fx/screen_fx.rs`（2046 行）——12 事件→效果状态机（uSpeed/uBlast/uChroma/uKill/uFlash/uFlood 等 uniform 值计算≡L589-667/L780-868），**composite pass 不接=Task 55**（D-T51-7①）。
- **TR-51.2 ✅**：REVIEW_CHECKLIST「Task 51 fx」条目登记（粒子/impostor 观感/flood 顺延/zone 带绘/swimWake 上传/触发对账缺口/粒子预算/确定性 7 项）；headless 600 帧 turf+boss 均不破帧不 panic；wasm 构建+冒烟通过。
- **评审**：独立实证评审 **PASS（0 P0 / 7 P1 / 8 P2）**→全部当场修复复验：P1-A api.rs 7 处 glow 缺 `.fade_in(0.0)`（JS 默认立即淡入）补齐；P1-B hooks.rs dive 守卫误用 swim 后形态→改原始 `a.form==Form::Squid`；P1-C specialSparkle 高度同改；P1-D beam 剔除半径 from/to（60/40）归位；P1-E slosh 判别加 `&& p.below_falloff>0.0`（blade 液滴误判）；P1-F enemyInk 上升沿补 footstep+bump("enemyInk")（`prev_on_enemy` 边沿）；P1-G zone_marks rotation wipe 未取消在途 fill→`Active{moved:true}` 记录 wipe FloodRec{team:-1,dur:WIPE_T=0.7,reach+3.5,linear} 覆盖 fill。P2 处置：`reset()` 改 JS 式部分重置（mem::take 保留 blast_t/heart_ph/charge_pulse/was_jump/last_blast/full_charge）、swim_wake fwd 改 normalize_or_zero、4 处留档注释（screen_fx respawn/superjump live 门不可达、LensInk 饱和角例、swim_wake 驱逐 d2、头⑦投影偏差措辞）。新增钉行为测试：`rotation_wipe_cancels_inflight_fill`、`reset_is_partial_like_js_object_assign`；`slam_blasts_with_dedup` 修正（剔除 sim 种入 Phase 事件——旧测试曾意外钉住 reset bug）。
- **基线**：`inkwave` 180→**249**（+69，`fx::` 69 全绿）/workspace **809→878 passed**（单调递增）；`inkwave_sim` 601 不变、`cargo tree -p inkwave_sim` 依赖面不变；JS 树/`server/`/wire/fixture/Cargo.toml·lock 零改动；inkwave 无新增依赖。
- **九段门禁**：`rust/tools/verify/gate-task51.sh`——`rust/tools/verify/logs/task51-gates-20261106T071026Z.log` FMT/CLIPPY/TEST(878)/WASM/TREE/**DETERM(BYTE_IDENTICAL)**/XCHK/**TRUNK(rc=0)**/HEADLESS(turf+boss)/JS_TREE_CLEAN **全绿**（DETERM 存量产物 `source.commit` 戳系门禁重跑产物，随本任务提交，Task 48/49/50 先例）；clippy 修复 12 项（too_many_arguments allow×2、collapsible if/match、mem::take、iter().enumerate()、assert!）。

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
