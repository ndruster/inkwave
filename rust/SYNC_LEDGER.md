# 上游同步台账（SYNC LEDGER）

Rust 代码无法用 `git merge` 合并上游 JS 改动。工作流：

```bash
rust/tools/sync-upstream.sh check              # 列出基线之后的上游新提交
# 对照 rust/PORT_MAP.md 逐项移植到 Rust，并在下表登记
rust/tools/sync-upstream.sh update-baseline    # 移植完成后推进基线
```

- **上游仓库**：https://github.com/jaydendavisnc/inkwave（remote 名 `upstream`，只读 HTTPS）
- **当前基线**：`3e9b5505ea900cb41851b4d75f16d488552b1794`（2026-09-29，上游 main，等同 M1 开工时 fork HEAD；存储于 `.upstream-baseline`）
- **状态图例**：`pending`（未移植）· `porting`（移植中）· `done`（已移植，含证据）· `wontport`（Rust 侧不适用，注明原因）

## 逐提交移植记录

| Upstream commit | 日期 | 摘要 | 受影响 JS 文件 | Rust 对应 | 状态 | 证据/备注 |
|---|---|---|---|---|---|---|
| _基线之后暂无新提交_ | | | | | | |

## M1 初始移植说明

M1 为**基线快照重写**（Tidewater + Spritzer + 涂地 + easy bot 的垂直切片），不是逐提交移植；
基线之前的所有 JS 内容整体视为“待移植参考”，其模块映射见 [PORT_MAP.md](./PORT_MAP.md)。
上游此后的每个提交才在上表逐行登记。

### M1 基线核对（Task 16 收尾）

`tools/sync-upstream.sh check` 于 2026-10-05 复核：本地 `upstream/main` = 基线
`3e9b5505ea900cb41851b4d75f16d488552b1794`，**基线之后零新提交**，无需逐行登记。
M1 已移植范围见 PORT_MAP 的 `M1` 行；未移植项全部标 `M2+`/`skip`。

### M2 基线核对（Task 27）

`tools/sync-upstream.sh check` 于 2026-10-16 复核：沙箱网络不可达（`git fetch upstream`
失败，脚本内置降级为本地 refs），本地 `upstream/main` = 基线
`3e9b5505ea900cb41851b4d75f16d488552b1794`，**基线之后本地已知零新提交**，无需逐行登记。
M2 移植范围为**联网栈专项**（非上游新提交驱动），见下节；网络可达环境建议复跑
`bash rust/tools/sync-upstream.sh check` 以确认远端无新提交。

## M2 移植说明（Task 17–26，联网栈）

M2 将 `src/net/` 全栈移植到 Rust（协议 PROTO v1 保持兼容，`server/` 与 JS 树零改动）。
逐块映射（详细状态见 [PORT_MAP.md](./PORT_MAP.md) 的 `M2 done` 行）：

| JS 源 | Rust 目标 | 移植方式/证据 |
|---|---|---|
| `netmatch.js` packer（packActor/packTrig/packEvent 等） | `inkwave_sim::net::wire` | 金标准逐字节：`rust/tools/net-fixtures/gen.mjs`（node 侧 import 真实 JS packer）→ `rust/assets/net-fixtures.json`，Rust codec 输出与之 byte-equal（Task 17） |
| `transport.js`（WS 客户端 + 8 s 握手定时器 L48） | `inkwave_sim::net::transport`（trait + FakeTransport）；`inkwave::net::{native_transport,wasm_transport}` | native = tungstenite 独立线程 + mpsc（PORT_MAP OQ-2 选型）；wasm = web-sys WebSocket；合规 Origin 头过 relay ORIGIN_OK（Task 18/26） |
| `session.js`（大厅/对局状态机） | `inkwave_sim::net::session::NetSession` | 逐字平移（turf-only；TR-20.x 状态机测试） |
| host 权威切分 / bot 托管 / 时钟 / 结算 | `inkwave_sim::net::replication` | 逐字平移 + lockstep 集成测试（Task 21/25） |
| 播放层（per-sender 时钟、Hermite、correction） | `inkwave_sim::net::playback` | 逐字平移 + 路径/teleport 统计对齐 net-test.mjs（Task 22/25） |
| 事件时间线重放（splat/ghost/trigger） | `inkwave_sim::net::replication`（`play_events` 时间线） | 逐字平移 + 重放守卫（Task 23） |
| 大厅 UI（hub/lobby/ready/start） | `inkwave::ui::online` + `inkwave::net::match_link` | 极简版（Task 24）；showcase 3D 大厅不做（M2+） |

