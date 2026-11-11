# INKWAVE — Rust/Bevy 移植（M1–M4）

INKWAVE（Splatoon-like 涂地对战）从 three.js 移植到 Rust + Bevy 0.19 的工作区。
本目录 `rust/` 与上游 JS 树（`../src`、`../server`、`../tools`）并存：
**JS 树永远零改动**，Rust 侧通过提取管线读取 JS 数据、通过 `tools/sync-upstream.sh` 跟踪上游提交。

M1 垂直切片：Tidewater 地图 + Spritzer 武器 + 涂地计分 + 4v4 easy bot + 墨汁渲染 + 菜单/HUD/结算 + 占位音效。
M2 联网栈：PROTO v1 与上游 JS 客户端逐字节兼容，大厅 + Rust↔Rust / JS↔Rust 完整局（第 3 节）。
M3 内容：12 主武器 / 15 副 / 19 特殊全量、Zone Control 模式、saltpan/crossmarket 新图、bot 三档、
大厅 loadout 选择与跨端开黑（第 4 节）。
M4 玩法优先收尾：Boss 战（六招/联网迁移/离线完整局）、bot 威胁 AI、8 图全量提取、小地图、过程化音频、fx、ink 着色与 46 层材质、props/角色弹簧绑定、环境主题/质量档位、3D showcase 大厅（第 7 节）。
模块对照见 [PORT_MAP.md](./PORT_MAP.md)，上游同步流程见 [SYNC_LEDGER.md](./SYNC_LEDGER.md)。

## 1. 环境准备

```bash
# Rust（rust-toolchain.toml 固定 stable + wasm32 目标 + clippy/rustfmt）
rustup component add clippy rustfmt
rustup target add wasm32-unknown-unknown

# Trunk（wasm 打包/开发服务器）
cargo install trunk --locked

# Node ≥ 22（数据提取管线零 npm 依赖；冒烟脚本用全局 WebSocket，需 ≥ 22）
node --version
```

- 原生构建在 Linux 需要系统库：`wayland`、`libxkbcommon`、`alsa-lib`、`systemd-devel`
  （Fedora/RHEL：`dnf install wayland-devel libxkbcommon-devel alsa-lib-devel systemd-devel`）。
- wasm 运行需要支持 WebGL2 的浏览器（Bevy 0.19 wasm 走 WebGL2/WebGPU）。

## 2. 双端运行

### 原生（native）

```bash
cd rust
cargo run -p inkwave --release            # 打开窗口：菜单 → Enter/点击 Play 开始
cargo run -p inkwave --release -- --headless --frames 600   # 无 GPU 冒烟（CI 用）
```

无头 autopilot（确定性 JSON 报告 + stderr 上的步耗时 p50/p95 统计，TR-16.1）：

```bash
cargo run --release -p inkwave_sim --bin inkwave-autopilot -- \
    --duration 30 --seed 12345 --autopilot --out /tmp/report.json
```

- stdout 为确定性核心（`frame_times: null`），同种子两次运行可逐字节 diff；
- `--out` 写完整报告（含 `frame_times`）；stderr 打印
  `[autopilot] sim steps=... sim p50 FPS=... sim p95 FPS=...`（**sim 固定步速率，非渲染帧率**）；
- 性能参考：8 bot 30 s 对局，sim 固定步 p50 ≈ 0.06 ms（≈17 000 步/秒），
  为 60 FPS 渲染留 >250× 余量（Xeon E5-2699 v4，无 GPU 环境；渲染帧率达标须在有 GPU 环境复核）。

渲染帧率实测（TR-16.1 的 GPU 复核，需真实 GPU + 桌面窗口）：

```bash
cargo run --release -p inkwave -- --bench 30   # 1920x1080、无 vsync、自动开局、30s
# 结束打印 stderr: [bench] frames=... | render p50 FPS=... render p95 FPS=...
```

窗口须保持前台（最小化/失焦会节流渲染循环，测到的是节流值不是 GPU 能力）。

### wasm（浏览器）

```bash
cd rust
# 干净 Linux 的 inotify 默认上限（8192）不足以让 trunk serve 监视整个工作区，
# 会报 "OS file watch limit reached" 并退出；容器/CI 里先提高上限：
sudo sysctl fs.inotify.max_user_watches=524288    # 或 echo 524288 | sudo tee /proc/sys/fs/inotify/max_user_watches
trunk serve --cargo-profile wasm-dev      # 开发迭代 http://localhost:8080
trunk build --release                     # 发布产物到 rust/dist/
```

