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
- **Status**: `completed`
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
- **Completion Evidence**:
  - `rust/tools/relay-shim/`（lib+bin，TEST-ONLY 声明置顶）：`Shim::spawn(ShimConfig)` 起 accept+sweep 双线程，per-connection 线程持 tungstenite blocking socket + mpsc 出站队列。fail 阶梯按 server L70-74 逐行（v≠PROTO→refresh 文案 / create&&有人→"Room code taken" / !create&&无人→"Room not found" / ≥room_max→"Room is full" / locked&&有人→"Match in progress"），welcome roster 含己 + host=all[0]（L81-82），`b|`→`m|` 排他 fan-out（L113）、`s|to|` 含发送者（L120），`{"t":"ping","c":n}`→pong（L125，缺 c 输出 `{t:'pong'}` 对齐 JS 字符串化），lock 仅 host（match guard），裸 `"ping"`→`"pong"` 不触房间态（L52），`_gone` leave 广播 + 重选举 + 空房解锁（L133-142），ORIGIN_OK 手编译双正则（L22-23），CODE 正则（L28），MSG_MAX 丢弃（L105）、RATE/BURST_STRIKES→close(4008)（L106-109）、静默 sweep→close(4001,'Connection timed out')（L94）——阈值全部 `ShimConfig` 参数化，`relaxed()` 供快测试。已知偏差文档化：session id 确定性计数器（同 4 位 base36 形态）、name 清洗按 Unicode 标量（JS 为 UTF-16 码元）、liveness 单时间戳（JS 三路 max）。
  - `rust/tools/net-harness/`（lib+bin+tests，TEST-ONLY）：双模式多客户端驱动器。**fake 模式**=FakeRelay/FakeTransport 虚拟时钟 + seeded LinkSim（`?netlag/&netjitter/&netspike` 语义），4 端 join→broadcast→send_to→lock→stranger 拒→离房重选举，6 项一致性 checks（roster/host 全一致、payload 到达顺序、stranger 拒且无载荷、host 保最老成员、locked 拒 join 可观测、RTT 与 sim 匹配）。**shim 模式**=真实 tungstenite socket 打 relay-shim：4 个 RawClient 直接说 PROTO v1 wire + 1 个 ProdClient 走生产 `RxState::handle` 帧语义路径（含 2s 心跳 + RTT EMA），7 项 checks（RxState client 与 raw 一致、广播顺序 pin、host 迁移、ping→pong、RTT 区间）。`cargo run -p net-harness [fake|shim|all]` 打印一致性报告，不一致 exit 1。
  - TR-19.1 ✅：`cargo test -p relay-shim` **8 passed**——3 socket welcome 成员表/host 选举、`b|` 排他回环、`s|` 定向含己、leave 后 host 迁移、lock 后 join 拒 "Match in progress"、fail 阶梯（taken/not found/full）、ping/pong、MSG_MAX 丢弃、RATE 4008 切断、静默 sweep 4001。
  - TR-19.2 ✅：`native_transport_end_to_end_against_relay_shim`（inkwave dev-test，relay-shim 经 `[target.'cfg(not(target_arch = "wasm32"))'.dev-dependencies]` 门控，不入 app/wasm 构建）——真实 NativeTransport 走 shim：A create welcome（solo host）→B join 双端 welcome/join 互见→`b|` 排他→`s|` 定向→lock→leave 迁移→close，全流程 Origin 合规 + 帧语义正确。
  - TR-19.3 ✅（审查修复后重跑）：fmt `--check` ✅、`clippy --workspace --all-targets -- -D warnings` 0 错 ✅、`cargo test --workspace` **194 passed**（42 inkwave + 140 sim + 8 shim + 4 harness acceptance）✅、wasm32 check（`-p inkwave -p inkwave_sim`，shim/harness 为 native-only 测试 crate 不参与 wasm 门禁）✅、`trunk build --release` exit 0 ✅、headless 冒烟（`--headless` boot+complete）✅、`cargo tree -p inkwave_sim --depth 1` 仅 glam/serde/serde_json（sim 依赖面不变）✅、JS 树（src/server/tools/docs）`git status` 零改动 ✅。
  - harness acceptance（`tests/acceptance.rs` 4 passed）：fake 无 lag 全端一致 + RTT 全 0；fake lag/jitter/spike 全端一致 + 同种子确定性回放（clients/rtts 逐值相等）+ RTT≥lag；shim 模式全端一致；心跳端到端（`heartbeat_samples`：None 300 tick RTT 全 0 且 bytes≥8 证明心跳 pong 路径活；lag50 前 5 tick 0、[10..] 全 50±1e-9，pong 经 SIM 单向延迟）。
  - **Notes 交叉验证 ✅**：沙箱 `npx wrangler dev`（4.145.0，本地 DO）可达——对**真实 server/**复跑 TR-19.1 场景 8/8 全绿（welcome 成员表/选举/`b|` 排他/`s|` 含己/lock 拒 join/leave 迁移 host 保最老/A 走后迁 C），shim 语义与 Worker 行为交叉核对通过。
  - **Task 18 备案 Minor 落地**：① fake `part()` 空房清 `locked`（对齐 server L141，经 harness fake 模式 "locked refuses join" probe-join 检查可观测）；② FakeRelay 实现 `{"t":"ping","c"}`→pong（经 RxState 路径端到端断言 `heartbeat_samples`）；⑥ SIM 双 rand 顺序（jitter 先、spike 后）钉死单测 `sim_draws_jitter_then_spike_in_that_order`（replay xorshift 逐值对照）。
  - **⚠ N-1 回归记录（诚实备案）**：Task 18 审查轮声称修复的 io_loop `try_recv` 探测（消费后丢弃 `Cmd::Connect` 的 flaky 源）**未进入 ddbe7e2 提交**——`git show ddbe7e2` 复核 L172 仍为 `if cmd_rx.try_recv().is_err() { sleep }`。本窗口随 Task 19 工作树真正落盘修复（改为无条件 sleep，注释点明命令由循环顶部统一 drain），目标测试连跑 10 次 0 失败，随本提交入仓。教训：审查修复必须 `git show HEAD` 复核落盘，不能只看工作树。
- **Review 修复附录**（独立审查：1 BLOCKER + 1 MAJOR + 5 MINOR，结论"有条件可合入"；修复后 resume 复审逐项核对磁盘代码，结论"可合入"）:
  - **B-1 ✅**（BLOCKER）：relay-shim `qs_decode` 合并链式 if 后未重跑 fmt（`lib.rs:543` 一处 diff）→ `cargo fmt --all`，`--check` 通过。
  - **M-1 ✅**（MAJOR）：`RateWindow::tick` 硬编码 `strikes >= 4`、`new` 未接收 `cfg.burst_strikes`——BURST_STRIKES 配置静默失效 → 增 `burst: u32` 字段，调用点传 `cfg.burst_strikes`，判定改 `self.strikes >= self.burst`（relaxed 测试走 3×RATE 路径不受影响，8 个 shim 测试仍全绿）。
  - **N-1' ✅**：`rand01` 的 `pub` 注释理由（"harness 用"）虚假——harness 实际未引用，钉序测试在 transport.rs 自身 cfg(test) 经同文件访问 → 改回私有 `fn rand01`，删虚假注释。
  - **N-2 ✅（备案）**：`origin_ok` 用 `parse::<u8>()` 接受前导零（`http://172.016.0.1` 放行、JS 正则拒绝）——仅 dev origin 形态、shim 不门控生产流量 → 文档注释记录偏差，不修。
  - **N-3 ✅（备案）**：并发 join 的 fail 阶梯检查与成员插入非原子（现不可观测，单 accept 线程串行 join 路径无竞态）→ 不阻塞，若后续多进程 harness 暴露再修。
  - **N-4 ✅**：net-harness `prod.pump(120)` 轻微 flaky 风险与 `let _ = i;` 噪音行 → 删噪音行；pump 窗口经 194 全绿 + 连跑验证。
  - **N-5 ✅（备案）**：sweep 线程 `sweep_ms.min(1_000)` 钳制未注明 → `ShimConfig.sweep_ms` 字段文档补记。

## Task 20: 会话状态机（NetSession）
- **Status**: `completed`
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
- **Completion Evidence**:
  - `inkwave_sim::net::session`（session.rs ~3000 行 = 实现 ~1950 + 测试 ~1080）：`NetSession<T: Transport, F: FnMut() -> T>` 泛型工厂（transport trait 注入，fake 可测）；状态机 `Offline/Connecting/Lobby/Starting/Match/Error` 逐行对齐 session.js——create 重试循环（L71-79：≤4 次、仅 "Room code taken" 重 mint、其余 `break` 即 `_fail`，`tries` 计数在 `Pending.tries`）、join 清洗（L82-83：大写 + 滤非字母数字 + `<4` 本地拒 "Room not found" 不建 transport）、`_connect`（L87-112：先 `leave(true)`、welcome 后 host 建己 + `_fixTeams`、guest 发 `me` 帧、`_pushLobby`）、leave/_fail/_closed（L114-142：`bye` 静默 offline、其余 "Lost connection to the room"、in-match 无条件 `Match{Aborted}` 对齐 L141）、`_control` join/leave（L145-166：host 加 `_newPlayer`+`_fixTeams`+广播；leave 摘除 + host 迁移 `host` 字段 + `p.host` 重标 + `Leave` 事件 + guest 侧也 `_pushLobby`）、`_newPlayer`/`_fixTeams`（L168-179 逐行：显式队 ≤4 超降 auto、auto 落 `count[0]<=count[1]?0:1` 即时更新）、`_wireLobby`/`_broadcastLobby`/`_pushLobby`（L181-194：10 键字段集 + 本地 you/host 标记）、`setMe`/`_applyMe`（L197-233：`!= null` 守卫=Option、`!!ready` 真值经 `js_truthy_opt`、`if (o.name)` 空串跳过、team 名额按"其他人在该队数" `n<4` 判定、ping `Math.round`）、`setSettings`（L235-251：map/time/duration/difficulty/palette/mode 校验、`if (s.duration)` 真值测试（0/NaN 跳过）、`| 0` 向零截断 + clamp 60..600、noBots 强制 bots off + `_botsPref` 恢复）、`canStart`/`startBlock`（L253-258：cargo 单人 "Needs 2+ players — no bots on this stage"、缺队 "Needs a player on each team"）、`start()`（L267-297：`bots = l.bots && !mapNoBots`、roster nid 先 0 队后 1 队、**每队独立洗武器**（L277）、bot 去重 `used`+`weapons[slot%len]` 兜底、sub/special 半概率、`lock(true)`+广播+`_begin`）、`_begin`/`_markReady`/`_go`/`_launch`（L299-341：`_ready` 集 + 加载期掉线视为 ready（L324）+ 全员齐即 go、否则 12s 兜底 `go_t` 单次设置、guest 收 `go` 且 state==Starting 才 `_launch`）、`endMatch`（L344-352：host `lock(false)`+清 ready+广播、回 Lobby+`Match{End}`）、`_message`（L355-378：lobby 仅 from==host 才应用 + palette `Number.isInteger` + 新玩家 `Join` 事件、me 仅 host、emote、start 仅 host+Lobby、ready 仅 host+cfg.id 匹配、go 仅 from==host+Starting、default→`GameMessage` 透传 Task 21）、`update`（L381-391：transport 事件 drain——Error 后 `break` 丢弃同批 Closed（transport.js L75/L108 nulled onclose 语义）、2s ping（host 改己 ping 仅变化时广播、guest `sendTo(host,{k:'me',ping})`）、`match?.update` 接缝）。
  - **M2 偏差（模块文档 L40-60 注明）**：① turf-only——host `setSettings` 拒 zones/boss mode、guest 收非 turf start 帧 `fail("Unsupported game mode")`+leave；② `style` 透传不解释、本地建员 style=null（JS `randomStyle()`）；③ bot 名/武器洗牌 = 种子 Fisher-Yates（JS `sort(()=>Math.random()-0.5)` 非均匀），RNG `seed|1` xorshift64* 确定性（NFR-M2-3）；④ `_begin` 的 `launchLobby`/`startNetMatch` await 收口为 `Match{Start}` 事件 + `mark_loaded()` 同步接缝（Task 24 应用层）；⑤ `match?.onMessage/onLeave/netMatchAborted` 收口为 `GameMessage`/`Leave`/`Match{Aborted}` 事件（Task 21）；⑥ `js_slice16` 按 Unicode 标量（JS UTF-16 码元，仅 astral 名字有差，Rust↔Rust 两端同实现）。
  - **JS 弱类型语义映射（本任务核心）**：`x != null`→`Option`（None=键缺席跳过）；`!!x`→`js_truthy_opt`（Num 0/NaN=false、Str ""=false、Arr/Obj=true）；`if (o.name)`→空串跳过；`+s.duration | 0`→`d as i64`（向零截断）；`Math.round`→`wire::js_round`（floor(x+0.5)）；`team === 0` 严格数字比较→`TeamSel::Zero`（`from_jval` 仅 0.0/1.0，其余归 Auto——JS 保留非法值是缺陷，Rust↔Rust 不可达，注释备案）；负 palette 经 `as usize` 饱和为 0（JS 保留原值，仅恶意 host 可达，注释备案）。
  - **wire 编码器**（`encode_me`/`encode_lobby`/`encode_start`/`encode_ready`/`encode_go`/`encode_emote`）：手工 `JVal` 键序逐字面对齐 JS 字面量；`encode_me(&MeChange)`——None=键缺席、`Some(None)`=显式 null（sub/special）；`jobj` 辅助（&str 键→String）。
  - TR-20.1 ✅：`cargo test -p inkwave_sim net::session` **23 passed**——create 码表合法性/重试（重放 rand01 预测 first/second 码 + squatter 腾位后第二码成功）/**重试耗尽 4 次全 taken→Error "Room code taken"**（squatter 保持 open，FakeRelay 单房间使每次 mint 必撞）、join 清洗（"ab!"<4 本地拒无 transport、" ab-12 "→AB12 走 relay）、join 未知房 "Room not found"、join 满房 **"Room is full"**（8 raw filler 占满 ROOM_MAX）、join 锁定房 "Match in progress"、leave 复位 + 对端可见、`simulate_closed` 断线 "Lost connection to the room"、host 迁移（leave 帧 host 字段 + `Host` 事件）。
  - TR-20.2 ✅：guest `me` 帧往返 host lobby 更新、host `set_me` 拒未知武器、`fix_teams` 8 人满房（5 显式 Zero→e/f/g/h 降 auto 后 4/4、auto 交替落边）、`set_settings` 钳制（60/600、120.7→120 截断、坏 map/time/difficulty/palette 忽略、zones mode 拒、cargo 强制 bots off + pref 恢复、guest 无权限）、`_broadcastLobby` 字段集、2s ping 流转（lag=40 → 双端 ping==40，pong 经 SIM 单向延迟 rtt=lag）。
  - TR-20.3 ✅：`start_ready_go_full_sequence`（can_start→start→mark_loaded→ready→go→Match 全时序、roster nid/team/slot/bot owner/武器去重逐字段、guest cfg==host cfg、lock 拒 late join、end_match 解锁回 lobby）、`go_fallback_fires_at_12s`（11_990ms 不触发 + 越界触发）、`member_dropped_while_loading_counts_as_ready`（L324）、`guest_refuses_a_non_turf_start`（fixture start 改 mode=zones → Error + Offline）、`wire_encoders_are_byte_exact_against_fixtures`（me/ready/go/emote/lobby/lobby2/start/start2 与 `net-fixtures.json` `fixtures.msgs.*` 逐字节）、`no_bots_stage_gates_the_start`（cargo 双人放行无 bot）+ **`solo_on_a_no_bots_stage_blocks_the_start`**（cargo 单人 startBlock 文案 + can_start/start 双拒 + 无 cfg 无锁）。
  - TR-20.4 ✅（审查修复后重跑）：`cargo fmt --all --check` ✅、`clippy --workspace --all-targets -- -D warnings` 0 错 ✅、`cargo test --workspace` **217 passed**（42 inkwave + 163 sim（session 23）+ 8 shim + 4 harness）✅、wasm32 check（`-p inkwave -p inkwave_sim`，shim/harness 为 native-only 测试 crate 不参与 wasm 门禁，Task 19 口径）✅、`trunk build --release` exit 0 ✅、headless 冒烟（`--headless --frames 120` boot+complete）✅、`cargo tree -p inkwave_sim --depth 1` 仅 glam/serde/serde_json（sim 依赖面不变）✅、JS 树（src/server/tools/docs）`git status` 零改动 ✅。
  - 配套改动面（git status）：`net.rs` +1 行模块声明、`transport.rs` 小改（`rand01` 提 `pub(crate)`+注释如实、`FakeRelay` 加 `Clone`、新增 `simulate_closed(reason)` 测试钩子：push `Closed{reason}` + 断链）、`session.rs` 新增；`Profile` 全 pub 字段 + `set_profile` setter（JS 每次 `_connect` 重读 `G.game.profile` L95 的 Rust 接缝，Task 24 应用层用）。
- **Review 修复附录**（独立审查：0 blocker / 1 major / 10 minor，结论"有条件可合入"；修复后 resume 复审逐项核对磁盘代码，结论"可合入"）:
  - **M-1 ✅**（MAJOR）：`start()` 武器洗牌在队循环外只洗一次（JS L277 每队独立洗）→ 移入 `for team` 内，两队 bot 武器集独立。
  - **N-1 ✅**：guest lobby 帧 palette 多加 `p >= 0.0` 下界（JS L363 裸 `Number.isInteger`）→ 删下界；复审指出负值 `as usize` 饱和为 0 与 JS 保留原值有差 → 注释如实备案（仅恶意 host 可达）。
  - **N-2 ✅**：`set_me`/`_applyMe` 空名未跳过（JS `if (o.name)` L219 falsy）→ `apply_me` name 加 `!s.is_empty()` 过滤。
  - **N-3 ✅**：`ready` 非 bool 值一律视为 true（JS `!!0===false`）→ 新增 `js_truthy_opt`（Null→None/Bool→值/Num 非零非 NaN/Str 非空/Arr,Obj→true）接入 `apply_me_jval`。
  - **N-4 ✅**：`_closed` bye 时漏发 Aborted（JS L141 无条件 `netMatchAborted(this.error)`）→ 条件改 `if in_match`。
  - **N-5 ✅**：`TeamSel::from_jval` 注释失真（JS 保留非法值而非归 auto）→ 改写为如实偏差备案（Rust↔Rust 不可达）。
  - **N-6 ✅（驳回，复审确认）**：create/join 入口 `slice16` vs JS 仅 `_newPlayer` 截断——Rust↔Rust 两端同实现，lobby/wire 逐字节一致；差异仅 relay 可见名/members 回退名，属跨端互操作（M3）范畴。
  - **N-7 ✅**：`set_settings` duration=0 被钳成 60（JS `if (s.duration)` 跳过）→ `filter(|d| *d != 0.0 && !d.is_nan())`。
  - **N-8 ✅**：Profile 无连接后更新接缝（复审驳回"字段全 pub 即可"——`NetSession.profile` 私有）→ 补 `set_profile()`，注释对齐 JS L95 每次重读语义。
  - **N-9 ✅**：测试覆盖缺口——create 重试耗尽、"Room is full"、startBlock 阻塞分支 → 新增 3 测试（`create_exhausts_four_retries_and_fails`/`join_a_full_room_gets_room_is_full`/`solo_on_a_no_bots_stage_blocks_the_start`），23 passed。
  - **N-10 ✅**：`decode_lobby` 的 `palette: 0` 死值易误读 → 注释点明占位、调用方另行按整数校验覆盖。

## Task 21: 权威切分与 host 职责（replication）
- **Status**: `completed`
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
- **Completion Evidence**:
  - 交付：`inkwave_sim::net::replication`（新文件 ~1500 行，实现+11 测试）；接缝最小改动：`match_.rs` `follower` 门控（Active→Finish/Finish→judge 仅 `!follower`，JS L216）+ `HitSend` 事件转发 + `remove_actor`（JS `_remove` L686-690）+ `step_masked(remote: Option<&[bool]>)`（proxy 跳 `Actor::step`/`WeaponRunner::update`，JS `a.remote` L110）；`actor.rs` `set_visual_y`（proxy 落样本写 pos.y+smoothY 合成值）+ `net_tp` 重生自增；`weapon.rs` `SimEvent::HitSend`（victim remote→发帧不扣血，JS sendHit L163-167）；`net.rs`/`wire.rs` `HostMsg::State.t: Option<f32>`（JS `_hostState` L647 typeof 守卫）。
  - TR-21.1 ✅：`cargo test -p inkwave_sim net::replication` **11 passed**——owner/proxy 划分：proxy 不受本地输入影响（drive_bots 只喂 owned）、owner actor 进 tick `a` 且 nid 路由（`tr21_1` 取 `find("t")` 帧断言 actors 集/`c` 快照 0.5s 周期/`ts` 单调）、soft push 掩码权重（`soft_push_mask_weights_follow_js_ternary`，JS match.js L239 嵌套三元逐字：双 remote ka=kb=0、local-vs-proxy 全量吸收）。
  - TR-21.2 ✅：`tr21_2` hit 通道端到端（FakeRelay 双端真实 id）：host 射中 guest actor→`{k:'hit'}`（`hit_wire_shape_matches_js` 断言 v/a/d(r2)/w:'shot' 形状）→victim owner 即时扣血→死亡→tp++→重生样本广播对端可见；`st_frame_without_t_leaves_the_clock_alone`（缺 t 帧不清 guest 时钟）。
  - TR-21.3 ✅：`tr21_3_host_migration`（host leave→最老 guest 接任：`follower=false`、clock_t 归零重发 `c`、bot/判定接管、nid 所有权映射不变、对局打完出结算）；`tr21_3_no_bots_stage_removes_leavers`（非 host 掉线：普通图 actor 原地转 bot 继续跑；noBots 图 `_remove` 剪除+平行数组移位）；`no_bots_bind_prunes_leavers`（JS bind L79 加载期掉线者 bind 时剪除，bot 图不动）；`apply_sample_writes_invuln_window_and_turf_ratchet`（JS applyRemote L377-378：invuln 位→0.1s 窗口、turf 只增）。
  - TR-21.4 ✅（审查修复后重跑）：`cargo fmt --all --check` ✅、`clippy --workspace --all-targets -- -D warnings` 0 错 ✅、`cargo test --workspace` **229 passed**（42 inkwave + 175 sim（replication 11）+ 8 shim + 4 harness）✅、wasm32 check（`-p inkwave -p inkwave_sim` 口径）✅、`trunk build --release` exit 0 ✅、headless 冒烟（`--headless --frames 120` boot+complete）✅、`cargo tree -p inkwave_sim --depth 1` 仅 glam/serde/serde_json ✅、JS 树（src/server/tools/docs）`git status` 零改动 ✅。
- **Review 修复附录**（独立审查：0 blocker / 2 major / 3 minor / 2 nit，结论"有条件可合入"；修复后 resume 复审逐条读盘核对，结论 **6/6 PASS、无新问题、可合入**）:
  - **M-1 ✅**（MAJOR）：`soft_push_masked` 双 remote 权重误用 0.5/0.5（JS match.js L239 嵌套三元 `ka = a.remote ? 0 : b.remote ? 1 : 0.5`）→ 逐字对齐，双 proxy 谁也不动；补回归测试。
  - **M-2 ✅**（MAJOR）：JS `bind` L79 的 noBots 加载期掉线清理缺失 → 新增 `prune_loading_leavers(m, members)`（app 传 session 成员表；no_bots 门控、own 永不剪、高→低移位），`on_leave` 删除路径重构共用 `remove_at`；补测试。
  - **M-3 ✅（驳回，复审确认）**：审查称 `_hit` 缺 attacker.remote→'drop' 守卫——核对 JS：`_hit` L633 置 `_applyingHit=true` 后调 applyHit，`shouldApplyHit` L713 `if (this._applyingHit) return 'local'` 在 L714 attacker.remote 检查**之前**短路，JS 接收侧对 remote attacker 同样扣血——Rust 不查 attacker.remote 与 JS 一致，不改。
  - **M-4 ✅**：`_hostState` 的 `t` 缺 typeof 守卫（JS L647）会致缺字段 st 帧清零 guest 时钟 → `HostMsg::State.t` 改 `Option<f32>`（encode Some 才写 "t" 键、decode 缺键 None、host_state `if let Some` 守卫）；golden fixture st/st_finish 均带 t，`host_messages_round_trip_to_golden_wire` 字节往返不变；补测试。
  - **M-5 ✅**：`applyRemote` L377-378 的 invuln 0.1s 窗口与 turf `Math.max` 棘轮未落地（原拟挂 Task 22）→ `apply_sample` 直接实现（proxy 不跑 `Actor::step`，无其他递减方）；补测试。
  - **N-6 ✅（偏差声明）**：JS `net.tp` 初值 -1（L85）/`netTp = net.tp || 0`（L702）vs Rust `net_tp` 初值 0——计数仅 Task 22 路径切断消费，Rust↔Rust 两端一致；记入模块文档。
  - **N-7 ✅（偏差声明）**：JS 弹道命中帧内联 sendHit，Rust HitSend 经事件总线排到下一次 `update()`，最多晚一帧（1/60s）；记入模块文档。

## Task 22: 播放层（时钟 + Hermite + correction）
- **Status**: `completed`
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
- **Completion Evidence**:
  - 交付：`inkwave_sim::net::playback`（新文件 1080 行，实现+11 测试）——`on_tick`（JS `_tick` L229-264：off 最快包地板+0.0025 爬移、late/gap 60 窗、want=clamp(0.07,0.3,q95(gap)+q90(late)+0.012)）；`advance_all`（JS `_advance` L271-284：delay 快加×2 慢减×0.35 eased、|target−tr|>0.5 硬同步、±8%/−20/+25% 驶向、ahead<0.015 降至 0.5×）；`push_sample`（L249-253 单调 t 守卫+40 容量尾裁）；`path_at`（JS `_pathAt` L288-303 + `hermite` L766-786：C1 位置用 owner 速度切量、Y 地板钳制、外推 ≤0.18s 弹道、tp 不等→Hold、Early=早于最旧）；`sample`（JS `_sample` L305-346：correction 每帧 prevT 重评、1e-6<j2<16 累加、≥16 清零、临界阻尼 ω=13、handoff err+=prevRaw−S）；`apply`（JS `applyRemote` L349-429 写序：ready 边沿状态机→!ready→!alive respawn_timer→pos=S+err→vel/yaw/aim→离散 flags（GT2 先判，L371 逐字）→hp/ink→invuln 0.1 窗口→turf 棘轮→spawn_pending 隐藏）。接缝改造：`replication.rs` 收包路由进 `Playback`（`on_tick`/`push_sample`/`push_events`），每帧 `advance_all`→`sample`→`apply`（JS 帧序 now→advance→sample→发送→apply）；`on_leave` 增 sim/t 参数走 adopt 语义；`apply_sample` 测试迁移为 `tr22_apply_writes_invuln_window_and_turf_ratchet`。时间算术全 f64，wire t/ts 保持 f32（r3 量化）。
  - TR-22.1 ✅：`tr22_1_hermite_is_c1_and_exact_through_owner_tangents`——恒速合成序列 9 个内插点误差 <1e-3（2cm 门限的 1/20）、knot 两侧连续无角点、hp u≥0.5 侧切换（L784）；`tr22_quantile_matches_js`（L761-764 floor 索引）。
  - TR-22.2 ✅：`tr22_2_delay_stays_in_the_85_to_300ms_band`——稳态迟到 20ms 收敛 0.07 下限；单尖峰在 60 窗内时（i=150）q90 忽略之 delay 仍 ≈0.07（审查 M-4 增强：钉住 q90 双侧），20% 持续迟到 q90 命中尖峰钳到 0.3 上限；`tr22_2_extrapolation_capped_at_180ms`（L297 e≤0.18）；`tr22_1_tp_jump_cuts_the_path`（L291-292 tp 不等→Hold 不滑行）；`tr22_buffer_capacity_and_monotonic_guard`（40 容量+等 t 拒收）。
  - TR-22.3 ✅：`tr22_3_correction_settles_without_a_pop`——中途改写已播样本，帧间 |Δpos| 有界（无 pop）且渐近收敛新路径（err/err_v 清零判据 L339-341）；`tr22_3_hermite_y_never_through_a_floor`（两端 grounded 时 y 钳进 [min−0.02, max+0.02]，L776）；`tr22_handoff_and_adopt_reset_the_path`（handoff err+=prevRaw−S、>16 清零，adopt 续 tp）。
  - TR-22.4 ✅：`cargo fmt --all --check` ✅、`clippy --workspace --all-targets -- -D warnings` 0 错 ✅、`cargo test --workspace` **239 passed**（42 inkwave + 185 sim（playback 11）+ 8 shim + 4 harness；基线 229→239 净增 10）✅、wasm32 check ✅、`trunk build --release` exit 0 ✅、headless 冒烟 ✅、`cargo tree -p inkwave_sim --depth 1` 仅 glam/serde/serde_json ✅、JS 树（src/server/tools/docs）零改动 ✅。NFR-M2-2 开销断言：`cargo test --release -p inkwave_sim nfr_m2_2 -- --ignored --nocapture` 三臂 bench（bare=step_masked only / playback=时钟+路径+apply 手驱 20Hz 无 JSON / full=整个 Replication overlay 含 tick 编解码仅报告），5 次交错取 min p50 抗调度噪声，shown 计数断言证明 proxy 真实插值——本机 bare p50≈11.8µs、+playback≈12.9µs（**+9.4% < 20% 门限** ✅）、+full≈16.7µs（含 Task 21 JSON 编解码，不在本任务门限内）。
- **Review 修复附录**（独立审查：0 blocker / 0 major / 4 minor / 4 nit，结论"可合入"；修复后 resume 复审读盘核对，结论 **4/4 PASS、无新问题、可合入**）:
  - **M-1 ✅**：`apply` 的 ground_team 判序 Rust 先 GT1——JS L371 `f & F.gt2 ? 2 : f & F.gt1 ? 1 : 0` 先查 GT2（双置位损坏帧 JS 报 2）→ 改先 GT2，附逐字注释。
  - **M-2 ✅**：偏差声明 #2 原"同一时刻"论证不属实。首轮修正误称 'splatted' 走攻击者时钟——复审纠正：JS actor.js L233 受害者 emit、netmatch.js L140 `e.actor || e.victim` 归受害者 owner，事件与死亡样本**同钟**；真实差异是事件按精确 `r3(now())` 打戳、样本按 tick 边界打包，可差 ≤1 tick(50ms)，'respawn' 同理。终版措辞 "approximately"+该论证，并补 owner 两样本间隙死而复生时 JS 短暂隐藏 vs Rust 直接 snap 的次生差异；Task 23 事件时间线关闭两缺口。
  - **M-3 ✅**：JS L414 `landT`/`lastDamage` 每帧递增未移植且未声明 → 并入偏差声明 #3：`last_damage` 为 Actor 私有字段，读者（step/damage 内 regen/归因、bot getter）全部只跑本地 owner actor，冻结 proxy 不执行。复审指出首轮 "every sim reader" 过宽（bot.rs L350/L889 经 pub getter 也读）→ 终版措辞显式含 bot getters。
  - **M-4 ✅**：`tr22_2` 尖峰用例 i=100 的尖峰到 i=200 断言时已滑出 60 窗，没真正钉住 q90 → 增强：i==150（尖峰仍在窗内）断言 delay≈0.07（q90 忽略单离群值）+ 新增 pb3（20% 迟到 → q90 命中 → 钳 0.3 上限），双侧钉住 q90 语义。
  - **N-1~N-4 ✅（核对无误留档）**：`snap_now=false` 死赋值（JS L326 同为死代码，唯一读者 L329 被 handoff 排除）已删并注释声明；quantile NaN 路径、`advance_all` 空 peers、`apply` 越界 idx 均核对与 JS 行为一致，无需改。

## Task 23: 事件时间线重放（墨汁/ghost/触发）
- **Status**: `completed`
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
- **Completion Evidence**:
  - **消费侧（`replication.rs::play_events` / `play_event`，JS `_playEvents`/`_play`/`_playEvent` L447-619）**：每帧在 sample 之后、tick 发送之前（JS L179 帧序）排空 `Playback::due_events`（`ts <= p.tr` 逐 peer 有序 drain）。`'s'`→还原 SplatOpts（stretch 三分量非零判据 L468-471、`e[8]` kind 名查 `SplatKind::from_name`、`stretchAmt` 随 stretch）后 `paint.splat`，seeded 形状各端一致；`'p'`→ghost `Projectile`（`damage:0`、`ghost:true`、seed 取本地 rng、trail -1.5/0.45，JS `ghostProjectile` L867-875；无 remote 守卫，对齐 L476 仅 byNid 查找）直接入 `projectiles.list`；`'tr'`→消费即弃（JS L480 守卫只门控 `_netTrig`/`_trigSideEffects` 表现层，sim 无消费者、守卫结果无从触及 sim 状态，模块 docs 声明）；`'ev'`→`play_event`：`actor||victim`→nid→idx→`!remote` return→`splatted`→`Playback::remote_splat`（JS `_remoteSplat` L603-619 全字段：alive=false/hp=0/respawnTimer/deaths++/runner.reset/buf 清/ready/has/deathTp，+attacker 存在时 `stats.splats++` L613）/`respawn`→`remote_respawn`（L621-627）/其余 FX 名忽略。
  - **录制侧（`rec`/`record_ink`/`record_fire`/`record_splat_event`/`record_event`，bus drain 内）**：`_rec` 语义 = `r3(now)` 前缀入 `out`，随下一 tick `e` 发出。ink 流游标 `recorded_ink` 逐帧录 `SplatRecord::from_ink_splat`（JS `recSplat` L109-114）；**防回环 = `PaintGrid::applying` + seq 游标**（`play_events` 以 `applying` 包裹重放 `splat`（L467/L473 逐字对应），推送条目带 `replayed`/`seq` 标记；`record_ink` 在 tick 发送后只录 `!replayed` 且 `seq >` 游标条目——JS `applying` 标志 L110 的等价物，对渲染层 `drain_ink` 与 `Match::restart` 重建网格免疫，模块 docs 声明接线契约：drain 须在 update 之后）；单发三记录 `p`→`ev weapon:fire`→`tr shoot` 顺序逐字对齐 JS（recProj L116 / _onLocalEvent L139 / trigger L179）；`MatchEvent::Respawn` 本地先 `tr spawn` 后 `ev respawn`（actor.js L172-175 顺序）；`MatchEvent::Splat` 仅本地 victim 录 `ev splatted`（packEvent 键序 victim/attacker/cause，attacker None→键缺失）；`'tr hit'` 不录（character 表现层 seam，M2 范围外，模块 docs 声明）。
  - **ghost 判定守卫（`weapon.rs step_one`）**：JS mute 语义核实——paint.js L470 `mute>0 return 0` 在光栅化**之前**（不喷墨但照常耗 rng）、weapons.js L1608 `dmg>0` 才 applyHit、L1610 impact emit 无条件。Rust：body_hit 伤害块套 `dmg > 0.0` 守卫（Impact 事件在守卫外）；world-hit 与 drip 分支 `ghost → area=0`（跳过 `paint.splat`，rng 抖动照常消耗）；死亡/世界判定/raycast 照常。`SimEvent::Fire{owner_slot,team,muzzle,dir}` 经 `MatchEvent::Fire` 上总线供录制。
  - **死亡/重生事件驱动（关闭 Task 22 偏差声明 #2 两缺口）**：`Playback::apply` 删 ALIVE 边沿机（只剩 !ready/!alive respawn_timer/路径写入）；`remote_splat`/`remote_respawn` 由 timeline 事件成熟时驱动；owner 两样本间隙死而复生按 JS 同序处理。playback.rs 与 replication.rs 模块 docs 偏差声明改写为已关闭表述。
  - TR-23.1 ✅：`tr23_1_splat_replay_matches_grid_and_never_loops_back`——双端 FakeRelay，A 本地 splat（r2 干净圆心，量化无损）→ B 重放后 `paint.grid()` **字节级相等**；B 出向 tick 的 `e` 中 SplatRecord 计数恒 0（回环检查在路由前扫描 B→host 帧）。
  - TR-23.2 ✅：`tr23_2_ghost_round_never_damages`——单发命中：B 侧 ghost 圆存在（`ghost && damage==0`）且受害者 hp 只掉一次 d（容差带排除双次应用与 hp regen 噪声），两端圆到期消亡；`tr23_2_ghost_never_paints_and_splats_land_on_the_sender_clock`——抛射落地：host 先喷墨、B 的 ghost 全程不新增墨（B `ink_stream` 纯重放、长度与 host 相等），且 B 喷墨帧号晚于 host ≥3 帧（重放等 playback 时钟成熟而非包到即放）；splat 时刻=sender-clock 由 `due_events` 的 `ts<=tr` 门与同帧路径采样共享时钟保证（与 TR-22 时钟测试合证）。
  - TR-23.3 ✅：`cargo fmt --all --check` ✅、`clippy --workspace --all-targets -- -D warnings` 0 错 ✅、`cargo test --workspace` **242 passed**（42 inkwave + 188 sim（replication 新增 3）+ 8 shim + 4 harness；基线 239→242）✅、wasm32 check（`-p inkwave -p inkwave_sim` 口径）✅、`trunk build --release` exit 0 ✅、headless 冒烟（`--headless --frames 120` boot+complete）✅、`cargo tree -p inkwave_sim --depth 1` 仅 glam/serde/serde_json ✅、JS 树（src/server/tools/docs）`git status` 零改动 ✅。
- **Review 修复附录**（独立审查：1 major / 7 minor / 3 nit；修复后复审读盘核对逐项 OK，结论"可合入"、无必修项）:
  - **M-1 ✅（游标与 `drain_ink`/`Match::restart` 不兼容，潜伏集成缺陷）**：改为 JS `applying` 的逐字等价物——`InkSplat` 增 `seq`（`PaintGrid::ink_seq` 单调推送计数）+ `replayed` 标志，`play_events` 以 `PaintGrid::applying` 包裹重放 splat（L467/L473）；`record_ink` 按 `partition_point(seq <= 游标)` 定位、跳过 `replayed`、`seq < 游标` 时重置（restart 重建网格）。渲染层每帧 drain（ink_render.rs L218）不再丢本地喷墨；新回归测试 `regression_recording_survives_render_draining`（双端每帧 update 后 drain + guest 重播后再本地 splat，断言两轮网格收敛、零回环）判别住该修复；模块 docs 声明接线契约（drain 须在 update 之后）。
  - **Minor 1 ✅（帧序）**：`record_ink` 从 tick 发送前移到发送后，与 bus 录制同侧——本地记录一律搭下一 tick，对齐 JS `Match.update` 在 `NetMatch.update`（`_sendTick`）之后；update 帧序注释、模块 docs、方法 doc 三处统一。
  - **Minor 2 ✅（'tr' 注释失实）**：原注释称"守卫运行"但代码直接消费——更正为诚实表述（守卫只门控表现层消费者，sim 无消费、守卫结果不影响 sim 状态）。
  - **Minor 3 ✅（trigger 录制范围未声明）**：模块 docs 新增偏差声明：JS 录所有 `character.trigger`（L90-93），sim 仅可观察 `'shoot'`/`'spawn'` 两个有 gameplay 含义的 trigger，其余角色动画名在 M1 Spritzer sim 不存在、永不触发。
  - **Minor 4 ✅（`actor||victim` 回退不等价）**：JS unpack 先行——`actor` 键存在但 nid 未命中 → null → 回退 `victim`；Rust 改为 `resolve("actor").or_else(|| resolve("victim"))`（先解 idx 再回退），与 L576 等价。
  - **Minor 5 ✅（`splats++` 仅本地 attacker）**：删除 `!remote` 条件，JS L613 任意 attacker 均计数；复审核验三端各恰好一次（owner 机器走 step 击杀记功且事件不回灌、攻击者/第三方机器仅事件路径一次）。
  - **Minor 6 ✅（emit 表述 + 击杀确认未声明）**：`play_event` doc 更正（JS `splatted`/`respawn` 早退 L580-581 不达尾部 `emit` L600）；模块 docs 新增偏差声明：L618 本地击杀确认 `'hit'` emit 为 HUD/FX，sim 无消费者，记功本体已移植。
  - **Minor 7 ✅（ghost 晚一帧起步）**：模块 docs 新增偏差声明：Rust `Match::step` 在 `Replication::update` 之前（JS 相反），ghost 起步晚一帧、重放 splat 落格在本地步进之后；Rust↔Rust 两端同规则，网格一致性不受影响。
  - **Nit 备案（不修）**：`TimedEvent.ts` f32（r3 量化毫秒，~1000s 处误差 6e-5s，无影响）；ghost 视觉字段常量内联；`unpackTrig(0)`→`Zero` vs JS `undefined`（消费层缺失，无行为差）。
  - **修复后全门禁 ✅**：fmt/clippy ✅、`cargo test --workspace` **243 passed**（基线 242→243，新增回归测试）✅、wasm32（`-p inkwave -p inkwave_sim`）✅、`trunk build --release` ✅、headless 冒烟 ✅、`cargo tree` 依赖面不变 ✅、JS 树零改动 ✅。

## Task 24: 极简大厅 UI
- **Status**: `completed`
- **Priority**: medium
- **Depends On**: Task 20
- **Description**:
  - `inkwave::ui::online`（bevy_ui）：主菜单加"Online"入口；创建房间（显示 5 位码）/输入码加入；名册列表（名字/队/ready/host/ping）；切队（0/1/auto）与 ready；host 设置最小子集（duration/difficulty/bots）与 start（全员 ready 才亮）；错误/断线提示；`starting`→载入对局、结算→回大厅。接 Task 20 会话事件流；无 showcase 场景。
- **Acceptance Criteria Addressed**: AC-M2-6（UI 层）
- **Test Requirements**:
  - `rule` TR-24.1: UI 状态→会话动作映射单测（可测部分抽纯函数；按钮禁用逻辑随状态）；证据为测试输出
  - `rule` TR-24.2: 双窗口原生人工试玩记录（create/join/ready/start/回大厅）；证据为操作记录
  - `rule` TR-24.3: 全门禁（含 trunk wasm 构建）+ JS 树零改动；证据为命令输出
- **Completion Evidence**:
  - **纯映射层（`ui/online.rs`，TR-24.1）**：`run_cmd` 11 命令逐一对应 JS 处理器（doCreate L2657-2684、doJoin L2617-2647 含短码 `caret=firstEmpty()`、cancelConnect L2648-2656、leaveRoom L3040-3046、toggleReady L3048-3058 host⇒tryStart、requestTeam L3080-3083、hostSet×3 L2944-2950、tryStart L3060-3072；lobby 命令均带 `state != Lobby` ≡ JS `S.launching` 守卫，requestTeam 另带 `any(p.you)` ≡ `!me`）；`project` 将会话事实投为 HubView/LobbyView（JOIN_ERR 六条 L49-54、`error_detail`/`room_gone_toast` L2378、码输入 `code_type`/`code_backspace`/`first_empty`/`code_full`/`auto_join_armed` L2508-2564、`list_names`/`waiting_banner` L3062-3073、`leave_text` L3028-3033 含 padded code L2781、renderBar L3344-3369、statusTxt L3282-3296）；`OnlineFlags::settle(err)` 按 JS 路由：join 失败→jstat 面板+`caret=4`+保留 "Connecting to room X…" hint（showError L2607-2616，render L2525-26 error 模式不重写 hintEl）、create 失败→create 卡 `create_error_text` 三分支（L2673-2682，U+2014/U+2019 逐字）、成功→hint 回默认。
  - **屏幕层（`ui/mod.rs`）**：`Screen::OnlineHub`/`OnlineLobby` + Bevy 节点树（含 `hub_create` create 卡状态行）；键盘输入（type/backspace/Enter/Esc/R）与 `handle_online_action` 按钮分发；session pump + `settle` + 屏幕同步（`_onNetState` L2359-2368）；**autoJoin**：`UiState.auto_join` 320 ms 倒计时逐字移植 L2554（第 5 有效字符落末格武装、回调守卫 `screen==OnlineHub && !busy && full` ≡ `alive && mode==='entry' && full`；backspace L2559/Enter L2619/JOIN 按钮/离开 hub（≡`st.alive=false`）清零；无效字符与 Cancel 不清=JS quirk 保留）。
  - **对局接线（`net/match_link.rs` + sim `member_ids`/`local_slot`）**：`start_net_match`（main.js L770-807）建局+bind+bot 脑+`prune_loading_leavers`（netmatch.js L79 humans-only 门控，members=`sess.member_ids()` ≡ `_members` session.js L31/L100-101）+`local_roster_index` 同门控解析 local（match.js L116 最终数组语义）；`net_tick` 驱动 replication，`!net_mode` 时自动 teardown；judge→`res`/`end`→results→回大厅（netMatchEnd L823-842、host 12 s 结果倒计时 L972、guest 等 host `end`）；Leave 臂 `on_leave` 收缩后经 `rep.local_slot` 重解析 `local_idx`/`demo.player_slot` 并 resize inputs。
  - TR-24.1 ✅：16 个 `tr24_*` 测试（`online.rs`：error_copy/leave_confirm/code_char_feedback/list_names+waiting_banner/project_hub_gating/project_start_enable/run_cmd_maps_to_session/settle_error_routing/first_empty_and_auto_join_armed/start_block_banner/project_start 等；`match_link.rs`：lobby_flow_two_clients/build_net_match_roster_order/match_end_returns_lobby/local_roster_index_follows_prune_gate/local_slot_follows_leave_removal）——FakeRelay 双 `NetSession` 走与 app 完全相同的 `project`/`run_cmd`/`build_net_match` 路径。
  - TR-24.2 ⏳（登记待复核）：沙箱无 GPU/窗口，真实双窗口人工试玩无法执行；等价验证=上述双会话全流程集成测试（create/join/ready/start/回大厅/断线提示 6 步均有断言）。**真实双窗口人工试玩登记为"待用户机器复核"条目**，随 Task 26 建立 REVIEW_CHECKLIST 时列入执行清单。
  - TR-24.3 ✅：`cargo fmt --all --check` ✅、`clippy --workspace --all-targets -- -D warnings` 0 错 ✅、`cargo test --workspace` **259 passed**（58 inkwave + 189 sim + 4 acceptance + 8 relay_shim；基线 255→259）✅、wasm32 check（`-p inkwave -p inkwave_sim` 口径）✅、`trunk build --release` ✅、headless 冒烟（`--headless --frames 120` boot+complete）✅、`cargo tree -p inkwave_sim` 依赖面不变（仅 glam/serde）✅、JS 树（src/server/tools/docs）`git status` 零改动 ✅。
- **Review 修复附录**（首轮独立审查：1 major / 8 minor / 2 nit，结论"需修复后合入"；修复后复审读盘核对 11 项全部落码且与 JS 等价，另核 autoJoin 补项并发现 1 minor + 2 nit，修复后再跑全门禁；除 m-7 转偏差⑧外全部代码修复）:
  - **M-1 ✅（local_idx 与 prune 门控不一致，真 bug）**：提取 `local_roster_index(roster, my, members, no_bots)` 纯函数，filter 门控与 `prune_loading_leavers` 一致（bots 舞台不剔除、humans 舞台剔除缺席 owner），`start_net_match` 与测试助手 `build_side` 共用；回归测试 `tr24_local_roster_index_follows_prune_gate`（bots 舞台缺席保留偏移=1 / humans 舞台剔除=0 / bot 条目永不作 local / 自身 `owner==my` 短路守卫）。
  - **m-1 ✅**：prune 的 members 改用会话权威集合——`session.rs` 新增 `pub fn member_ids()`（`members.keys()` ≡ `_members`），match_link 调用之。
  - **m-2 ✅**：`net_tick` 顶部 `!st.net_mode → teardown_net_match + return`（Pause→Menu 离开后 replication 不再孤儿步进，≡ netMatchEnd dispose 语义）。
  - **m-3 ✅**：run_cmd Join 短码臂 `caret = first_empty(draft)`（L2621-2622）；测试断言 caret==3。
  - **m-4 ✅**：ToggleReady/SetTeam/Start/SetDuration/SetBots/SetDifficulty 六臂补 `state != Lobby`（≡`S.launching` L2945/L3051/L3061/L3080）；测试补 no-double-launch 断言。
  - **m-5 ✅**：join 失败 settle 路由（err_panel+caret=4+connecting hint 保留）；投影 label=JOIN、join_ready=full()（mode 'error' ≠ 'connecting'，L2520/L2522）。
  - **m-6 ✅**：create 失败走 create 卡（`create_error_text` known→`title — text`/unknown→raw/empty→"Couldn't open a room"），不碰 jstat/hint；`hub_create` 标签节点渲染。
  - **m-7 ✅→偏差⑧**：pad 输入模式省略，声明落 online.rs 模块 docs 并回填计划文档。
  - **m-8 ✅**：Leave 臂 on_leave 前后长度比对，收缩时 `rep.local_slot(my)` 重解析并同步 `nms.local_idx`/`demo.player_slot`、`inputs.resize`；`replication.rs` 新增 `pub local_slot`；回归测试 `tr24_local_slot_follows_leave_removal`（cargo 舞台 Some(1)→on_leave→Some(0)、缺席 id→None）。
  - **n-1 ✅**：leave_text 插值 padded code（大写+padEnd(5,'•')+≤8，L2781/L3032-33）。
  - **n-2 ✅**：solo 用例显式 `bots:true/false` 双分支确定断言（"Just you and the bots"/"Nobody to play against yet!"）。
  - **autoJoin ✅（首轮审查漏项，本轮自主补齐）**：L2554 320 ms 自动加入定时器逐字移植（见 Completion Evidence 屏幕层条目）；复审推演确认 busy 期间到期丢弃、Cancel 后到点重 Join 的 quirk 与 JS 净效果一致。
  - **复审 minor-1 ✅**：JOIN 按钮点击路径与 Esc/HubBack 离 hub 清 `auto_join`（doJoin L2619 clearTimeout 与触发源无关；离 hub ≡ `st.alive=false` 永久丢弃，防冻结后复活）。
  - **复审 nit-1 ✅**：注释锚点 `session.js L788` 更正为 `L31/L100-101`（session.js 实存 399 行，原锚笔误）。
  - **复审 nit-2 ✅**：run_cmd Create 与 settle create 臂不再清 hint（doCreate/catch 不触 hintEl、不调 render，L2657-2666/L2673-2682）。
  - **修复后全门禁 ✅**：fmt/clippy ✅、`cargo test --workspace` 259 passed（基线只增）✅、wasm32 ✅、`trunk build --release` ✅、headless 120 帧 ✅、`cargo tree` 依赖面不变 ✅、JS 树零改动 ✅。

## Task 25: 端到端集成——Rust↔Rust 完整对局
- **Status**: `completed`
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
- **Completion Evidence**:
  - **建局器下沉**：`build_net_match`/`local_roster_index`/`fnv1a` 逐字平移至 `inkwave_sim::net::matchbuild`（app crate bin-only 无法被 harness 依赖；一处建局真相，app 与 harness 共用；app `match_link.rs` import 切换、行为不变，sim 依赖面不变）。
  - **场景驱动器（`net-harness/src/match_e2e.rs`，TR-25.1/2/3）**：`run_match` 组装 FakeRelay + 3×`NetSession` + 3×`Replication` + 3×`Match` 于共享虚拟时钟（tr24 `step2` 范式：首端 `advance(dt)`、余端 `advance(0)`），lockstep 子步循环 drive_bots→`step_masked(remote_mask)`→`rep.update`→drain→dispatch；事件分派逐镜像 app `net_tick` 契约（Host→`set_host`+host_changed；Leave→`on_leave(flag)`+`local_slot` 重解析+inputs.resize；GameMessage→`on_message`；MatchEnd→`end_match`）；1 host + 2 guest + 5 bot 补位至 8；duration 15s 经本地克隆 `StartCfg` 覆盖（≡ net-test.mjs L85 `600/40` 本地覆盖接缝；lobby 路径 JS `Math.max(60,…)` 钳制未动，session.rs L1284-1290）；掉线用 `close()` ≡ net-test `--leave` 的 `DROP=kill`（L118-125）。计划批准口径记录：shim 真实 socket 完整局留给 Task 26（计划文档决策 6），本任务用确定性 FakeRelay 全栈。
  - TR-25.1 ✅：`tr25_full_match_three_clients` + `tr25_full_match_deterministic`（同 seed 双跑：逐端最终网格 FNV 与 result 精确一致）；CLI `cargo run -p net-harness -- match` overall **CONSISTENT**（1311 帧；覆盖率两两差 <0.5pp：4.97/7.40 vs 4.98/7.41；result==host `res`：wire 权威字段 coverage/winner/elapsed 精确相等 + points 舍入上界 <2.0；全员回 Lobby；无 panic）。
  - TR-25.2 ✅：`tr25_lag_scenario_stats`（LinkSim{lag 40, jitter 30, spike 0.01}）复跑 CONSISTENT；统计公式逐字移植 `net-test.mjs`：pct（L242）、kink/turn-kink（L244-257，稳帧 6-45ms 过滤、y 权重 0.5、wrapped yaw）、sample（L233-241）、path-error 拟合（L278-291，2s 窗、lag 40-400 步 5、n>20）、teleport（L275，>1.2m）——行号锚点 + 5 个公式单测；**数值门限由独立审查定标**（Open Question 3）并写入断言：kink p95 ≤5cm、turn-kink p95 ≤3°、path-error p95 ≤30cm、teleports ≤2/端且总 ≤5（p99/max 为 netspike 场景输入的既定离群，仅报告不判决 ≡ JS 打印语义）；观测值：kink p95 ≤1.6cm、path p95 ≤11.6cm、teleports 0/0/1、owners' own kink p95 1.8cm。
  - TR-25.3 ✅：`tr25_host_migration`（5s 时 host `close()`→relay 重选→guest `adopted_host` 接管权威→打完出结算，全员 result 一致）+ `tr25_guest_dropped_to_bot`（5s 时 guest `close()`→host `on_leave` 收养转 bot→结算核验被弃 actor 在 host 端位移 >0.05m，即"继续游玩"）；两场景 checks 全 PASS。
  - TR-25.4 ✅（审查修复后重跑）：`cargo fmt --all --check` ✅、`clippy --workspace --all-targets -- -D warnings` 0 错 ✅、`cargo test --workspace` **269 passed**（58 inkwave + 189 sim + 5 harness lib + 4 acceptance + 5 e2e_match + 8 relay_shim；基线 259→269）✅、wasm32 check（`-p inkwave -p inkwave_sim` 口径）✅、`trunk build --release` ✅、headless 冒烟（`--headless --frames 120` boot+complete）✅、`cargo tree -p inkwave_sim` 依赖面不变（仅 glam/serde/serde_json）✅、JS 树（src/server/tools/docs）`git status` 零改动 ✅。
- **Review 修复附录**（独立审查：0 blocker / 0 major / 1 minor / 4 nit，结论"可合入"；全部当场修复后重跑全门禁）:
  - **A3 ✅（minor，points 容差非数学上界）**：`<1.0`→`<2.0+1e-3`（`wire::round_int` 逐 actor ±0.5 m² × pointsPerM2=1 × 每队 4 actor），注释写明推导；固定 seed 下无 flake 风险。
  - **A4 ✅（nit）**：dispatch `Host` 臂补 `gone[i]` 守卫，与 Leave/GameMessage 臂对称。
  - **A5 ✅（nit）**：`drop_track` 注释声明索引一致性前提（bots 舞台 `no_bots=false`、`on_leave` 收养不删员、无左移）。
  - **A6 ✅（nit）**：模块 docs 补偏差——harness 结算后立即 `end`，跳过 app 的 12s `_netEndT` 结果页倒计时（握手帧一致）。
  - **A7 ✅（nit）**：CLI 注释声明 `match` 不入 `all` 的耗时取舍。
  - **审核心验通过项**：建局器平移逐行一致；5 个统计公式守卫/权重/过滤/百分位选取/拟合窗逐字等价；dispatch 与 net_tick 及 session.js L156-163 时序镜像；结算边界（follower 门控不本地判决、elapsed 精确相等、gone 端全程跳过、c0 仅 2 remote 合理）。

## Task 26: 双端联网冒烟与跨端握手
- **Status**: `completed`
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
- **Completion Evidence**:
  - **test-only 接缝（`rust/crates/inkwave/src/nettest.rs`，新）**：query 门控 `net=create`/`netjoin=CODE`/`netready`/`netstart`/`netdur=N`/`autopilot`/`netprobe`；证据行 `[nettest] boot/err/code/lobby/probe` 走 `web_sys::console::log_1`（wasm `println!` 不进 console；native println）。`parse_query` 纯函数 + 4 单测（空/全参/坏码/部分参）；无接缝参数 cfg=None 零行为、native 恒 None。驱动语义逐字对齐 `tools/net-test.mjs` L73/L77/L80/L81-87/L85 与 `ui/mod.rs` L1339-1341（EnterHub）。
  - **autopilot 接线（`match_link.rs`）**：`start_net_match` 本地槽 `rep.attach_bot(…difficulty.easy…)` ≡ `match.js` L112 `mine && (r.bot || o.autopilot)`；`net_tick` 本地输入覆写块加 `!autopilot` 守卫 ≡ L118 不建 PlayerController。
  - **冒烟脚本（`rust/tools/verify/net-smoke.mjs`，新）**：raw CDP（Node≥22 全局 WebSocket，零 npm 依赖，复用 wasm-smoke.mjs 探针模式：fresh profile/RENDER_ERR_RE/console.error+exception+Log 收集）；场景 A wasm↔wasm 完整局、场景 B JS(:8490 只读 `__G.net`)↔Rust 跨端握手；`--enable-unsafe-swiftshader`（chromium 154 已弃用自动软件 WebGL，缺省则 wasm 应用启动即退）；shim 端口转发 + 子进程早退 fail-fast；parseProbe 往返自检（加载即校验与 nettest.rs 格式互配，失配 exit 2）；渲染死亡/停滞→DEGRADED（保留证据、exit 0），非渲染类失败→FAIL（exit 1）。
  - **TR-26.1 ⚠️ DEGRADED（沙箱降级条款，如实记录）**：headless chromium 仅有 SwiftShader 软件渲染，Bevy/wgpu PBR 管线（`pbr_prepass_pipeline` VERTEX）ANGLE 翻译失败 → "Quitting the application due to Internal"；**离线 demo（无任何网络参数）同样崩溃**，证明属环境渲染限制而非本任务代码。且软件渲染长时间占住 wasm 主线程，饿死 transport 的 8s 握手定时器（≡ transport.js L48，端口不可改）。落盘证据：`rust/tools/verify/logs/net-smoke-2026-10-16T09-45-42-683Z.json`（boot/err/render-error 行）、`…T14-15-06-886Z.json`（修复后最终轮判定）。完整局复跑命令已入 REVIEW_CHECKLIST §2。
  - **TR-26.2 ⚠️ 部分实证 + 降级登记**：JS 客户端经 shim 建房 ✓（`3UZ9X`）；**Rust wasm 客人 join 落地被 JS 大厅实收** ✓（`players=[["Host",0],["Player",0]]`，`logs/net-smoke-2026-10-16T09-55-51-090Z.json`）——跨端 join 方向在真实 WS 上打通；Rust 侧 welcome 证据行与 tick 段因主线程 stall 未达，降级入 REVIEW_CHECKLIST §3（含复跑命令）。
  - **REVIEW_CHECKLIST.md（新建）**：①Task 24 TR-24.2 双窗口人工试玩 6 步；②TR-26.1 wasm↔wasm 完整局复跑；③TR-26.2 tick 段复跑；④原生↔wasm 同房局（沙箱无 DISPLAY）；⑤net-test.mjs Linux executablePath 提示（JS 树只读不改）。
  - **TR-26.3 ✅**：`cargo fmt --check` ✅、`clippy --workspace --all-targets -- -D warnings` ✅、`cargo test --workspace` **273 passed**（基线 269→273，nettest 4 单测）✅、wasm32 check（`-p inkwave -p inkwave_sim`）✅、`trunk build --release` ✅（dist 为 gitignore 本地产物）、headless 120 帧 ✅、`cargo tree -p inkwave_sim` 仅 glam/serde/serde_json ✅、JS 树（src/tools/server）`git status` 零改动 ✅。
- **偏差声明**：①接缝为 test-only query 门控（JS 先例 `__G.net` API + `?skipTitle&autopilot`；Rust 无 JS API 面，Create/Start 为鼠标按钮）；②`netdur` 走既有 `SetDuration` 钳制路径（lobby ≥60 未动，冒烟用 60——修订设计文档"直写覆盖"方案）；③probe 证据语义 = `[nettest]` console 行（wasm println 不可见）；④`--enable-unsafe-swiftshader` 属沙箱 chromium 环境要求，非产品行为。
- **Review 修复附录**（独立审查：1 blocker / 2 major / 3 minor / 2 nit，结论"需修复后合入"；必修项全部修复）:
  - **A1 ✅（blocker，PROBE_RE 与 format! 不匹配）**：远端坐标正则改圆括号精确形 `^(\d+):\((-?[\d.]+),…\)$`；修复中发现原 `m[5]` 组索引亦错（rem 为第 4 组）→ 改 `m[4]`；新增加载期往返自检（自检曾在最终轮真实拦截崩溃，证明必要）。
  - **A2 ✅（major，CHECKLIST §2 证据不实）**：删除未落盘的 `4BNV9` 断言，§2 严格以 logs/*.json 已落盘行为准（join 方向已证、host welcome 行未在任何落盘运行出现如实写明）。
  - **A3 ✅（major，无 FAIL 路径）**：场景 A 无 `[nettest]` 行→FAIL；场景 B JS 侧失败（rustTab 未建）→FAIL；两 PASS 分支 checks 不过记 FAIL；DEGRADED 仅在渲染死亡或有 boot 行的停滞时成立。
  - **A4 ✅（minor）**：shim/dist 子进程早退 fail-fast（残留进程占端口不再被静默容忍）；shim 端口经 `--` 转发（原恒绑 8787）。
  - **A5 ✅（minor）**：PASS 标签与 checks 一致（见 A3）。
  - **A6 ⏸（minor，err 一次性闩锁）**：有意保留——重复 err 行无额外证据价值，console.error 已被 Tab.errors 全量收集。
  - **A7 ✅（nit）**：删除多余 `too_many_arguments` allow（6 参低于阈值 7）。
  - **A8 ⏸（nit，labelSeq 消费模式）**：不改——脚本内全部 tab 均显式设 label，前缀过滤无漏。
  - **审核心验通过项**：JS 树零改动；锚点行号实核全部正确（match.js L112/L118、transport.js L43/L48、net-test.mjs L73/L77/L80-87/L85/L144、ui/mod.rs L1002/L1339-1341、online.rs L235-236、replication.rs L318-321/L331-333）；native 恒 None；`!autopilot` 守卫为原代码包裹、无接缝时行为不变；host/guest 分支与 JS 驱动等价；借用/let-chain/单测边界 ✅。

## Task 27: M2 收尾——门禁、台账、文档与总评审
- **Status**: `completed`
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

### Task 27 Completion Evidence（2026-10-16）
- **TR-27.1 ✅ 全量门禁矩阵**（同 M1 Task 16 口径；日志 `rust/tools/verify/logs/task27-gates-20261016T161626Z.log` 共 8 段）：`cargo fmt --all -- --check` 无 diff ✅；`cargo clippy --workspace --all-targets -- -D warnings` exit 0 ✅；`cargo test --workspace` **273 passed**（62 inkwave + 189 sim + shim 8 + harness/acceptance 5 + e2e_match 4 等；>147，基线 147→164→180→194→217→229→239→243→259→269→273 单调递增）✅；`cargo check --target wasm32-unknown-unknown -p inkwave_sim -p inkwave` ✅；`cargo tree -p inkwave_sim --depth 1` 仅 glam/serde/serde_json（sim 零引擎依赖不破）✅；`trunk build --release` ✅；原生 headless 120 帧 boot+complete ✅；`git status --porcelain -- src tools server docs` 空（JS 树 + `server/` 零改动）✅。
- **TR-27.2 ✅ README 增「3. 联网运行」章节并逐条命令实跑验证**（原 3/4 节顺延 4/5；门禁速查补 wasm32 双 crate 与 net-smoke）：
  ① `cargo run -p relay-shim -- 8796` 实测就绪（banner 打印 MSG_MAX=65536/RATE=90/sweep）；
  ② `cargo run -p inkwave -- --relay=ws://127.0.0.1:8796`：参数解析通过、失败点为 winit 事件循环创建（`DISPLAY is not set`，沙箱无窗口环境；窗口视觉复核 = CHECKLIST §1），如实标注；
  ③ dist 静态服务：`trunk build --release` 产物 + http.server（net-smoke 内同路径实跑）；
  ④ wasm query URL 与冒烟命令：`node rust/tools/verify/net-smoke.mjs --scenario all --secs 25 --relay-port 8796 --dist-port 8496 --js-port 8495 --cdp-port 9226` **exit 0**——A=DEGRADED（`[nettest] boot` 参数解析 + `err Could not connect` + render-error 记录在案）、B=DEGRADED（JS host 建房 `L74QY` 成功；本轮 lobby 行为 null = stall 波动，join 落地实据仍以 09-55 记录为准），证据 `logs/net-smoke-2026-10-16T16-31-48-848Z.json`；
  ⑤ 跨端说明引 §3 实据。
  **追加实据（真实 Worker 探针）**：`npx wrangler dev --port 8799` Ready + raw WS（Origin `http://127.0.0.1:8495` + `/room/<code>?name&v=1&create=1`，即 Rust 客户端形态）→ `HTTP/1.1 101` + welcome（id=K4PZ），日志 `logs/task27-wrangler-probe.txt`——真实 Worker 放行 Rust 形态 Origin 与 PROTO v1 握手已在沙箱实证。
- **台账/文档更新**：`PORT_MAP.md`（net 三行 `M2 done`、`server/` 行改 M2 验证、net-test 行 Rust 等价物、新增 shim/fixtures/harness 三行、「M2 完成核对（Task 27）」小节、图例加 `M2 done`）；`SYNC_LEDGER.md`（「M2 基线核对」：沙箱 fetch 网络不可达、脚本降级本地 refs，`upstream/main`=基线 `3e9b5505` 零新提交，github 远端不可达如实记录；「M2 移植说明」JS→Rust 逐块映射表 + fixture/harness/shim 纪律；「M2 新增依赖登记」4 行表，sim 零依赖不破）；`REVIEW_CHECKLIST.md` 追加 §6 远端角色播放平滑视觉复核（TR-22）、§7 真实 relay（部署端）完整对局复核、§8 跨端握手总览索引。
- **TR-27.3 ✅ 独立总评审**（见下 Review 附录）：0 blocker / 2 major / 4 minor / 1 nit，全部闭环或说明不修；复审追加 1 处证据缺口亦修复；复审结论**可合入**。
- **M2 里程碑**：Task 17–27 全部 completed。

### Task 27 Review 附录（M2 总评审：0 blocker / 2 major / 4 minor / 1 nit → 修复后可合入 → 复审闭环）
- **M-1 ✅**：README/SYNC_LEDGER/CHECKLIST §7 三处「真实 relay 沙箱不可达」措辞与 Task 19 记录矛盾（`wrangler dev` 本地 DO 可达且 TR-19.1 场景 8/8 交叉验证过）——改为「本地 wrangler dev 可达、shim 语义已交叉验证；未验证 = Rust 客户端直连真实 Worker 完整对局路径与部署端」，并新增真实 Worker 握手探针实证（见 TR-27.2 追加实据）。
- **M-2 ✅**：PORT_MAP/SYNC_LEDGER/计划文档使用不存在的模块名 `net::codec`/`net::replay`——修正为 `inkwave_sim::net::wire`（Task 17 codec 实名）与 `net::replication::play_events`（事件时间线重放归属），与 `net.rs` L63-68 模块声明一致。
- **m-1 ✅**：SYNC_LEDGER 依赖表 web-sys features 补全（`Performance`/`MessageEvent`/`CloseEvent`，对照 Cargo.toml L28-39）。
- **m-2 ✅**：gates 日志补齐 fmt/`cargo tree`/JS 树三段（共 8 段完整留档）。
- **m-3 ✅**：spec.md OQ-3（AC-M2-3 数值门限）勾选并注记定标值（与 e2e_match.rs 断言逐值一致）。
- **m-4 ✅**：README §3 首句「完全兼容」→「wire 格式逐字节兼容」（spec Non-Goals：全武器跨端玩法留 M3）。
- **n-1 ⏸（说明不修）**：Task 18 段称 TR-18.1「11 passed」而 `transport.rs` 现 12 个 `#[test]`——当时按模块过滤口径计数，与现全量口径无矛盾（门禁 273 以实跑为准）。
- **复审追加 ✅**：§7「房间 TEST27P」断言不被 probe 日志支撑（welcome 帧无 room 字段）——删除房间号断言，改以日志实存的 `HTTP/1.1 101` + welcome `id=K4PZ` 为据（严格以落盘日志为准纪律）。
- **总评审实核通过项**：门禁复跑 273 一致；REVIEW_CHECKLIST 全部「已证」断言有落盘日志支撑（`3UZ9X`/players 列表/boot/err 行/code 行缺失均 grep 实核）；JS 锚点抽核 12+ 处命中（transport.js L43/L48、match.js L112/L118、server L22-23、netmatch.js packer、session.js、net-test.mjs L73/L77/L80-87 等）；偏差声明覆盖 spec Non-Goals 与 AC 降级条款；README §3 命令逐条对码一致。

---

## Review 附录（逐任务追加）

（实施时按 M1 格式：独立审查发现 → 修复 → 复审结论）
