# Task 27 执行计划 — M2 收尾：门禁、台账、文档与总评审

## Context

M2 Task 17–26 全部 completed（HEAD `c8033a9`，工作树干净，测试基线 273）。Task 27（tasks.md L320-331）对应 AC-M2-7，三项验收：

- **TR-27.1** 全量门禁矩阵（同 M1 Task 16 口径）：测试总数 > 147、`cargo tree -p inkwave_sim` 仍仅 glam/serde/serde_json、trunk release 产物、原生 headless 冒烟、JS 树/`server/` 零改动；证据为日志。
- **TR-27.2** 文档逐条命令实际执行验证（README 联网章节）；证据为输出。
- **TR-27.3** 独立审查 agent 总评审 M2 全部产出并闭环（无未闭环 Blocker/Major）。

Notes：完成后提交 M2 里程碑并 push origin/main（沿用 M1 流程与 ndruster 身份）。

现状核对（已实核）：
- `rust/SYNC_LEDGER.md`：只有「M1 基线核对」，无 M2 段落、无新依赖登记。
- `rust/PORT_MAP.md`：net 三行仍是 `M1（骨架）`/`M2 Task 18`/`M2+`（L84-86），`server/` 行「M2 对接」，无 relay-shim/net-fixtures/net-harness 条目。
- `rust/README.md`：无「联网运行」章节（仅 M1 四节 + 门禁速查）。
- `.trae/specs/rust-bevy-port-m2/REVIEW_CHECKLIST.md`：已有 5 条（Task 24/26 降级项）；Task 27 需追加「远端角色平滑、跨端握手」GPU 复核条目（§2-4 已覆盖握手复跑，需补平滑视觉专条）。
- 新依赖事实（登记用）：`tungstenite 0.26 (rustls-tls-webpki-roots)` 仅 inkwave native；`relay-shim` 仅 native dev-dependency；`web-sys` wasm features（Window/Location/UrlSearchParams/WebSocket/console）；`inkwave_sim` 保持零引擎依赖；`tools/relay-shim` 自身依赖 tungstenite（handshake 默认特性、无 TLS）。

## Step 1 — 全量门禁矩阵（TR-27.1）

cwd `rust/`，命令与 M1 Task 16 / Task 26 口径一致，输出留档：

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`（总数 > 147，基线 273 只增）
4. `cargo check --target wasm32-unknown-unknown -p inkwave_sim -p inkwave`
5. `cargo tree -p inkwave_sim`（仅 glam/serde/serde_json）
6. `trunk build --release`（产物落 rust/dist/）
7. 原生 headless 冒烟：`cargo run --release -p inkwave -- --headless --frames 120`
8. `git status --porcelain -- src tools server docs` 为空（JS 树 + server/ 零改动）

## Step 2 — PORT_MAP.md 更新

- net 三行状态落地：`src/net/netmatch.js` → `M2 done`（wire/session/replication/playback，锚 Task 17/20-25）；`src/net/transport.js` → `M2 done`（native tungstenite 线程 + wasm web-sys，锚 Task 18/26）；`src/net/session.js`/`mock.js` → `M2 done`（NetSession + 极简大厅 UI，锚 Task 20/24）。
- 「仓库其他部分」：`server/` 行改「M2 实核：零改动，协议兼容经 fixture/shim/冒烟验证」；`tools/net-test.mjs` 行补 M2 状态（JS 基线工具未动，Rust 侧等价物 net-smoke.mjs）。
- 新增 Rust 侧测试资产条目：`rust/tools/net-fixtures/`（金标准 fixture）、`rust/tools/relay-shim/`（仅测试 shim）、`rust/tools/net-harness/`（集成 harness + match_e2e）。
- 加「M2 完成核对（Task 27）」小节（仿 M1 完成核对格式），注明 M2 未做项（zones/sub/special、showcase 大厅、真实 relay 部署验证等）仍标 M3+。

## Step 3 — SYNC_LEDGER.md 更新

- 加「M2 移植说明」：net 模块 JS→Rust 逐块映射（netmatch packer→wire、transport.js→双端 transport、session.js→NetSession、replication/playback 对应 JS 侧归属，事件重放在 replication::play_events），fixture/harness/shim 条目与「shim 非部署替代」纪律。
- 新依赖登记表：native-only tungstenite/rustls（webpki-roots）、dev-only relay-shim、wasm web-sys features；`inkwave_sim` 零依赖不破（TR-27.1 门禁 5 佐证）。
- 基线复核：跑 `bash tools/sync-upstream.sh check`；沙箱 github 不可达则如实记录（沿用 M1「基线之后零新提交」口径，以本地 `upstream/main` refs 为据）。

## Step 4 — README「联网运行」章节 + 逐条执行验证（TR-27.2）

新章节内容（置于「双端运行」之后）：
1. 本地 relay：`cd rust && cargo run -p relay-shim`（ws://127.0.0.1:8787；注明仅测试、不部署、真实 relay 为 `server/` Cloudflare Worker，wrangler dev 可达性见 REVIEW_CHECKLIST）。
2. 原生双端连房：两终端 `cargo run -p inkwave -- --relay=ws://127.0.0.1:8787`（Create→码→Join）。
3. wasm 端：`trunk serve` 后浏览器开 `?relay=ws://127.0.0.1:8787&net=create…` / `&netjoin=<CODE>…`（test-only query 接缝表）。
4. 冒烟：`node rust/tools/verify/net-smoke.mjs [--scenario A|B]`（沙箱 DEGRADED 语义 + 真 GPU 期望 PASS）。
5. 跨端（JS 客户端 ↔ Rust 客户端）同房说明（JS 树只读）。

