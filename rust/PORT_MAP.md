# JS → Rust 模块对照表（PORT MAP）

上游 JavaScript 模块与 Rust 工作区模块的对应关系，供上游同步时快速定位。
状态：`M1`（本里程碑移植）· `M2+`（后续里程碑）· `skip`（Rust 侧不适用）。

## 技术栈基线

| 项 | 选型 | 备注 |
|---|---|---|
| 引擎 | Bevy **0.19.1**（固定） | wgpu 渲染；wasm 走 WebGL2/WebGPU |
| Rust | stable（rust-toolchain.toml），edition 2024 | 开发机 1.98 |
| 数学 | glam 0.30（sim 与 bevy 统一） | 坐标沿用上游：米、Y-up、Alpha -Z / Bravo +Z |
| API 注意 | Bevy 0.19 旧 Event 拆为 `Message`/`Event` | `MessageWriter<T>` 对应旧 `EventWriter` |
| 资产格式 | JSON（serde），由 node 提取管线从 JS ESM 生成 | 见 `tools/extract/`，禁止手抄数值 |

## M1 完成核对（Task 16）

Task 9–15 已闭环，上表所有标 `M1` 的行均已落地：sim 侧（物理/角色/武器 Spritzer/对战涂地/easy bot/nav/墨汁覆盖网格/net 类型骨架）与 bevy 侧（输入/相机/世界几何/墨汁图集渲染/角色体块占位/菜单-HUD-结算 UI/占位音效）。标 `M2+` 的行为**未移植**，逐项范围见各行"移植备注"；标 `skip` 的 Rust 侧不适用。运行入口与门禁见 [README.md](./README.md)。

## 质量档位

M1 渲染为**固定简化档**，唯一运行时可调项：

| 档位 | 控制 | 说明 |
|---|---|---|
| 太阳光级联阴影 | `K` 键开关（`world.rs::toggle_shadows`，`DirectionalLightShadowMap` 2048²） | 低配/软件渲染（SwiftShader）建议关闭 |

编译期固定简化（M2+ 再补，均标于上表）：无后处理链（GTAO/bloom/grade/SMAA/tilt-shift diorama）、程序化材质仅 pattern 最小子集、角色为体块占位（完整弹簧姿态绑定 M2+）、环境为白天简化（黄昏/海况/灯光烘焙 M2+）、HUD 仅核心元素（小地图 M2+）、音效为过程化占位。

## `src/` 玩法核心

| JS 模块 | Rust 目标 | 状态 | 移植备注 |
|---|---|---|---|
| `src/config.js`（PLAYER/WEAPONS/MATCH/SUBS/...） | `inkwave_sim::tuning` + `assets/tuning.json` | M1 | 仅提取 M1 所需字段；其余武器随里程碑扩充 |
| `src/game/physics.js` | `inkwave_sim::physics` | M1 | 地面探测/胶囊 sweep/推出 |
| `src/game/actor.js` | `inkwave_sim::actor` | M1 | 角色状态机；移动常量逐一对齐 PLAYER |
| `src/game/player.js` | `inkwave` crate 输入映射 + actor 驱动 | M1 | 键盘鼠标；手柄 M2+ |
| `src/game/weapons.js`（Spritzer 分支） | `inkwave_sim::weapon::shooter` | M1 | 其他 11 把武器 M2+ |
| `src/game/match.js` | `inkwave_sim::match` | M1 | intro/active/end；涂地计分；zones M2+ |
| `src/game/zones.js` | `inkwave_sim::match::zones` | M2+ | Zone Control 规则 |
| `src/game/bots.js`（easy 参数） | `inkwave_sim::bot` | M1 | 仅 easy；normal/hard M2+ |
| `src/game/nav.js` | `inkwave_sim::nav` | M1 | 顶面连通航点图的简化实现 |
| `src/game/specials.js` / `subs.js` / `kits/*` | `inkwave_sim::weapon::{sub,special,kit}` | M2+ | 15 副 + 19 特殊 + kit 武器 |
| `src/game/cameraRig.js` | `inkwave::camera` | M1 | 简化第三人称；tilt-shift diorama M2+ |
| `src/game/minimap.js` | `inkwave::ui::minimap` | M2+ | M1 HUD 只显百分比 |
| `src/game/character.js`（4062 行，弹簧姿态系统） | `inkwave::render::character` | M1（占位） | M1 体块占位；完整过程化绑定 M2+ |
| `src/game/character-geo/hair/face/outfit/...` | `inkwave::render::character::*` | M2+ | 更衣室相关，locker 前不做 |
| `src/game/showcase.js` / `lobbySet*` | — | M2+ | 线上大厅场景 |
| `src/game/special-props.js` | `inkwave_sim::world::props` | M2+ | |

## `src/world/` 关卡与墨汁