冒烟脚本（CDP 驱动 headless chromium：加载→开局→60 s，收集 console error/panic/渲染错误/FPS，TR-16.2）。
以下三条命令均在**仓库根目录**（`inkwave/`，不是 `rust/`）执行：

```bash
# 终端 1（仓库根）：静态服务 dist/（任意方式，例如）
python3 -m http.server 8002 --directory rust/dist
# 终端 2（仓库根）：headless 浏览器开 CDP 端口
#   --disable-dev-shm-usage：容器 /dev/shm 过小时 SwiftShader 会撑爆共享内存、
#   GPU 进程 SIGSEGV，表现为虚假的 canvas.getContext() panic（非应用 bug）
#   --user-data-dir 用全新目录，避免持久 profile 回放旧的 index.html/wasm
rm -rf /tmp/smoke-profile
chromium-browser --headless=new --no-sandbox --disable-gpu \
    --disable-dev-shm-usage \
    --user-data-dir=/tmp/smoke-profile \
    --remote-debugging-port=9222 --window-size=1920,1080 about:blank &
# 终端 3（仓库根）：跑冒烟（Node ≥ 22 全局 WebSocket，零依赖）
node rust/tools/verify/wasm-smoke.mjs http://localhost:8002/index.html 60 9222
```

输出 JSON：`{frames, fps_mean, fps_p50, fps_semantics, panicked, rendering_errors, error_count, errors}`，
有 error/panic/渲染错误时退出码非 0。`fps_*` 衡量 rAF tick 率（`fps_semantics:"raf_tick"`），
不代表游戏渲染帧率。

> **无 GPU 环境的已知限制（TR-16.2）**：本仓库开发容器无 GPU，chromium headless 只能走
> SwiftShader 软件渲染；实测 Bevy 0.19 的 PBR 预通道顶点着色器在 SwiftShader/ANGLE-GL 下
> 编译失败（`Shader compilation failed` → `Caught rendering error: Validation Error` →
> `Quitting the application due to Internal RenderError`），页面白屏。冒烟脚本已把这类
> 渲染失败计入 `rendering_errors` 并以非 0 退出码反映。因此 **TR-16.2 的"画面正常渲染 +
> p50 FPS ≥ 30"必须到有 GPU 的环境复核**；本环境能给出的证据只有：wasm 产物可加载、
> 启动无 panic、应用逻辑在 headless 原生冒烟（`--headless`）下正确运行。

### 操作与质量档位

| 键 | 功能 |
|---|---|
| 鼠标移动 / 左键 | 视角 / 射击（进入对局后指针锁定） |
| `WASD` / `空格` / `Shift` | 移动 / 跳 / 潜墨 |
| `Enter` | 菜单 → 对局；结算 → 再来一局 |
| `Esc` / `P` | 暂停 / 恢复 |
| `M` | 静音开关 |
| `K` | **阴影开关**（太阳光级联阴影，尺寸随档位 1024/2048/4096） |
| `L` | **质量档位循环** low → medium → high → ultra（M4 Task 55） |
| `T` | **主题循环** day → sunset → golden（M4 Task 55） |

M4 Task 55 起，主题色/雾/太阳/环境光/阴影尺寸/角色 LOD/相机 Bloom 全部由
`rust/assets/environment.json`（从只读 JS 树提取）驱动；启动时可用
`INKWAVE_THEME=day|sunset|golden`、`INKWAVE_QUALITY=low|medium|high|ultra` 指定。
`low` 档 `post_processing_enabled=false`，是 SwiftShader/无 GPU 的保底可玩路径。
GTAO、GradeShader 12 uniform、SMAA、tilt-shift diorama、screenfx composite、
云烘焙/PMREM/远景反射、海面 swell·marina·泡沫仍是 GPU 视觉复核项，见
`.trae/specs/rust-bevy-port-m4/REVIEW_CHECKLIST.md`「Task 55」。

## 3. 联网运行（M2/M3）

