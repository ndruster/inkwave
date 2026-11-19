# M5 复核清单（表现终稿与内容收尾）

> **双轨验收口径（NFR-M4-5 → NFR-M5-4 延续）**：沙箱只验无头可测部分（数据确定性、
> 独立 crosscheck、锚定单测、headless 600 帧、wasm check、九段门禁）；以下 GPU/听感项
> **登记待用户机器复核**，不做伪验收。本文件随任务实施逐任务扩充，收尾时由
> Task 70 汇总成「M5 汇总复核清单」。

## 预登记：承接 M4 的复核基线

M4 登记的 GPU/听感项（`.trae/specs/rust-bevy-port-m4/REVIEW_CHECKLIST.md`「M4 汇总
复核清单」7 项收尾清单 + 逐任务索引）在 M5 对应任务落地前**仍然有效**；M5 任务
把对应项转正式档后，在下方逐任务区登记新的复核点，并在收尾时标注「M4 项已被
M5 TaskNN 取代」或「M4 项与 M5 无关、维持原状」。

## 预登记：M5 各任务待复核面（实施后扩充）

| Task | 预登记复核面 |
|---|---|
| 58 | bow 持握/开镜姿态、brolly 伞面、bowl 形态；kit 姿态手感 |
| 59 | 设置屏 UI 布局/焦点/与 online 屏共存；持久化在 wasm 与 native 双载体生效 |
| 60 | 42 名 kit SFX 触发时机/音量平衡；footstep 节奏；射击原点与 FX 对位 |
| 61 | hub/lobby 无世界叠加、切换过渡、双相机下 UI target 正确 |
| 62 | 名牌可读性/遮挡、满房/空房、emote→dance 流畅度 |
| 63 | 混响空间感/尾音、压限响度、3D panner 方位感 |
| 64 | 叠层观感、与 bloom/后处理顺序、伤害数字可读性 |
| 65 | GTAO 强度、grade 色彩、SMAA 锯齿、tilt-shift 景深；逐档差异 |
| 66 | 三主题下云/PMREM/海面观感；`low` 档降级可接受度 |
| 67 | alley 反射清晰度/霓虹发光/蒸汽密度 |
| 68 | Boss 六招动画、weak 点可读性、telegraph 清晰度、`low` 档剪影降级 |
| 69 | locker 预览观感/切换流畅度/与 loadout 联动 |

## 用户 GPU 机器复核顺序（沿用 M4 收尾清单格式，Task 70 定稿）

- [ ] `cd rust && trunk build --release` 一次通过（沙箱 D-T40-1 DEGRADED 持续登记）。
- [ ] 窗口态按 `rust/README.md` M5 章节操作，逐项勾上方表格。
- [ ] `low`/`ultra` 两档 + 三主题 headless 组合在用户机复跑。
- [ ] 网络可达时 `bash rust/tools/sync-upstream.sh check` 确认基线后无新上游提交。

> 本清单不构成「已通过」；收尾结论以 `tasks.md` Task 70 的 Completion Evidence
> 与 M5 总评审为准。

## Task 58 复核登记（2026-11-12）

无头可测部分已全部锚定（6097 帧 pinned replay 逐帧全绿 + wpKit 状态比对 +
九段门禁）。以下为**待用户 GPU 机器复核**的视觉项：

- [ ] **bow 持握/开镜姿态**：`HOLD.bow` 持握/瞄准/举高/大厅姿态（character.js
  L467-477 全表已入 char-poses.json），瞄准时侧身（hip/chest 负值）、满蓄拉弓
  到颊侧、松弦跟随；地面侧倾 BOW_CANT=-0.85（扇面平飞）、空中回正（扇面直立）。
- [ ] **bow 弓体动画**：弦两半随拉弓/松弦回弹（欠阻尼 ring）、limb 屈曲、三箭
  hide/nock 窗口（release 后 0.3s 隐没、0.12s 内补箭）；蓄能环发光与箭头发光
  （`BOW_RING1`/满蓄双档）为材质侧，需 GPU 复核。
- [ ] **brolly 伞面**：变换面（collar/trigger）已锚定；伞面 gores/struts/tears/
  lamp 由 runner kit 状态机驱动、Rust 无对应状态——**登记为明确偏差**：Rust 侧
  brolly 恒呈收拢（furled）形态 + trigger 收弹面，开伞/破损/重生/发射的伞面
  表现待总评审裁决口径。