测试基础设施（仅测试用，不进产物）：
- `rust/tools/relay-shim/`：原生 relay 语义复刻（welcome/join/leave/选举/`b|`/`s|to|`/lock/ping/sweep 参数化）。**不是 `server/` 的替代、不部署**。与真实 relay 的交叉验证：Task 19 对 `wrangler dev`（本地真实 Worker）复跑 TR-19.1 场景 8/8 全绿；Task 27 实证 Rust 形态 Origin + PROTO v1 握手被真实 Worker 放行（`rust/tools/verify/logs/task27-wrangler-probe.txt`）；Rust 客户端完整对局直连真实 Worker 与部署端复核登记于 M2 REVIEW_CHECKLIST §7。
- `rust/tools/net-harness/`：FakeRelay + 虚拟时钟的集成测试 harness（`tests/acceptance.rs`、`tests/e2e_match.rs` 完整局 CONSISTENT + netlag 统计）。
- `rust/tools/verify/net-smoke.mjs`：浏览器联网冒烟（CDP raw，场景 A wasm↔wasm / B JS↔Rust 跨端握手；`[nettest]` console 接缝证据行落 `rust/tools/verify/logs/`）。

### M2 新增依赖登记

| 依赖 | 位置 | 范围 | 用途 |
|---|---|---|---|
| `tungstenite 0.26`（features: `rustls-tls-webpki-roots`） | `crates/inkwave` | 仅 native（`cfg(not(wasm32))`） | 原生 WebSocket 客户端；TLS 纯 Rust、离线根证书 |
| `relay-shim`（path） | `crates/inkwave` | 仅 native **dev-dependency** | TR-19.2 集成测试驱动真实 NativeTransport |
| `web-sys` features：`Window`/`Location`/`UrlSearchParams`/`Performance`/`WebSocket`/`MessageEvent`/`CloseEvent`/`console` | `crates/inkwave` | 仅 wasm32 | wasm WebSocket transport + 页面参数 + 单调时钟 + `[nettest]` 证据行 |
| `tungstenite 0.26`（默认 handshake、无 TLS） | `tools/relay-shim` | 测试工具二进制 | 本地 `ws://` 服务端 |

`inkwave_sim` 保持零引擎依赖（仅 `glam`/`serde`/`serde_json`，Task 27 门禁 `cargo tree` 实核）。

### 同步分类原则（移植 triage）

| 上游改动类型 | 移植方式 |
|---|---|
| 调参/常量（`src/config.js`）、关卡布局数据（`stages/*/layout.js`） | 优先重跑 `rust/tools/extract/` 提取管线，生成物更新后跑测试；一般零手工代码 |
| 玩法逻辑（actor/physics/weapons/match/bots） | 对照 PORT_MAP 手工移植到 `inkwave_sim`，同步补确定性测试 |
| 渲染/着色器/角色表现 | 手工移植到 Bevy 侧；M1 简化项可标记 `wontport` 并注明 M2+ 再做 |
| `server/` relay | 协议保持兼容；除非协议变更否则不动 Rust，协议变更需同步更新 `inkwave_sim::net` 文档与类型 |
| `tools/`、`electron/`、构建脚本 | 一般 `wontport`（Rust 有等价工具链：`--headless`、trunk、cargo） |
