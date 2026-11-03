# JS → Rust 模块对照表（PORT MAP）

上游 JavaScript 模块与 Rust 工作区模块的对应关系，供上游同步时快速定位。
状态：`M1`（M1 移植）· `M2 done`（M2 移植）· `M3 Task n`（M3 移植，见 M3 完成核对）· `M2+`（未移植，顺延 M4）· `skip`（Rust 侧不适用）。

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

## M2 完成核对（Task 27）

Task 17–26 已闭环：联网链路全栈落地——codec（逐字节金标准）、双端 transport（native tungstenite 线程 / wasm web-sys）、NetSession 状态机、权威切分与 host 职责、播放层（per-sender 时钟 + Hermite + correction）、事件时间线重放、极简大厅 UI、Rust↔Rust 端到端完整局、浏览器联网冒烟与 JS↔Rust 跨端握手。上表标 `M2 done` 的行均已落地；`server/` 与 JS 树零改动（门禁实核）。M2 未做的行（zones、sub/special/kit 武器、showcase 大厅、小地图、完整角色绑定、后处理链、boss 等）维持 `M2+`，顺延 M3+。需真实 GPU/桌面环境复核的联网条目登记于 `.trae/specs/rust-bevy-port-m2/REVIEW_CHECKLIST.md`。

## M3 完成核对（Task 40）

Task 28–39 已闭环：内容与模式侧全量落地——tuning 全量提取（Task 28，schema v2：WEAPONS 12/SUBS 15/SPECIALS 19/ZONES/DIFFICULTY，锚定单测锁值）；12 主武器全部 wired（Tasks 29–33，net tick 槽 `ch`/`lock`/`ks` 与 `['k']` ghost 语义对齐 JS packer）；15 副 + 设备网络语义 `'dh'`/`['b']`（Task 34）；19 特殊 + `sp`/`spx`/`spst` 实值化（Task 35）；Zone Control 模式（Task 36：规则/`['z']` 8 事件广播/session 放开 zones、boss 仍拒；Task 37：zones 多边形合并与 onlyIn/notIn 按模式过滤）；saltpan/crossmarket 新图（Task 37，提取器泛化为 stage CLI，三图 stable-json 逐字节）；bot normal/hard 档位 + 逐 kind 武器/副/特殊使用 + zone tactic（Task 38）；大厅 loadout UI 与跨端开黑接缝（Task 39；**跨端完整局 scenario C/D 沙箱 DEGRADED，首次真实执行登记于 `.trae/specs/rust-bevy-port-m3/REVIEW_CHECKLIST.md`**）。上表标 M3 的行均已落地；`server/` 与 JS 树零改动、`cargo tree -p inkwave_sim` 依赖面不变（门禁实核，workspace 553 passed，日志 `rust/tools/verify/logs/task39-gates-20261103T073400Z.log`）。M3 未做的行（showcase 大厅、小地图、完整角色绑定、后处理链、boss、props/murals 装饰、flood 呈现层、threat AI 等）顺延 M4，逐项范围见各行"移植备注"。

## 质量档位

M1 渲染为**固定简化档**，唯一运行时可调项：

| 档位 | 控制 | 说明 |
|---|---|---|
| 太阳光级联阴影 | `K` 键开关（`world.rs::toggle_shadows`，`DirectionalLightShadowMap` 2048²） | 低配/软件渲染（SwiftShader）建议关闭 |

编译期固定简化（M2+ 再补，均标于上表）：无后处理链（GTAO/bloom/grade/SMAA/tilt-shift diorama）、程序化材质仅 pattern 最小子集、角色为体块占位（完整弹簧姿态绑定 M2+）、环境为白天简化（黄昏/海况/灯光烘焙 M2+）、HUD 仅核心元素（小地图 M2+）、音效为过程化占位。

## `src/` 玩法核心

