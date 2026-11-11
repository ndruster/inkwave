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

### M3 基线核对（Task 40）

`tools/sync-upstream.sh check` 于 2026-11-03 复核：沙箱网络不可达（`git fetch upstream`
失败，脚本内置降级为本地 refs），本地 `upstream/main` = 基线
`3e9b5505ea900cb41851b4d75f16d488552b1794`，**基线之后本地已知零新提交**，无需逐行登记。
M3 移植范围为**内容与 Zone Control 专项**（非上游新提交驱动），见「M3 移植说明」节；网络可达环境建议
复跑 `bash rust/tools/sync-upstream.sh check` 以确认远端无新提交。

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

## M3 移植说明（Task 28–39，武器/副/特殊/Zone Control/新图/bot/loadout）

M3 将武器内容、Zone Control 模式、两张新图、bot 三档与大厅 loadout 移植到 Rust
（`server/` 与 JS 树全程零改动）。逐块映射（详细状态见 [PORT_MAP.md](./PORT_MAP.md) 的
M3 行；数字与测试名取自 `.trae/specs/rust-bevy-port-m3/tasks.md` 各 Completion Evidence）：

| JS 源 | Rust 目标 | 移植方式/证据 |
|---|---|---|
| `src/config.js` WEAPONS/SUBS/SPECIALS/ZONES/DIFFICULTY | `tools/extract/extract-tuning.mjs` → `assets/tuning.json`（schema v2）+ `inkwave_sim::tuning` | Task 28：12 主武器按 `WEAPON_ORDER` 投影（retired dualies/slosher/splatling 不入产物）、15 副、19 特殊、zones 17 字段、difficulty 三档、weaponRange 预计算表；双跑逐字节一致 + 锚定单测锁值（workspace 300 passed） |
| `src/game/weapons.js` + `kits/{brolly,blade,bow,mitts}.js` | `inkwave_sim::weapon`（+`weapon/{brolly,blade,bow,mitts}.rs`） | Tasks 29–33：kind 分发与弹丸泛化（29）；twins/blaster/brolly（30）；charger/spinner/bucket（31）；roller/brush/blade（32）；bow/mitts（33）——**12 主武器全部 wired**；net tick 槽 `ch`/`lock`/`ks` 与 `['k']` ghost 重放对齐 JS packer 语义（Task 30/33，workspace 389 passed） |
| `src/game/subs.js` + `kits/{shaker,waddle,torpedo,tracer,boomerang}.js` | `inkwave_sim::sub`（+`sub/*.rs` 五 kit sub） | Task 34：**15 副全部落地**（throw/plant 物理、fuse/触发、15 效果逐 key，58 固定种子测试）+ 设备网络语义 `'dh'` 直发/`['b']`/`['k','subs']` ghost（workspace 447 passed） |
| `src/game/specials.js` | `inkwave_sim::special`（+`special/*.rs`） | Task 35：**19 特殊全部 wired**（13 kind，barrage×5 共享主体）；充能 gauge 与消耗、`sp`/`spx`/`spst` tick 槽实值化、`['ev']` special:use/slam 事件重放、`special/world.rs` 世界侧交互（workspace 488 passed） |
| `src/game/zones.js` + `match.js` mode 分支 | `inkwave_sim::match::zones` + `match_` turf/zones 分支 | Task 36：Zone Control 规则全量平移（占领/争夺/penalty/轮换/终局中心锁定/加时四出口）+ `['z']` 8 事件 host 权威广播/guest 应用 + zone 形态结算 + session 放开 zones（boss 仍拒）；金标准 fixture `zone_seq` 257 条逐字节（workspace 509 passed） |
| `src/world/stages/{saltpan,crossmarket}/` + `variants.js`/`zones-data.js` | `assets/maps/{saltpan,crossmarket}.json` + `geometry::for_mode` | Task 37：提取器泛化为 stage CLI；三图 stable-json 双跑逐字节一致；onlyIn/notIn 旗标按模式过滤；harness 完整局 saltpan/crossmarket turf + crossmarket zones（workspace 525 passed） |
| `src/game/bots.js` | `inkwave_sim::bot`（+`bot::zones` ZonePlan） | Task 38：**normal/hard 档位接线**（`DifficultySet::get` ≡ L276 兜底）+ 12 kind 逐 kind fight/paint/refill/tail + sub 投掷/特殊驾驶 + zone tactic；ladder 单调性报告 `rust/tools/verify/logs/task38-ladder-1793672288.log`（workspace 546 passed） |
| `src/net/session.js` loadout + 大厅 UI | `inkwave::ui::online` + `crates/inkwave/src/nettest.rs` 测试缝 | Task 39：lobby click-cycle loadout（WEAPON 12/SUB 15+NONE/SPECIAL 19+NONE/MODE turf⇄zones host-only）+ roster `{weapon,sub,special,style}` 实值透传 + `matchbuild` `a.sub` 接线；进程内闭环 harness `tr39_loadout_end_to_end`；**跨端完整局（net-smoke scenario C/D）沙箱 DEGRADED**（环境性 SwiftShader 崩溃 + 内存压力，D-T39-1），首次真实执行列入用户机器复核 `.trae/specs/rust-bevy-port-m3/REVIEW_CHECKLIST.md`（workspace 553 passed） |

