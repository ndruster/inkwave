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
| `src/config.js`（PLAYER/WEAPONS/MATCH/SUBS/...） | `inkwave_sim::tuning` + `assets/tuning.json` | M1/M3 Task 28 | M1 仅提取所需字段；Task 28 全量提取 schema v2：WEAPONS 12（按 `WEAPON_ORDER` 投影，retired dualies/slosher/splatling 不入产物）/SUBS 15/SPECIALS 19/ZONES 17 字段/DIFFICULTY 三档/weaponRange 预计算表/TEAM_PALETTES，锚定单测逐值锁 config.js 行号；Task 41 升 schema v3：`BOSS`/`BOSS_MODE`/moves 时序与 hazard 参数/hitShapes 球表（提取源扩 `src/boss/*.js`，锚定单测逐值锁 boss.js/bossMode.js/bossHazards.js 行号）；Task 42 升 schema v4：`hit[].pos` rest 模型空间坐标（纯平移骨链静态求值 + `tools/verify/boss-hitpos-crosscheck.mjs` 真 THREE 动态骨链交叉校验）+ `nav.planWall/planFloor/stride`（bossNav.js L12-14）；Task 43 升 schema v5：`brain.pref`（bossBrain.js L16 各阶段目标偏好距离）；Task 44 升 schema v6：`MAPS` 旗标（`mapBossOk`/`bossFallbackMap`/`noBotsStartBlock`，config.js L515-545 + session.js L244-249 语义，锚定单测逐值锁行号；`embedded_maps()` 当前 3 项，JS 8 项，Task 48 提取后对齐） |
| `src/game/physics.js` | `inkwave_sim::physics` | M1 | 地面探测/胶囊 sweep/推出 |
| `src/game/actor.js` | `inkwave_sim::actor` | M1/Task 33 | 角色状态机；移动常量逐一对齐 PLAYER；Task 33 扩展 `Form::Cling/Hang` 贴墙态 + `KitArmor` 每帧快照（mitts guard 弧/leap 甲，JS `damageTaken` L883-903）；偏差声明：`busy()` 未接 squid dive（L282）/idle refill（L358）——`Actor::step` 无 runner 句柄，12 武器共性，留 follow-up |
| `src/game/player.js` | `inkwave` crate 输入映射 + actor 驱动 | M1 | 键盘鼠标；手柄 M2+ |
| `src/game/weapons.js`（WeaponRunner 分发 + Spritzer 分支） | `inkwave_sim::weapon` | M1/Tasks 29–33 | kind 枚举/分发骨架与弹丸泛化（Task 29）；twins/blaster/brolly 接线完成（Task 30，brolly 行为在 `kits/brolly.js`→`weapon/brolly.rs`，含伞挡弹/发射伞/合并一击）；charger hitscan+蓄力、spinner burst 流、bucket flick windup+抛洒弧 volley 接线完成（Task 31，含 `['ev']` charge/len、F.charging/F.flick、volley 穿透/belowFalloff）；roller 滚动带墨+flick 抛墨、brush 拖墨细线+侧扫泼溅接线完成，blade kit 行为在 `kits/blade.js`→`weapon/blade.rs`（Task 32，tap 新月/重击波/近战弧/ghost 波经 `SimEvent::Kit`→`TickEvent::Kit` 闭环）；bow/mitts kit 行为在 `kits/bow.js`→`weapon/bow.rs`、`kits/mitts.js`→`weapon/mitts.rs`（Task 33，12 把主武器全部 wired，net-harness Task 30 bot-kind shim 已移除） |
| `src/game/match.js` | `inkwave_sim::match` | M1/M3 Task 36/M4 Task 44 | intro/active/end；涂地计分；Task 36 落 `mode`（Turf/Zones）分支：zones 时长/时钟停加时/`timeUp` host 判定/`endZones`/`_zoneTurf`/zones 结算形态；Task 44 落 boss 分支（match.js L20-21/L55-62/L100/L193/L216/L229/L262 实读平移：`MatchMode::Boss` 单队构造/`BossMode` 驱动层 wrapper/finish 窗口/judge 分支/result boss 形态）；`B` 快照/`bm`/`bc`/res 6 宽行（Task 41 已备） |
| `src/game/zones.js` | `inkwave_sim::match::zones` | M3 Task 36 | Zone Control 规则全量平移（zoneCells/insidePoly/flood 联动/占领-争夺-中立/penalty 公式/轮换/终局中心锁定/加时四出口/`state()`）；`['z']` 8 事件录制+重放（host 权威/guest 预测计数）；session 放开 zones 模式（boss 仍拒）；tidewater zones 经 `extract-layout.mjs` 合并 `ZONE_DEFS` 入 `assets/maps/tidewater.json`；flood 呈现层（atlas 带绘/ripple/ZoneMarks）顺延 M4 |
| `src/game/bots.js` | `inkwave_sim::bot`（+`bot::zones` ZonePlan、+`bot::boss` M4 Task 44） | M1/M3 Task 38/M4 Task 44 | Task 38 全量：档位接线（`DifficultySet::get` ≡ L276 `DIFFICULTY[d]||normal`，cfg.difficulty 经 `bot_difficulty` 入脑）；12 kind 逐 kind fight/paint/refill/tail（lead/pref/strafe 门/MELEE 冲脸/twins planted/开火分发/charge keep/游泳/dodge/sweep/scan reach/pitch 梯/spinner-charger 节奏）；家族表 MELEE/CHARGES/LONG/PAINTERS/THROWN 含 8 条 kits 运行时补丁（blade/mitts∈MELEE、bow∈CHARGES/LONG、waddle/torpedo/shaker/boomerang∈THROWN）；`_fightSub`/`_paintSub`（L950-983）+ SUB_KITS 五件 bot 钩子 + bombCd 两帧释放链；`_wantSpecial`/`_specialCtl`（L831-948，cheer/stamp 否决/九 kind 驱动）；ZonePlan（L68-268）+ zone 手臂（`_zoneSync/_zoneEngage/_zoneHoldGround/_pickZoneGoal/_zoneAim/_zoneSpecial/_zoneBombAim`、strike 热点 retarget）。Task 44 boss 段：`_bossTick`（L1874-2050 逐字平移：think 门/hz.threat/evade/_bossGoal/stunned rush/crablet pop/清墨）+ `_bossPerceive`（L1982-2022）+ `_bossGoal`（L2026-2067）+ `_bossEvade`（L2070-2087）+ 4 测试（crablet 感知/refill 门/slam 反应/集火选形）。顺延：threat AI（L1288+）、`_zoneJump`（无 superJump）、climb 边（M1 偏差）、`_shieldFight`/`_canopyCtl`、mitts leap/wall-cling、blade 深战术、stamp threat-steer（L460-470） |
| `src/game/nav.js` | `inkwave_sim::nav` | M1 | 顶面连通航点图的简化实现 |
| `src/game/specials.js` / `subs.js` / `kits/*` | `inkwave_sim::sub`（+`sub::{shaker,waddle,torpedo,tracer,boomerang}`）/ `inkwave_sim::special`（+`special::*`）/ `weapon::kit` | M3 Tasks 34–35 | 15 副 + 19 特殊 + kit 武器（`subs.js` 已随 Task 34 落 `sub.rs`（SubSystem 核心 + 10 非 kit 设备 + `'dh'`/`['b']`/`['k','subs']` 网络语义）与 `sub/*.rs` 五 kit sub；`kits/brolly.js` 已随 Task 30 落 `weapon/brolly.rs`，`kits/blade.js` 已随 Task 32 落 `weapon/blade.rs`，`kits/bow.js`/`kits/mitts.js` 已随 Task 33 落 `weapon/bow.rs`/`weapon/mitts.rs`；`specials.js` 已随 Task 35 落 `special.rs` + `special/*.rs`（19 特殊全 wired：充能 gauge 与消耗、`sp`/`spx`/`spst` tick 槽实值化、`['ev']` special:use/slam 事件、ghost/replication 对账、`special/world.rs` 世界侧交互）；Task 38 另落 `MAIN_KITS[k].bot` 钩子（brolly/blade/bow/mitts 的 fight/paint/paintPitch/stayIn/targetBias → `bot.rs`；mitts tactics 浅段，深段裁剪见 PORT_MAP bots.js 行）与 `SUB_KITS[k].bot` 五件 |
| `src/game/cameraRig.js` | `inkwave::camera` | M1 | 简化第三人称；tilt-shift diorama M2+ |
| `src/game/minimap.js` | `inkwave::ui::minimap`（+`minimap_ui`） | M4 Task 49 | base/ink/zone 三层纯函数栅格化→RGBA 纹理（投影/采样表/全像素公式/双线性+emboss/flash/时钟分带 ≡ JS 逐式）；player dots ≡ main.js L1293-1304（swim 含 actor.js L994 specialActive.body/noSquid 覆盖）；boss 点=Rust 扩展（JS 无）；HUD 角标+TAB 展开 ≡ hud.js `_updMap`（弹簧 k190/c21）；档位 `MinimapTier{4/7/10}` pxPerM。**顺延**：live 矢量层（fx 环/云/炸弹/subs/跳跃线/specials.drawMap）→Task 51；beacon 面板/死亡标记/strike 光标→Task 56；dots 圆形/ping 环=占位档（REVIEW_CHECKLIST） |
| `src/game/character.js`（4062 行，弹簧姿态系统） | `inkwave::render::character` | M1（占位） | M1 体块占位；完整过程化绑定 M2+ |
| `src/game/character-geo/hair/face/outfit/...` | `inkwave::render::character::*` | M2+ | 更衣室相关，locker 前不做 |
| `src/game/showcase.js` / `lobbySet*` | — | M2+ | 线上大厅场景 |
| `src/game/special-props.js` | `inkwave_sim::world::props` | M2+ | |