联网协议为 PROTO v1，wire 格式与上游 JS 客户端（`src/net/`）**逐字节兼容**
（金标准 fixture `rust/assets/net-fixtures.json`），`server/` relay **零改动**：
Rust 客户端直连同一 relay。线上 relay 是 Cloudflare Worker（`server/`，本地
`wrangler dev` 起服——开发沙箱可达：Task 19 对真实 Worker 复跑 TR-19.1 场景 8/8 交叉验证
shim 语义；Task 27 实证 Rust 客户端形态的 Origin + PROTO v1 join 握手被真实 Worker 放行
并收到 welcome，见 `rust/tools/verify/logs/task27-wrangler-probe.txt`）；离线/测试用
**仅测试**的原生 shim（`rust/tools/relay-shim/`，语义复刻 `server/` 房间规则，
**不是部署替代**）。Rust 客户端直连真实 Worker 的完整对局路径与部署端 relay 未验证，
复核项见 `.trae/specs/rust-bevy-port-m2/REVIEW_CHECKLIST.md` §7。

### 本地 relay（测试 shim）

```bash
cd rust
cargo run -p relay-shim                  # ws://127.0.0.1:8787
cargo run -p relay-shim -- 8791          # 指定端口
```

### 双端连房（native ↔ native）

```bash
# 终端 2/3（rust/ 目录）：两个原生客户端同房
cargo run -p inkwave -- --relay=ws://127.0.0.1:8787   # 窗口 A：Play Online → Create → 记房间码
cargo run -p inkwave -- --relay=ws://127.0.0.1:8787   # 窗口 B：Play Online → 输入房间码 → Join
```

guest 在 lobby Ready、host Start 开新局。host 掉线 → 最老成员迁移接管；guest 掉线 →
host 以其最后可见状态转 bot（≡ JS session 语义）。

### wasm 客户端（浏览器）

`trunk serve`（第 2 节）起服后，浏览器打开：

```text
http://localhost:8080/?relay=ws://127.0.0.1:8787&net=create&netdur=60&netstart&autopilot&netprobe   # 建房
http://localhost:8080/?relay=ws://127.0.0.1:8787&netjoin=<CODE>&netready&autopilot&netprobe          # 加入
```

`net=create` / `netjoin` / `netready` / `netstart` / `netdur` / `autopilot` / `netprobe` 是
**test-only query 接缝**（`crates/inkwave/src/nettest.rs`，等价于 JS 侧 `__G.net` /
`?skipTitle&autopilot` 先例），向浏览器 console 打 `[nettest] boot/code/lobby/err/probe`
证据行供冒烟脚本断言；正常游玩不使用这些参数，走鼠标/键盘大厅 UI。
M3 起另有 loadout/模式四参数（`netweapon/netsub/netspecial/netmode`，见下文「跨端开黑」）。

### 联网冒烟（CDP headless，仓库根执行）

```bash
node rust/tools/verify/net-smoke.mjs --scenario all   # A: wasm↔wasm 全流程；B: JS↔Rust 跨端握手（无条件跑）；
                                                      # C: turf 跨端完整局；D: zones 跨端完整局（M3）
node rust/tools/verify/net-smoke.mjs --scenario A     # 单跑 A（--scenario A|C|D|all；B 始终随跑）
node rust/tools/verify/net-smoke.mjs --scenario C     # 单跑 C（loadout 跨端 turf 完整局）
node rust/tools/verify/net-smoke.mjs --scenario D     # 单跑 D（zones：JS 300 s 固定钟 + 加时，
                                                      # 终局等待预算内置 540 s；--secs 只控制 A/B 中段观察窗）
```

脚本自动拉起 shim 与静态服务器，以 `[nettest]` 证据行断言握手与远端 actor 移动；
证据落 `rust/tools/verify/logs/net-smoke-<utc>.json`。无 GPU 沙箱判 DEGRADED
（SwiftShader PBR 渲染限制，见第 2 节已知限制），真实 GPU 环境期望 PASS。

### 跨端（JS 客户端 ↔ Rust 客户端）

JS 客户端（`tools/serve.py`，只读）与 Rust 客户端可同房：wire 格式经金标准 fixture
（`rust/assets/net-fixtures.json`）逐字节一致。沙箱实据：JS 大厅实收 Rust 客人
（M2 `REVIEW_CHECKLIST.md` §3，房间 `3UZ9X`）。

### 跨端开黑（M3）

