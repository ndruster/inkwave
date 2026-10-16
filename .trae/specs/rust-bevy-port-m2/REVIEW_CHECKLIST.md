# REVIEW_CHECKLIST — 沙箱不可达项的用户机器复核清单（M2）

> 背景：开发沙箱无 DISPLAY/Xvfb（原生窗口端无法实跑），且 headless chromium 仅有
> SwiftShader 软件渲染——Bevy/wgpu 的 PBR 管线（`pbr_prepass_pipeline` VERTEX）在
> ANGLE/SwiftShader 下 shader 校验失败，wasm 应用在启动约 10–30 s 后
> "Quitting the application due to Internal" 退出；软件渲染还会长时间占住 wasm 主线程，
> 令 transport 的 8 s 握手定时器（≡ transport.js L48）被饿死。
> 以下条目按 tasks.md Task 24/26 的降级条款登记，需在用户机器（真 GPU + 桌面窗口）复核。

## 1. Task 24 TR-24.2 — 双窗口人工试玩（原生 ↔ 原生，6 步）

```bash
# 终端 1：本地 relay（PROTO v1 shim）
cd rust && cargo run -p relay-shim            # ws://127.0.0.1:8787
# 终端 2/3：两个原生客户端同房
cargo run -p inkwave -- --relay=ws://127.0.0.1:8787   # 窗口 A：Create → 记下房间码
cargo run -p inkwave -- --relay=ws://127.0.0.1:8787   # 窗口 B：输入房间码 Join
```

复核点：①大厅互见双方玩家；②ready 后 host Start 双方同屏进局；③对局中远端角色移动/喷涂平滑；
④结算页分数一致；⑤host 掉线 → guest 迁移 host；⑥guest 掉线 → 转 bot。

## 2. Task 26 TR-26.1 — wasm↔wasm 完整一局（沙箱降级：SwiftShader 渲染失败）

沙箱内已证（均在 logs/net-smoke-*.json 落盘）：接缝 boot/参数解析正常；wasm 客户端
对 shim 的真实 WS 连接可建立并完成 join 落地（见 §3 的 09-55 记录：JS 大厅实收 Rust
客人 `[["Host",0],["Player",0]]`）；但 host 侧建房 welcome 证据行（`[nettest] code`）
未在任何落盘运行中出现——8 s 握手定时器常被软件渲染 stall 饿死（err "Could not connect"）。
完整对局（probe 位移/coverage 断言）因 PBR 渲染退出无法在沙箱跑通。

```bash
cd rust && trunk serve --port 8491            # 或 trunk build --release + 任意静态服务器
cargo run -p relay-shim                        # 终端 2
node rust/tools/verify/net-smoke.mjs --scenario A   # 终端 3（仓库根）
```

复核点：场景 A 输出 `PASS`（双方 `[nettest] probe` 行、远端位移 >0.5 m、coverage>0、无 render-error）。
带真实 GPU 的浏览器直接开两页亦可：
`http://localhost:8491/?relay=ws://127.0.0.1:8787&net=create&netdur=60&netstart&autopilot&netprobe`
与 `…&netjoin=<CODE>&netready&autopilot&netprobe`。

## 3. Task 26 TR-26.2 — 跨端握手 tick 段（沙箱降级：welcome 被 8 s 定时器饿死）

沙箱内已证（logs/net-smoke-2026-10-16T09-55-51-090Z.json）：JS 客户端（:8490）经 shim
建房 `3UZ9X`，Rust wasm 客人 join 落地——JS 大厅 `players` 实收
`[["Host",0],["Player",0]]`；Rust 侧 welcome 行与对局 tick 因主线程 stall 未达。

```bash
node rust/tools/verify/net-smoke.mjs --scenario B   # 期望 PASS：Rust [nettest] lobby/probe 行 + JS playing
```

## 4. Task 26 — 原生窗口端 ↔ wasm 同房一局（沙箱无 DISPLAY，未跑）

```bash
cargo run -p relay-shim
# 原生窗口 A：Create（UI 手动），记下房间码
# 浏览器 B：http://localhost:8491/?relay=ws://127.0.0.1:8787&netjoin=<CODE>&netready&autopilot&netprobe
cargo run -p inkwave -- --relay=ws://127.0.0.1:8787
```

复核点：跨端（native↔wasm）welcome/join/start/tick 互收，双方 probe/渲染一致，无 err。

## 5. tools/net-test.mjs — Linux 机器提示（JS 树只读，不修改）

`tools/net-test.mjs` L46-48 硬编码 macOS Chrome
`executablePath: '/Applications/Google Chrome.app/…'`；Linux 复跑需临时改为
`/usr/bin/chromium-browser`（或设 `CHROME_PATH`），属 JS 基线工具，本端口不触碰。

## 复跑证据落盘位置

`rust/tools/verify/logs/net-smoke-<utc>.json`（tabs 原始 console 行 + 场景判定）。
沙箱最终判定样例：A=DEGRADED（boot/err/render-error 记录在案）、
B=DEGRADED（join 互收已证、tick 段降级）。