| JS 模块 | Rust 目标 | 状态 | 移植备注 |
|---|---|---|---|
| `src/config.js`（PLAYER/WEAPONS/MATCH/SUBS/...） | `inkwave_sim::tuning` + `assets/tuning.json` | M1/M3 Task 28 | M1 仅提取所需字段；Task 28 全量提取 schema v2：WEAPONS 12（按 `WEAPON_ORDER` 投影，retired dualies/slosher/splatling 不入产物）/SUBS 15/SPECIALS 19/ZONES 17 字段/DIFFICULTY 三档/weaponRange 预计算表/TEAM_PALETTES，锚定单测逐值锁 config.js 行号 |
| `src/game/physics.js` | `inkwave_sim::physics` | M1 | 地面探测/胶囊 sweep/推出 |
| `src/game/actor.js` | `inkwave_sim::actor` | M1/Task 33 | 角色状态机；移动常量逐一对齐 PLAYER；Task 33 扩展 `Form::Cling/Hang` 贴墙态 + `KitArmor` 每帧快照（mitts guard 弧/leap 甲，JS `damageTaken` L883-903）；偏差声明：`busy()` 未接 squid dive（L282）/idle refill（L358）——`Actor::step` 无 runner 句柄，12 武器共性，留 follow-up |
| `src/game/player.js` | `inkwave` crate 输入映射 + actor 驱动 | M1 | 键盘鼠标；手柄 M2+ |
| `src/game/weapons.js`（WeaponRunner 分发 + Spritzer 分支） | `inkwave_sim::weapon` | M1/Tasks 29–33 | kind 枚举/分发骨架与弹丸泛化（Task 29）；twins/blaster/brolly 接线完成（Task 30，brolly 行为在 `kits/brolly.js`→`weapon/brolly.rs`，含伞挡弹/发射伞/合并一击）；charger hitscan+蓄力、spinner burst 流、bucket flick windup+抛洒弧 volley 接线完成（Task 31，含 `['ev']` charge/len、F.charging/F.flick、volley 穿透/belowFalloff）；roller 滚动带墨+flick 抛墨、brush 拖墨细线+侧扫泼溅接线完成，blade kit 行为在 `kits/blade.js`→`weapon/blade.rs`（Task 32，tap 新月/重击波/近战弧/ghost 波经 `SimEvent::Kit`→`TickEvent::Kit` 闭环）；bow/mitts kit 行为在 `kits/bow.js`→`weapon/bow.rs`、`kits/mitts.js`→`weapon/mitts.rs`（Task 33，12 把主武器全部 wired，net-harness Task 30 bot-kind shim 已移除） |
| `src/game/match.js` | `inkwave_sim::match` | M1/M3 Task 36 | intro/active/end；涂地计分；Task 36 落 `mode`（Turf/Zones）分支：zones 时长/时钟停加时/`timeUp` host 判定/`endZones`/`_zoneTurf`/zones 结算形态；boss 分支仍 M2+ |
| `src/game/zones.js` | `inkwave_sim::match::zones` | M3 Task 36 | Zone Control 规则全量平移（zoneCells/insidePoly/flood 联动/占领-争夺-中立/penalty 公式/轮换/终局中心锁定/加时四出口/`state()`）；`['z']` 8 事件录制+重放（host 权威/guest 预测计数）；session 放开 zones 模式（boss 仍拒）；tidewater zones 经 `extract-layout.mjs` 合并 `ZONE_DEFS` 入 `assets/maps/tidewater.json`；flood 呈现层（atlas 带绘/ripple/ZoneMarks）顺延 M4 |
| `src/game/bots.js` | `inkwave_sim::bot`（+`bot::zones` ZonePlan） | M1/M3 Task 38 | Task 38 全量：档位接线（`DifficultySet::get` ≡ L276 `DIFFICULTY[d]||normal`，cfg.difficulty 经 `bot_difficulty` 入脑）；12 kind 逐 kind fight/paint/refill/tail（lead/pref/strafe 门/MELEE 冲脸/twins planted/开火分发/charge keep/游泳/dodge/sweep/scan reach/pitch 梯/spinner-charger 节奏）；家族表 MELEE/CHARGES/LONG/PAINTERS/THROWN 含 8 条 kits 运行时补丁（blade/mitts∈MELEE、bow∈CHARGES/LONG、waddle/torpedo/shaker/boomerang∈THROWN）；`_fightSub`/`_paintSub`（L950-983）+ SUB_KITS 五件 bot 钩子 + bombCd 两帧释放链；`_wantSpecial`/`_specialCtl`（L831-948，cheer/stamp 否决/九 kind 驱动）；ZonePlan（L68-268）+ zone 手臂（`_zoneSync/_zoneEngage/_zoneHoldGround/_pickZoneGoal/_zoneAim/_zoneSpecial/_zoneBombAim`、strike 热点 retarget）。顺延：threat AI（L1288+/L1883-2050）与 boss 段、`_zoneJump`（无 superJump）、climb 边（M1 偏差）、`_shieldFight`/`_canopyCtl`、mitts leap/wall-cling、blade 深战术、stamp threat-steer（L460-470） |
| `src/game/nav.js` | `inkwave_sim::nav` | M1 | 顶面连通航点图的简化实现 |
| `src/game/specials.js` / `subs.js` / `kits/*` | `inkwave_sim::sub`（+`sub::{shaker,waddle,torpedo,tracer,boomerang}`）/ `inkwave_sim::special`（+`special::*`）/ `weapon::kit` | M3 Tasks 34–35 | 15 副 + 19 特殊 + kit 武器（`subs.js` 已随 Task 34 落 `sub.rs`（SubSystem 核心 + 10 非 kit 设备 + `'dh'`/`['b']`/`['k','subs']` 网络语义）与 `sub/*.rs` 五 kit sub；`kits/brolly.js` 已随 Task 30 落 `weapon/brolly.rs`，`kits/blade.js` 已随 Task 32 落 `weapon/blade.rs`，`kits/bow.js`/`kits/mitts.js` 已随 Task 33 落 `weapon/bow.rs`/`weapon/mitts.rs`；`specials.js` 已随 Task 35 落 `special.rs` + `special/*.rs`（19 特殊全 wired：充能 gauge 与消耗、`sp`/`spx`/`spst` tick 槽实值化、`['ev']` special:use/slam 事件、ghost/replication 对账、`special/world.rs` 世界侧交互）；Task 38 另落 `MAIN_KITS[k].bot` 钩子（brolly/blade/bow/mitts 的 fight/paint/paintPitch/stayIn/targetBias → `bot.rs`；mitts tactics 浅段，深段裁剪见 PORT_MAP bots.js 行）与 `SUB_KITS[k].bot` 五件 |
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
| `src/world/maps.js` | `assets/maps/*.json` 索引（`inkwave_sim::embedded_layout(id)`） | M1/M3 Task 37 | tidewater + saltpan + crossmarket 三图；zones 合并口径 L118-119 走提取器（Task 36） |
| `src/world/stages/tidewater/layout.js` | `assets/maps/tidewater.json` | M1 | 含 half 镜像展开与 meta 统计 |
| `src/world/stages/tidewater/surfaces.js` | 提取到 layout JSON 的 color/pattern + `inkwave::render::material` | M1 | 程序化材质简化 |
| `src/world/stages/tidewater/props.js`（2093 行） | `inkwave::render::props` | M2+ | M1 只保留影响碰撞/地标的体块 |
| `src/world/stages/tidewater/murals.js` | — | M2+ | 装饰贴花 |
| `src/world/stages/{saltpan,crossmarket}/layout.js` | `assets/maps/{saltpan,crossmarket}.json` | M3 Task 37 | 提取器已泛化（stage CLI 参数）；含 onlyIn/notIn 变体旗标与 zones-data 合并；surfaces 槽位 34-39 入 meta；props/murals 装饰顺延 M4 |
| `src/world/stages/{kelpline,halyard,lockgate,terraces,cargo}` | 同名 assets + 渲染 | M2+ | 数据管线 M1 已通用，新增地图以数据为主 |
| `src/world/level.js` | `inkwave_sim::world`（碰撞）+ `inkwave::render::level` | M1 | |
| `src/world/paint.js`（929 行） | `inkwave_sim::paint`（覆盖网格/计分）+ `inkwave::render::ink`（图集） | M1 | 架构照搬：每面独立二维网格 |
| `src/world/inkShading.js`（314 行 GLSL） | `inkwave::render::ink` wgpu 材质 | M1（简化） | 高度/光泽/湿润 M2+ |
| `src/world/levelMaterial.js`（719 行） | `inkwave::render::material` | M1（简化） | pattern 贴图做最小子集 |
| `src/world/texlib.js` / `decor.js` / `dressing.js` / `murals.js` / `mapThumb.js` / `props*.js` | `inkwave::render::*` | M2+ | |
| `src/world/environment.js`（3116 行） | `inkwave::render::environment` | M1（白天简化） | 黄昏主题、海况、灯光烘焙 M2+ |
| `src/world/variants.js` / `zones-data.js` | `inkwave_sim::geometry`（`in_mode`/`StageLayout::for_mode`）+ `collision::from_layout_for`；zones 入 `assets/maps/*.json` | M3 Task 36/37 | zones-data 合并（Task 36）；onlyIn/notIn 旗标随行入 JSON、按模式过滤保持 block id 位置序（Task 37） |

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
| `src/net/netmatch.js`（825 行） | `inkwave_sim::net::{wire,session,replication,playback}`（事件重放在 `replication::play_events`） | M2 done | codec 与 JS packer 逐字节一致（金标准 `assets/net-fixtures.json`，Task 17）；session/replication/playback/replay 逐字平移（Task 20–25），Rust↔Rust 完整局一致性 TR-25.1 |
| `src/net/transport.js` | `inkwave_sim::net::transport`（trait + FakeTransport）/ `inkwave::net::transport`（native + wasm） | M2 done | **选型（spec OQ-2）**：native = `tungstenite` 0.26 同步 API + 独立线程 + `std::mpsc`（Bevy 主线程零阻塞），TLS 走 `rustls-tls-webpki-roots`（纯 Rust、离线可解析）；弃 `fastwebsockets`（tokio 异步栈，违反 sim/最小依赖纪律）。wasm = `web-sys` WebSocket。必须发合规 `Origin` 头过 relay ORIGIN_OK |
| `src/net/session.js` / `mock.js` | `inkwave_sim::net::session` + `inkwave::net`（transport 接线）+ `inkwave::ui::online`（极简大厅） | M2/Task 39 | relay 复用、协议不改（`server/` 零改动）；大厅 UI 为极简版（Task 24），showcase 3D 大厅场景未做（M2+）；Task 39：lobby loadout cycle 选择（WEAPON 12/SUB 15+NONE/SPECIAL 19+NONE）+ MODE turf⇄zones host-only，roster `{weapon,sub,special,style}` 经 `set_me`/`hostSet` 实值透传（JS L200-206/L244/L279），`matchbuild` 按 actor.js L38 falsy 链接线 `a.sub` |