大厅 loadout 选择已落地：原生/wasm 大厅为 click-cycle 四行（WEAPON 12 / SUB 15+NONE /
SPECIAL 19+NONE / MODE turf⇄zones host-only），roster `{weapon,sub,special,style}` 实值
透传（style 仍透传不解释）。Rust wasm 测试缝四个 loadout 参数（`crates/inkwave/src/nettest.rs`
`parse_query` 实形）：

```text
?netweapon=<id>        主武器，id 见第 4 节 weaponOrder 清单；非法 id 由 sim 丢弃保留原值
                       （≡ JS session.js L201 WEAPONS[x] 守卫，缝不二次校验）
?netsub=<id|none>      副武器，id 见 subOrder；字面 none = 显式卸下（对齐 cycle UI 的 NONE 停靠位）
?netspecial=<id|none>  特殊武器，id 见 specialOrder；同 none 规则
?netmode=turf|zones    host-only 模式开关；其他值该参数作废
```

latch 后打 `[nettest] loadout w=<id> sub=<id|-> sp=<id|-> mode=<turf|zones>`（读 session
**生效值**，非法 id 显示旧武器）；局中 `[nettest] probe` 行带 `sub=`/`sp=` 部署计数；
终局打 `[nettest] result w=<0|1> cov=<a>/<b>`（zones 局追加
`zone=<c0>/<c1> reason=<id|-> pen=<p0>/<p1> ot=<0|1>`）。JS 客户端侧同口径走
`__G.net`（`tools/net-test.mjs` L75/L80 形态，只读）：`setMe({weapon,sub,special,ready})`
与 `setSettings({mode})`。

net-smoke scenario C（turf：JS host `charger/mine/crab` ↔ Rust guest `brolly/bomb/stamp`）
与 D（zones）断言跨端完整局：双端 roster 互认、终局 coverage 差 <0.5%、winner 一致、
Rust probe `sub>=1 && sp>=1`、无 err 线（create/join 重试吸收的连接失败不计）、
D 另逐项对比 zone 终局 `counts/penalty/reason/overtime`。
**沙箱三次实跑均 DEGRADED**（环境性：SwiftShader PBR 崩溃 + 内存压力，非代码回归，
spec D-T39-1；`[nettest] boot/loadout` 行已证 knobs 端到端解析）——跨端完整局断言的
首次真实执行登记于 `.trae/specs/rust-bevy-port-m3/REVIEW_CHECKLIST.md`（用户机器复核）；
进程内等价闭环由 harness `tr39_loadout_end_to_end`（FakeRelay 双 client + guest 端
sub/special 部署断言）兜底。

## 4. 武器与 Zone Control（M3）

12 主武器 / 15 副 / 19 特殊全部移植并 wired（Tasks 29–35），Zone Control 模式（Task 36/37），
saltpan + crossmarket 新图（Task 37），bot 三档（Task 38），大厅 loadout（Task 39，跨端开黑见第 3 节）。
数值全部走 `assets/tuning.json`（提取自 `src/config.js`，禁手抄）；武器 id 即其顶层
`weaponOrder`/`subOrder`/`specialOrder`（下列清单读自该文件）：

- **12 主武器**（`weaponOrder`，id == kind 字符串）：`shooter`（Spritzer）、`twins`、`brolly`、
  `blaster`、`spinner`、`charger`、`bow`、`roller`、`brush`、`blade`、`mitts`、`bucket`。
  retired 武器（`dualies`/`slosher`/`splatling`）不入产物（TR-28.2）。
- **15 副**（`subOrder`）：`bomb`、`sticky`、`burst`、`shaker`、`seeker`、`waddle`、`torpedo`、
  `tracer`、`boomerang`、`scan`、`curtain`、`sprinkler`、`mine`、`beacon`、`mist`。
- **19 特殊**（`specialOrder`，13 个 kind，`barrage`×5 共享主体）：`slam`、`storm`、`barrage`、
  `barrage_sticky`、`barrage_burst`、`barrage_seeker`、`barrage_mist`、`bubbler`、`sonar`、
  `strike`、`zooka`、`wail`、`kraken`、`blower`、`jetpack`、`stamp`、`booyah`、`zipcaster`、`crab`。

### Zone Control 玩法口径

