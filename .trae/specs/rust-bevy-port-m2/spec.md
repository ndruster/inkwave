# INKWAVE Rust/Bevy 移植 — M2 线上对战（PROTO v1）产品需求文档（PRD）

> 前置：M1（`.trae/specs/rust-bevy-port/`，Task 1–16）已全部完成。本文档只描述 M2 增量；M1 的总体架构、同步机制、门禁基线继续有效。

## Overview
- **Summary**: 让 Rust/Bevy 版 INKWAVE 接入现有 Cloudflare relay（`server/` 零改动），实现 **Rust↔Rust 同版本线上 4v4 涂地对战**（原生↔原生、原生↔WASM、WASM↔WASM），协议逐字段对齐 PROTO v1，为未来与 JS 客户端跨端互打（M3）留好兼容边界。
- **Purpose**: 落实 spec.md（M1）开放问题 L160 的决议：**联网对战优先**；互操作档位为 **Rust↔Rust 同版本对战**（协议对齐并做兼容性测试，但不承诺全武器跨端互打）；大厅 UI 为**极简 bevy_ui**（不做 showcase 大厅场景）；GPU 复核**并行推进、不阻塞 M2**。
- **Target Users**: 仓库维护者本人（开黑对象为另一台机器/标签页上的 Rust 客户端）；验证期终端用户即开发者。

## Goals
- **G-1 协议兼容**：wire 层（信封、tick 数组、事件记录、控制帧）与 JS 端**逐字节等价**，以 node 从真实 `src/net/` 代码捕获的金标准 fixture 断言。
- **G-2 可玩联网对局**：2–4 个 Rust 客户端（原生/WASM 混合）经本地 relay 完成一整局 Tidewater 涂地（Spritzer、turf 模式），空位 host 补 bot；所有画面涂地覆盖与最终结果一致。
- **G-3 回放质量**：远端角色经 owner-tick + 播放时钟 + Hermite 路径 + correction 平滑呈现；注入模拟 lag/jitter/spike 后无跳变（kink/path-error 指标达标）。
- **G-4 会话与大厅**：create/join（5 位码）、名册/队伍/ready/start 全流程、host 迁移、掉线 bot 接管，极简 UI 可用。
- **G-5 架构纪律**：网络**逻辑**（codec/播放/会话/权威）全部落 `inkwave_sim`（无渲染依赖、无 I/O，fake transport 可测）；**I/O**（WebSocket）落 `inkwave` crate 的双端实现。延续 M1 的"玩法不依赖渲染"约束（AC-10 的兑现）。

## Non-Goals（M2 明确不做）
- **不做与 JS 客户端的全功能跨端互打**：JS 大厅 roster 可携带 12 主武器/副/特殊/style（session.js L279-287），Rust 侧仅 Spritzer + 占位角色。跨端**协议握手与 tick 交换**要验证（G-1/AC-2），同房间实际开黑留 M3（随内容移植）。
- 不做 Zone Control / Boss 战联网（lobby wire 的 `mode` 字段透传；Rust host 只允许 `turf`，guest 收到非 turf cfg 报错退出房间）。
- 不做 kit/sub/special 的 `['k']`/`['dh']`/`['bm'|'bc']`/`['z']` 事件重放（类型已在 M1 net.rs 注释占位）。
- 不做 showcase/lobbySet 大厅 3D 场景、emote 动画、语音、匹配/排位、跨 NAT 打洞（relay 本就是中继）。
- 不改 `server/` 任何文件；不改 `src/`、`tools/`、`docs/` 等上游树（金标准捕获脚本放 `rust/tools/`，只 import 上游代码）。
- 不做移动端、不做发布部署（本地 wrangler dev / relay shim 验证）。

