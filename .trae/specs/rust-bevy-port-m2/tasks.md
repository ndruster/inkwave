# INKWAVE Rust/Bevy 移植 — M2 线上对战（PROTO v1）实施计划

> 任务号延续 M1（Task 1–16）。一次只推进一个 `pending` 中最高优先级项；每个任务收尾流程沿用 M1：实现+测试 → 全门禁 → 独立审查 agent → 修复 → resume 复审 → 重跑门禁 → 本文件写 Completion Evidence + Review 附录、Status=completed。数值/协议对齐一律以 JS 源码行号为准写注释。

**M2 决议（2026-10-07 与用户确认）**：联网对战优先；互操作档位 = Rust↔Rust 同版本对战（协议对齐 PROTO v1 并做兼容测试，不承诺全武器跨端互打，跨端实际开黑留 M3）；大厅 = 极简 bevy_ui（无 showcase 场景）；GPU 复核并行推进不阻塞。

---

## Task 17: wire codec 与金标准 fixture
- **Status**: `completed`
- **Priority**: high
- **Depends On**: None
- **Description**:
  - `inkwave_sim::net::wire`：`ActorSample` ↔ 24 槽数组（packActor netmatch.js L747-750 逐槽量化：r2/r3/round；槽序、缺省值、`s[22]` spx=M1 恒 0 语义全对齐）；`Tick` ↔ `{k:'t',ts,a,e}`（ts=r3(now())，`a` 为数组的数组）；事件记录 `['s'|'p'|'b'|'tr'|'ev'|'k'|'z']` 前缀数组 ↔ sim 类型（`['ev']` packed 对象与 `['tr']` 对象分支用 `serde_json::Value` 原样透传，收口 M1 net.rs 两处 Deviation）；`DirectMsg`/`HostMsg`/`ControlFrame` ↔ JSON；信封 `b|`/`s|to|`/`m|from|` 构造与解析（`m|` 解析仅客户端侧需要）。
  - `rust/tools/net-fixtures/`（node）：优先引导真实 `src/net/` packer（stub three/ctx 依赖）对已知状态捕获 JS 原始帧字符串 → fixture JSON；若引导不可行，降级为逐行对照复刻 + 台账记录（spec Open Question 1 落定处）。
  - 金标准测试：Rust 编码输出与 fixture **逐字节相等**；解码 JS 样本无损；边界：-0、x.xx5 半值舍入、满 flag、hp/ink 取整。
- **Acceptance Criteria Addressed**: AC-M2-1
- **Test Requirements**:
  - `rule` TR-17.1: fixture 文件存在且由脚本生成（删除后测试失败）；覆盖 tick/splat/proj/trig/ev/hit/lobby/start 各至少 2 样本；证据为脚本输出与文件
  - `rule` TR-17.2: `cargo test -p inkwave_sim` 金标准逐字节断言全绿（编码相等 + 解码往返）；证据为测试输出
  - `rule` TR-17.3: 舍入边界用例（-0、0.125→r2、0.0005→r3、round 半值）与 JS `Math.round`/`toFixed` 语义一致；证据为测试输出
  - `rule` TR-17.4: 全门禁（fmt/clippy/test/wasm build/trunk build/headless 冒烟）+ `cargo tree -p inkwave_sim` 依赖面不新增 I/O 栈 + JS 树零改动；证据为命令输出