M3 **零新增依赖**：`cargo tree -p inkwave_sim` 仍仅 `glam`/`serde`/`serde_json`
（Task 27 门禁口径延续，`task39-gates-20261103T073400Z.log` 实核）。测试基础设施沿用
M2 脚手架扩展：`rust/tools/verify/net-smoke.mjs` 新增 scenario C/D（跨端 turf/zones 完整局），
`rust/tools/net-harness` 新增 zones/新图/loadout/ladder e2e scenario；`gen.mjs` fixture
捕获扩 `zone_seq`（Task 36）。未移植项（showcase 大厅、小地图、完整角色绑定、后处理链、
boss、props/murals 装饰、flood 呈现层等）顺延 M4，状态见 PORT_MAP。

### M4 基线核对（Task 57）

`tools/sync-upstream.sh check` 于 2026-11-11 复核：沙箱网络不可达（`git fetch upstream`
失败，脚本内置降级为本地 refs），本地 `upstream/main` 仍等于基线
`3e9b5505ea900cb41851b4d75f16d488552b1794`，**基线之后本地已知零新提交**。
M4 与 M2/M3 同属「对照上游**现存**源码的专项移植」，不是上游新提交驱动，因此上方
「逐提交移植记录」表保持 `_基线之后暂无新提交_`；本节按**模块**而非按 commit 登记。
网络可达环境建议复跑 `bash rust/tools/sync-upstream.sh check` 以确认远端无新提交。

## M4 移植说明（Task 41–57，Boss 战 / 玩法收尾 / 表现完整度）

M4 主线经用户确认为**玩法优先**：Phase A Boss 战（41–46，其中 46 为呈现占位档）→
Phase B 玩法收尾（47–48）→ Phase C 表现完整度（49–56）→ 收尾（57）。
`server/`、上游 JS 树、`net/wire.rs` PROTO v1、net-fixtures 全程零改动；所有 JS 数值经
`rust/tools/extract/` 切片 eval + `assertContains` 钉源生成 stable-json（连跑逐字节一致），
禁手抄；`inkwave_sim` 依赖面不变。逐任务证据/偏差见
`.trae/specs/rust-bevy-port-m4/{tasks.md,spec.md}`，模块级对照见 [PORT_MAP.md](./PORT_MAP.md)「M4 完成核对」。

