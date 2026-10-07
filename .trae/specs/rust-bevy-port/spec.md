# INKWAVE Rust/Bevy 移植与上游同步 - 产品需求文档（PRD）

## Overview
- **Summary**: 将 fork 来的 INKWAVE（~9.9 万行 three.js 浏览器游戏 + Cloudflare relay）逐步移植为 Rust + Bevy 实现，同一份代码同时编译为原生桌面应用与浏览器 WASM；并建立可持续的上游功能同步工作流。本规格覆盖移植工程的总体架构、同步机制，以及第一个可玩垂直切片（M1）。
- **Purpose**: 用户长期维护自己的 fork，并定期从上游 `jaydendavisnc/inkwave` 获取新功能；需要一套 Rust 代码库承载游戏，同时保证上游新特性能够被低成本、可追踪地移植过来。
- **Target Users**: 仓库维护者（本人，承担移植/同步工作）；M1 阶段的终端用户为通过原生窗口或浏览器试玩切片的开发者本人。

## Goals
- 在**本 fork 仓库内**新增独立的 `rust/` Cargo 工作区，JS 源码树保持原样、可持续 merge 上游。
- 一套 Rust 代码双端运行：原生桌面（x86_64-linux 为开发主平台）与浏览器 WASM（WebGL2/WebGPU）。
- 建立**上游同步机制**：upstream 远端、同步检查脚本、移植台账（ledger）、JS→Rust 模块对照表、数据提取管线（让数据类改动可机械同步）。
- M1 交付可玩的垂直切片：**Tidewater Plaza + Spritzer + 涂地模式（Turf War）+ bot 填满 4v4**，端到端跑通"移动/射击/墨汁/计分/胜负"闭环。
- 架构上为 M2 的**线上跨端对战（与 JS 客户端协议兼容、复用现有 Cloudflare relay）**预留边界。

## Non-Goals
- M1 **不**包含：其余 6 张地图、其余 11 把主武器、副武器/特殊武器、Zone Control、Boss 战、更衣室（locker）、线上大厅/showcase、排名/经验成长。
- M1 不追求与 JS 版 1:1 的视觉表现：GTAO/bloom/色调映射后处理、60 骨过程化角色绑定、过程化音乐与音效、全部布景道具（props）均降级或占位。
- 不用 Rust 重写 Cloudflare relay（`server/` 原样保留）；M1 不实现实际联网对战。
- 不做移动端、不做 macOS/Windows 打包发布（仅保证代码可跨平台编译）。
- 不修改 `src/`、`server/`、`electron/`、`tools/` 等任何上游 JS 文件（保证 fork 可干净 merge）。

## Background & Context
- 上游：`https://github.com/jaydendavisnc/inkwave`，MIT，v1.1.0；当前 fork origin 为 `ndruster/inkwave`，本地 `main` 与 origin 同步。
- 代码结构：`src/game`（37k 行：角色/武器/bot/比赛）、`src/world`（35k 行：地图/墨汁绘制/环境）、`src/ui`（9.7k）、`src/fx`（4.7k）、`src/boss`（4.5k）、`src/audio`（4.2k）、`src/net`（1.7k，20Hz tick 状态同步 + 回放插值）。
- 关键事实（已核实）：
  - 关卡是**数据**：地图 DSL（`mapkit.js` 的 `B/R/O/OCT/ARC`）产出普通对象；Tidewater 布局 240 行、surfaces 144 行、props 2093 行。
  - 调参是**数据**：`src/config.js` 的 `PLAYER`/`WEAPONS`/`MATCH` 等纯常量对象，无导入依赖。
  - 墨汁：GPU 绘制到每面可喷涂面拥有区域的 4K 图集，CPU 粗网格负责涂地计分与查询（`src/world/paint.js`，929 行）。
  - 角色控制器调参全部在 `PLAYER` 常量中（米/秒制），手感要求逐数值对齐。
  - relay 协议为 PROTO v1：`b|`/`s|to|` 透传帧 + JSON 控制消息；客户端 tick 形如 `{k:'t',ts,a:[],e:[]}`。
- 已确认的决策（用户）：① WASM+原生双端；② Bevy 引擎；③ 同仓 `rust/` 子目录 + upstream 远端 + 移植台账；④ 垂直切片起步；⑤ 联网方向为协议兼容/跨端对战；⑥ 切片选 Tidewater + Spritzer。
- 工具链现状：rustc/cargo 1.98.1 已装；尚无 `wasm32-unknown-unknown` target 与 trunk；node v24、python3 可用；本机有 Chrome（tools 冒烟测试依赖）。

## Functional Requirements