- [ ] **bowl**：上游 JS 无 bowl 武器（config/kits/HOLD 均无入口，仅注释残留），
  无可复核形态——此登记即结论，非待办。
- [ ] **kit 姿态手感**：bow 拉弓走位（moveSpeedDrawing=2.6）与 charging 呼吸/颤动
  层（charger 家族姿态）在实机手感。

## Task 59 复核登记（2026-11-13）

无头可测部分：13 项锚测（默认值/词表/合并/clamp/启动覆盖/往返/真实点击系统/low 保底）
与独立 crosscheck（21 键双向）全绿；九段门禁结论见 `tasks.md` Task 59 Completion
Evidence。以下为**待用户 GPU/浏览器机器复核**的视觉与载体项：

- [ ] **设置屏 UI 布局/焦点**：8 行 + RESET/BACK 在 16:9/超宽/低分辨率下的排版、
  鼠标 hover/焦点态、键盘导航（Esc/B/Back、Enter=BACK）；与 online hub/lobby 屏
  叠加时的层级与返回路径（`settings_return`）。
- [ ] **持久化双载体生效**：native 配置文件与 wasm localStorage 在真实重启后生效；
  wasm 用独立键 `inkwave.rust.settings`（D-T59-4，避免覆写 JS 版 9 个未移植字段），
  需复核浏览器端不会与 JS 版设置互相覆盖。
- [ ] **运行期切档观感**：画质/主题/阴影/泛光切换后灯光/雾/Bloom 的即时刷新
  （`EnvRefresh` 一帧内生效）；天空贴图与已生成 rig 不重建（D-T55-⑦）是否符合预期；
  low 档保底可玩。
- [ ] **音量手感**：三轨 0.05 步进、`taper^1.5` 响度曲线与 τ=0.04 平滑在实机听感。
- [ ] **RESET 缺二次确认 / Saved! 脉冲**：JS menus.js L2145-2163 有确认与提示，
  Rust 版 RESET 立即生效且只有静态「Changes save automatically」——登记为
  行为偏差（P2），待总评审裁决是否补齐。
- [ ] **未接线字段**（保守范围，非待办除非总评审要求）：`padSensitivity`、
  `colorblind`、`minimap`、`matchLength`、`lastMode`、`difficulty`、`rumble`、
  `aimAssistMouse`、`fpsCap`；`fov` 存但不应用（D-T59-3）；
  `invertY/aimAssist/showFps/cameraShake` 存而暂无运行期消费者。

## Task 60 复核登记（2026-11-18）

无头可测部分：53 名 kit SFX 提取 + 独立 crosscheck + 5 项锚测
（sfx_kit × 4 / footstep 生产消费者 × 1）；九段门禁结论见 tasks.md。
以下为**待用户 GPU/听感机器复核**的项：

- [ ] **53 名 kit SFX 听感**：M4 文档旧口径「42 名」与 live 上游不符；实有 53 名
  （bow 7 / mitts 9 / blade 5 / brolly 7 / boomerang 6 / torpedo 6 / shaker 5 /
  tracer 4 / waddle 4）。Rust 携带提取的 gain/max/jitter/reverb/minGap 元数据，
  门控逐名正确；`build(v,p)` 音色函数体属上游代码、无法提取，现由通用合成
  `b_kit_generic` 代替（**D-T60-4 明确偏差**，音色不等同原版）——逐名听感需实机对照。
- [ ] **footstep 节奏/材质**：生产系统按 **surface** 选 base 表的
  `step_dry`/`step_ink`/`step_enemy`（main.js L543-546），音量按 local/remote
  与 speed 衰减；`FootstepEvent` 无 surface 字段（默认 dry，D-T60-2）、远端 18m
  剔除未接线（D-T60-5），需复核不同地面/远近距离的脚步观感。
- [ ] **kit 事件→cue 覆盖**：`SfxGate` 零改动、元数据回退到 kit 表；
  实际挥拍/开伞/拉弓/蓄力等事件在游戏内的触发时机与音量平衡需实机复核。
- [ ] **muzzle 回写偏差（D-T60-3）**：`char/bridge.rs` 的 `get_muzzle` /
  `getMuzzleHand` / `getHeadPosition` / `aimReady` getter 已在 M4 交付，但
  `MatchEvent::Fire.muzzle` 由 `inkwave_sim` 生成且 hooks 只读，零改动 sim
  约束下未回写 FX/射击事件原点（仍用 M4 的 `approx_muzzle`）——**登记为明确
  偏差而非已接线**，待总评审裁决是否在后续任务以 inkwave 侧改写事件实现。