规则实现在 `inkwave_sim::match::zones`（zones.js 全量平移）。**数值以 `assets/tuning.json`
的 `zones` 节为准**（17 字段，提取自 config.js L485-501），此处仅锚值示例：`duration=300`
（每局秒数，加时停钟）、`count=100`（中心区倒计时目标）、`rateCenter/rateHome/rateAway`
=1/0.5/2（按占方归属的扣速档）、`contest`/`control`/`warn`=0.4/0.8/0.3（争夺/占领/警戒
阈值）、`penaltyK=0.75`（penalty 公式系数）、`rotateMin/rotateMax`=30/60（side 区轮换
窗口）、`finalCentre=30`（终局中心锁定）、`overtimeGrace/overtimeMax`=10/300（加时）、
`sampleHz=5`（zone 涂地采样频率）。玩法：占住 active zone 推进倒计时，对方夺回转中立并按
公式扣 penalty；时间到进入加时，四出口（comeback/retake/neutralised/cap）或 sudden death；
终局结算 `counts/penalty/reason/overtime`（zones 结算形态，HUD 镜像 JS 文案）。zone 多边形
经提取管线合并入 `assets/maps/*.json` 的 `zones` 键（tidewater/saltpan/crossmarket 三图）；
side 区按 `onlyIn/notIn` 旗标随模式过滤（Task 37）。

### 无头测试与测量

```bash
cd rust
cargo test -p inkwave_sim                    # sim 固定种子单测：逐武器/副/特殊/zones 规则逐值断言，
                                             # 含 TR-5.4 handling 测量（口径 ≡ tools/measure-handling.mjs）
cargo test -p net-harness --test e2e_match   # 集成完整局：zones 三客户端（tr36）、saltpan/crossmarket
                                             # turf + zones（tr37）、loadout 端到端（tr39）、档位 ladder（tr38）
cargo test --workspace                       # 全量门禁（M3 收尾 553 passed）
cargo run --release -p inkwave -- --headless --frames 600   # 无 GPU 冒烟
```

bot 三档（`easy`/`normal`/`hard`，`tuning.json` 顶层 `difficulty` 节）大厅可选；bot 使用全部
12 主武器/15 副/19 特殊并在 zones 局执行 zone tactic（Task 38；ladder 单调性报告
`tools/verify/logs/task38-ladder-*.log`）。

## 5. 数据提取

Rust 侧所有关卡/数值 JSON 由 node 管线从上游 JS 求值生成，**禁止手抄数值**：

```bash
cd rust
bash tools/extract/run-all.sh     # 重新生成 tuning.json + tidewater/saltpan/crossmarket 三图（Task 40 起全量覆盖）
node tools/extract/extract-layout.mjs saltpan      # 单图按 stage 提取（Task 37 泛化）
```

- `extract-tuning.mjs`：`src/config.js`（PLAYER/WEAPONS 12/SUBS 15/SPECIALS 19/ZONES/DIFFICULTY…，
  schema v2）→ `assets/tuning.json`
- `extract-layout.mjs [stage]`：`src/world/stages/<stage>/layout.js`（mapkit DSL 在 node 侧求值）
  → `assets/maps/<stage>.json`；stage 缺省 `tidewater`，zones 多边形（`zones-data.js`）合并进 `zones` 键
- 产物确定性：同一 HEAD 连跑两次逐字节一致（TR-3.4/TR-28.1/TR-37.1 已验证）。`source.commit`
  provenance 戳有**两种口径**：
  ① **随 Rust HEAD 移动**（D-T37-3）：`extract-tuning.mjs`、layout/maps、texlib、props、
  `extract-character.mjs`（char）仍用 `git rev-parse HEAD`，故每次 Rust 提交后存量产物重跑会
  **一次性刷新戳**，门禁以 `DETERM=BYTE_IDENTICAL (rerun)` 留痕（Task 48/49/50/52/55 先例）；
  ② **钉上游 JS 数据文件的最后提交**：M4 新增的 `extract-environment.mjs` 与
  `extract-lobbyset.mjs` 用 `git log -1 -- <JS paths>`，Rust 侧提交不改变产物。
  `char` 属①不属②；内容字段稳定；改完跑 `cargo test -p inkwave_sim` 回归。
- 交叉验证：`bash tools/verify/run-crosscheck.sh`（JS 与 Rust 碰撞查询逐点对照）。

## 6. 上游同步

JS 树是 https://github.com/jaydendavisnc/inkwave 的零改动 fork（remote 名 `upstream`）。
Rust 无法 `git merge` JS，工作流为：