- **Notes**: 时间戳量化在 wire 层做（r3），sim 内部保持全精度（M1 决策延续）。JSON 序列化键序不做要求（JS `JSON.parse` 无关），但值必须逐字节。
- **Completion Evidence**:
  - **fixture 引导方案落定（spec Open Question 1）**：`rust/tools/net-fixtures/gen.mjs` 用"源码切片 + `new Function` eval"直接驱动 netmatch.js **真实 packer**（r2/r3/F/packActor/unpackActor/blankSample/copySample/packTrig/unpackTrig/packEvent/unpackEvent），stub `MAIN_KITS={}`/`specialNetState=()=>0`/`THREE.Vector3` 复刻 M1 恒 0 路径；上游 `src/` 零改动。全部手写复刻区（recBomb/recKit/recZone/recSplat/recProj 表达式、hit/dh/bhit/st/res/end 消息、`_sendTick` 键序与 `msg.c`/`msg.e`、`_rec` 时间戳前置、session lobby/start/ready/go/emote、server welcome/join/leave/err/pong、transport lock 与 `b|`/`s|` 信封）共 32 条 `assertContains` 锚定上游真实字面量，漂移即挂生成器。产物 `rust/assets/net-fixtures.json`（`include_str!` 编译期嵌入 wire.rs）。
  - **f32/f64 字节兼容（本任务核心决策）**：JS 全程 f64、Rust 字段 f32，`f64::from(0.61_f32)` 噪声会跨 r2/r3 半值边界。解法：`wire::dec(x: f32) -> f64` 取 f32 最短十进制 `to_string()` 再 parse 回 f64，恢复 JS packer 看到的精确十进制值；fixture 输入一律精确十进制（无 Math.fround），契约 = "Rust f32 存储经 dec 回到同一 f64"。整数槽直接 `f64::from`。
  - `rust/crates/inkwave_sim/src/net/wire.rs`（~1500 行）：`num()` 手工 JSON 数字打印（整数不带 `.0`、`-0`→`0`，serde_json/ryu 不满足 JS `JSON.stringify` 语义）；`pack_actor`/`unpack_actor` 24 槽逐槽对齐；`encode_tick`/`decode_tick`（键序 k,ts,a[,e][,c] 手工 `obj()` 固定）；`EventRecord{name,payload:JVal}`、`TriggerRecord.data:TrigData`（Zero/Num/Obj 三分支，收口 M1 两处 Deviation）、`ProjectileRecord` 扩至 26 字段（透传不量化槽 grav/drag/trailEvery/head/vis/tail0/tailK/wob/wobF/nose/sats 与 JS 缺省值全对齐）；`ForwardEvent` 名表 ↔ netmatch.js L47 FORWARD；`DirectMsg`/`HostMsg`（res turf 形态省略 mode/bo 对齐 JS undefined 被 stringify 丢弃）/`ControlFrame`；`JVal`（保序保 token JSON 值，手写 Serialize/Deserialize，无 preserve_order/indexmap 依赖）；信封 `b|`/`s|to|`/`m|from|`；`encode_timed_event`（`_rec` 时间戳前置）。
  - TR-17.1 ✅：fixture 由 `node rust/tools/net-fixtures/gen.mjs` 生成；删除后 `cargo test` 编译失败（`couldn't read ...net-fixtures.json`，实测验证）。覆盖度：tick=3、actors=6、splat=2、proj=2、trig=4、ev=3、hit=2、lobby=2、start=2、control=7。
  - TR-17.2 ✅：wire.rs 17 个金标准测试全绿——6 actor 样本编码逐字节相等 + 解码对照 `unpacked`；tick guest/host/with_events 往返；事件记录（splat/proj/bomb/kit/zone/trig/ev）往返金标准；`direct_messages`/`host_messages`/`control_frames`/`session_messages`（me/lobby/lobby2/start/start2/ready/go/emote JVal 透传逐字节）；`envelopes_split_relay_frames`；`event_payloads_pass_through_verbatim`。
  - TR-17.3 ✅：`round_edges_are_js_half_up_and_minus_zero_collapses`——`round_int`=`floor(x+0.5)` 向 +∞（61.5→62、0.5→1）、r2/r3 半值（0.125→0.13、-0.125→-0.12、±0.0005→r3 0）、`-0`→`"0"`、`0.005`→r2 `0.01`，与 JS `Math.round`/量化语义逐字节一致（fixture `round_edges` 金标准锚定）。
  - TR-17.4 ✅（审查修复后重跑）：`cargo fmt --all --check` ✅、`clippy --workspace --all-targets -D warnings` ✅、`cargo test --workspace` **164 passed**（36 inkwave + 128 sim，wire 17）✅、`cargo check --target wasm32-unknown-unknown` ✅、`trunk build --release` ✅（exit 0）、原生 headless 冒烟（`--headless --frames 5` boot+complete）✅、`cargo tree -p inkwave_sim` 仅 glam/serde/serde_json ✅、JS 树（src/server/tools/docs）`git status` 零改动 ✅。
