# Task 26 执行计划（已获设计批准，本文件为落地步骤）

## Context

M2 Task 26「双端联网冒烟与跨端握手」（tasks.md L292-299，TR-26.1/2/3）。设计文档 `.trae/documents/task-26-browser-native-smoke.md` 已获用户批准。沙箱无 DISPLAY → 原生窗口端实局降级入清单；沙箱内主证据 = wasm↔wasm 完整一局（场景 A）+ JS↔Rust 跨端握手（场景 B，JS 树只读）。Rust wasm 端缺测试接缝（Create/Start 仅鼠标按钮），需新增 query 门控的 test-only 接缝（JS 侧有 `__G.net`/`?skipTitle&autopilot` 同类先例）。

## Step 1 — Cargo.toml：web-sys console feature

`rust/crates/inkwave/Cargo.toml` wasm32 依赖的 web-sys features 增加 `"console"`（现有 Window/Location/UrlSearchParams/WebSocket 保留）。

## Step 2 — 新模块 rust/crates/inkwave/src/nettest.rs

- `NetTestCfg { create: bool, join: Option<String>, ready: bool, start: bool, dur: Option<f64>, autopilot: bool, probe: bool }`。
- 纯函数 `parse_query(search: &str) -> Option<NetTestCfg>`：解析 `net=create`、`netjoin=CODE`（长度过滤，合法性交给 Join 路径）、`netready`、`netstart`、`netdur=<N>`、`autopilot`、`netprobe`；无任何 nettest 参数 → None。带单元测试（≥4 个：空/全参/坏码/部分参）。
- `#[derive(Resource)] NetTest { cfg: Option<NetTestCfg>, code_logged: bool, joined: bool, readied: bool, dur_sent: bool, started: bool, probe_acc: f32 }`。
- `pub fn console_log(s: &str)`：cfg wasm32 → `web_sys::console::log_1`；native → println!。
- 系统 `nettest_update`，排在 `online_ui_update` 之后（world.rs L557 后），每帧状态机（cfg None → 立即 return；native 恒 None）：
  1. create：Menu 时 EnterHub 等价（net_mode=true + screen=OnlineHub，≡ ui/mod.rs L1339-1341）+ run_cmd(Create)（≡ net-test L73）。
  2. code 落定 → console_log("[nettest] code <CODE>")（≡ net-test L73 返回值）。
  3. join：Menu 时 EnterHub + 写 os.flags.code_draft + run_cmd(Join)（≡ net-test L77）。
  4. ready：首次 state==Lobby 且非 host 时 ToggleReady（≡ net-test L80）。
  5. dur：host 进 Lobby 后 run_cmd(SetDuration(N)) 一次（走既有钳制路径 ≥60，修订设计文档：不直写绕过钳制——冒烟用 netdur=60；差异如实写入偏差声明）。
  6. start：host 且 can_start() 时 run_cmd(Start) 一次（≡ net-test L81-87）。
  7. probe：screen==Hud 时每 2s console_log("[nettest] probe t=.. cov=.. rem=<nid:x,y,z;..>")——远端 actor 采样表达式对齐 match_e2e.rs，coverage 取 paint.coverage()（match_e2e L143 同款）。

## Step 3 — autopilot 接线（match_link.rs）

- start_net_match（L85）：local_roster_index 解析后、demo.m = built 之前，若 autopilot → rep.attach_bot(nms.local_idx, &built.actors[nms.local_idx], difficulty.easy, seed)（≡ match.js L112 mine && o.autopilot；L113-116 已有同款调用）。
- net_tick（L199）：本地输入覆写块（L338-348）加 !autopilot 守卫（≡ match.js L118 autopilot 不建 PlayerController，drive_bots 输出保留）。
- 经 Res<NetTest> 读标志。

## Step 4 — 接线（main.rs / world.rs）

- main.rs：mod nettest; 非 headless 分支显式 insert NetTest{ cfg: parse_query(search) }——cfg wasm32 用 web_sys window().location().search()（复用 wasm_transport.rs L36-40 模式），native 恒 None（无 CLI 接缝）。
- world.rs Update 链（L541-563）：online_ui_update 之后插入 nettest_update。
- headless 分支不建 OnlineSession（main.rs L75-81）→ 接缝在 headless 下不运行（冒烟走真窗口 wasm）。

## Step 5 — 冒烟脚本 rust/tools/verify/net-smoke.mjs

raw CDP（Node≥22 全局 WebSocket，零 npm 依赖，复用 wasm-smoke.mjs 探针模式：fresh profile、about:blank 附着、RENDER_ERR_RE、console.error 收集、非 0 退出）：
- 子进程：cargo run -p relay-shim（cwd rust/，就绪探测 8787）、python3 -m http.server <P1> --directory dist（cwd rust/）；场景 B 另起 python3 tools/serve.py 8490（cwd 仓库根，只读）。
- 场景 A（wasm↔wasm）：tab1 ?relay=ws://127.0.0.1:8787&net=create&netdur=60&netstart&autopilot&netprobe → console 抓 [nettest] code；tab2 &netjoin=CODE&netready&autopilot&netprobe。waitFor：双方出现 [nettest] probe（wasm println 不进 console，probe 首行 = 进入比赛证据）。断言：两侧 probe 远端 actor 位移 >0.5m、coverage>0、error_count==0、无 panic/render-error。
- 场景 B（JS↔Rust 握手）：JS tab ?skipTitle&autopilot → evaluate __G.net.create('Host') 取 code；Rust tab join；JS until(canStart) → __G.net.start()。断言：JS lobby.players 含 Rust 玩家、Rust probe 含 JS 远端 actor 移动、JS __inkwave.match.state==='playing'、双方无 error。握手日志落盘。
- 输出：JSON 摘要 + rust/tools/verify/logs/net-smoke-<date>.json（TR-26.1/26.2 证据）。
- 失败降级：WS/relay 不可达 → 场景记 SKIPPED(原因)，条目入 REVIEW_CHECKLIST，脚本保留可运行。

## Step 6 — REVIEW_CHECKLIST.md

新建 .trae/specs/rust-bevy-port-m2/REVIEW_CHECKLIST.md：
1. Task 24 TR-24.2 双窗口人工试玩 6 步 + 命令（cargo run -p relay-shim；trunk serve；cargo run -p inkwave -- --relay=ws://127.0.0.1:8787）。
2. Task 26 原生窗口端 ↔ wasm 同房一局（沙箱无 DISPLAY 降级条目，同款命令 + 断言点）。
3. tools/net-test.mjs Linux 需改 executablePath（L48 macOS 硬编码）——只读提示，不改 JS 树。
4. 冒烟脚本在用户机器的复跑命令。

## Step 7 — 门禁与收尾（固定流程）

fmt --check / clippy --workspace --all-targets -D warnings / cargo test --workspace（基线 269 只增）/ wasm32 check（-p inkwave -p inkwave_sim）/ trunk build --release（cwd rust/）/ headless 120 帧 / cargo tree -p inkwave_sim / JS 树 git status 零改动 → 独立审查 agent → 修复复审 → tasks.md Task 26 段（completed + evidence + 偏差声明：test-only 接缝、netdur 走钳制路径修订、probe 证据语义）→ 提交（ndruster 身份）→ git show 复核 → memory 写入。

## Verification

- cargo test -p inkwave nettest 单测绿。
- node rust/tools/verify/net-smoke.mjs --scenario A|B|all 实跑，JSON exit 0。
- 全门禁清单命令输出留档（TR-26.3）。