### 工程与同步
- **FR-1 工作区**：仓库根下新增 `rust/` Cargo workspace（不触碰 JS 文件），至少含 `inkwave_sim`（无渲染、可无头单测的模拟核心）与 `inkwave`（Bevy 应用，双端入口）两个 crate；Bevy 固定到具体 0.x 版本。
- **FR-2 双端构建**：`cargo run` 在桌面开窗运行；`trunk`（或等价工具）构建/起服 WASM 版，键盘鼠标输入双端可用，wgpu 在浏览器走 WebGL2 兜底、支持 WebGPU。
- **FR-3 upstream 配置**：新增只读远端 `upstream` 指向上游仓库 HTTPS 地址；记录当前基线提交（baseline）；JS 树未来可 `git merge upstream/main` 而不与 `rust/` 冲突。
- **FR-4 同步脚本**：提供脚本（如 `rust/tools/sync-upstream.sh`）：fetch upstream，列出基线之后的新提交（主题/受影响文件），输出"建议移植"清单；并提供命令更新基线标记。
- **FR-5 移植台账与模块对照**：`rust/SYNC_LEDGER.md`（上游提交 → 涉及 JS 文件 → Rust 对应模块 → 状态/证据）与 `rust/PORT_MAP.md`（JS 模块 → Rust 模块的完整对照与移植备注），M1 范围内的模块全部登记。
- **FR-6 数据提取管线**：Node 脚本直接 import 上游 ESM（`src/config.js`、`src/world/stages/tidewater/layout.js`、`mapkit.js` 等），将调参与 Tidewater 几何体（含 180° 镜像展开、斜坡/旋转盒参数）序列化为 Rust 侧 serde 可反序列化的 JSON，输出到 `rust/assets/`；重新运行即可机械同步数据类上游改动。

### 切片玩法
- **FR-7 关卡几何**：加载 Tidewater 全部碰撞体（AABB 盒、斜坡板、绕 Y 轴旋转盒），含半区镜像；生成可行走碰撞世界；出生点/出生屏障/海水死亡 Y 值与上游一致；视觉上呈现可辨认的 Tidewater 白天场景（盒体材质按 surfaces 配色/简单图案，海面平面）。
- **FR-8 角色控制器**：第三人称；kid/squid 双形态；跑动、跳跃（含 jump buffer/coyote time）、重力、台阶 step-up/down、地面贴合；鱿鱼游墨（加速+回墨+可爬喷墨墙在 M1 可降级为水平面游泳，爬墙列入切片内尽力项）、敌方墨水减速/扣血；参数逐数值取自 `PLAYER`。
- **FR-9 Spritzer**：左键连射，`fireInterval=0.1s`、单发 36 伤害、墨耗 0.95/发、弹速/散布/射程按配置；弹丸先直线后抛物；落地/命中产生墨点；墨水箱耗尽停火、kid 形态延迟回墨、鱿鱼形态快速回墨；被击杀 5.5s 重生、出生无敌 1.6s。
- **FR-10 墨汁与涂地**：弹丸与墨点喷涂到可喷涂面；每面拥有独立二维覆盖网格；按面积统计两队覆盖率（1 分/m² 新增涂地）；墨水视觉在地面上以两队颜色清晰可见且随射击即时更新。
- **FR-11 比赛流程**：4v4，空位由 bot 补足（1 人 + 7 bot）；开局入场/倒计时；默认 180s 时钟；终局结算（双方涂地百分比、胜负）；可重新开始；掉进海里按死亡处理。
- **FR-12 Bot**：至少 easy 难度行为：在关卡中导航移动、喷涂地面、索敌并开火、会被击杀并重生；保证比赛可完整打完。
- **FR-13 相机/输入/HUD**：WASD 移动、鼠标瞄准、Shift 鱿鱼、Space 跳、左键开火、Esc 暂停；HUD 显示时钟、双方涂地百分比、墨水箱、准星；主菜单可一键开始切片对局；结算屏显示比分并重开。
- **FR-14 角色与武器占位表现**：队伍色可辨的简洁过程化占位角色（kid 直立体 + squid 贴地形态，带基本姿态切换），手持 Spritzer；保证未来替换为完整过程化绑定而不改模拟层接口。
- **FR-15 网络边界预留**：sim crate 中以文档/类型方式定义与 PROTO v1 对齐的消息/tick 概念（不实现传输），保证模拟层状态结构未来可直接序列化复刻，不引入无法联网化的设计（如渲染层持有玩法状态）。
- **FR-16 无头验证接口**：应用支持 autopilot/headless 参数（对应 JS 版 `?autoplay` 与 botlab 思路）：以固定步长跑 bot 对局并输出比分/断言结果，供 CI 与后续"逐帧对齐"测试使用。