- **Review 修复附录**（独立审查：0 blocker / 2 major / 若干 minor，全部处理；复审 agent 独立重验 30 条锚定逐字命中、生成物零 diff、门禁复跑，结论"可合入"）:
  - **M-1 ✅**：wire.rs 测试 `session_messages_pass_through_verbatim` 的 for 数组 fmt 差异 → `cargo fmt --all` 修复，`--check` 通过。
  - **M-2 ✅**：gen.mjs 手写复刻区缺上游锚定 → `assertContains(src, needle, where)` 扩展至 session.js/transport.js/server/src/index.js 三源，30 条 needle 全部取上游真实字面量（见 Completion Evidence），重跑生成器零 diff。
  - **Minor 备案（不阻塞，后续任务处理）**：① tick 的 `msg.B`（boss.pack，netmatch.js L196）未建模——Rust↔Rust 同版本档不跑 Boss，Task 18+/Boss 联网移植时补；② `own`/`me` 无发送侧单一字面量（`_ownership` 空桩/`...o` 展开），gen.mjs 注释已点明形态取接收侧契约；③ `num()` 极端区间（>1e21 等）文档、Rust-only 测试复用 fixture、i5 debug_assert 为备忘项。
  - **范围外改动归属（复审确认）**：`bench.rs` clippy 1.98 新 lint 修复（`to_vec()`）拆独立 chore 提交；M1 spec.md 开放问题勾选随 M2 spec 提交；Task 17 提交面 = net.rs、net/wire.rs、gen.mjs、net-fixtures.json。

## Task 18: 双端 Transport（WebSocket I/O）
- **Status**: `completed`
- **Priority**: high
- **Depends On**: Task 17
- **Description**:
  - `inkwave_sim::net::transport`：纯 trait（connect/broadcast/sendTo/lock/close/事件回调或 channel 语义 + rtt），fake 实现供 sim 测试注入 lag/jitter/spike（对齐 JS SIM 语义：netlag 附加延迟、netjitter 乱序聚束、netspike 250ms 尖峰，transport.js L68-72）。
  - `inkwave::net::transport`：native（tungstenite，独立线程 + mpsc，Bevy 主线程零阻塞；**必须发送合规 Origin 头**，如 `http://127.0.0.1:<port>`，过 relay ORIGIN_OK server/src/index.js L22-23）；wasm（web-sys WebSocket）。2s `"ping"` 心跳、rtt EMA(0.3)、welcome 握手 8s 超时、`err`→reject、close→onClose(reason)（transport.js 全文件行为）。
  - 地址解析对齐 `relayURL()`：CLI `--relay=`/环境变量 > localhost/LAN 检测（`ws://<h>:8787`）> `PROD_RELAY`。
- **Acceptance Criteria Addressed**: AC-M2-5（基础）、AC-M2-8（基础）
- **Test Requirements**:
  - `rule` TR-18.1: fake transport 单测：注入 lag/jitter/spike 后消息时序断言（确定性种子）；证据为 `cargo test -p inkwave_sim` 输出
  - `rule` TR-18.2: native transport 对本地 echo/ws-server（测试内起）完成 connect→welcome→b|/s|→close 全流程；证据为测试输出
  - `rule` TR-18.3: wasm 目标编译通过（`cargo build --target wasm32-unknown-unknown`）；浏览器实连验证并入 Task 26；证据为构建输出
  - `rule` TR-18.4: 全门禁 + sim 依赖面不变（trait 无 I/O）+ JS 树零改动；证据为命令输出