## Task 61 复核登记（2026-11-18）

无头可测：双相机冒烟（layer0/layer1 位集、第二相机 inactive→active→inactive、
主相机位姿/near/far 不变）+ 全部 7 处相机消费者 Query 收紧（5 reader + 2 writer，
旧文档「6 处」为口径笔误）；门禁结论见
`tasks.md` Task 61 Completion Evidence。以下为**待用户 GPU 机器复核**：

- [ ] **hub/lobby 无世界叠加**：第二相机只渲染 layer1 的 3D alley set，
  主相机渲染 layer0 关卡世界；full_frame 时应无关卡世界透出（D-T56-⑦ 占位
  语义已消除）。
- [ ] **切换过渡观感**：full_frame 进出的 set 显隐与第二相机启停时机；
  上游 `showcase.js` L3057-3085 的 cross-dissolve / `covered` 清屏尚未移植
  （D-T61-2），需确认切换无闪烁。
- [ ] **UI 层级与重复渲染 / 双相机清屏与顺序（D-T61-4）**：第二相机
  `clear_color=Default` 且与主相机同 target、同 order；两者同时 active 时，
  若第二相机后渲染会清掉 layer0（含 UI）输出。代码层面成立但仅 GPU 可确认，
  需复核相机 order / clear 策略，必要时给第二相机设 `ClearColorConfig::Nothing`。
  名牌 DOM 叠层（Task 62）与大厅 3D 的相对层级。
- [ ] **lobbySet planar reflection / 水面**：`lobbySet.js` L643-690 的地面反射
  相机与主场景水面反射相机（`environment.js` L1818-1822）和本任务第二相机的
  交互（D-T61-3）。
- [ ] **D-T61-1 层映射**：上游无 THREE.Layers 层号，layer0/layer1 为 Rust
  设计决策，需确认实际画面与「独立 Scene + 第二相机 + renderTarget」等价。

## Task 62 复核登记（2026-11-20）

无头可测：5 项 Task62 锚测（emote `t0/trig` 提取、名牌投影 NDC/scale/behind/
UI 安全区、8 站位 roster/ready/host/you 数据面、per-slot 预览增删与 hop 弧线、
emote 事件按 relay id 映射 slot 与 serial 重放）
+ 既有 11 项 showcase 冒烟 + `lobbyset-crosscheck.mjs`（emote 全字段独立重解析）；
门禁结论见 `tasks.md` Task 62 Completion Evidence。以下为**待用户 GPU/浏览器复核**：

- [ ] **名牌可读性与遮挡**：projected x/y/s 来自原始提取相机位姿与 1920×1080
  回退画布；实机不同分辨率下的字号、8 人重叠、背景霓虹上的对比度需复核。
- [ ] **满房/空房与 DOM 叠层（D-T62-2）**：数据面始终给 8 行，占用行带姓名/
  ready/host/you/emote，空位给 free 标记；真实 DOM 节点由 Task 64 接线后才出现。
  名牌与 layer1 大厅的层级还要结合 D-T61-4 双相机清屏/顺序一起复核。
- [ ] **emote→dance 流畅度（D-T62-1）**：Rust 仅以预览实体 facade 消费提取的
  `dance/v/dur/hop/t0/trig`（hop 弧线 0.42 s、variant/t0 相位；`trig` 只按
  存在性产生 squash 脉冲，`"hairflip"` 字符串语义未逐帧播放），不是完整
  `CharAnim` 骨树逐帧 pose；满房四人同时 emote 的流畅度与骨树观感需实机对照。
- [ ] **UI 安全区与 kFront（D-T62-3）**：系统层目前用 `None`/`1.0` 回退；上游
  `_measureLobby` 的动态 DOM 安全矩形与 `_lobFrame` 的 `kFront` 缩放待 Task 64
  通道接入后复核，投影纯函数已支持传入实测值。
- [ ] **动态成员锚点（D-T62-4）**：占用行目前按 `member_anchor(spot.pos,0,1)`
  与 `vis=true` 投影；JS `lobbyAnchor` 的 `M.y / M.plateH / M.sy / M.vis`
  属于运行时角色 rig 状态，空位的 `M.dest / launchT` 也未提供给 Rust 状态层，
  动态跳台/发射期间的跟随尚不完整，需 GPU/联网实机复核。
