# Task 25 — 端到端网络集成（Rust↔Rust 完整对局）实施计划

Status: Approved（用户已批准，按此执行）
Spec: `.trae/specs/rust-bevy-port-m2/tasks.md` Task 25（TR-25.1 … TR-25.4）

## 目标

用 test-only `net-harness` 组装完整 Rust 客户端栈（FakeRelay + N×NetSession +
N×Replication + N×Match），在共享虚拟时钟上跑完整对局，覆盖：

- TR-25.1：≥3 无头客户端（1 host + 2 guest，bot 补位至 8、缩短时长）打完一局；
  断言各端涂地覆盖率一致（两两差 <0.5pp）、结果与 host `res` 一致、无 panic。
- TR-25.2：`netlag=40 / netjitter=30 / netspike=0.01` 复跑 + kink / path-error
  p95 统计（对照 JS `tools/net-test.mjs` 口径；数值门限按 spec Open Question 3
  由独立审查 agent 定标）。
- TR-25.3：对局中 host 掉线迁移 + 玩家掉线转 bot 两场景打完出结算。
- TR-25.4：全门禁。

## 范围决策

1. **建局器下沉 sim**：app crate 是 bin-only，harness 无法依赖；把
   `build_net_match` / `local_roster_index` / `fnv1a` 逐字平移至
   `inkwave_sim::net::matchbuild`，app 与 harness 共用同一建局代码（一处真相）。
   sim 依赖面不变（仅 glam/serde/serde_json）。
2. **场景落 net-harness**：新模块 `src/match_e2e.rs`（驱动器 + 统计 + 报告）、
   新测试文件 `tests/e2e_match.rs`（tr25_* 场景矩阵）、CLI 增 `match` 模式。
3. **局制参数**：3 人类客户端（1 host + 2 guest）+ 5 bot 补位至 8；
   duration 缩短为 15s（≡ net-test.mjs L85 `duration = 600/40` 的本地覆盖：
   lobby 路径按 JS `Math.max(60,…)` 钳制（session.rs L1284-1290），harness
   在本地克隆的 `StartCfg` 上覆盖，与 JS harness 相同的接缝）。
4. **统计逐字移植**：kink（net-test.mjs L244-257，稳帧 6-45ms 过滤、y 权重
   0.5）、path error（L280-291，2s 窗、lag 40-400 步 5 拟合）、teleport
   （L275，>1.2m）、pct（L242，`sort[floor(p·n)]`）；输出 p50/p90/p95/p99/max；
   本任务不设数值门限（审查定标）。JS 记录渲染墙钟帧，harness 记录虚拟 60Hz
   固定子步（`frame×1000/60` ms），稳帧过滤在恒定步长下恒通过，公式语义保持。
5. **掉线用 `close()`**：≡ `net-test.mjs` `--leave` 的 `DROP=kill`（浏览器进程
   被杀、relay 观察 socket 断开，L118-125）；不做 freeze 变体。
6. **不移植**：boss 场景 / fps 统计 / 字节数统计；完整局（shim 端到端）留给
   Task 26。
7. **驱动范式**：沿用 `tr24_lobby_flow_two_clients` 的 lockstep 子步循环
   （drive_bots → step_masked(remote_mask) → rep.update → drain session →
   GameMessage → on_message）；事件分派镜像 app `net_tick` 契约（Host →
   `set_host` + host_changed 标记；Leave → `on_leave(flag)` + local_slot 重解析；
   GameMessage → `on_message`；MatchEnd → `end_match`）。

## 步骤

1. 建局器下沉 `inkwave_sim::src/net/matchbuild.rs` + app import 切换（回归 259 全绿）。
2. `match_e2e.rs`：HSess 工厂 / drain 虚拟时钟 / MatchOpts / ClientView /
   MatchReport（checks 内建）。
3. 统计模块：Frame 采样、pct/kink_seq/sample/fit_path_error/teleports 逐字移植
   （行号锚点注释）+ 公式单测。
4. `run_match`：lobby→start→build→go→lockstep（含 drop 注入）→settlement→
   end handshake→checks。
5. `tests/e2e_match.rs` 五场景 + CLI `match` 模式。
6. 全门禁 + 独立审查（含 TR-25.2 门限定标）+ 修复复审 + tasks.md + 提交。

## 风险

- R1：15s 局 × 多场景在 debug 下运行时长——必要时缩 duration 并如实记录。
- R2：host 迁移时序（Host/Leave 事件先后）与 app 契约不一致——以
  session.js L156-163 与 net_tick 镜像为准。
- R3：guest 本地 judge 覆盖 host res——follower 门控（match_.rs L430/L438）
  保证 guest 不本地判决。
- R4：统计公式在虚拟时钟下的语义漂移——恒定 16.67ms 步长使稳帧过滤恒通过；
  审查对照 JS 口径定标。
- R5：工具链间歇性写盘失败——落盘后用 shell 复核。

## 已记录偏差

- ① duration 覆盖走本地 cfg 接缝（≡ JS harness），lobby 钳制逻辑未动。
- ② result 一致性断言：wire 权威字段（coverage/winner/elapsed）精确相等；
  `points` 由整数化 turf 行重建（replication.rs L80-84 既有偏差），容差 <2 分（round_int 逐 actor ±0.5 × 每队 4 人，审查 A3 定标）。
- ③ JS `net-test.mjs` 的 `sj`（super jump）项在 Rust M1 无对应状态，恒为
  false，相应守卫结构性满足（`Frame` 文档注释）。
