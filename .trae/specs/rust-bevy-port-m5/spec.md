# INKWAVE Rust/Bevy 移植 — M5 表现终稿与内容收尾产品需求文档（PRD）

> 前置：M1（`rust-bevy-port/`，Task 1–16）、M2（Task 17–27）、M3（Task 28–40）、
> M4（`rust-bevy-port-m4/`，Task 41–57 + M4 后技术债批次）已全部完成并 push origin/main
> （基线 workspace **1018 passed**，末次收尾矩阵 `M4_MATRIX_VERDICT=PASS`，日志
> `rust/tools/verify/logs/task57-m4-matrix-20261111T152915Z.log`）。本文档只描述 M5 增量；
> 既有架构/同步机制/网络栈/内容集/提取管线全部继续有效。
>
> **立项口径（2026-11-12）**：本文档为**计划**——所有任务 `pending`，不含任何实施结论；
> 每个任务开工时按 M4 惯例先实读 JS 锚点、在 `spec.md` 尾部补「TaskNN 实施设计」§0–§7，
> 再分批实施。范围以 M4 台账登记的顺延项为准（`rust/PORT_MAP.md`「M4 完成核对」、
> `.trae/specs/rust-bevy-port-m4/REVIEW_CHECKLIST.md`「M4 汇总复核清单」），不新增未登记范围。

## M5 决议（2026-11-12：表现终稿优先）
- **主线 = 把 M4 的全部 `placeholder` / `visual-review` / `done∗` 顺延项转正式档**，按段推进：
  - **Phase A（Tasks 58–60）内容与输入收尾**：kit 武器几何/姿态补齐（bow/brolly/bowl +
    `poseBow` + 5640 帧 goldens 重生成）、Rust 设置屏与持久化、kit 系 42 名 SFX 与
    footstep/射击原点回写消费者。
  - **Phase B（Tasks 61–67）表现终稿**：RenderLayers 多相机分层（升级 M4 D-T56-⑦ 的
    单相机前提）、3D 大厅角色 preview 与名牌锚定、audio 混响总线与 3D panner、
    fx composite pass 与 DOM 叠层、GTAO/grade/SMAA/tilt-shift 后处理链、
    天空/云/PMREM/海面终稿、3D alley 材质/反射/霓虹终稿。
  - **Phase C（Tasks 68–70）模型与收尾**：Boss 高保真模型（bossModelGeo/Anim/Mats/Fx）、
    locker 更衣室、M5 收尾（台账/门禁矩阵/总评审/提交 push）。
- **延续 M4 双轨验收**（NFR-M4-5 → NFR-M5-4）：沙箱只验无头可测部分，GPU/听感项
  登记 `REVIEW_CHECKLIST.md` 待用户机器复核，不做伪验收。
- **Task 70 = M5 收尾**（沿用 Task 57/27/40 流程）。

## Overview
- **Summary**: 把 Rust 版从「功能对等 + 占位表现」推进到「表现终稿」：kit 武器全套
  几何/姿态/音效、设置屏、多相机分层与 3D 大厅终稿、音频总线终稿、后处理链与
  环境终稿、Boss 高保真模型与 locker 更衣室。
- **Purpose**: M4 后玩法与数据面已对等且全绿，剩余缺口几乎全部集中在渲染/音频的
  **保真度**与**菜单态体验**（设置屏、locker、3D 大厅角色）。这些项验收依赖真实 GPU/音频
  设备，故逐项落地并如实登记复核项，`low` 档保底可玩路径（D-T55-⑦ 语义）必须始终可用。
- **Target Users**: 仓库维护者；最终验收口径 = Rust 客户端在用户 GPU 机器上的
  画面/声音与 JS 上游不可区分（逐项对照 REVIEW_CHECKLIST）。

## Goals
- **G-1 kit 专项收尾**：`extract-character.mjs` 目前只注册 mitts/blade 两个 kit 模型
  （D-T54-5⑩）；补 bow/brolly/bowl 几何 + `build.rs::pose_bow` 空实现 + kit 姿态，
  重生成 5640 帧 pinned goldens；42 名 kit 系 SFX（`kits/*-sfx.js` 自注册，Task 50 顺延）
  接线；`getMuzzle/getHeadPosition/aimReady` 与 footstep 队列接生产消费者（D-T54-5⑫）。