- [ ] **双相机 clear/order**：承接 D-T61-4，名牌叠层在 bloom/后处理与 layer0
  世界之上的相对顺序需在用户 GPU 机器确认。

## Task 63 复核登记（2026-11-20）

无头可测：8 项 Task63 锚测（总线图拓扑/IR 确定性烘焙逐值、湿/干与 sendScale/
distanceCut 曲线、立体声 PCM 元数据、空间 cue 湿声发声、mute 守卫、team1 声源按
(team,slot) 身份解析、wet 尾音双耳极性/低通前取样）+ `bus.json` 图/参数与
`audio-bus-crosscheck.mjs` 独立重解析（61 断言）+ kit-sfx 交叉；门禁结论见
`tasks.md` Task 63 Completion Evidence。独立评审首轮 P1（team1 声源索引 alias、
wet 取样点/双耳尾音拓扑）已返工并复验。以下为**待用户听感/GPU 复核**：

- [ ] **3D 空间感（D-T63-1）**：上游使用 WebAudio PannerNode HRTF + inverse
  distance；Rust 离线总线用等功率 pan + 提取的 inverse distance gain 近似，
  双耳定位/前后感与距离衰减听感需实机对照（headless 仅校验曲线数值）。
- [ ] **混响尾音（D-T63-2）**：上游 1.5 s / decay 3.4 卷积 IR（含 5 组早反射）；
  Rust 生产用提取早反射派生的紧凑反馈尾音（IR 烘焙仅作确定性锚测），
  plaza 空间残响的时长/亮度/尾音密度与 wasm 尺寸需用户机复核。
- [ ] **压限/限幅响度（D-T63-3）**：上游 DynamicsCompressor + limiter 为时域
  动态处理；Rust 用提取参数（glue -12/3/8/0.003/0.18、lim -2/20/0/0.001/0.08）
  驱动的静态软限幅近似，实际峰度/pumping 听感与响度一致性需实机复核。
- [ ] **混响 send 默认值（D-T63-4）**：基础 148 cue 的 per-name `reverb` 未提取，
  沿用上游默认 0.08；kit 53 名用已钉源 `kit-sfx.json` 的 per-cue reverb。
  基础 cue 混响量的场景适配需听感复核。
- [ ] **mute/low tier 保底**：静音开关与 low 档下空间湿声总线的旁通在 headless
  有锚测，实际外放设备的静音瞬时性（无残响泄漏）需用户机确认。

## Task 64 复核登记（2026-11-20）

无头可测：7 项 Task64 锚测（composite uniform 初值/逐项阈值、decay+池参数、
pass gate low 档强制关、伤害数字 combo/上限/退场、minimap 落地年龄+类别过期、
生产系统真实事件生产者与低档全关）+ `screenfx-crosscheck.mjs` 67 断言；门禁结论见
`tasks.md` Task 64 Completion Evidence。独立评审首轮 P1（落地用绝对年龄、池无生产者）
已返工复验。以下为**待用户 GPU/浏览器复核**：

- [ ] **composite pass 观感与 bloom 顺序（D-T64-2）**：shader 本体与真实 pass 未接，
  Rust 只算 pass_enabled 判据与 uniform；叠层画面、grade→screenfx→output 顺序、
  与 bloom/后处理的交互需用户 GPU 机确认，沙箱不伪验收。
- [ ] **伤害数字 DOM 可读性**：池上限 6/combo 0.45 s/存活 1.05 s 的数据面已锚测，
  真实 DOM 节点（字号/CRIT 动画/重叠避让/世界坐标跟随）需浏览器复核。
- [ ] **小地图点标与死亡标记**：8 点标/12 死亡标记与 transient FX 生命周期已钉测，
  canvas 绘制与 DOM 节点的动画观感需浏览器复核。
- [ ] **名牌 DOM 叠层通道（D-T64-3）**：Task 62 名牌数据面已并入统一 overlay
  （visible_nameplates 计数），真实 DOM 节点与 kFront/安全区接线待 wasm DOM 桥。
- [ ] **heal 叠层（D-T64-1）**：上游无对应屏幕回血效果，Rust 未新增伪效果；如需
  回血反馈应作为新需求设计，不在本任务复刻范围。
- [ ] **jump/land/boom FX 生产者（D-T64-4）**：superjump 飞行态与 `bomb:explode`
  无 MatchEvent 承载，`land_jump()` 未按 actor 过滤（JS 按 `f.actor`）；这些 FX
  的生命周期状态机仅单测覆盖，实机点标动画需浏览器复核。