- **Notes**: 选型记录进 PORT_MAP（spec Open Question 2 落定处）。消息大小/限速是 relay 侧行为，客户端只需不超（tick 50ms 一帧远低于 RATE=90/s）。
- **Completion Evidence**:
  - **Open Question 2 落定**：native = `tungstenite` 0.26 同步 API + 独立 I/O 线程 + `std::mpsc`（无 async runtime、Bevy 主线程零阻塞）；弃 `fastwebsockets`（需 tokio 栈）；TLS = `rustls-tls-webpki-roots`（离线可解析，`MaybeTlsStream::{Plain,Rustls}` 均设 50ms read timeout 保线程可唤醒）；wasm = `web-sys` WebSocket（浏览器事件回调驱动）。Cargo.toml 把 tungstenite 门进 `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`——wasm 构建不拉 rustls/getrandom（wasm32 不支持）。全文记 PORT_MAP。
  - `inkwave_sim::net::transport`（~1240 行，纯 trait 无 I/O）：`relay_url`/`room_url`/`js_encode`（encodeURIComponent 免转义集含 `!'()*` 五 mark）/`is_local_host`/`rtt_ema` 对齐 transport.js L9-16/L44/L53；`TransportEvent`（Welcome/Join/Leave/Message/Error/Closed）+ `Transport` trait（connect/poll_events/send_raw/is_open/rtt/close + broadcast/send_to/lock 默认方法走 wire 信封）；**`RxState::handle` 为 fake/native/wasm 三实现共用的唯一帧语义路径**（pong RTT、`m|` 信封、err→reject、welcome、join/leave、`{"t":"pong","c"}` 旧式回显）；`FakeRelay`+`FakeTransport`：共享虚拟时钟 + 确定性 xorshift64* RNG，SIM 公式 `max(arrival+lag+rand*jitter+(rand<spike?250:0), last)` 每帧两次 rand 先 jitter 后 spike（L68-72 逐行），单调 last 地板复现 TCP 聚束；relay 语义对齐 server/src/index.js（welcome roster 含己、host=seq 最小、`b|` 排他、`s|` 含己、lock 仅 host、fail 阶梯四文案 L70-74、ping→pong、err 后 Error+Closed 成对、8s 握手超时）。
  - `inkwave::net::native_transport`（tungstenite）：io_loop 线程拥有 socket+RxState+计时器（2s 心跳首拍即时/`ping_sent` 计最旧未答、8s 握手计时、epoch 用 `Instant`）；`build_and_connect` 经 `IntoClientRequest` 生成 5 必备头后**追加合规 Origin**（`origin_for`：ws→http、wss→https，过 ORIGIN_OK localhost/LAN/`:port` 形态）；`resolve_relay_base`：CLI `--relay=` > `INKWAVE_RELAY` > `INKWAVE_HOST` 探测 > PROD_RELAY；`close()`→`Cmd::Close`→`close(1000,"bye")` 不发 Closed；黑洞网络防御：closing 后连续 10×50ms read 超时即退出（Drop::join 有界）。
  - `inkwave::net::wasm_transport`（web-sys）：`relay_url_from_location`（?relay= > hostname 探测 > PROD，对齐 L10-16）；`Closure` 挂 onmessage/onclose，`setInterval(2000)` 心跳、`setTimeout(8000)` 握手超时；`reject(e, closed_reason)` 双参——err 拒绝 (e,e)（server `close(4000,e)`），超时 ("Could not connect","Disconnected")；显式 close 先 `set_onclose(None)`。
  - TR-18.1 ✅：`cargo test -p inkwave_sim net::transport` **11 passed**——URL 解析×2、rtt_ema、RxState 帧语义、fake 握手/路由/lock 全流程、join 阶梯四文案、8s 超时、未开放发送丢弃、lag 保序、jitter 聚束+种子确定性、spike 250ms 阻塞。
  - TR-18.2 ✅：`cargo test -p inkwave net::` **5 passed**——`origin_for_maps_ws_scheme`、`resolve_relay_base_prefers_cli_override`、`native_connect_welcome_ping_close`（测试内起忠实模拟 server 语义的本地 ws relay：welcome 断言 + **Origin 头实捕断言 `http://127.0.0.1:<port>`** + 显式 close 无 Closed）、`native_broadcast_and_send_to_between_two_clients`（双客户端 join 可见、`b|` 排他回环、`s|` 定向含己路由）、`native_join_ladder_rejects_unknown_room`（"Room not found"+Closed 成对）。
  - TR-18.3 ✅：`cargo check --workspace --target wasm32-unknown-unknown` 通过 + `trunk build --release` ✅（exit 0，审查修复后重跑）。浏览器实连留 Task 26。
  - TR-18.4 ✅（审查修复后重跑）：fmt ✅、`clippy --workspace --all-targets -D warnings` 0 错 ✅、`cargo test --workspace` **180 passed**（41 inkwave + 139 sim）✅、wasm32 check ✅、trunk release ✅、headless 冒烟（`--headless --frames 120` boot+complete）✅、`cargo tree -p inkwave_sim` 仅 glam/serde/serde_json（trait 无 I/O 依赖面不变）✅、JS 树 `git status` 零改动 ✅。