| 阶段 | Task | 上游 JS 面 | Rust 落地 | 状态 |
|---|---|---|---|---|
| A Boss 数值/协议 | 41 | `config.js` BOSS/BOSS_MODE、`src/boss/*` 参数/时序/hitShapes | `assets/tuning.json` v3、`net/wire.rs` BossSnap 19 槽 + `B`/`['bm']`/`['bc']`/`'bhit'`/res 形态 fixture | done |
| A Boss sim | 42–43 | `boss.js`/`bossNav.js`/`bossBrain.js`/`bossHazards.js` | `inkwave_sim::boss::{Boss,BossNav,brain,hazards}`（六招、victim-side 伤害、host-only 墨、threat 查询） | done∗（D-T42/43） |
| A Boss 规则与联网 | 44–45 | `bossMode.js`/`match.js`/`bots.js` boss 段/`netmatch.js` | `boss/mode.rs`、`bot/boss.rs`、`match_.rs`、`net/replication.rs`（Hermite 播放、`'bhit'` 对账、host 迁移、scenario E） | done∗（D-T45-9） |
| A' Boss 呈现 | 46 | `bossModel.js`/`hud-boss.js`/`bossAudio.js` | `boss_render.rs`、`ui/boss_hud.rs`、结算 boss 行、roar 震屏 | placeholder（GPU 复核） |
| B 玩法收尾 | 47–48 | `bots.js` threat/盾/canopy、kits blade/mitts；`maps.js` 五图 + `config.js` MAPS | `bot/threat.rs`、`bot/blade_tactics.rs`、`sub.rs`；`assets/maps/*.json` 8 图、schema v7 | done∗（D-T47-11/11a 兼容超集；cargo onlineOnly/noBots/noBoss 旗标） |
| C 提取管线 | 48/50/52/53/54/55/56 | tuning/texlib/layout/props/character/environment/lobbyset 各 JS 字面块 | `assets/{tuning,texlib,maps,props,char,environment,lobbyset}.*`（schema 逐版升级 v3→v7）+ 各 crosscheck 独立正则重解析 | done（DETERM 连跑逐字节一致） |
| C 小地图/音频 | 49–50 | `minimap.js`、`audio.js`/`music.js`/`bossAudio.js` | `ui/minimap*.rs`（三档栅格）；`audio/*`（148 SFX、9 曲、过程化 PCM 烘焙） | done∗（DOM 点标效应/混响总线/42 名 kit SFX 顺延） |
| C fx/材质 | 51–52 | `fx.js`/`screenfx.js`/`fxHooks`/`swimWake`/`zoneMarks`；`inkShading.js`/`levelMaterial`/`texlib.js` | `fx/*` 六池 + hooks；`texlib/*` 46 层 CPU 烘焙 + `level.wgsl` + `ink_ripple.rs`/`quality.rs` | done∗ + placeholder（composite pass 未接；CPU≠GPU 逐像素、wasm32 烘焙 inert） |
| C 装饰/角色 | 53–54 | `props.js`/murals/dressing；`character.js` 4062 行弹簧姿态 | `props/*` IKP1 8 图 2069 placements；`char/*` IKC1 骨树 + 5640 帧 × 87 骨 pinned replay | done∗（atlas/ShaderMaterial、bow/brolly/bowl 几何顺延） |
| C 环境/大厅 | 55–56 | `environment.js`/`renderer.js`/`config.js` QUALITY；`showcase.js`/`lobbySet*.js` | `environment.rs` + 四档三主题接线；`ui/showcase_state.rs`/`showcase_scene.rs` + `assets/lobbyset.json` | done∗ + placeholder（D-T55-⑦/D-T56-⑦；GTAO/grade/SMAA/PMREM/3D 名牌与角色 preview 属 GPU 或 M5） |

**状态口径**：`done∗` = 含声明偏差（逐条见 `spec.md` D-T 表与 `REVIEW_CHECKLIST.md`）；
`placeholder` = 行为/数据面已验、视觉面按 NFR-M4-5 登记用户 GPU 机器复核。
`M4 wontport`：locker 更衣室、高保真 Boss 模型、Rust 设置屏（M5）、DOM 名牌叠层。

**门禁与基线**：九段门禁（FMT/CLIPPY/TEST/WASM/TREE/DETERM/XCHK/TRUNK/HEADLESS +
`JS_TREE_CLEAN`/`SIM_TREE_CLEAN`）逐任务落盘 `rust/tools/verify/logs/task*-gates-*.log`；
Task 54 起脚本自动判定 `GATE_VERDICT`，41–53 用逐段 `*_EXIT=0` 口径。
TRUNK 段在本沙箱因 wasm-bindgen OOM `signal: 9` 判 DEGRADED（D-T40-1 环境性、非阻塞）。
workspace 测试基线逐任务单调递增（554 → 1017+；精确数字见各任务 Completion Evidence）。