## Non-Functional Requirements
- **NFR-1 手感/数值对齐**：切片范围内的调参（PLAYER、Spritzer、MATCH）必须从提取数据反序列化，禁止手抄；Rust 单测断言关键值与上游一致。
- **NFR-2 性能**：原生端 1080p 下 8 人 bot 对局中位数 ≥ 59 FPS；桌面 Chrome（wasm）同等场景 ≥ 30 FPS；切片不依赖资源下载（无网络素材，字体可省略）。
- **NFR-3 代码质量**：`cargo fmt --check`、`cargo clippy -D warnings`、`cargo test` 全部通过；禁止对 JS 树的修改进入提交。
- **NFR-4 可同步性**：模块边界与上游 JS 目录结构保持可映射关系（PORT_MAP 覆盖）；玩法逻辑与渲染分离，使上游逻辑改动可定位到单一 Rust 模块。
- **NFR-5 可重复构建**：锁版本（Cargo.lock 提交）；README 说明双端运行、数据提取、上游同步三条命令路径。
- **NFR-6 版权**：保留 MIT LICENSE 与上游署名；Rust 新增文件使用同一许可证声明。

## Constraints
- **Technical**: Rust 1.98（stable）；Bevy 0.x（实施时选定并固定最新稳定版，记录于 PORT_MAP）；wgpu（Bevy 内置）；wasm32-unknown-unknown + trunk；glam 数学库（Bevy 自带，与 three.js 同为列向量/左手系差异需在校验时注意 Y-up 坐标一致即可，两者均 Y-up）。
- **Business**: 工作量大（10 万行级），M1 刻意只做端到端最小闭环；所有未移植内容必须能通过对照表追踪。
- **Dependencies**: 上游 ESM 数据提取依赖本机 node；wasm 冒烟依赖本机 Chrome；不得引入 GPL 等不兼容依赖。
- **Sync 约束**: Rust 代码无法 merge JS 上游，所有上游同步均为"对照 diff 人工移植"，因此数据与逻辑必须尽量分离（数据走提取管线，逻辑走台账）。

## Assumptions
- M1 视觉为"可辨认的简化版"：纯色/简单图案材质、占位角色、无后处理；用户接受以手感/玩法正确性优先。
- M1 音效仅做最少量占位（射击/击杀反馈），过程化音频完整移植在后续里程碑。
- 鱿鱼爬墙、超级跳跃、地图 diorama、手柄支持在 M1 可降级或省略（键盘鼠标为准）。
- 开发与验证平台为 Linux x86_64 + Chrome；不实际发布构建产物。
- 上游短期内不会把 ESM 数据模块改成需要打包器才能消费的形态；若发生，提取脚本相应维护。

## Acceptance Criteria

### AC-1: 双端零警告构建
- **Type**: `rule`
- **Given**: 干净检出本仓库且安装稳定 Rust 与 wasm 工具链
- **When**: 依次执行 `cargo build`、`cargo build --target wasm32-unknown-unknown`（或 `trunk build`）、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`cargo test`
- **Then**: 全部成功退出码为 0，且构建过程不修改/依赖对 `src/` 等 JS 路径的改动
- **Pass Condition**: 所有命令通过；`git status --short` 中无 JS 树文件被改动
- **Evidence**: 命令输出日志

### AC-2: upstream 同步基线与脚本可用
- **Type**: `rule`
- **Given**: 配置好 upstream 远端
- **When**: 运行同步检查脚本
- **Then**: 成功 fetch 上游；输出当前基线提交哈希及其之后的提交列表（基线=当前 HEAD 时列表为空）；台账与 PORT_MAP 文件存在且登记 M1 全部模块
- **Pass Condition**: 脚本退出码 0；基线哈希与实际 `git merge-base` 记录一致；`rust/SYNC_LEDGER.md`、`rust/PORT_MAP.md` 存在
- **Evidence**: 脚本输出与文件内容

### AC-3: 数据提取管线与数值一致性
- **Type**: `rule`
- **Given**: 上游 JS 源文件
- **When**: 运行提取脚本生成 JSON，再运行 Rust 测试
- **Then**: `rust/assets/` 下生成 tuning 与 tidewater 几何数据；Rust 测试断言 Spritzer 的 `fireInterval/damage/inkPerShot/projSpeed/range`、`PLAYER.runSpeed/swimSpeed/hp/respawnTime`、`MATCH` 关键值与 `src/config.js` 完全一致；断言 Tidewater 碰撞体数量、出生点、bounds 与从 JS 侧导出的值一致
- **Pass Condition**: 测试通过；删除生成物后测试无法通过（证明数据确实来自提取管线而非手抄）
- **Evidence**: 提取脚本输出、`cargo test` 结果