```bash
cd rust
bash tools/sync-upstream.sh check             # fetch + 列出基线之后的新提交（含受影响目录）
# 对照 PORT_MAP.md 逐项移植，并在 SYNC_LEDGER.md 逐行登记（pending/porting/done/wontport）
bash tools/sync-upstream.sh update-baseline   # 全部处理完才推进基线
```

- 当前基线：见 `SYNC_LEDGER.md` 与 `.upstream-baseline`；
- 调参/关卡数据改动优先重跑第 5 节提取管线（一般零手工代码）；
- `server/` relay 协议保持兼容；协议变更须同步 `inkwave_sim::net` 类型与 NET 文档注释。

## 门禁速查

M4 沿用九段门禁（fmt → clippy → test → wasm32 check → tree → DETERM → XCHK → trunk → headless → 上游树守卫），
每段退出码逐项落盘并自动判定 `GATE_VERDICT`；脚本为 `rust/tools/verify/gate-task47..56.sh`，
收尾矩阵见第 7 节。TRUNK 段在本沙箱常被 wasm-bindgen OOM `signal: 9` 杀掉，按 D-T40-1
判 DEGRADED（环境性、非阻塞），用户机器应一次通过。

```bash
cd rust
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace               # M4 基线逐任务单调递增（数字见 tasks.md / 门禁日志）
cargo check -p inkwave_sim -p inkwave --target wasm32-unknown-unknown
cargo tree -p inkwave_sim            # 依赖面不变（sim 零引擎依赖）
node tools/lobbyset-crosscheck.mjs   # 191 checks；其余独立对账见 gate 脚本 XCHK 段
bash tools/verify/gate-task56.sh     # 九段全跑，日志落 tools/verify/logs/
trunk build --release
cargo run --release -p inkwave -- --headless --frames 600   # 无 GPU 冒烟
cd .. && git status --short -- src server tools docs         # 上游树须为空
```

联网冒烟另在仓库根执行：`node rust/tools/verify/net-smoke.mjs --scenario all`
（沙箱 DEGRADED 语义见第 3 节）。

## 7. M4：Boss 战与表现完整度（Task 41–57，AC-M4-1..5）

M4 主线 = **玩法优先**：Phase A Boss 战（41–46）→ Phase B 玩法收尾（47–48）→
Phase C 表现完整度（49–56）→ 收尾（57）。完整设计、逐任务证据、偏差定案与复核清单见
`.trae/specs/rust-bevy-port-m4/{spec.md,tasks.md,REVIEW_CHECKLIST.md}`；
模块级对照见 [PORT_MAP.md](./PORT_MAP.md)「M4 完成核对」。

### Boss 战运行

```bash
cd rust
cargo run --release -p inkwave -- --headless --mode boss --frames 600  # 离线完整局
# 窗口态：Play 菜单按 MODE 切到 boss；在线房由 host 选 mode=boss（relay + 8 人名册）
# 仓库根另开终端：npm run relay
```

- 六招（slam / barrage / sweep / charge / crablets / frenzy）由
  `inkwave_sim::boss::{BossBrain, BossHazards}` 求值，几何由 move record + 时间全端一致；
  victim-side 伤害、host-only 涂墨、`['bm']`/`['bc']` 时间线、`'bhit'` 上行、host 迁移
  `adopt/handoff` 与 PROTO v1 逐字节兼容（`net/wire.rs`）。
- 呈现为**占位档**：体块剪影、telegraph 标记、HUD boss 条、结算 VICTORY/DEFEAT +
  bossDmg/weakHits 行、bossAudio。高保真模型属 Non-Goal；视觉复核见 REVIEW_CHECKLIST「Task 46」。

### 表现档位

| 档位 | 阴影 | pixelRatio | MSAA | Bloom | 角色 LOD | 3D 大厅 |
|---|---|---|---|---|---|---|
| low | 1024 | 0.75 | 0 | 关 | far | 关（极简 UI） |
| medium | 2048 | 1 | 2 | 开 | game | 开 |
| high | 4096 | 1.5 | 4 | 开 | hero | 开 |
| ultra | 4096 | 2 | 4 | 开 | hero | 开 |