- **Review 修复附录**（独立审查：3 blocker + 若干 minor，全部处理；resume 复审确认修复正确并新报 1 项，再修后二复审"可合入"）:
  - **B-1 ✅**：native `Message::Close` 分支 unsettled 时静默挂起到 8s 超时——拆为两条路径对齐浏览器事件序：Close 帧 unsettled 仅发 Error（reason 回退，JS `done` 吞 onClose）；socket 错误 unsettled 发 Error+Closed 成对（onerror+onclose 双触发）。
  - **B-2 ✅**：黑洞网络下 `close()` 后对端永不回 Close，`Drop::join` 永久阻塞——closing 后累计 10 次 50ms read 超时即 return（join 有界 ~0.5s）。
  - **B-3 ✅**：wasm 8s 超时 `Closed.reason` 误为 "Could not connect"（JS 超时 close 无 reason→'Disconnected'）——`reject` 增 closed_reason 参数，三端（fake/native/wasm）超时路径统一 Closed("Disconnected")。
  - **N-1 ✅（resume 复审新报）**：io_loop 空闲分支 `cmd_rx.try_recv()` 探测会消费后丢弃 `Cmd::Connect`（实测 ~10-20% flaky）——删除探测，命令一律由循环顶部统一 drain；修复后目标测试连跑 10 次 0 失败。
  - **Minor 修复**：测试 relay 畸形 `s|x`（无第二竖线）panic → `let Some(k) … else { continue }`。
  - **Minor 备案（不阻塞，后续任务处理）**：① fake `part()` 不清 `locked`（join 时 open==0 清除掩盖，仅测试旋钮可观测）；② FakeRelay 未实现 `{"t":"ping","c"}`→pong（trait 路径不可观测，Task 19 shim 覆盖）；③ fake 显式 close 后在途帧仍 deliver（JS `this.ws===ws` 守卫）；④ `bytes_in` 按字节 vs JS UTF-16 码元（非 ASCII 载荷有差）；⑤ `is_local_host` 接受残缺 "172.16"（JS 正则要求尾点）；⑥ 2s 心跳节奏/多次 RTT EMA 端到端断言、SIM 双 rand 顺序钉死测试留 Task 19+ harness。

## Task 19: relay shim 与集成测试 harness（仅测试用）
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 17
- **Description**:
  - `rust/tools/relay-shim/`：原生可执行，语义复刻 server/src/index.js：welcome/join/leave 帧、host=seq 最小选举、`b|`/`s|to|` 透传（不解析载荷）、`lock`（仅 host）、`ping`→`pong`、静默 sweep（match/lobby 阈值参数化）、MSG_MAX/RATE 检查（可放宽为测试配置）。README/代码注释明确**不是** server/ 替代、不部署、仅测试。
  - `rust/tools/net-harness/`（或 sim 集成测试 bin）：多客户端无头对局驱动器——起 shim、拉起 N 个 in-process 客户端会话、注入 lag 场景、收集各端状态快照输出一致性报告。
- **Acceptance Criteria Addressed**: AC-M2-2/3/4 的测试基建
- **Test Requirements**:
  - `rule` TR-19.1: shim 单测：3 socket 场景下 welcome 成员表/host 选举/`b|` fan-out 不排他回环（sendTo 自己可达）/leave 后 host 迁移/lock 拒 join（"Match in progress"）；证据为测试输出
  - `rule` TR-19.2: shim 与 native transport 互通（Task 18 的 connect 全流程走 shim）；证据为测试输出
  - `rule` TR-19.3: 全门禁 + JS 树与 `server/` 零改动；证据为命令输出
- **Notes**: 若沙箱可跑 `npx wrangler dev`，追加用真实 server/ 复跑 TR-19.1 场景作为交叉验证（可达性未验证，列入实施时探测；不可达则记入用户机器复核清单）。