## Background & Context（M2 调研结论，均已核实）
- **所有权模型**（NET.md "Ownership"）：每人全帧率模拟自己的角色，20Hz 广播 tick `{k:'t',ts,a:[24槽数组],e:[...]}`；host 额外拥有 bot、比赛时钟（tick 搭载 `c:[state,time]`，0.5s 一次）与结算判定（`res`/`end`）。玩家掉线→host 以其最后可见状态接管为 bot；host 离开→最老成员迁移。
- **回放时钟**（netmatch.js L229-284）：per-sender offset（最快包定网络地板、缓慢爬移防漂移）+ delay（1 tick + 2.5×jitter，clamp 85–300ms，平滑）；时钟随本地帧时推进，以 ±20–25% 速率驶向 `now−offset−delay`，样本将耗尽时降速至 0.35×。
- **路径**：位置三次 Hermite（owner 速度为切线，C1 连续）；两端有 grounded 时 Y 钳进 [min−0.02, max+0.02] 不穿地板；最新样本之外弹道外推 ≤180ms 后保持；`tp`（netTp）计数跳变→切断滑行；重生后隐藏至新生命首个样本。
- **correction**：每帧以"上一帧时刻"重评路径，差值作为偏移按临界阻尼曲线（ω=13）沉降，无 pop。
- **墨汁与命中**：splat 由喷涂者逐条广播（`['s',…]`，seeded 形状）→ 各端在同一播放时刻重放 → 全员涂地一致；他人弹丸为 visual-only ghost（`['p',…]`）；命中由射击方屏幕判定、发 `{k:'hit'}` 给受害者 owner 应用（不走时间线，血量须即时）。
- **relay 语义**（server/src/index.js）：文本帧；`b|`/`s|to|`/`m|from|` 信封；`{"t":"lock","v"}` 仅 host；`"ping"`→运行时自动 `"pong"`（兼心跳）；host=seq 最小成员；**ORIGIN 白名单**（游戏站点 + localhost/LAN/`:port` 形态）——原生 WS 客户端必须自带合规 Origin 头否则 403；`sendTo` 不排除自己；限速 RATE=90 msg/s（超 3× 或 4 次 burst 踢）；MSG_MAX=64KB；sweep：对局静默 20s/大厅 150s 断开。
- **时钟源**：JS `now()=performance.now()/1000`（任意起点）。跨端只需 per-sender 相对可比 → Rust 用进程启动起点的单调 f64 秒；wire 层 r3 量化（会话时长内 f64 精度充裕，禁用 f32 做时间算术）。
- **M1 已备**：`inkwave_sim::net` 类型骨架（ActorSample/Tick/SplatRecord/TriggerRecord/ForwardEvent/DirectMsg/HostMsg/ControlFrame，serde 无损 + 往返测试）；两处已声明的 Deviation 由 M2 wire 层收口（packTrig 对象分支、`['ev']` packed 对象透传——用 `serde_json::Value` 原样携带）。
- **relay 可达性**：沙箱对 github HTTPS 超时（M1 Task 2 记录）；wrangler dev（workerd 本地模拟）与部署 relay 是否可达**未验证**。规划采用双轨：测试主轨用 Rust 侧自写 **relay shim**（仅测试用，语义复刻 server/ 的 fan-out/选举/lock/ping，不替代 server/）；真实 relay（wrangler dev 或部署端）验证若沙箱不可达，则列入 GPU 复核同款"用户机器复核清单"。

## Functional Requirements