`L` 循环档位、`T` 循环主题（day / sunset / golden）、`K` 切阴影；启动可用
`INKWAVE_QUALITY` / `INKWAVE_THEME`。运行期切档不重建天空渐变贴图与已生成角色 rig
（D-T55-⑦），下次关卡/spawn 生效。`low` 档关闭全部后处理，是软件渲染保底路径。
3D showcase 大厅在 `low` 档不生成；hub/lobby 屏复用主相机并覆盖为提取的
`hubCamera`/`CAM` 位姿（D-T56-⑦），关卡世界未做 RenderLayers 分层，属 GPU 复核项。

### 复核命令（用户 GPU 机器）

```bash
cd rust
bash tools/extract/run-all.sh                 # 全部 json/gz 提取产物重生成（连跑逐字节一致）
                                              # SVG 缩略图与 pinned fixture 是只读校验基准，收尾矩阵另核哈希
node tools/environment-crosscheck.mjs        # 93 checks（主题/档位）
node tools/lobbyset-crosscheck.mjs           # 191 checks（后巷/8 站位/pedestal/emotes）
node tools/props-crosscheck.mjs              # 8 图 props 对账
bash tools/verify/run-crosscheck.sh          # 碰撞几何独立 Python 对账（TR-4.1/4.2）
cargo build --release -p inkwave
for spec in day:low sunset:low golden:high day:ultra; do
  INKWAVE_THEME=${spec%%:*} INKWAVE_QUALITY=${spec##*:} \
    target/release/inkwave --headless --frames 600 || exit 1
done
target/release/inkwave --headless --mode boss --frames 600
trunk build --release                        # 本沙箱常 OOM SIGKILL；用户机应一次通过
```

视觉/听感复核清单（黄昏主题、逐档后处理、ink 湿润/光泽、46 层材质、props、
角色姿态、fx、小地图、音频、Boss 剪影、3D 大厅）：
`.trae/specs/rust-bevy-port-m4/REVIEW_CHECKLIST.md`。按 NFR-M4-5 **双轨验收**——
沙箱只验无头可测部分，GPU 项一律登记待复核，不做伪验收。

### 九段门禁矩阵

| 任务 | 最终日志 | 判定口径 |
|---|---|---|
| 41–53 | `logs/task41..53-gates-*.log` | 逐段 `*_EXIT=0`（脚本当时尚未引入自动 `GATE_VERDICT`）；TRUNK DEGRADED 的任务 41/42/43/44/45/49 按 D-T40-1 环境性 OOM 登记 |
| 54 | `logs/task54-gates-20261110T202656Z.log` | `GATE_VERDICT=PASS`（P0 修复后复跑；**TRUNK=DEGRADED**） |
| 55 | `logs/task55-gates-20261111T050724Z.log` | `GATE_VERDICT=PASS`（首轮 `source.commit` 戳刷新致 DETERM MISMATCH → FAIL，复跑 BYTE_IDENTICAL；**TRUNK=DEGRADED**） |
| 56 | `logs/task56-gates-20261111T060902Z.log` | `GATE_VERDICT=PASS`（独立评审 P1 全处置后复跑；**TRUNK=DEGRADED**） |
| 57 | `logs/task57-m4-matrix-*.log` | 收尾矩阵：文档命令实跑、41–56 日志汇总、上游树守卫；**以日志内 `M4_MATRIX_VERDICT` 为最终口径**，总评审 P0/P1 处置记录见 `tasks.md` Task 57 |

矩阵口径如实披露：41–56 共 **9 个任务的 TRUNK 段 DEGRADED**（41/42/43/44/45/49/54/55/56，
均为 wasm-bindgen `signal: 9` OOM，D-T40-1 环境性非阻塞），41–53 的
「逐段 `*_EXIT=0`」判定还包含 Task 49/50 已登记的「存量产物 `source.commit`
戳刷新 → `DETERM=BYTE_IDENTICAL (rerun)」先例；`gate-task57.sh` 的 `legacy_ok()`
对这两类例外做**显式证据要求**，无证据即判 FAIL，不做无差别吞错。
用户 GPU 机器应 `trunk build --release` 一次通过。

九段 = FMT / CLIPPY / TEST / WASM / TREE / DETERM / XCHK / TRUNK / HEADLESS，
外加 `JS_TREE_CLEAN` + `SIM_TREE_CLEAN` 守卫（NFR-M4-1/4：上游 JS 树与 sim 依赖面零改动）。