## Task 20: 会话状态机（NetSession）
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 17, Task 18
- **Description**:
  - `inkwave_sim::net::session`：状态机 `offline/connecting/lobby/starting/match/error`；create（码表 `BCEFGHJKLMNPQRTUVXYZ23456789` 5 位、"Room code taken" 重试 ≤4）/join（≥4 位、错误文案 'Room not found'/'Room is full'/'Match in progress'/'Could not connect'）/leave；host 拥有 lobby：`{k:'me'}`→`_applyMe`、`{k:'lobby'}` 广播（`_wireLobby` 字段集）、`_fixTeams` 逐行对齐 session.js L173-179（≤4/队、auto 补小数）；`start()`：roster 生成（nid 顺序先 0 队后 1 队、bot 补位、cfg 字段 `{k:'start',roster,map,time,duration,difficulty,palette,mode,host,id}`）+ `lock(true)`；`ready`/`go` 握手（12s 超时兜底，session.js L320-335）；`endMatch`→lock(false)+ready 清空回 lobby；ping→host 2s 周期（L381-390）；emote 透传；host 迁移（leave 帧 host 字段）。mode 仅允许 turf（非 turf cfg：host 拒发、guest 收到报错退房）。
  - 全部经 transport trait 注入，fake 可测。
- **Acceptance Criteria Addressed**: AC-M2-6（逻辑层）、AC-M2-4（逻辑层）
- **Test Requirements**:
  - `rule` TR-20.1: 单测：create/join/leave 状态转移与错误路径（含 relay err 帧文案透传）；证据为测试输出
  - `rule` TR-20.2: 单测：host 侧 me/lobby 广播、_fixTeams 平衡（8 人满房、auto 分配、拒绝超 4）、setSettings 最小子集校验；证据为测试输出
  - `rule` TR-20.3: 单测：start→ready→go 时序（含 12s 兜底、加载期掉线视为 ready，session.js L324）；roster 结构与 JS `start()` 输出逐字段一致（fixture 对照）；证据为测试输出
  - `rule` TR-20.4: 全门禁 + sim 依赖面不变 + JS 树零改动；证据为命令输出
- **Notes**: `style/sub/special` 字段透传不解释（Rust 仅 Spritzer/占位角色）。

## Task 21: 权威切分与 host 职责（replication）
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 20
- **Description**:
  - `inkwave_sim::net::replication`：本地 actor（owner 全帧率 sim）vs proxy（不跑物理/输入，状态由 Task 22 播放层写入）的组件/标记与调度改造（现有 match/actor 循环按 owner 过滤）；命中通道：射击方判定→`{k:'hit'}`（sendHit L163-167，d=r2）→受害者 owner 应用（死亡/重生/扣血即时，不走时间线）；`{k:'dh'}` 收口为忽略 stub（M2 无设备）；host：bot 并入其 tick `a`、时钟快照 `c:[state,r2(time)]` 0.5s（_sendTick L197-200）、终局 `res`/`end`；掉线接管：以最后 proxy 状态原地转 bot（NET.md "Ownership"）；host 迁移：新 host 接管 bot/时钟/判定，所有权映射不变。
  - 与现有 `match_`/`bot`/`actor` 模块的接缝以最小改动落地（不重构 M1 逻辑）。
- **Acceptance Criteria Addressed**: AC-M2-2（骨架）、AC-M2-4
- **Test Requirements**:
  - `rule` TR-21.1: 单测：owner/proxy 划分——proxy 不受本地输入/物理影响、owner actor 进 tick `a` 且 nid 路由正确；证据为测试输出
  - `rule` TR-21.2: 单测：hit 通道端到端（fake transport 双端）：A 射中 B→B owner 应用伤害→B 死亡→tp++→重生样本广播；死亡瞬间各端状态一致；证据为测试输出
  - `rule` TR-21.3: 单测：host 掉线迁移（leave 帧）→新 host 接管 bot+时钟+判定，对局可打完出结算；非 host 掉线→其 actor 转 bot；证据为测试输出
  - `rule` TR-21.4: 全门禁 + JS 树零改动；证据为命令输出