- **G-2 Rust 设置屏**：`set_volumes({master,music,sfx})` ≡ audio.js L165-172 已实现但
  **无 UI 调用点**（`#[allow(dead_code)]` 在案）；补音量/画质/主题/灵敏度 UI 与持久化
  （JS 侧由设置菜单写 localStorage；Rust 侧 native/wasm 的持久化载体是实施设计决策点，
  登记 D-T 候选）；`INKWAVE_QUALITY`/`INKWAVE_THEME` 仍保留为启动覆盖。
- **G-3 3D 大厅与多相机终稿**：M4 D-T56-⑦ 为避免破坏 6 处单相机消费者而复用主相机，
  关卡世界未分层（hub/lobby 与占位 alley 叠加）；M5 引入 RenderLayers 分层 + 正式
  第二相机，`level_render.rs` 已按 `Camera::is_active` 选相机（M4 加固）作为基础；
  接 `spawn_char_rig` 菜单态驱动（脱离 `CharBridge` 的 `DemoSim/paused` 耦合）、
  `lobbyAnchor/lobbySlotAnchor` 3D→DOM 名牌、emote→dance、ready/host/you 名牌态
  （状态层 `ShowcaseMember`/`emote()`/`ShowcaseCue` M4 已交付并钉测）。
- **G-4 音频总线终稿**：Task 50 的 dry 占位档升级——convolver 混响 / glue compressor /
  limiter / 3D panner / 湿信号（D-T50 顺延），不新增 crate 依赖（沿用 bevy_audio +
  PcmSound 选型）。
- **G-5 fx 与 DOM 叠层**：screenfx composite pass（fx.js/screenfx.js 的 vignette/受伤/
  治疗等叠层）、伤害数字 DOM 叠层、小地图 DOM 点标效应（Task 49/51 顺延）。
- **G-6 后处理链**：GTAO、GradeShader 12 uniform、SMAA、tilt-shift diorama（renderer.js
  218，D-T55-⑦ 顺延）；**逐档可关**，`low` 档 `post_processing_enabled=false` 语义不破坏。
- **G-7 环境与大厅材质终稿**：云烘焙/PMREM/远景反射、海面 swell·marina·泡沫
  （environment.js/renderer.js）；3D alley 的湿地面 planar reflection/霓虹/蒸汽/PMREM
  （lobbySet*，Task 56 占位转正式）。
- **G-8 Boss 高保真模型与 locker**：bossModelGeo 875 / bossAnim 859 / bossMats 567 /
  bossModelFx 143（≈2400 行，100-bone 蒙皮/≤150k tri 预算/5 材质+stencil atlas），
  替换 M4 的体块剪影占位档；locker 更衣室（character-outfit）。
- **G-9 门禁与纪律延续**：基线自 1018 单调递增；上游 JS 树/`server/` 零改动；
  `cargo tree -p inkwave_sim` 依赖面不变；九段门禁每任务全绿
  （`gate-task57.sh` 为模板，TRUNK 段沙箱 DEGRADED 按 D-T40-1 登记）。

## Non-Goals（M5 明确不做）
- **手柄/移动端/发布部署**：延续不做。
- **不改上游树**：`src/`、`server/`、`tools/`、`docs/` 零改动（fixture 扩展脚本仍放 `rust/tools/`）。
- **PROTO v1 / net-fixtures**：网络协议冻结，M5 不动（Boss 联网 M4 已对等）。
- **`_zoneJump`、climb 边**：维持 M1 既有 follow-up 口径。
- **新玩法模式/新地图**：内容集在 M4 已封顶（8 图 + turf/zones/boss）。

## Background & Context（承接 M4，未重新实读）
- M4 已交付并钉测、M5 直接消费的资产：`assets/lobbyset.json`（8 站位/相机/pedestal/emotes）、
  `assets/environment.json`（四档三主题）、`assets/char/*`（IKC1 rig + 5640 帧 pinned replay，
  仅缺 bow/brolly/bowl 几何）、`audio/sfx_table.rs`（148 名全表 + SfxGate）、
  `ui/showcase_state.rs`（CameraPose/AlleyDims/Pedestal/sync_from_online_view）。