### 协议与传输
- **FR-M2-1 wire codec**（`inkwave_sim::net::wire`）：`ActorSample` ↔ 24 槽数组（逐槽 r2/r3/round，与 packActor L747-750 完全一致，含 `s[22]` spx=`SPECIAL_ORDER.indexOf+1` 的 M1 恒 0 语义）；`Tick`/事件记录（`['s'|'p'|'b'|'tr'|'ev'|'k'|'z'|'bm'|'bc']` 前缀数组）/`DirectMsg`/`HostMsg`/`ControlFrame` ↔ JSON 对象（`k:'t'` 等键名逐一对齐）；信封 `b|`/`s|to|`/`m|from|` 的构造与解析；JSON 序列化不得引入 JS 不认识的字段（未知字段接收时忽略）。
- **FR-M2-2 金标准 fixture**：`rust/tools/net-fixtures/`（node）import/驱动真实 `src/net/` 打包路径，对已知 actor 状态与事件序列捕获 JS 原始帧字符串存为 fixture；Rust codec 输出与之**逐字节相等**。若 JS 模块在 node 下不可引导（three/ctx 依赖），降级方案：fixture 脚本内复刻 packer 并逐行对照评审 + 用户机器对真实 relay 抓样复核；此决策记录于台账。
- **FR-M2-3 双端 Transport**（`inkwave::net`）：trait/枚举封装 + 两实现——原生（tungstenite，独立线程 + mpsc，Bevy 主线程零阻塞）、WASM（web-sys WebSocket，事件回调）；2s `"ping"`、rtt EMA(0.3)、welcome 握手（8s 超时）、`err`→reject、close→`onClose(reason)`；连接地址解析对齐 `relayURL()`：CLI 参数/环境变量 > localhost 检测 > `PROD_RELAY`；**原生实现必须发送合规 `Origin` 头**（如 `http://127.0.0.1:<port>`）以过 relay ORIGIN_OK。
- **FR-M2-4 relay shim（仅测试）**：`rust/tools/relay-shim/` 原生可执行：同端口语义（welcome/join/leave/host 选举/`b|`/`s|to|`/lock/ping-pong/静默 sweep 参数化）。文档注明**不是** server/ 的替代、不部署。

### 会话与大厅
- **FR-M2-5 会话状态机**（`inkwave_sim::net::session`，纯逻辑 + 注入 transport）：`offline/connecting/lobby/starting/match/error`；create（码表 `BCEFGHJKLMNPQRTUVXYZ23456789`、5 位、"Room code taken" 重试 ≤4）/join（≥4 位校验、错误文案对齐）/leave；host 拥有 lobby：`me`/`lobby` 广播、`_fixTeams`（≤4/队、auto 补小数）逐行对齐 session.js L173-179；`start()` 产 roster（nid 顺序、bot 补位、`{k:'start'}` cfg 字段齐全）+ `lock(true)`；`ready`/`go` 握手（12s 超时兜底）；`endMatch`→lock(false) 回大厅；ping→host（2s 周期）；emote 透传（UI 忽略）；host 迁移（`leave` 帧带新 host）。
- **FR-M2-6 极简大厅 UI**（`inkwave::ui::online`，bevy_ui）：在线入口（创建房间显示 5 位码/输入码加入）、名册列表（队/ready/host/ping）、切队与 ready 按钮、host 的 start 按钮与设置（仅 turf/duration/difficulty/bots 最小子集）、错误与断线提示；进入对局/回大厅的状态切换。无 showcase 场景。

### 对局复制
- **FR-M2-7 权威切分**（`inkwave_sim::net::replication`）：每端仅模拟自己 actor +（host）bot；远端 actor 为 proxy（状态由播放层写入，不跑本地物理/输入）；命中：射击方判定后发 `hit`，受害者 owner 应用伤害/死亡/重生；受害者端重生走 owner 自己的 sim 并广播（tp++）。
- **FR-M2-8 播放层**（`inkwave_sim::net::playback`）：per-sender 时钟（offset/delay/jitter/steering/降速，参数逐一对齐 netmatch L229-284）；样本环形缓冲与 Hermite（含 Y 地板钳制）；≤180ms 弹道外推后保持；correction（临界阻尼 ω=13）；tp 切断与重生隐藏；离散状态取较早样本；`ch/lock/ink` 线性、`hp` 取 u≥0.5 侧——与 hermite() L766-786 逐行对齐。
- **FR-M2-9 事件时间线重放**：`['s']`→paint 应用（applying 模式防回环再广播；seeded 形状保证覆盖一致）；`['p']`→visual-only ghost 弹丸（不喷墨不判伤）；`['tr']/['ev']`→按名分发（M2 仅接 Spritzer 相关 + `splatted`/`respawn` 的最小 payload，packed 对象 `serde_json::Value` 透传）；事件按 sender-clock 时刻与路径同序播放。
- **FR-M2-10 host 职责**：bot 模拟并入 host tick；比赛时钟与 `c` 快照（0.5s）；终局判定→`res`（双方百分比）+`end`；掉线玩家以最后 proxy 状态接管为 bot；host 迁移时新 host 接管 bot/时钟/判定，各端 actor 所有权不变。