## `src/world/` 关卡与墨汁

| JS 模块 | Rust 目标 | 状态 | 移植备注 |
|---|---|---|---|
| `src/world/mapkit.js`（B/R/O/OCT/ARC DSL） | `tools/extract/extract-layout.mjs`（JS 侧求值）→ `inkwave_sim::geometry` | M1 | 图元在 node 侧求值后序列化，Rust 不重写 DSL |
| `src/world/maps.js` | `assets/maps/*.json` 索引（`inkwave_sim::embedded_layout(id)`） | M1/M3 Task 37/M4 Task 48 | 全 8 图（tidewater/kelpline/halyard/saltpan/crossmarket/lockgate/terraces/cargo）；zones 合并口径 L118-119 走提取器（Task 36） |
| `src/world/stages/tidewater/layout.js` | `assets/maps/tidewater.json` | M1 | 含 half 镜像展开与 meta 统计 |
| `src/world/stages/tidewater/surfaces.js` | 提取到 layout JSON 的 color/pattern + `inkwave::render::material` | M1 | 程序化材质简化 |
| `src/world/stages/tidewater/props.js`（2093 行） | `inkwave::render::props` | M2+ | M1 只保留影响碰撞/地标的体块 |
| `src/world/stages/tidewater/murals.js` | — | M2+ | 装饰贴花 |
| `src/world/stages/{saltpan,crossmarket}/layout.js` | `assets/maps/{saltpan,crossmarket}.json` | M3 Task 37 | 提取器已泛化（stage CLI 参数）；含 onlyIn/notIn 变体旗标与 zones-data 合并；surfaces 槽位 34-39 入 meta；props/murals 装饰顺延 M4 |
| `src/world/stages/{kelpline,lockgate,terraces,cargo}/layout.js` | `assets/maps/{同名}.json` | M4 Task 48 | stage CLI 扩图；surfaces 槽位 31-33/40-42/43-45/46-48 入 meta；cargo 无 zones-data（turf 档）；渲染接线顺延 Phase C |
| `src/world/maps.js` L30-114（`HALYARD` 内联） | `assets/maps/halyard.json` | M4 Task 48 | 提取器特殊分支（source.path=maps.js）；surfaceSlots={}（共享 PATTERN 0-27）；intro/decor/water 不入 v1 schema（呈现层消费，D-T48-6） |
| `src/config.js` L510-513/L524-539（MAPS theme/times + 五旗标函数） | `inkwave_sim::tuning::{MapFlags, map_theme, map_offline_ok, offline_map_ids, boss_fallback_map, no_bots_start_block}` | M4 Task 44/48 | tuning schema v7（maps 段 theme/times 增量）；`map_theme` 含 JS 真值回退链；OFFLINE_MAPS=7 项无 cargo；运行时选图不接（main.rs 恒 tidewater，Phase C 收口） |
| `src/world/level.js` | `inkwave_sim::world`（碰撞）+ `inkwave::render::level` | M1 | |
| `src/world/paint.js`（929 行） | `inkwave_sim::paint`（覆盖网格/计分）+ `inkwave::render::ink`（图集） | M1 | 架构照搬：每面独立二维网格 |
| `src/world/inkShading.js`（314 行 GLSL） | `inkwave::level.wgsl` INK_* 9 chunk + `inkwave::ink_ripple::RippleTable` | M4 Task 52 | 9 chunk 全量（INK_PARS/COLOR/ROUGH/GEL/SLOPE/EMISSIVE/LIGHTS/LIGHT_MAPS/SHADE）≡L49-303 逐式；uRip/uRipP 打包 ≡ inkBeforeRender L40-44；`RippleTable` ≡ paint.js L551-587（RIP_N=24 保分置换/38m 剔除/`_rippledNear` 去重/on_splat 门）；INK_LIGHTS/LIGHT_MAPS 无 Bevy 灯钩子→折入 emissive/diffuse 近似（REVIEW_CHECKLIST）；`InkExt`/ink.wgsl 标 legacy（世界面走 `LevelExt`） |
| `src/world/levelMaterial.js`（719 行） | `inkwave::level_render::LevelExt` + `level.wgsl` TEXLIB 运行时 | M4 Task 52 | uTL/uTLt/uTLs 三表 ≡ L96-99（`schema.rs::slot_uniforms`，50 槽锚测）；onWall/onTop 片元重映射 ≡ L297-302；texlib_sample 三模式+sym 位掩码+Toksvig ≡ TEXLIB_GLSL；clearcoat→roughness 近似（D-T52-7②）；灯位表 binding 37（`u_lamp_n=0` 恒关=Task 53）；材质基线 ≡ L40-41（roughness .82/metal 0） |
| `src/world/texlib.js`（1424 行） | `inkwave::texlib::{schema,gen,bake,materials,materials_stage,res}` + `assets/texlib.json` | M4 Task 52 | GEN_COMMON ≡ L97-169 全平移；25 基础层+21 stage 层 prep/surf 逐一（`materials*.rs` 逐层 `≡ L##` 锚）；**CPU 烘焙替代 GPU MRT**（D-T52-7①，2×2 子采样/sRGB/orm/buildMips box ≡ GEN_MAIN L170-201，golden sha256 钉）；提取管线 `extract-texlib.mjs`（46 层 meta+50 槽表+ink/lamps/mat/tex 常量，禁手抄）；TexLibRes 后台线程烘焙 43 用层→三数组纹理（Bevy 0.19 prepare_asset 原生多 mip 数组，上传路径偏离预设登记）；档位 256/512（`quality.rs`，JS 真实门=main.js L121） |
| `src/world/decor.js` / `dressing.js` / `murals.js` / `mapThumb.js` / `props*.js` | `inkwave::render::*` | M4 Task 53 | murals/gel 的 shader 面与 uniform 表已随 Task 52 平移（binding 26-27/38-40，数据恒失活=D-T52-7⑤） |
| `src/world/environment.js`（3116 行） | `inkwave::render::environment` | M1（白天简化） | 黄昏主题、海况、灯光烘焙 M2+ |
| `src/world/variants.js` / `zones-data.js` | `inkwave_sim::geometry`（`in_mode`/`StageLayout::for_mode`）+ `collision::from_layout_for`；zones 入 `assets/maps/*.json` | M3 Task 36/37 | zones-data 合并（Task 36）；onlyIn/notIn 旗标随行入 JSON、按模式过滤保持 block id 位置序（Task 37） |