- 渲染基座现状：`level_render.rs` 单相机 + `is_active` 选择；`world.rs` 相机带
  `Bloom`（档位开关）；`level.wgsl` 湿润/光泽与 46 层材质 CPU 烘焙已接（Task 52）；
  fx 六池 + hooks 已接但 composite pass 未接（Task 51）。
- 具体行号锚点**一律在各任务开工时实读定稿**（上游树自基线 `3e9b550…` 起未变，
  但 M4 文档中的行号是当时快照，不作为 M5 实施依据）。

## Functional Requirements
### Phase A：内容与输入收尾（Tasks 58–60）
- kit 几何/姿态/goldens、42 名 kit SFX、footstep 与射击原点回写消费者、Rust 设置屏
  与持久化（音量三轨 taper^1.5/τ=0.04 语义对齐 audio.js L165-172）。

### Phase B：表现终稿（Tasks 61–67）
- RenderLayers 分层 + 第二相机、3D 大厅角色/名牌/emote、混响总线、screenfx composite、
  伤害数字与小地图 DOM 叠层、GTAO/grade/SMAA/tilt-shift、云/PMREM/海面、alley 材质终稿。

### Phase C：模型与收尾（Tasks 68–70）
- Boss 高保真模型、locker 更衣室、M5 收尾。

## Non-Functional Requirements
- **NFR-M5-1**：上游 JS 树/`server/` 零改动；`cargo check --target wasm32-unknown-unknown` 通过。
- **NFR-M5-2**：`cargo tree -p inkwave_sim` 依赖面不变（新依赖只进 `inkwave`）。
- **NFR-M5-3**：workspace 测试基线单调递增（1018 → …），每任务九段门禁落盘日志。
- **NFR-M5-4**：双轨验收（NFR-M4-5 延续）——GPU/听感项登记复核，不伪验收。
- **NFR-M5-5**：`low` 档保底可玩路径不被破坏（所有新后处理/音频特性必须可关；
  SwiftShader 下不 panic、不显著掉帧的降级路径要有 headless 冒烟）。
- **NFR-M5-6**：数据一律走提取管线（禁手抄），`source.commit` 钉上游 JS 提交
  （`upstream-commit.mjs` 口径，D-T37-3 已消除）。

## Acceptance Criteria
### AC-M5-1: kit 武器与音频消费闭环
kit 武器（bow/brolly/bowl）几何/姿态/5640 帧 goldens 全绿；42 名 kit SFX 与
footstep/射击原点消费者接线并有锚测；`low`/`ultra` 两档 headless 600 帧不 panic。

### AC-M5-2: 设置屏与持久化
设置屏可调音量/画质/主题并在重启后生效（wasm 与 native 双载体）；`INKWAVE_*`
环境变量仍为启动覆盖；设置变更不重建天空贴图与已生成 rig 的语义与 D-T55-⑦ 一致或有明确偏差登记。

### AC-M5-3: 多相机分层与 3D 大厅/Boss/locker 终稿
关卡世界与 UI/大厅分层，hub/lobby 不再与关卡世界叠加；3D 大厅角色 preview、名牌、
emote 接运行期消费者；Boss 呈现替换为高保真模型档；locker 可用。视觉面登记复核。

### AC-M5-4: 后处理与环境终稿
GTAO/grade/SMAA/tilt-shift、云/PMREM/海面、alley 材质逐档落地且逐档可关；
`low` 档全关保底；REVIEW_CHECKLIST 逐项登记待用户复核。

### AC-M5-5: 门禁与纪律延续
收尾矩阵（模板 `gate-task57.sh`）全绿；上游树/sim 依赖面守卫通过；
独立只读总评审 P0/P1 闭环；提交 + push origin/main。

---

## Task58 实施设计（2026-11-12，开工时实读定稿）

### §0 范围修正（开工实读结论）
- **上游 JS 没有 bowl 武器**：`src/config.js` 无 `WEAPONS.bowl`，`src/game/kits/` 无
  bowl kit 文件，`HOLD` 无 bowl 入口——"bowl"只存在于 blade-model.js 注释与
  char.schema.md batch-C 备注。Task 58 实际范围 = **bow + brolly**（与
  extract-character.mjs `kitWeapons` 一致），bowl 作为「上游不存在」事实登记，不移植。