## Non-Functional Requirements
- **NFR-M2-1 sim 纯净性**：codec/session/playback/replication 全在 `inkwave_sim`，`cargo tree -p inkwave_sim` 依赖面不新增 I/O 栈（允许 serde_json 已有；禁止 tokio/tungstenite 入 sim）。
- **NFR-M2-2 带宽与开销**：单客户端 ≤ relay RATE 预算（4 真人 + host 4 bot：host tick 含 5 actor ≈ 每 50ms 1 帧，远低于 90/s）；播放层 CPU 开销使原生对局帧时较 M1 基线（p50 sim 步 0.063ms）增幅 < 20%。
- **NFR-M2-3 确定性可测**：全部网络逻辑可在无 socket 环境以 fake transport + 注入 lag/jitter（对齐 JS `?netlag/netjitter/netspike` 语义：附加延迟、乱序聚束、250ms 尖峰）跑确定性测试。
- **NFR-M2-4 门禁延续**：M1 全部门禁（fmt/clippy/test/wasm/trunk/headless 冒烟/JS 树零改动）在 M2 每个任务收尾时全绿；测试总数只增不减。

## Acceptance Criteria

### AC-M2-1: wire 逐字节兼容
- **Type**: `rule`
- **Given**: `rust/tools/net-fixtures/` 生成的 JS 权威帧样本
- **When**: Rust codec 对相同输入状态编码
- **Then**: 信封字符串、tick JSON（键序无关、值逐字节）、24 槽数组（含量化舍入）与 fixture 完全相等；解码 JS 样本无损
- **Pass Condition**: 金标准测试全绿；fixture 覆盖：常规/鱿鱼/重生 tp 跳变/满 flag/边界舍入（-0、.005 半值）
- **Evidence**: `cargo test -p inkwave_sim` 输出 + fixture 文件

### AC-M2-2: Rust↔Rust 完整联网对局
- **Type**: `rule`
- **Given**: relay shim + ≥3 个无头 Rust 客户端（含 1 host，bot 补位至 8）
- **When**: 固定种子跑完一整局 turf（缩短时长）
- **Then**: 全员完成 intro→active→结算；各端涂地网格逐面一致（覆盖率差 < 0.5%）；结果与 host 判定一致；无 panic
- **Pass Condition**: 集成测试断言通过；另以 2 真人 + lag 注入复跑一次
- **Evidence**: 测试输出（逐端覆盖率表）

### AC-M2-3: 回放平滑度
- **Type**: `rubric`
- **Dimension**: 注入 netlag=40/netjitter=30/netspike=0.01 时远端角色的视觉连续性（对照 JS net-test 的 kink/path-error 口径）
- **Scale**: 1-5
- **Anchors**: 1=明显瞬移/穿地；3=偶见轻微修正、无 pop 无穿模；5=盲测与本地角色无异
- **Pass Threshold**: ≥ 3，且量化指标：每帧 kink 计数与路径误差 p95 在自写 harness 中与 JS net-test 同量级（报告数值，评审定标）
- **Evidence**: 无头 harness 输出统计 + 有 GPU 环境人工复核（并入复核清单）

