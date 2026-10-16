# Task 26 — 双端联网冒烟与跨端握手（计划）

Spec: `.trae/specs/rust-bevy-port-m2/tasks.md` L292-299（TR-26.1/2/3）。依赖 Task 24（Online UI/会话命令）、Task 25（全栈 e2e 范式）。

## 1. 调研结论（事实基线）

- 沙箱环境：`/usr/bin/chromium-browser`（Chromium 154，headless CDP 可用，M1 `rust/tools/verify/wasm-smoke.mjs` 已验证同方案）；node v24.21（全局 WebSocket，raw CDP 脚本零 npm 依赖）；无 wrangler；**无 DISPLAY/Xvfb** → 原生窗口客户端在沙箱内无法运行。
- `rust/dist/` 已有 trunk 产物；relay-shim 默认 127.0.0.1:8787（`rust/tools/relay-shim/src/main.rs` L14-33）。
- wasm relay 接缝：`?relay=` 覆盖（`wasm_transport.rs` L33-36）；native：`--relay=`（`main.rs` L59-61）。
- **JS 端驱动（零改动）**：JS app 暴露 `window.__G.net` API（`create/join/setMe/start/canStart/lobby/state`）+ `?skipTitle&autopilot` 测试接缝；`tools/net-test.mjs` L73-87 即经此驱动（本任务只读复用该思路，不改 JS 树）。
- **Rust wasm 端无等价接缝**：Online UI 键盘路径不完整——EnterHub/Create/Start 仅鼠标按钮（`ui/mod.rs` L408/752/769、L1133-1137）；`auto_join` 只是房码输满的 0.32s 便利（L1167）。命令原语已齐：`SessionCmd::Create/Join/ToggleReady/Start`（`online.rs` L226-246、L344-376、L460）、`sess.code()`（`session.rs` L856）、`sess.can_start()`（L1323）、`rep.attach_bot`（`match_link.rs` L113-116）。
- wasm 上 `println!` 不进 console（wasm32-unknown-unknown 无 stdout）→ 接缝日志必须走 `web_sys::console::log_1`。

## 2. 范围决策

1. **沙箱内主证据**：
   - 场景 A（wasm↔wasm 完整一局）：两个 chromium 标签页（host+guest）经 relay-shim 真 WebSocket 建房开局，断言互见移动/喷涂、console 无 error → TR-26.1。
   - 场景 B（JS↔Rust 跨端握手，尽力项）：JS 游戏客户端（:8490 serve，只读）当 host、Rust wasm 客户端 join，验证 welcome/join/tick 互收不报 err → TR-26.2 握手日志。
2. **降级项（如实记录，入 REVIEW_CHECKLIST）**：原生窗口客户端 ↔ wasm 同房（沙箱无 DISPLAY，spec 降级条款适用）；Task 24 TR-24.2 双窗口人工试玩（既定并入本任务建立的清单）。
3. **REVIEW_CHECKLIST 建立**：`.trae/specs/rust-bevy-port-m2/REVIEW_CHECKLIST.md`（task-24 文档 L12 既定由 Task 26 建立），含上述待复核条目 + 每条的用户机器可执行命令。

## 3. 设计：test-only 查询接缝（app crate，wasm-only）

声明偏差：JS 侧测试基建本有同类接缝（`?skipTitle&autopilot`、net-test.mjs 直写 `lobby.duration` L85、直调 `__G.net.*`），Rust 移植缺失；本接缝为等价测试基建，全部 **query 参数门控、参数缺省零行为**，不触碰 JS 逐字逻辑。