- JS 锚点实读：`character.js` L466-496（`ANIM_OF.bow='charger'`、`HOLD.bow` 全表、
  `BOW_CANT=-0.85`、`poseBow`）、`kits/bow-model.js`（230 行：`buildBow`/`animateBow`、
  BRACE=0.1/DRAW=0.22/FLEX=0.3、弦欠阻尼弹簧 ωm=2π·15/z=0.3）、
  `kits/brolly-model.js`（383 行：`brollyDef`/`animateBrolly`）。
- `animateBrolly` 的伞面机构由 `st.runner.kit.brolly`（kits/brolly.js 状态机）驱动，
  Rust 无对应 runner kit 状态；变换面 = runner collar z（L354）+ trigger follow 弹簧
  （L355-359），其余（gores/struts/tears/lamp/fabric）登记偏差。

### §1 数据面（提取管线，NFR-M5-6）
- `extract-character.mjs`：注册 `kits/bow-model.js`（自注册）与 brolly（手动
  `registerWeaponModel`）；`upstreamCommit` 路径追加两 kit 文件钉源；
  `kitWeapons=['blade','brolly','mitts','bow']` + bowl 不存在备注。
- `char-pose-reference.mjs` 镜像注册；场景追加 K2 段（bow 满蓄→release→regen、
  tap shot、空中 cant 往返；brolly shoot 相）+ `wpKit` dump（bowDraw/bw 五字段/trig）。
- 四件套重生成：templates 152→184、weapons 17→19、weaponKinds 15、fixture
  5640→6097 帧；`char.schema.md` 补 twirl 可选 / drawLen 仅 bow 的口径。

### §2 Rust 数据结构
- `tables.rs`：`WeaponFrames.draw_len: Option<f64>`（消费点镜像 JS `|| 0.2` 语义
  → `.filter(|v| *v != 0.0).unwrap_or(0.2)`）。
- `anim.rs`：`WeaponInstance.bw: Option<BowKin>`（rest/ax/str/str_v/loose/last_rel/glow）；
  `CharAnim.bow: BowAnim{lr:Option,hold,draw,cant:Option}`（`Option` 镜像 JS `??`）。
- `weapon.rs`：`Wst.bow_draw/bow_len`（pose 层 → weapon 层路由，≡ JS `w.bowDraw`）。

### §3 姿态层 `pose_bow`（build.rs ≡ character.js L479-496）
- 满签名纯函数（p/anc/ancr/bow/charge/last_release/grounded/w_aim/len/dt），
  调用点保留在 `pose_weapon` L2028 位置；Euler-YXZ 用 `math.rs::euler_yxz`。

### §4 武器层 `anim_bow` / `anim_brolly`（weapon.rs）
- `anim_bow` ≡ bow-model.js L148-199（harness 路径无 runner）：每实例 init、释放沿、
  欠阻尼弹簧、limb 屈曲、`tipAt`/`aimString`（弦两半 aim+stretch）、箭矢 show 窗口、
  glow 包络状态（材质写 = B2 no-op）。
- `anim_brolly` ≡ L353-359 变换面；`BOW_RING1` 常量以锚测钉在
  `assets/tuning.json`（`weapons.bow.ring1`）上，禁止静默漂移。

### §5 锚测（TR-58.1）
- 既有 pinned replay 扩展：`wpParts`（全部 part pos/quat/scale）+ `wpOff` +
  `wpKit`（bowDraw、弦弹簧状态、loose 布尔、brolly trig）逐帧比对；6097 帧全绿。
- 新增 `bow_ring1_matches_extracted_tuning`；`cargo fmt/clippy/test/wasm` 全过。

### §6 评审结论（独立只读评审，2026-11-12）
- P0：0；P1：1（BOW_RING1 手抄 → 已修：tuning.json 互钉锚测）；P2：7（注释级/
  不可达路径，已修 6 处注释与 `||` 语义镜像、struts 偏差措辞、死常量清除）。
- 复验：REVIEW_VERDICT: PASS。

### §7 门禁
- `gate-task58.sh`（模板 gate-task56.sh，DETERM 增补 char-pose-ref.json 双跑）；
  TRUNK 段按 D-T40-1 登记；REVIEW_CHECKLIST 登记 bow 持握/开镜、brolly 伞面、
  （bowl 不存在）视觉复核项。