### AC-M2-4: 会话弹性
- **Type**: `rule`
- **Given**: 3 客户端对局中
- **When**: ①非 host 玩家掉线；②host 掉线
- **Then**: ①其角色由 host 以最后可见状态接管为 bot 继续活动，其余端无报错；②最老成员升为 host，bot/时钟/判定迁移，对局打完并出结算
- **Pass Condition**: 两个场景的集成测试通过
- **Evidence**: 测试输出

### AC-M2-5: 双端联网
- **Type**: `rule`
- **Given**: trunk 起服的 WASM 客户端 + 原生客户端（同 relay shim 或 wrangler dev）
- **When**: 大厅组房、开一局、操作 60s
- **Then**: 互见对方角色平滑移动/喷涂；WASM 控制台无 error/panic
- **Pass Condition**: 冒烟脚本（沿用 M1 chrome-headless 方案 + 网络断言）通过；若沙箱无法跑通浏览器 WS 对局，降级为代码评审 + 用户机器复核清单，并如实记录
- **Evidence**: 冒烟日志/截图

### AC-M2-6: 大厅流程闭环
- **Type**: `rule`
- **Given**: 两个 UI 客户端
- **When**: create→join→切队/ready→host start→对局→结算→回大厅→再开一局
- **Then**: 全程状态正确、名册一致、lock 生效（对局中 join 被拒 "Match in progress"）
- **Pass Condition**: session 状态机集成测试 + 人工双窗口记录
- **Evidence**: 测试输出 + 试玩记录

### AC-M2-7: 门禁与架构纪律
- **Type**: `rule`
- **Given**: M2 全部任务收尾
- **When**: 全量门禁 + `cargo tree -p inkwave_sim` + `git status`
- **Then**: M1 基线门禁全绿且测试数 > 147；sim 依赖仍仅 glam/serde/serde_json；JS 树零改动
- **Pass Condition**: 命令输出
- **Evidence**: 门禁日志

### AC-M2-8: 跨端协议握手（尽力项，可顺延）
- **Type**: `rule`
- **Given**: 本地真实 relay（wrangler dev；沙箱不可达则列用户机器复核）
- **When**: Rust 客户端与 JS headless 客户端（tools/net-test.mjs 方式，只读不改）同房
- **Then**: welcome/join/leave 互认、tick 互收不报 err；不要求玩法一致（武器集不同）
- **Pass Condition**: 握手与 tick 交换日志；若环境不可达，本条标记"待用户机器复核"不算失败
- **Evidence**: relay 日志 + 双端控制台记录

## Constraints
- 同 M1（Rust stable、Bevy 0.19.1、trunk、不引入 GPL）。新增依赖：native `tungstenite`（或 `fastwebsockets` client，实施时定）、wasm `web-sys`（Bevy 已带）；fixture 生成仅用 node（已有）。
- relay/`server/`/上游树零改动是硬约束（fork 可干净 merge）。
- 所有 JS 侧行为对齐以**源码行号**为准写注释（沿用 M1 net.rs 风格）。

## Assumptions
- 同机/局域网多开即满足验证需求（shim/wrangler 本地跑）；公网部署 relay 的真实联网体验由用户在 GPU 复核窗口一并验证。
- 角色外观差异（占位体块 vs JS 过程化角色）不影响协议兼容验证。
- 大厅 `style/weapon/sub/special` 字段以透传+忽略呈现处理（Rust 仅 Spritzer 视觉）。

## Open Questions
- [ ] FR-M2-2 fixture 引导路径：node 直接 import `src/net/netmatch.js`（three/ctx 依赖能否在 node 下 stub）vs 复刻 packer——Task 17 实施时定，降级方案已在文内。
- [ ] 原生 WS 库选型（tungstenite 线程模型 vs 其他）——Task 18 实施时定并记 PORT_MAP。
- [ ] AC-M2-3 的 kink/path-error 数值门限——先产出 Rust 侧统计，对照 JS net-test 报告后评审定标。