## Task 22: 播放层（时钟 + Hermite + correction）
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 17
- **Description**:
  - `inkwave_sim::net::playback`：per-sender 播放时钟（offset=最快包+缓慢爬移；delay=1 tick+2.5×jitter clamp 85–300ms eased；随本地帧时推进、±20–25% 驶向 `now−offset−delay`、样本将耗尽降至 0.35×——逐参数对齐 netmatch.js L229-284）；样本环形缓冲（容量对齐 JS）；Hermite 路径（位置 C1、Y 地板钳制 L776、外推 ≤180ms 后保持、tp 跳变切断、重生隐藏至新生命首样本）；correction（每帧以昨刻重评路径、差值临界阻尼 ω=13 沉降）；离散字段取较早样本、`ch/lock/ink` 线性、`hp` u≥0.5 侧（L766-786 逐行）。时间算术全 f64。
- **Acceptance Criteria Addressed**: AC-M2-3
- **Test Requirements**:
  - `rule` TR-22.1: 单测：合成样本序列（已知解析运动）→ 播放输出与真值路径误差（无 lag 时 p95 < 2cm；Hermite C1 无角点）；证据为测试输出
  - `rule` TR-22.2: 单测：lag/jitter 注入下 delay 收敛进 85–300ms 带、时钟不越界（不外推超 180ms 后不瞬移）；tp 跳变→切断不滑行；证据为测试输出
  - `rule` TR-22.3: 单测：correction——中途改写已播样本，输出无 pop（帧间差值有界）且渐近收敛新路径；Y 钳制：两端 grounded 时不穿地板；证据为测试输出
  - `rule` TR-22.4: 全门禁 + NFR-M2-2 开销断言（harness 步时增幅 < 20%）；证据为命令输出

## Task 23: 事件时间线重放（墨汁/ghost/触发）
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 21, Task 22
- **Description**:
  - 事件按 sender-clock 时刻与路径同序播放（_playEvents 语义）：`['s']`→paint 应用（`applying` 模式防回环再广播，seeded 形状保证各端覆盖一致）；`['p']`→visual-only ghost 弹丸（不喷墨、不判伤、到期消失）；`['tr']`→角色触发（M2 仅 Spritzer 相关名 + 数值/对象双形态 payload）；`['ev']`→按名最小分发（`splatted`/`respawn`/`weapon:fire`，packed `serde_json::Value` 透传，未知名忽略）；录制侧：本地 splat/射击/触发经 `_rec`（r3(now()) 前缀）挂 outgoing tick `e`。
- **Acceptance Criteria Addressed**: AC-M2-2（覆盖一致性）
- **Test Requirements**:
  - `rule` TR-23.1: 单测：双端 fake transport——A 喷涂→B 重放后逐面覆盖网格与 A 一致（同 seed 同形状，断言网格字节级相等）；B 本地不产生回环广播；证据为测试输出
  - `rule` TR-23.2: 单测：ghost 弹丸只表现不判定（不扣血不喷墨）；事件与路径同序（splat 时刻 = 其 sender-clock）；证据为测试输出
  - `rule` TR-23.3: 全门禁 + JS 树零改动；证据为命令输出
- **Notes**: 涂地计分仍由 host 权威（各端本地网格仅用于呈现与本地预测，结算以 `res` 为准）。

## Task 24: 极简大厅 UI
- **Status**: `pending`
- **Priority**: medium
- **Depends On**: Task 20
- **Description**:
  - `inkwave::ui::online`（bevy_ui）：主菜单加"Online"入口；创建房间（显示 5 位码）/输入码加入；名册列表（名字/队/ready/host/ping）；切队（0/1/auto）与 ready；host 设置最小子集（duration/difficulty/bots）与 start（全员 ready 才亮）；错误/断线提示；`starting`→载入对局、结算→回大厅。接 Task 20 会话事件流；无 showcase 场景。
- **Acceptance Criteria Addressed**: AC-M2-6（UI 层）
- **Test Requirements**:
  - `rule` TR-24.1: UI 状态→会话动作映射单测（可测部分抽纯函数；按钮禁用逻辑随状态）；证据为测试输出
  - `rule` TR-24.2: 双窗口原生人工试玩记录（create/join/ready/start/回大厅）；证据为操作记录
  - `rule` TR-24.3: 全门禁（含 trunk wasm 构建）+ JS 树零改动；证据为命令输出