TR-27.2 执行验证：沙箱可跑的命令逐条实跑（shim 启动就绪探测、`--relay=` 参数被 native 客户端接受至窗口创建前或 headless 不适用如实标注、trunk build 产物静态服务、net-smoke.mjs 运行 exit 0）。无 DISPLAY 的窗口命令以「进程可启动 + 参数被正确解析」为限，视觉复核引 REVIEW_CHECKLIST §1。

## Step 5 — GPU 复核清单追加 M2 条目

REVIEW_CHECKLIST.md 新增：
- 「M2 远端角色播放平滑（TR-22 Hermite/correction）视觉复核」：真 GPU 双窗口同房，观察远端角色移动平滑、无 teleport 抖动、喷涂重放一致；命令 + 断言点。
- 「M2 跨端握手总览」：指向 §2-4 汇总（wasm↔wasm、JS↔Rust、native↔wasm），标注沙箱已证部分（join 落地实据）与待复核部分（welcome/tick 段）。

## Step 6 — 独立审查 agent 总评审 M2（TR-27.3）

审查范围：Task 17–26 全部提交 + 本任务文档改动。重点：
- JS 逐字对齐锚点抽核（抽查各任务关键行号）；
- 门禁与测试证据真实性（落盘日志 vs 声明）；
- 台账/README/PORT_MAP 与代码现状一致性；
- 偏差声明完整性（对照各 tasks.md 段）；
- 遗留 REVIEW_CHECKLIST 是否有不实断言。
Blocker/Major 当场修复并复审；minor/nit 逐项处理或说明不修。评审记录入 tasks.md Review 附录。

## Step 7 — 收尾（固定流程）

1. tasks.md Task 27 段：completed + Completion Evidence（TR-27.1/2/3）+ Review 附录。
2. 提交（`git -c user.name=ndruster -c user.email=ndruster@users.noreply.github.com commit`，标题 "M2 Task 27: …"）→ `git show` 复核 + `git status --porcelain` 干净。
3. push origin/main：先 `git remote -v` 核实 origin；沙箱网络不可达则如实记录，push 留待用户机器执行（REVIEW_CHECKLIST 追加一条）。
4. memory 写入（topics.md M2 完成记录）。

## Verification / 验收映射

- TR-27.1 ← Step 1 八条命令输出留档（测试数、tree、dist 产物、headless、JS 树空）。
- TR-27.2 ← Step 4 每条 README 命令的实跑记录（含不可跑项的如实标注）。
- TR-27.3 ← Step 6 评审记录 + 修复闭环 + 复审结论。
- 门禁在 Step 6 修复后重跑一遍（若修复触及代码/文档）。

## 纪律与已知风险

- Edit/Write 假成功问题：每个文档修改后 shell grep 磁盘复核。
- 端口冲突：冒烟用独立端口（--relay-port/--dist-port/--js-port/--cdp-port），绝不 kill 外部进程。
- 测试基线 273 只增不减；JS 树/server/ 零改动。
- push 可能受沙箱网络限制（M1 记录 github HTTPS 超时）——如实降级记录，不谎报。