### AC-4: 原生端可完成一局完整涂地比赛
- **Type**: `rule`
- **Given**: 原生构建
- **When**: 启动游戏 → 主菜单开始 → 完成入场、180s（验证时可用缩短时长配置）对局 → 结算
- **Then**: 玩家可跑/跳/变鱿鱼/游墨/射击/喷涂/被击杀重生/掉海死亡；7 个 bot 全程活动（移动、喷涂、开火、死亡重生）；结算屏显示两队百分比且总和规则正确（按可喷涂面积归一），可重开
- **Pass Condition**: 无头 autopilot 模式以固定步长跑完一整局不 panic，输出双方百分比、至少发生过击杀与大量涂地变化；人工窗口操作上述动作均有效
- **Evidence**: headless 运行日志/输出 JSON + 人工试玩记录

### AC-5: WASM 端可玩
- **Type**: `rule`
- **Given**: trunk 起服
- **When**: 用 Chrome 打开页面并开始对局、操作 60s
- **Then**: 画面正常渲染、输入有效、无 JS console error/panic
- **Pass Condition**: 浏览器中可移动射击且控制台无错误；页面不依赖被删除的 JS 游戏资源
- **Evidence**: 冒烟脚本/浏览器控制台记录

### AC-6: 切片玩法数值/规则与上游等价
- **Type**: `rule`
- **Given**: sim crate 无头测试
- **When**: 构造确定性场景（固定随机种子/固定输入）
- **Then**: ①Spritzer 3 发命中造成 108 伤害并触发击杀→5.5s 重生流程；②墨水箱以 9/s（kid 延迟 0.9s 后）与 42/s（鱿鱼）回复；③涂地得分按 1 分/m² 新增累计且只计一次（重复喷涂不重复计分）；④弹丸 0.13s 直线后受重力、射程截止行为与配置一致
- **Pass Condition**: 四个场景断言全部通过（容差：时间步进累计误差 < 1ms，位置 < 1cm）
- **Evidence**: `cargo test` 用例输出

### AC-7: 墨汁渲染可读性
- **Type**: `rubric`
- **Dimension**: 墨汁视觉对玩法信息的表达（两队颜色对比、边缘可读、即时反馈）
- **Scale**: 1-5
- **Anchors**: 1 = 看不清哪里被谁涂了；3 = 两队色块清晰、射击反馈即时但形状/质感简单；5 = 接近 JS 版的边缘扩散/湿润质感
- **Pass Threshold**: >= 3
- **Evidence**: 同场景截图（Rust vs JS）对比说明

### AC-8: 移动手感还原度
- **Type**: `rubric`
- **Dimension**: 第三人称移动/射击手感与 JS 版的相似程度（加减速曲线、跳跃、鱿鱼游泳）
- **Scale**: 1-5
- **Anchors**: 1 = 明显不同的物理游戏感；3 = 速度感/转向/惯性大体一致，细节有差；5 = 盲测难以区分
- **Pass Threshold**: >= 3（M1），并要求关键运动学常量 100% 一致（由 AC-3/AC-6 保证）
- **Evidence**: 操作记录 + 可复用的无头测量数据（加速到 90% 极速距离/时间等）与 JS `measure-handling` 口径对照

### AC-9: 运行性能达标
- **Type**: `rule`
- **Given**: 8 人 bot 对局、1080p 窗口（wasm 为桌面 Chrome 全屏标签）
- **When**: 对局进行中采样 ≥ 30s 帧时间
- **Then**: 原生中位 FPS ≥ 59；wasm 中位 FPS ≥ 30
- **Pass Condition**: 内置帧时间统计输出满足阈值
- **Evidence**: autopilot 输出的帧时间统计

### AC-10: 网络协议边界对齐
- **Type**: `rule`
- **Given**: sim crate 文档与类型
- **When**: 对照 `docs/NET.md` 与 `src/net/netmatch.js` 审查
- **Then**: Rust 侧存在与 PROTO v1 tick/event 概念对应的类型或设计文档（actor 网络状态字段、20Hz tick、事件枚举可表达 hit/tr/ev/z 等），玩法状态不依赖渲染层
- **Pass Condition**: 文档评审通过；sim crate 不依赖 bevy/render 类型（`cargo test -p inkwave_sim` 无 wgpu 依赖即可编译运行）
- **Evidence**: 模块文档 + `cargo tree -p inkwave_sim` 无 bevy 渲染依赖

## Open Questions
- [ ] M1 之后 M2 的优先级排序（联网跨端对战 vs 更多武器/地图 vs 角色/视觉完整度）——M1 评审后确认。
- [ ] Bevy 具体版本（实施首任务时按当时最新稳定版固定并记录，若与 wasm WebGL2 支持冲突则回退选择）。
- [ ] 是否需要在 M1 提供 JS 版网页同域共存部署（如 `/rust/` 路径）——默认不需要，M1 仅本地运行。
- [ ] 鱿鱼爬墙 M1 是否必须：默认做平面游墨+简单斜坡，垂直爬墙若影响切片进度可顺延，审批计划时确认。
