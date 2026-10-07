# INKWAVE — Rust/Bevy 移植（M1）

INKWAVE（Splatoon-like 涂地对战）从 three.js 移植到 Rust + Bevy 0.19 的工作区。
本目录 `rust/` 与上游 JS 树（`../src`、`../server`、`../tools`）并存：
**JS 树永远零改动**，Rust 侧通过提取管线读取 JS 数据、通过 `tools/sync-upstream.sh` 跟踪上游提交。

M1 垂直切片：Tidewater 地图 + Spritzer 武器 + 涂地计分 + 4v4 easy bot + 墨汁渲染 + 菜单/HUD/结算 + 占位音效。
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
| `K` | **阴影开关**（M1 唯一质量档位：太阳光级联阴影 2048²，低配/软件渲染可关） |

M1 其余渲染为固定简化档（无 GTAO/bloom/SMAA/tilt-shift，材质为 pattern 最小子集）；
后处理与画质档位属于 M2+，见 PORT_MAP。

## 3. 数据提取

Rust 侧所有关卡/数值 JSON 由 node 管线从上游 JS 求值生成，**禁止手抄数值**：

```bash
cd rust
bash tools/extract/run-all.sh     # 重新生成 assets/tuning.json + assets/maps/tidewater.json
```

- `extract-tuning.mjs`：`src/config.js`（PLAYER/WEAPONS/MATCH…）→ `assets/tuning.json`
- `extract-layout.mjs`：`src/world/stages/tidewater/layout.js`（mapkit DSL 在 node 侧求值）→ `assets/maps/tidewater.json`
- 产物确定性：同一上游提交重复运行不产生 diff（TR-3.4 已验证）；改完跑 `cargo test -p inkwave_sim` 回归。
- 交叉验证：`bash tools/verify/run-crosscheck.sh`（JS 与 Rust 碰撞查询逐点对照）。

## 4. 上游同步

JS 树是 https://github.com/jaydendavisnc/inkwave 的零改动 fork（remote 名 `upstream`）。
Rust 无法 `git merge` JS，工作流为：

```bash
cd rust
bash tools/sync-upstream.sh check             # fetch + 列出基线之后的新提交（含受影响目录）
# 对照 PORT_MAP.md 逐项移植，并在 SYNC_LEDGER.md 逐行登记（pending/porting/done/wontport）
bash tools/sync-upstream.sh update-baseline   # 全部处理完才推进基线
```

- 当前基线：见 `SYNC_LEDGER.md` 与 `.upstream-baseline`；
- 调参/关卡数据改动优先重跑第 3 节提取管线（一般零手工代码）；
- `server/` relay 协议保持兼容；协议变更须同步 `inkwave_sim::net` 类型与 NET 文档注释。

## 门禁速查

```bash
cd rust
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo check --target wasm32-unknown-unknown -p inkwave_sim
cargo tree -p inkwave_sim            # 应仅 serde/serde_json/glam（sim 零引擎依赖）
trunk build --release
cd .. && git status --porcelain -- src tools server docs   # 上游树须为空
```