## `src/` 其他

| JS 模块 | Rust 目标 | 状态 | 移植备注 |
|---|---|---|---|
| `src/core/renderer.js`（three.js + 后处理） | Bevy/wgpu 渲染管线（架构替代） | M1（清屏/基础光照） | GTAO/bloom/grade/SMAA M2+ |
| `src/core/input.js` | Bevy `ButtonInput` + `inkwave::input` | M1 | |
| `src/core/ctx.js`（全局事件总线 G） | Bevy Resource/Message | M1 | |
| `src/main.js`（1399 行引导/状态流） | `inkwave` App + states | M1（最小） | 主菜单流程精简 |
| `src/fx/fx.js`（2267 行粒子/墨团/命中 GPU 特效） | `inkwave::fx::particles` + `inkwave::fx::api` | M4 Task 51 | 六池（Drop/Sprite/Ring/Shell/Beam/Mote）≡L726-1017 逐式（swap-with-last 回收/水面命中 ripple/`_after` 延迟调度 OP 枚举/`_near` 相机剔除入参化/固定种子 PRNG≡Math.random 声明 D-T51-7④/quality×particles 预算 cap=TR-51.1 满池单测）；api ≈60 公开方法逐一 ≡（默认值/旗标组合/`_color` 归一/内部调用链逐值锚行号）；`onSpeck/onRipple` 接 `PaintGrid` 既有 API（sim 零改动）。**GLSL 着色精度=instanced billboard 占位档**（impostor 拉伸=view 对齐 quad 近似，D-T51-7②，REVIEW_CHECKLIST 视觉复核） |
| `src/fx/fxHooks.js`（712 行事件→特效接线） | `inkwave::fx::hooks` | M4 Task 51 | SimEvent/TickEvent 消费 ≡ 订阅表 22 项（L58-79）逐项对账+触发计数 `count` 单测（TR-51.1）；**actor:\* 总线 Rust 无对应→≡JS polling fallback**（per-actor 30 字段状态机：grounded/form/vel.y/stepAcc 边沿推断，D-T51-7③）；缺口登记：bomb:explode/weapon:impact/damage 无 Rust 事件源（REVIEW_CHECKLIST） |
| `src/fx/screenfx.js`（1068 行全屏 composite+效果状态机） | `inkwave::fx::screen_fx` | M4 Task 51 | 12 事件→效果状态机（触发/衰减/阈值→uniform 值 ≡ L589-667/L780-868 纯数据平移）；**composite pass 不接=Task 55**（D-T51-7①，M4 Non-Goal「无后处理链」一致） |
| `src/fx/swimWake.js`（107 行） | `inkwave::fx::swim_wake` | M4 Task 51 | swimmer 筛选（≤4/本地优先/form swim\|climb/相机距离）+轨迹 ring-buffer（BREAK/LIFE/SPACING ≡ L50-107）；**uWake 组 uniform 上传点已随 Task 52 收口**（D-T51-7⑦ 闭环：`level_render.rs::level_material_wiring` 四 getter→LevelExt binding 33-36 每帧上传，getter 逐值钉测） |
| `src/fx/zoneMarks.js`（517 行 zone/flood 呈现，M3 顺延项） | `inkwave::fx::zone_marks` | M4 Task 51 | flood 进度/contested/active/sweep/flash 数据层 ≡ L417-456+rotation wipe 取消在途 fill（FloodRec team:-1/dur 0.7/reach+3.5/linear，评审 P1-G 钉测）；单 mesh ShaderMaterial→简化边界带 mesh 占位档（D-T51-7⑥）；呈现接线 `fx/mod.rs`+`fx/render.rs`（FxState Resource+`fx_update` 挂 world.rs 非 headless 分支） |
| `src/ui/hud.js`（2073 行） | `inkwave::ui::hud`（bevy_ui） | M1（核心元素） | 比分/时钟/墨箱/准星；其余 M2+ |
| `src/ui/menus.js` 等 | `inkwave::ui::menu` | M1（Play/暂停/结算） | |
| `src/audio/audio.js`（2505 行 WebAudio 合成） | `inkwave::audio`（+`audio::synth/graph/sfx_table`） | M4 Task 50 | **过程化样本烘焙**（选型 D-T50：不新增依赖，bevy_audio+PcmSound 离线 PCM）：synth 原语（Osc/Noise/Biquad-RBJ/Envelope/mulberry32 固定种子≠Math.random 声明）+graph 离线求值+sfx_table 148 名全表（≡SFX_GROUPS L2479-2501 1:1，评审程序化对账零失配）+SfxGate ≡play() L258-304（minGap/voice cap steal 最旧/MAX_VOICES=48 JS 实值/jitter→速率近似）+set_volumes ≡L165-172（taper^1.5/τ=0.04，暂无 UI 调用点 M5 接）。**未烘**：convolver 混响/glue/limiter/3D panner/湿信号（dry 占位档）；kit 系 42 名（kits/*-sfx.js 自注册）超范围顺延 |
| `src/audio/music.js` / `bossAudio.js` | `inkwave::audio::music` / `inkwave::audio::boss_audio`+`BossAudioDirector` | M4 Task 46+50 | Task 46：`BossAudioDirector` ≡ bossAudio.js L84-176 全表（move×phase 15 行 + intro/hit·crit 去重门 0.06/0.07/0.3/stun·dizzy 0.35s 延迟/phase/defeat duck+sunk/crablet spawn·pop + TRACK 1/2/3 表），输出 `BossCue` 记录表供断言。Task 50 升级：`audio/music.rs` ≡ music.js（9 曲 title/menu/battle/battle_final/results_win/results_lose/boss/boss_2/boss_3，`compile_song` ≡ L601-777 grid-string 编译器 PC/QUAL/VEL/16-step/chordRow/swing/pump，synth 轨离线烘焙整曲 loop；play/fade/remap/escalation/duck ≡ L1378-1417/L357-378；file-backed 本仓无资产恒 false 占位）+`boss_audio.rs`（BossCue 6 类→真实播放，remap battle→boss 曲 ≡ L58，boss 系 20 名接线）+场景接线 ≡ main.js L426/L555/L548/L1028（title 屏/judge_drumroll 缺口在案） |
| `src/boss/*` | `inkwave_sim::boss` + 渲染 | M4 进行中 | Boss 战：Task 41 已落 tuning 提取（schema v3）+ boss wire 金标准（`B` 快照 full/delta、`'bhit'`、`['bm']`、`['bc']`、res boss 形态）；Task 42 已落 sim 核心：`boss/nav.rs` ≡ bossNav.js 260 行（_build 直方图/kind 场/Dw/Dv/连通域/A* path，JS 逐点钉定）+ `boss/mod.rs` ≡ boss.js 497 行（实体状态机/_spawnPoint/hitShapes 世界球查询 segHit·rollHit·rayDist/伤害倍率链/splash/rain 批/remoteHit/crab 生命周期/pack→BossSnap·unpack·_follow·adopt·handoff/checkPhase 阈值）；schema v4 hitPos+nav 常量；Task 43 已落 brain+hazards：`boss/brain.rs` ≡ bossBrain.js 376 行（目标选取评分/walk·turn 侧移/选招权重/六招 `_start` record 构造/crablet 管理/roar 拍）+ `boss/hazards.rs` ≡ bossHazards.js L1-352（MoveId+record JVal 访问器/几何自由函数/hz 状态上移 Boss/victim-side 六招伤害/paint host-only Splat/threat 查询）+ `Boss::update(dt,squad)` 接线（boss.js L100-116 时序、model_update move 分支+_moveAim 实化、adopt 建 brain）；schema v5 brain.pref；Task 45 联网接线：`BossNet` 扩 `prev_t/prev_raw`（JS 惰性 scratch 显式化）+ `replication.rs` 八处 boss 钩子（`sample_boss` ≡ `_sampleBoss` L504-564 Hermite/阻尼 w=13/三段修正；`send_tick` 每 10 tick full；`on_tick` B 消费单调门+40 trim；`play_events` 'bm' 无 !sim 门/'bc' 有门；`on_message` 'bhit' is_host+remote_hit；`on_leave` 迁移 adopt/handoff；`drive_bots` ctx.boss；`send_result/host_result` res boss 形态+停钟）+ 6 e2e 测试（路径收敛/bhit 收敛/迁移/bm 重放/res 结算/带宽 <64KB）；Task 46 呈现占位档 + app 驱动：`Boss`/`BossMode` owned 化（nav/tb/collision 按值持有，'static 可进 Bevy Resource，D-T45-9 解）+ 武器弹丸管线接 `boss.hit/splash/rain`（≡ weapons.js `_bossImpact` L1650-1665：seg_hit 优先序/volley 去重/drop 衰减/flick 窗；未接线=subs/specials 直伤链与 kits bow/mitts/brolly 近战，D-T46-2 顺延）+ `DemoSim.bm/boss_events` 离线驱动（maxHp ≡ bossMode.js L21-22）+ 离线菜单 MODE turf⇄boss（zones 卡待离线 zones demo）+ `match_link` 在线 boss 构造 + nettest guest B 探针（D-T45-11 收口）+ `boss_render.rs`（体块剪影 ≡ bossModelGeo CONT/ARMS/HEAD/LEGS 值 + telegraph 标记 ≡ HazardFX L357-600 + crablet pop）+ `ui/boss_hud.rs`（BossHudState 纯函数 ≡ hud-boss.js S 机全表）+ 结算 boss 行 + `BossAudioDirector` 占位 cue 表 ≡ bossAudio.js L84-176 |
| `src/net/netmatch.js`（825 行） | `inkwave_sim::net::{wire,session,replication,playback}`（事件重放在 `replication::play_events`） | M2 done | codec 与 JS packer 逐字节一致（金标准 `assets/net-fixtures.json`，Task 17）；session/replication/playback/replay 逐字平移（Task 20–25），Rust↔Rust 完整局一致性 TR-25.1；Task 41 fixture 扩 boss 条目（`B` 快照 full/delta、`'bhit'`、`['bm']`、`['bc']`、res boss 形态），`wire.rs` 编解码逐字节回环断言，`replication.rs` 两 no-op 臂待 Task 45 实化；Task 42：`BossHit.c` 加宽 i8→i32（`crabSeq++` 无上限，bossBrain.js L336，防 255→-1 别名本体哨兵）+ guest 上行 `d` r2 量化与 `log.sent/sentDmg` 计数（netmatch.js L152-153）+ `has_nid` 门控；Task 45：两 no-op 臂实化（`play_events` 'bm'/'bc' + `on_message` 'bhit'）+ `send_tick` `msg.B` 发布 + `on_tick` B 消费 + `sample_boss` 播放 + res boss 形态 + `on_leave` 迁移 + 6 e2e 测试（TR-45.1 ①-⑤ + NFR-M4-3 带宽）+ scenario E（net-smoke.mjs，DEGRADED 口径） |
| `src/net/transport.js` | `inkwave_sim::net::transport`（trait + FakeTransport）/ `inkwave::net::transport`（native + wasm） | M2 done | **选型（spec OQ-2）**：native = `tungstenite` 0.26 同步 API + 独立线程 + `std::mpsc`（Bevy 主线程零阻塞），TLS 走 `rustls-tls-webpki-roots`（纯 Rust、离线可解析）；弃 `fastwebsockets`（tokio 异步栈，违反 sim/最小依赖纪律）。wasm = `web-sys` WebSocket。必须发合规 `Origin` 头过 relay ORIGIN_OK |
| `src/net/session.js` / `mock.js` | `inkwave_sim::net::session` + `inkwave::net`（transport 接线）+ `inkwave::ui::online`（极简大厅） | M2/Task 39/M4 Task 44 | relay 复用、协议不改（`server/` 零改动）；大厅 UI 为极简版（Task 24），showcase 3D 大厅场景未做（M2+）；Task 39：lobby loadout cycle 选择（WEAPON 12/SUB 15+NONE/SPECIAL 19+NONE）+ MODE turf⇄zones host-only，roster `{weapon,sub,special,style}` 经 `set_me`/`hostSet` 实值透传（JS L200-206/L244/L279），`matchbuild` 按 actor.js L38 falsy 链接线 `a.sub`；Task 44：MODE 三值 cycle（turf→zones→boss→turf，JS L244）+ session 放开 `mode:"boss"`（JS L244-249 旗标联动：`mapBossOk` 回退 `bossFallbackMap`、`noBotsStartBlock` 检查）+ guest boss cfg 放行（不步进，联网归 Task 45）+ `matchbuild` boss 单队构造（squad=8，spawn 环除数 `boss ? squad : 4`，JS match.js L87） |

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
| `src/game/bots.js` L1260-1620（threat 段/_shieldFight/_canopyCtl/_range）、L460-470（stamp steer）、L525/L621-630/L635-660（接线/tail snappy）、L779-783（_dryLine）、L1781-1793（_fatLos） | `inkwave_sim::bot::threat`（threat.rs 1331 行 + THREAT_AI/THREAT_STATS/AimHold/ShieldView） | M4 Task 47 done（D-T47-1..12；A/B 开关+固定种子 TR-47.1） |
| `src/game/kits/waddle.js` L664-685 / `torpedo.js` L709-726（threatOf） | `inkwave_sim::sub::{ThreatView,ThreatRef}` + `SubSystem::threats/find_threat`（值视图重建 ≡ JS getter 描述符） | M4 Task 47 done |
| `src/game/kits/brolly.js` L172-190（heldPose/refreshShields）/L604-627（shieldOf） | `bot/threat.rs` `rebuild_shields` + `ShieldRef{Held,Launched}`（held=`held_shield`、launched=`LaunchedCanopy.cur_c/cur_n`；swap-remove 滤己队序复刻） | M4 Task 47 done |
| `src/game/kits/blade.js` L388-620（botTactics 全文/_blHold/botHolds） | `inkwave_sim::bot::blade_tactics`（新）+ `weapon/blade.rs` 救援窗（`_bl_hold` 同钟比较 ≡ projectiles.time） | M4 Task 47 done（Task 38 batch-2 裁剪项收口） |
| `src/game/kits/mitts.js` L755-835（bot.fight/paint：leap/cling/pounce） | `bot.rs` Mitts 段 + `weapon/mitts.rs`（`predict_leap` 恢复、`MittsReq/bot_want`） | M4 Task 47 done（Task 38 batch-2 裁剪项收口） |
| `src/game/zones.js` L272/L376/L401-407（follower `_tick` 预测） | `inkwave_sim::match_::zones` `'zo'` field2/`'zs'` field10 + `catch_up` 时延补偿 | M4 Task 47（**JS 无对应新逻辑**，D-T47-11/11a；2026-11-05 用户裁定：**兼容超集定位**——Rust host 记录为 JS 字段严格超集，JS 按下标读、尾部字段忽略，Rust 消费按 `len` 门降级；字节等价门只对 fixture 重放路径生效。性质=**网络一致性强化**（sim 网络路径断言强化，非玩法保真平移），JS 树零改动纪律不变） |