| JS 模块 | Rust 目标 | 状态 | 移植备注 |
|---|---|---|---|
| `src/world/mapkit.js`（B/R/O/OCT/ARC DSL） | `tools/extract/extract-layout.mjs`（JS 侧求值）→ `inkwave_sim::geometry` | M1 | 图元在 node 侧求值后序列化，Rust 不重写 DSL |
| `src/world/maps.js` | `assets/maps/*.json` 索引 | M1 | M1 仅 tidewater |
| `src/world/stages/tidewater/layout.js` | `assets/maps/tidewater.json` | M1 | 含 half 镜像展开与 meta 统计 |
| `src/world/stages/tidewater/surfaces.js` | 提取到 layout JSON 的 color/pattern + `inkwave::render::material` | M1 | 程序化材质简化 |
| `src/world/stages/tidewater/props.js`（2093 行） | `inkwave::render::props` | M2+ | M1 只保留影响碰撞/地标的体块 |
| `src/world/stages/tidewater/murals.js` | — | M2+ | 装饰贴花 |
| `src/world/stages/{kelpline,halyard,saltpan,crossmarket,lockgate,terraces,cargo}` | 同名 assets + 渲染 | M2+ | 数据管线 M1 已通用，新增地图以数据为主 |
| `src/world/level.js` | `inkwave_sim::world`（碰撞）+ `inkwave::render::level` | M1 | |
| `src/world/paint.js`（929 行） | `inkwave_sim::paint`（覆盖网格/计分）+ `inkwave::render::ink`（图集） | M1 | 架构照搬：每面独立二维网格 |
| `src/world/inkShading.js`（314 行 GLSL） | `inkwave::render::ink` wgpu 材质 | M1（简化） | 高度/光泽/湿润 M2+ |
| `src/world/levelMaterial.js`（719 行） | `inkwave::render::material` | M1（简化） | pattern 贴图做最小子集 |
| `src/world/texlib.js` / `decor.js` / `dressing.js` / `murals.js` / `mapThumb.js` / `props*.js` | `inkwave::render::*` | M2+ | |
| `src/world/environment.js`（3116 行） | `inkwave::render::environment` | M1（白天简化） | 黄昏主题、海况、灯光烘焙 M2+ |
| `src/world/variants.js` / `zones-data.js` | `assets/maps/*.json`（mode variants） | M2+ | |

## `src/` 其他

| JS 模块 | Rust 目标 | 状态 | 移植备注 |
|---|---|---|---|
| `src/core/renderer.js`（three.js + 后处理） | Bevy/wgpu 渲染管线（架构替代） | M1（清屏/基础光照） | GTAO/bloom/grade/SMAA M2+ |
| `src/core/input.js` | Bevy `ButtonInput` + `inkwave::input` | M1 | |
| `src/core/ctx.js`（全局事件总线 G） | Bevy Resource/Message | M1 | |
| `src/main.js`（1399 行引导/状态流） | `inkwave` App + states | M1（最小） | 主菜单流程精简 |
| `src/fx/*` | `inkwave::render::fx` | M2+ | M1 枪口闪光等极少量占位 |
| `src/ui/hud.js`（2073 行） | `inkwave::ui::hud`（bevy_ui） | M1（核心元素） | 比分/时钟/墨箱/准星；其余 M2+ |
| `src/ui/menus.js` 等 | `inkwave::ui::menu` | M1（Play/暂停/结算） | |
| `src/audio/audio.js`（2505 行 WebAudio 合成） | `inkwave::audio` | M1（占位音效） | 过程化合成 M2+ |
| `src/audio/music.js` / `bossAudio.js` | `inkwave::audio::music` | M2+ | |
| `src/boss/*` | `inkwave_sim::boss` + 渲染 | M2+ | Boss 战 |
| `src/net/netmatch.js`（825 行） | `inkwave_sim::net`（M1 类型/文档骨架） | M1（骨架） | PROTO v1 字段逐项注释；实现在 M2 |
| `src/net/session.js` / `transport.js` / `mock.js` | `inkwave::net` | M2+ | relay 复用，协议不改 |

## 仓库其他部分

| 路径 | 处理 | 状态 |
|---|---|---|
| `server/`（Cloudflare Worker relay，144 行） | **保留不改**；Rust 客户端 M2 直连现有 relay | M2 对接 |
| `electron/` | skip — 桌面端即原生 Rust 可执行文件 | skip |
| `tools/serve.py` | `trunk serve`（wasm）/ 直接运行（native） | skip |
| `tools/botlab/*` | `inkwave --headless` autopilot + JSON 输出 | M1（等价最小版） |
| `tools/measure-handling.mjs` | sim 无头测量（TR-5.4 对照口径） | M1 |
| `tools/net-test.mjs` / `tools/smoke.sh` | Rust 冒烟/网络一致性测试 | M2（net-test）/M1（smoke） |
| `build/*`、`tools/build-dist.py`、`tools/release-pages.sh` | cargo/trunk/未来发布脚本替代 | skip/M2+ |