## Task 25: 端到端集成——Rust↔Rust 完整对局
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 18, 19, 20, 21, 22, 23
- **Description**:
  - harness 组装全栈：shim + ≥3 无头客户端（1 host + bot 补位至 8、缩短时长）跑完整局；断言各端涂地覆盖率一致（差 < 0.5%）、结果与 host `res` 一致、无 panic；lag/jitter 注入复跑；host 迁移/掉线接管场景复跑（对局中）。
  - 输出逐端一致性报告（供 AC-M2-2/3/4 证据）。
- **Acceptance Criteria Addressed**: AC-M2-2、AC-M2-3（量化）、AC-M2-4（集成级）
- **Test Requirements**:
  - `rule` TR-25.1: 集成测试：3 客户端完整局通过（覆盖率一致性断言 + 结算一致）；证据为 harness 报告
  - `rule` TR-25.2: 集成测试：netlag=40/netjitter=30/netspike=0.01 场景复跑通过 + kink/path-error p95 统计输出（对照 JS net-test 口径写报告）；证据为统计
  - `rule` TR-25.3: 集成测试：对局中 host 掉线迁移与玩家掉线转 bot 两场景打完出结算；证据为测试输出
  - `rule` TR-25.4: 全量门禁；证据为命令输出
- **Notes**: TR-25.2 的数值门限按 spec Open Question 3：先产出 Rust 统计，独立审查 agent 对照 JS net-test 语义定标。

## Task 26: 双端联网冒烟与跨端握手
- **Status**: `pending`
- **Priority**: medium
- **Depends On**: Task 24, Task 25
- **Description**:
  - WASM 浏览器客户端 ↔ 原生客户端经 shim（或 wrangler dev，若可达）组房开一局：冒烟脚本沿用 M1 chrome-headless 方案 + 网络断言（互见角色移动/喷涂、console 无 error）；若沙箱限制跑不通浏览器 WS 对局，降级为代码评审 + 用户机器复核清单条目（如实记录）。
  - 跨端握手（尽力项）：JS headless 客户端（tools/net-test.mjs 思路，只读）与 Rust 同房，验证 welcome/join/tick 互收不报 err；环境不可达则标记"待用户机器复核"。
- **Acceptance Criteria Addressed**: AC-M2-5、AC-M2-8
- **Test Requirements**:
  - `rule` TR-26.1: 冒烟脚本输出（含渲染错误检测，沿用 M1 B-1 修复后的探针）或降级记录；证据为日志/截图
  - `rule` TR-26.2: 跨端握手日志或"待复核"登记（并入 REVIEW_CHECKLIST）；证据为记录
  - `rule` TR-26.3: 全量门禁；证据为命令输出

## Task 27: M2 收尾——门禁、台账、文档与总评审
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 26
- **Description**:
  - 全量门禁矩阵（同 M1 Task 16 口径）；SYNC_LEDGER/PORT_MAP 更新（net 各行 M2 状态、新依赖登记、fixture/harness/shim 条目）；README 增"联网运行"章节（shim/wrangler/双端连房命令）；GPU 复核清单追加 M2 条目（远端角色平滑、跨端握手）；独立审查 agent 总评审 M2 全部产出并闭环。
- **Acceptance Criteria Addressed**: AC-M2-7
- **Test Requirements**:
  - `rule` TR-27.1: 全部门禁命令输出（测试总数 > 147、`cargo tree -p inkwave_sim` 仍仅 glam/serde/serde_json、trunk release 产物、原生 headless 冒烟、JS 树/server/ 零改动）；证据为日志
  - `rule` TR-27.2: 文档逐条命令实际执行验证（README 联网章节）；证据为输出
  - `rule` TR-27.3: 独立审查 agent 无未闭环 Blocker/Major；证据为评审记录
- **Notes**: 完成后提交 M2 里程碑并 push origin/main（沿用 M1 提交流程与身份环境变量）。

---

## Review 附录（逐任务追加）

（实施时按 M1 格式：独立审查发现 → 修复 → 复审结论）