## 仓库其他部分

| 路径 | 处理 | 状态 |
|---|---|---|
| `server/`（Cloudflare Worker relay，144 行） | **保留不改**；Rust 客户端直连现有 relay（协议兼容经 fixture 逐字节 + shim 语义复刻 + JS↔Rust 跨端冒烟验证；wrangler/部署 relay 复核见 REVIEW_CHECKLIST） | M2 验证 |
| `electron/` | skip — 桌面端即原生 Rust 可执行文件 | skip |
| `tools/serve.py` | `trunk serve`（wasm）/ 直接运行（native） | skip |
| `tools/botlab/*` | `inkwave --headless` autopilot + JSON 输出 | M1（等价最小版） |
| `tools/measure-handling.mjs` | sim 无头测量（TR-5.4 对照口径） | M1 |
| `tools/net-test.mjs` / `tools/smoke.sh` | Rust 等价物 `rust/tools/verify/net-smoke.mjs` / `wasm-smoke.mjs` | M2 done · M3 Task 39（net-smoke 扩 scenario C/D 跨端完整局；JS 基线工具只读未动） |
| `build/*`、`tools/build-dist.py`、`tools/release-pages.sh` | cargo/trunk/未来发布脚本替代 | skip/M2+ |
| （Rust 新增）`rust/tools/net-fixtures/gen.mjs` | 金标准捕获：node 侧 import 真实 `src/net/` packer → `rust/assets/net-fixtures.json` | M2 done（Task 17）· M3 Task 36（+ `zone_seq` 257 条逐字节稳定） |
| （Rust 新增）`rust/tools/relay-shim/` | 仅测试用原生 relay（复刻 `server/` 房间语义：fan-out/选举/lock/ping/sweep；**非部署替代**） | M2 done（Task 19） |
| （Rust 新增）`rust/tools/net-harness/` | 网络集成测试 harness（FakeRelay + 虚拟时钟；`tests/acceptance.rs`、`tests/e2e_match.rs` 完整局） | M2 done（Task 19/25）· M3 Tasks 36–39（zones/新图/loadout/ladder e2e scenario） |