参数（仅 wasm 生效，`online_ui_update`/`net_tick` 消费一次性标志）：
- `net=create`：boot 后进 hub + `SessionCmd::Create`；`sess.code()` 落定后 `console.log("[nettest] code <CODE>")`。
- `netjoin=<CODE>`：进 hub、写入 code_draft、`SessionCmd::Join`。
- `netready`：首次进 Lobby 时 `ToggleReady`（≡ net-test L80 guest ready）。
- `netstart`：host 侧 `can_start()` 为真时 `SessionCmd::Start`（≡ net-test L81-87）。
- `netdur=<N>`：host 直写 lobby duration（≡ net-test L85 直写绕过 ≥60 钳制；harness 同款先例 Task 25）。
- `autopilot`：本地人类 actor 挂 BotBrain（≡ `match.js` L112 `if (mine && (r.bot || o.autopilot))`），使无人工输入也产生移动/喷涂。
- `netprobe`：比赛中每 ~2s `console.log("[nettest] probe …")`：远端 actor 位置样本 + 本地 coverage，供脚本断言互见。

纯函数接缝解析器（`nettest.rs`，query string → `NetTestCfg`）带单元测试，测试基线 269 只增不减。

## 4. 冒烟脚本 `rust/tools/verify/net-smoke.mjs`

raw CDP（复用 wasm-smoke.mjs 探针模式：fresh profile、about:blank 附着、`RENDER_ERR_RE` 渲染错误检测、console.error 收集、非 0 退出）：
1. 拉起 relay-shim（`cargo run -p relay-shim`，cwd rust/）+ 静态服务 `rust/dist`（python3 http.server）+（场景 B）JS 树 `python3 tools/serve.py 8490`；就绪探测后继续。
2. 场景 A：tab1 `?relay=ws://127.0.0.1:<shim>&net=create&netdur=20&netstart&autopilot&netprobe` → 从 console 抓 `[nettest] code`；tab2 同参 `netjoin=<CODE>&netready`。等待双方 `ui Hud`；采样 probe：远端 actor 位移 > 阈值、coverage 增长；console error_count==0 且无 render-error/panic。
3. 场景 B：JS tab `?skipTitle&autopilot` → evaluate `__G.net.create('Host')` 取 code；Rust tab join；断言 JS `lobby.players` 含 Rust 玩家、Rust console 出现 lobby→match 状态、probe 见 JS actor 移动、双方无 err。
4. 输出 JSON 摘要（沿用 wasm-smoke 字段 + 场景断言表），落盘日志文件作为 TR-26.1/26.2 证据。
5. 若 chromium WS 对局在沙箱不可达（连接被拒/relay 不通）：按 spec 降级——保留接缝+脚本代码评审记录，场景输出登记"待用户机器复核"，如实写入 checklist。

## 5. 验收映射

- TR-26.1：net-smoke.mjs 场景 A 输出（含渲染错误检测）或降级记录 → 证据文件 `rust/tools/verify/logs/`（或如实说明落点）。
- TR-26.2：场景 B 握手日志，或"待复核"条目并入新建 REVIEW_CHECKLIST.md。
- TR-26.3：全门禁（fmt/clippy/test/wasm32/trunk build/headless 120 帧/cargo tree/JS 树零改动）+ 测试基线只增。

## 6. 实施步骤

1. `nettest.rs` 接缝解析器 + 单测；app 侧接线（wasm-only：boot 读 query、标志驱动 run_cmd、probe 日志、autopilot 挂脑）。
2. `trunk build --release`（cwd rust/）刷新 dist。
3. `net-smoke.mjs` 编写 + 场景 A/B 实跑，证据落盘。
4. REVIEW_CHECKLIST.md 建立（Task 24 条目 + 原生↔wasm 条目 + net-test.mjs macOS 路径适配提示）。
5. 全门禁 → 独立审查 → 修复复审 → tasks.md → 提交（ndruster）→ git show 复核 → memory。

## 7. 风险

- chromium headless 真 WS 时延/性能：44MB wasm ×2 标签软渲染，开局可能慢——脚本用 waitFor 语义（probe 出现）而非固定 sleep。
- 接缝触碰 app crate：严格 query 门控 + 默认零行为 + 审查重点核对。
- JS 树 serve 依赖（场景 B）：`tools/serve.py` 只读运行，不修改。
