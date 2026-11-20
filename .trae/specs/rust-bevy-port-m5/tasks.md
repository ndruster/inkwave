# M5 任务清单（表现终稿与内容收尾）

> 状态口径沿用 M4：`pending` → `in_progress` → `completed`（含 Completion Evidence）。
> 每任务固定流程：实读 JS 锚点 → 本目录 `spec.md` 尾部「TaskNN 实施设计」§0–§7 →
> 分批实施 → 主上下文实读核验 → 独立评审（P0/P1/P2 当场修复复验）→ 九段门禁
> （模板 `rust/tools/verify/gate-task57.sh`）→ 勾选 + Completion Evidence →
> PORT_MAP/台账 → commit → 中文汇报。
> 所有条目当前为**计划**，行号/体量以开工时实读为准。

## Task 58: kit 武器几何与姿态收尾（bow/brolly/bowl）
- **Status**: `completed`
- **Priority**: high
- **Depends On**: M4 Task 54
- **Description**: `extract-character.mjs` 目前只注册 mitts/blade 两个 kit 模型（M4 D-T54-5⑩）；
  补 bow/brolly/bowl 几何注册、`build.rs::pose_bow` 空实现、kit 姿态锚测，并重生成
  5640 帧 × 87 骨 pinned goldens（char.bin.gz/char.json/char-poses.json/char-pose-ref.json 四件套）。
- **Acceptance Criteria Addressed**: AC-M5-1
- **Test Requirements**:
  - `rule` TR-58.1: 提取双跑逐字节一致 + 新增 kit 段锚测 + pinned replay 全量重生成后
    既有锚测不回退；wasm32 check 通过（内嵌 archive 逐字节钉测同步更新）。
  - `rule` TR-58.2: REVIEW_CHECKLIST 视觉登记（bow 持握/开镜姿态、brolly 伞面、bowl）。
- **Notes**: bow 是正式可选武器但 fixture 场景无 bow 段，需补 fixture 或逐值锚测；视觉复核项登记。
- **Completion Evidence（2026-11-12）**：
  - 范围修正：上游 JS **无 bowl**（config/kits/HOLD 零入口，仅注释残留）；实际交付 bow + brolly，bowl 作为事实登记不移植。
  - 提取器：`extract-character.mjs` 注册 bow/brolly kit 模型并钉两文件上游提交；`char-pose-reference.mjs` 镜像注册 + K2 场景 + `wpKit` dump。
  - 四件套：`char.json` templates 152→184 / weapons 17→19 / weaponKinds 15；`char.bin.gz` 4.89 MB；`char-pose-ref.json` 5640→**6097 帧**（bow 336 + brolly 3014 帧）。
  - 移植：`pose_bow` ≡ character.js L479-496；`anim_bow` ≡ bow-model.js L148-199；`anim_brolly` ≡ L353-359 变换面（伞面 kit 状态机偏差登记）；`WeaponFrames.draw_len` 走提取管线；`BOW_RING1` 由 tuning.json 互钉锚测防漂移。
  - 测试：`cargo test --workspace` 全绿（393 项，含新增 `bow_ring1_matches_extracted_tuning`）；6097 帧 pinned replay 逐帧比对 bones/wpOff/wpParts/wpKit（bowDraw/弦弹簧/loose/trig）全绿；`wasm32 check` 通过。
  - 独立评审：首轮 PASS（P0=0/P1=1/P2=7），P1/P2 已修，复验 REVIEW_VERDICT: PASS。
  - 门禁：`rust/tools/verify/logs/task58-gates-20261112T002725Z.log` → `GATE_VERDICT=PASS`
    （FMT/CLIPPY/TEST/WASM/TREE/DETERM/XCHK/HEADLESS/HEADLESS_BOSS/RELEASE_BUILD 全 0，
    `DETERM=BYTE_IDENTICAL`，JS_TREE_CLEAN/SIM_TREE_CLEAN；TRUNK 一次尝试被
    wasm-bindgen SIGKILL → `TRUNK=DEGRADED` 按 D-T40-1 环境性非阻断登记）。
  - 复核登记：`.trae/specs/rust-bevy-port-m5/REVIEW_CHECKLIST.md`「Task 58 复核登记」
    （bow 持握/开镜、弓体发光、brolly 伞面偏差、bowl 不存在、手感）。

## Task 59: Rust 设置屏与持久化
- **Status**: `completed`
- **Priority**: high
- **Depends On**: 无（可与 Task 58 并行）
- **Description**: 音量三轨（`set_volumes` ≡ audio.js L165-172 已实现、无 UI 调用点）+
  画质四档/主题三档/灵敏度 UI；持久化载体（JS 用 localStorage；Rust native 用配置文件、
  wasm 用 web-sys storage——实施设计定稿，登记 D-T 候选）。`INKWAVE_QUALITY`/`INKWAVE_THEME`
  保留为启动覆盖；运行期切档不重建天空贴图/已生成 rig 的 D-T55-⑦ 语义保持或有偏差登记。
- **Acceptance Criteria Addressed**: AC-M5-2
- **Test Requirements**:
  - `rule` TR-59.1: 设置往返持久化单测（写→读→应用）、音量参数锚测（taper^1.5/τ=0.04）、
    low 档全关语义回归；headless 600 帧不 panic。
  - `rule` TR-59.2: REVIEW_CHECKLIST 登记（UI 布局/焦点/与 online 屏共存）。
- **Notes**: JS 侧设置菜单是 DOM 屏，Rust 侧复用既有 egui/自绘 UI 体系，实施设计时定稿。
- **Completion Evidence（2026-11-13）**：
  - 数据面：`environment.json:defaultSettings`（21 键，来自 `config.js` L561-591，
    `source.commit` 已钉）由 `settings.rs` 以 `include_str!` + `JVal` 读取，
    缺键即 panic（无手抄回退）；新增 `settings-crosscheck.mjs` 独立正则解析器
    双向核对 21 键 → `SETTINGS_CROSSCHECK=PASS keys=21`。
  - 实现：新增 `settings.rs`（native 配置文件 / wasm `inkwave.rust.settings`
    localStorage 双载体、宽容逐键解码、0.05 网格、启动覆盖不落盘）、BEVY
    设置屏（8 行 + RESET/BACK + 实时值标签 + Esc/Enter 返回）、三音量轨
    `set_volumes` τ=0.04 接线、画质/主题/阴影/泛光经 `EnvRefresh` 一帧刷新、
    `LookSettings` 灵敏度启动应用。
  - 偏差登记：D-T59-1 native 文件载体、D-T59-2 Rust 扩展 theme、D-T59-3
    fov 存而不应用、D-T59-4 wasm 独立键（避免覆写 JS 版 9 个未移植字段）；
    未接线的 9 个 JS 键与 5 个只存不用字段、RESET 无确认等登记于
    `REVIEW_CHECKLIST.md`「Task 59 复核登记」。
  - 测试：`cargo test --workspace` 全绿；13 项 Task 59 锚测含真实
    `Pointer<Click> → settings_clicks → save() → reload` 链路、真实 `save()`/
    `load()` 往返、启动覆盖不落盘、low 档全关、`taper^1.5` 初值；
    `wasm32 check` 通过、`cargo tree -p inkwave_sim` 依赖面不变。
  - 独立评审：首轮 FAIL（3×P0 + 6×P1）→ 全部修复（wasm 分键、宽容解码、
    boot 覆盖隔离、真实点击锚测、无 HOME 依赖、Esc 返回、设计落档）；
    复验 `REVIEW_VERDICT: PASS`。
  - 门禁：`rust/tools/verify/logs/task59-gates-20261113T145044Z.log` →
    `GATE_VERDICT=PASS`（FMT/CLIPPY/TEST/WASM/TREE/DETERM/XCHK/HEADLESS/
    HEADLESS_BOSS/RELEASE_BUILD 全 0，`DETERM=BYTE_IDENTICAL`，
    JS_TREE_CLEAN/SIM_TREE_CLEAN；TRUNK 一次尝试 wasm-bindgen SIGKILL
    → `TRUNK=DEGRADED` 按 D-T40-1 环境性非阻断登记）。前两轮日志
    （首轮评审前 / wasm `write_at` 编译错误）保留作为完整证据链。

## Task 60: kit SFX 与生产消费者接线
- **Status**: `completed`
- **Priority**: high
- **Depends On**: M4 Task 50/54；与 Task 58 可并行
- **Description**: **53 名**（实读修正，旧文档写 42）kit 系 SFX（`kits/*` 自注册，
  Task 50 事件缺口）接线；footstep 队列接音频 hook（M4 D-T54-5⑫ 测试自 drain → 生产消费）；
  `getMuzzle/getHeadPosition/aimReady` 回写射击/FX 消费者（替换 sim 射击原点直用）。
- **Acceptance Criteria Addressed**: AC-M5-1
- **Test Requirements**:
  - `rule` TR-60.1: 53 名 SFX 表提取锚测（≡JS 自注册顺序/参数）+ 事件→cue 映射单测 +
    footstep 消费节奏锚测；`low` 档音频关闭路径回归。
  - `rule` TR-60.2: REVIEW_CHECKLIST 听感登记（kit SFX 触发时机/音量平衡，用户机复核）。
- **Notes**: 不新增 crate 依赖（D-T50 选型延续）；SfxGate voice cap 语义复用。
- **Completion Evidence（2026-11-18）**：
  - 口径修正：实读 live 上游 **53 名** kit SFX（旧文档 42 为笔误，D-T60-1）；
    新增 `extract-kit-sfx.mjs`（import 真实 SFX 注册表，14 个上游文件钉
    `source.commit`）→ `assets/audio/kit-sfx.json`（53 名，gain/max/jitter/
    reverb/minGap）；接入 `run-all.sh`；`kit-sfx-crosscheck.mjs` 独立正则双向
    核对 53 名 → `KIT_SFX_CROSSCHECK=PASS names=53`。
  - Rust：`audio/sfx_kit.rs` 以 `include_str!` + JVal 解析 53 名，
    `kit_meta()` 供 `sfx_table::meta()` 回退，**SfxGate 零改动**；
    53 名 `build(v,p)` 属上游代码不可提取 → `b_kit_generic` 通用合成
    （D-T60-4 明确偏差，音色需 GPU/听感复核）。
  - 脚步生产消费者：`audio.rs::footstep_sfx` 注册于 `char_pose_bridge` 之后，
    `take_footsteps()` 真 drain → surface 规则 `step_dry/step_ink/step_enemy`
    （≡ main.js L543-546）→ 真 `sfx.cue()` bake + 真 `AudioPlayer` spawn
    + `SfxGate` + 音量公式（local 0.7/remote 0.45 × speed 衰减）。
  - 偏差登记：D-T60-1（42→53）、D-T60-2（FootstepEvent 无 surface，默认 dry）、
    D-T60-3（muzzle getter 已有但 sim 事件只读，未回写）、D-T60-4（通用合成
    音色）、D-T60-5（远端 18m 剔除未接线）均在 M5 REVIEW_CHECKLIST。
  - 测试：`sfx_kit` × 4（53 名/唯一性、meta 对齐提取值、门控零改动播 kit cue、
    surface cue + 三 base cue 真 bake）+ `task60` × 1（真实 Bevy app：bridge 入队
    → 系统 drain → 真 bake + voice 登记 → 静音守卫）；设备缺失时 AudioPlayer
    entity 被即时回收，锚定 bake/voice 记帐而非 entity 寿命。
  - 独立评审：首轮 FAIL（P0 假接线：只计 gate 数且 cue 名不存在；P1 无门禁、
    53 名无合成音色、speed 门伪造）→ P0 修为真 cue + 真 bake/spawn，
    P1 修为 surface 规则 + 通用合成偏差 + 文档 §0-§6/42→53 同步；
    复验 `REVIEW_VERDICT: PASS`（余 P2 纯注释项已修）。
  - 门禁：`rust/tools/verify/logs/task60-gates-20261118T050954Z.log` →
    `GATE_VERDICT=PASS`（FMT/CLIPPY/TEST/WASM/TREE/DETERM/XCHK/HEADLESS/
    HEADLESS_BOSS/RELEASE_BUILD 全 0，`DETERM=BYTE_IDENTICAL`，
    JS_TREE_CLEAN/SIM_TREE_CLEAN；TRUNK 一次尝试 wasm-bindgen SIGKILL
    → `TRUNK=DEGRADED` 按 D-T40-1 环境性非阻断登记）。前两轮日志
    （启动 clippy 失败 / 中途返工）保留作为完整证据链。

## Task 61: RenderLayers 分层与第二相机
- **Status**: `completed`
- **Priority**: high
- **Depends On**: M4 Task 56（`level_render.rs` `is_active` 加固是基础）
- **Description**: 关卡世界/UI/大厅按 RenderLayers 分层并启用正式第二相机，消除 M4
  D-T56-⑦ 的「复用主相机 + 世界叠加」占位语义；6 处单相机 `Query<&GlobalTransform, With<Camera>>`
  消费者逐一迁移/收紧；`full_frame` 相机覆盖改为第二相机启停。
- **Acceptance Criteria Addressed**: AC-M5-3
- **Test Requirements**:
  - `rule` TR-61.1: 最小 Bevy App 运行期冒烟（双相机 build/分层可见性/切换恢复，
    沿用 M4 `wired_systems_*` 模式）+ 全部相机消费者 Query 迁移后的冲突检查。
  - `rule` TR-61.2: REVIEW_CHECKLIST 登记（hub/lobby 无叠加、切换过渡观感）。
- **Notes**: M4 REVIEW_CHECKLIST L142 的分层方案（逐实体加层 + UI 指定 target camera）为起点。
- **Completion Evidence（2026-11-18）**：
  - 口径修正：上游无 THREE.Layers 层号（独立 Scene + 第二相机 + renderTarget，
    showcase.js L3042-3085）；Rust layer0=关卡/FX/UI、layer1=3D lobby set
    为设计决策（D-T61-1）。旧文档「6 处消费者」实读为 5 reader + 2 writer。
  - `world.rs` 增 `MainCamera`/`ShowcaseCamera` marker 与层常量；主相机显式
    layer0；`camera_rig` 与 Bloom 的 `environment_controls` 均限定 `MainCamera`。
  - `showcase_scene.rs` 从提取的 `camera_pose` spawn layer1 第二相机（初始
    inactive），`showcase_camera_update` 只写第二相机位姿/near/far 与 is_active；
    主相机 transform/near/far 完全不变（D-T56-⑦ 占位语义消除）；set 根与子
    实体标记 layer1。
  - 7 处消费者收紧：`fx/render.rs`、`fx/hooks.rs`、`fx/swim_wake.rs`、
    `fx/screen_fx.rs`、`level_render.rs`（reader）+ 上述两 writer，双相机下
    `.single()` 不再失效、FX 不静默关闭。
  - 测试：`showcase_scene` 双相机冒烟（真实 Bevy app + 层位集 helper +
    inactive→active→inactive + 主相机不变性 + pedestal cue）11 项全绿；
    `cargo test --workspace` 全绿；clippy/wasm check 通过。
  - 独立评审：首轮 FAIL（P1 Bloom 应用到第二相机、P1 双相机清屏/顺序风险
    未登记、P1 门禁未闭合）→ Bloom query 显改 `With<MainCamera>`、
    D-T61-4 登记清屏/order 为 GPU 复核项，文档「6 处」口径改为 5 reader +
    2 writer、过时调度注释修正；修复后 clippy 复验干净。
  - 门禁：`rust/tools/verify/logs/task61-gates-20261118T082702Z.log` →
    `GATE_VERDICT=PASS`（FMT/CLIPPY/TEST/WASM/TREE/DETERM/XCHK/HEADLESS/
    HEADLESS_BOSS/RELEASE_BUILD 全 0，`DETERM=BYTE_IDENTICAL`，
    JS_TREE_CLEAN/SIM_TREE_CLEAN；TRUNK 一次尝试 wasm-bindgen SIGKILL
    → `TRUNK=DEGRADED` 按 D-T40-1 环境性非阻断登记）。
  - 复核项：D-T61-2（cross-dissolve 未移植）、D-T61-3（planar reflection
    交互）、D-T61-4（双相机清屏/order）均在 M5 REVIEW_CHECKLIST 登记为
    GPU-only 复核项；未在沙箱伪验收。

## Task 62: 3D 大厅终稿（角色 preview / 名牌 / emote）
- **Status**: `completed`
- **Priority**: medium
- **Depends On**: Task 61
- **Description**: 接 `spawn_char_rig` 菜单态驱动（脱离 `CharBridge` 的 `DemoSim/paused` 耦合）、
  站姿/舞蹈、`lobbyAnchor/lobbySlotAnchor` 3D→DOM 名牌跟随、emote→dance 播放、
  ready/host/you 名牌态、站位标记随 roster 增减（M4 D-T56-⑧ 全项）；
  消费状态层 `ShowcaseMember`/`emote()`/`ShowcaseCue`（M4 已交付并钉测）。
- **Acceptance Criteria Addressed**: AC-M5-3
- **Test Requirements**:
  - `rule` TR-62.1: 状态→场景联动单测扩展（roster 增减/ready 切换/emote 触发）+
    名牌投影锚测；headless 不 panic。
  - `rule` TR-62.2: REVIEW_CHECKLIST 登记（名牌可读性/遮挡/满房空房）。
- **Notes**: DOM 叠层依赖 Task 64 的 DOM 通道，可交叉。
- **Completion Evidence** (2026-11-20):
  - 数据面：`showcase_state::Emote` 补 `t0/trig`（flex 2.86/hairflip，
    `lobbyset-crosscheck.mjs` 独立重解析 JS 全字段）；新增 8 站位名牌投影纯函数
    （NDC→像素/behind/right-vector scale/kFront/UI 安全区）与 `LobbyNameplates`
    数据面（ready/host/you/free/emote 随 roster 同步）。
  - 生产链路：`SessionEvent::Emote {id,name}` 在 `ui_state_machine` 的单一
    `poll_events` 泵中消费，用 `slot_for_player` 映射同一 team-ordered slot；
    `ShowcaseMember::emote_serial` 保证同名 emote 重放是新的 one-shot。
  - `showcase_scene` 按 roster 增删 layer1 per-slot preview，消费提取的
    `dance/v/dur/hop/t0/trig` 驱动 0.42 s hop 弧线与 dance 变体相位；新成员入场
    在 spawn 时写入起始时间（修掉 Commands 延迟导致的漏启动）；low tier 不建实体，
    主相机不被触碰；Bevy 冒烟锚定层位集/位置/roster 增减/重放/名牌增删。
  - 锚测：5 项 Task62 专项 + 16 项 showcase + 既有 showcase/相机/环境/props
    crosscheck 全绿；workspace 412 + 601 测试通过。
  - 九段门禁 `rust/tools/verify/logs/task62-gates-20261119T174824Z.log`：
    `FMT/CLIPPY/TEST/WASM/TREE/DETERM/XCHK/TRUNK/RELEASE_BUILD/HEADLESS/
    HEADLESS_BOSS_EXIT=0`，`DETERM=BYTE_IDENTICAL`，`JS_TREE_CLEAN`、
    `SIM_TREE_CLEAN`，`GATE_VERDICT=PASS`（TRUNK 本次真实通过，非 DEGRADED）。
    独立评审首轮 P0（emote 事件断链）与 P1（同名重放/spawn 时序/台账）已返工
    并复验；D-T62-1..4 与 D-T61-4 相关项登记于 M5 REVIEW_CHECKLIST。

## Task 63: 音频总线终稿（混响/压限/3D panner）
- **Status**: `completed`
- **Priority**: medium
- **Depends On**: M4 Task 50
- **Description**: convolver 混响 / glue compressor / limiter / 3D panner / 湿信号
  （Task 50 dry 占位档升级）；混响 IR 与总线拓扑走提取或程序化（实施设计定稿）；
  不新增 crate 依赖。
- **Acceptance Criteria Addressed**: AC-M5-4
- **Test Requirements**:
  - `rule` TR-63.1: 总线图结构单测 + 混响 IR 确定性烘焙 + 湿/干比例锚测 +
    3D panner 衰减曲线锚测；mute 回归延续。
  - `rule` TR-63.2: REVIEW_CHECKLIST 听感登记（空间感/混响尾音，用户机复核）。
- **Notes**: 烘焙成本与 wasm 尺寸需评估（`char.bin.gz` 内嵌先例，D-T54-5⑨）。
- **Completion Evidence** (2026-11-20):
  - 数据面：`extract-audio-bus.mjs` 从只读 `audio.js`/`music.js` 提取总线拓扑
    （11 边 / glue+lim 参数 / highpass / revSend/ret / taper / sfxIn）、3D 参数
    （distCut 10/22000/900/13、panning HRTF→equalpower 回退、inverse/ref 3/
    rolloff 1.2/maxDistance、sendScale、reverbDistance、defaultReverb 0.08）与 IR
    描述符（1.5 s/decay 3.4/pre 0.012/seed 3/5 taps/attack 0.002/darken 4/
    right 0.0031），钉源 `assets/audio/bus.json`（commit 159ba72b）。
  - `audio/bus.rs` 消费提取表：`bake_ir` 是 `makeImpulse` 的 bit-exact 端口（仅作
    确定性锚测）；生产用提取 taps 派生的 O(n) 紧凑 plaza 尾音，`distance_cut`/
    `send_scale`/`distance_gain`/`pan`/`compressor_gain`/`mix_voice` 复刻 JS 曲线。
  - 生产接线：`Pcm` 扩为带 `channels` 的交错缓冲、`PcmIter` 向 rodio 报真实声道数；
    `audio_sfx`/`footstep_sfx` 以 `MainCamera` 位姿为 listener，shot/empty 用本地
    actor、splat 用 victim 位置作声源，经立体声湿/干 voice 播放；`sfx_table::
    reverb_for` 让 kit 53 名用各自 reverb、基础 148 cue 用默认 0.08。
  - 锚测：8 项 Task63 专项（图拓扑/IR 逐值+双跑 hash、湿/干与距离曲线、立体声
    元数据、空间 cue 湿声发声、mute 守卫、team1 声源按 (team,slot) 身份解析、
    wet 尾音双耳极性/低通前取样）+ `audio-bus-crosscheck.mjs` 61 断言独立重解析
    + kit-sfx 交叉全绿；IR 修订后 clippy 复验通过。
  - 九段门禁 `rust/tools/verify/logs/task63-gates-20261119T192251Z.log`：
    `FMT/CLIPPY/TEST/WASM/TREE/DETERM/XCHK/TRUNK/RELEASE_BUILD/HEADLESS/
    HEADLESS_BOSS_EXIT=0`，`DETERM=BYTE_IDENTICAL`，`JS_TREE_CLEAN`、
    `SIM_TREE_CLEAN`，`GATE_VERDICT=PASS`（TRUNK 本次真实通过）。
    独立评审首轮 P1（team1 声源 5+slot 索引 alias/越界、wet 从低通后取样且乘
    voice gain/pan、尾音单声道）已返工：改用 `find_actor(team,slot)`、wet 从
    `head` 取样并走双耳极性尾音；D-T63-1..4 与听感项登记于 M5 REVIEW_CHECKLIST，
    未在沙箱伪验收 HRTF/完整 FIR/WebAudio envelope。

## Task 64: fx composite pass 与 DOM 叠层
- **Status**: `completed`
- **Priority**: medium
- **Depends On**: M4 Task 51/49
- **Description**: screenfx composite pass（vignette/受伤/治疗/复活叠层，fx.js/screenfx.js）、
  伤害数字 DOM 叠层、小地图 DOM 点标效应（Task 49/51 顺延）；与 Bevy UI 的合成顺序定稿。
- **Acceptance Criteria Addressed**: AC-M5-4
- **Test Requirements**:
  - `rule` TR-64.1: 叠层状态机单测 + composite 参数提取锚测 + `low` 档全关回归；
    headless 不 panic。
  - `rule` TR-64.2: REVIEW_CHECKLIST 登记（叠层观感/与 bloom 交互）。
- **Notes**: wgpu 渲染管线新增 pass，实施设计需给出与 Task 65 后处理链的顺序关系。
- **Completion Evidence** (2026-11-20):
  - 数据面：`extract-screenfx.mjs` 从只读 `screenfx.js`/`hud-boss.js`/`hud.js`/
    `minimap.js` 提取 composite uniform 初值、pass_enabled 逐项阈值（20 项）、
    decay/心跳常数、伤害数字池（上限 6/combo 0.45/存活 1.05/CRIT 900 ms）与小地图
    点标（8 点/12 死亡标记/FX 生命），钉源 `assets/screenfx.json`（commit a48f1546）。
  - `fx/composite.rs`（新模块）消费提取表：`PassActivity` 忠实复刻 JS `any` 表达式
    （逐字段阈值 + lens_on）；`DamageNumberPool` 实现 combo 合并/上限/退场/CRIT scale；
    `MinimapFxLane` 按 JS 相对年龄语义（t+=dt、landing `life=min(life,t+0.35)`）；
    `CompositeOverlay` Resource 统一名牌/伤害数字/小地图 FX，生产系统接 MatchEvent
    真实生产者（HitSend→伤害数字、SpecialSlam/Respawn→小地图 FX）。
  - 锚测：7 项 Task64 专项（composite spec/阈值、decay+池参数、pass gate low 档、
    伤害数字池、minimap 落地年龄+类别过期、生产系统事件生产者与低档全关）+
    `screenfx-crosscheck.mjs` 67 断言独立重解析全绿。
  - 九段门禁 `rust/tools/verify/logs/task64-gates-20261119T195857Z.log`：
    `FMT/CLIPPY/TEST/WASM/TREE/DETERM/XCHK/TRUNK/RELEASE_BUILD/HEADLESS/
    HEADLESS_BOSS_EXIT=0`，`DETERM=BYTE_IDENTICAL`，`JS_TREE_CLEAN`、`SIM_TREE_CLEAN`，
    `GATE_VERDICT=PASS`（TRUNK 真实通过）；workspace 427+601 测试通过。
    独立评审首轮 P1（minimap 落地用绝对年龄、池无生产者）已返工并复验；D-T64-1..4
    登记于 M5 REVIEW_CHECKLIST，未在沙箱伪验收 GPU/DOM 观感。

## Task 65: 后处理链（GTAO/grade/SMAA/tilt-shift）
- **Status**: `completed`
- **Priority**: medium
- **Depends On**: M4 Task 55
- **Description**: GTAO、GradeShader 12 uniform（主题 overlay 链已有数据层）、SMAA、
  tilt-shift diorama（renderer.js，D-T55-⑦ 顺延）；逐档开关组合与 `low` 全关保底语义保持。
- **Acceptance Criteria Addressed**: AC-M5-4
- **Test Requirements**:
  - `rule` TR-65.1: grade uniform 链单测（主题 overlay 已有 93 checks 扩展）+
    档位开关组合参数单测 + headless 四组合冒烟；wasm32 check 通过。
  - `rule` TR-65.2: REVIEW_CHECKLIST 登记（逐档观感/GTAO 强度/SMAA 锯齿，用户机复核）。
- **Notes**: 沙箱无 GPU，shader 正确性只能靠数据层锚测 + 用户机复核；实施设计需给
  Bevy 内建/自写 pass 的选型对比。
- **Completion Evidence** (2026-11-20):
  - 数据面：`extract-postprocess.mjs` 从只读 `core/renderer.js`/`config.js`/`styles/hud.css`
    提取 GradeShader 12 uniform（含 `uHurtColor`，D-T65-2）、GTAO+denoise 参数、SMAA 门控、
    tilt-shift CSS 数据、四档 QUALITY 开关与真实 `comp.addPass` 顺序，钉源
    `assets/postprocess.json`（commit 0d7ab048）。`postprocess-crosscheck.mjs` 独立重解析 54 断言。
  - `fx/post_process.rs`（新模块）消费提取表：12-uniform grade 链、四档 pass 派生
    （gtao⇔ao、smaa⇔msaa>0、low 全关 `effective_combo`）、GTAO/SMAA/tilt 参数、pass order
    （grade→screenfx→output）；`environment.rs::Grade` 补 `hurt_color` 与 postprocess 同源比对。
    Bevy 内建 Bloom 已在 world.rs 接线，GTAO/SMAA/grade GPU pass 本体只做数据面（D-T65-3）。
  - 锚测：4 项 Task65 专项（grade 12 uniform + hurt_color 跨模块比对、四档开关+low 全关、
    GTAO/SMAA/tilt 参数、pass order）+ `postprocess-crosscheck` 54 断言 +
    `environment-crosscheck` 97 checks（含 uHurtColor 扩展）+ headless 四组合 + wasm32 check。
  - 九段门禁 `rust/tools/verify/logs/task65-gates-20261119T202624Z.log`：
    `FMT/CLIPPY/TEST/WASM/TREE/DETERM/XCHK/TRUNK/RELEASE_BUILD/HEADLESS/
    HEADLESS_BOSS_EXIT=0`，`DETERM=BYTE_IDENTICAL`，`JS_TREE_CLEAN`、`SIM_TREE_CLEAN`，
    `GATE_VERDICT=PASS`（TRUNK 真实通过）。独立评审无 P0/P1，2 项 P2（spec 数字笔误、
    SMAA 数据面近似）已修正/登记；D-T65-1..3 与选型对比登记于 spec/REVIEW_CHECKLIST，
    未在沙箱伪验收 GPU 观感。

## Task 66: 环境终稿（云/PMREM/远景反射/海面）
- **Status**: `completed`
- **Priority**: medium
- **Depends On**: Task 65（pass 顺序）
- **Description**: 云烘焙、PMREM 环境反射、远景反射、海面 swell·marina·泡沫
  （environment.js/renderer.js，D-T55-⑦ 顺延）；主题三档数据已有（Task 55）。
- **Acceptance Criteria Addressed**: AC-M5-4
- **Test Requirements**:
  - `rule` TR-66.1: 云/PMREM 烘焙确定性 + 参数提取锚测 + headless 四组合冒烟。
  - `rule` TR-66.2: REVIEW_CHECKLIST 登记（黄昏/金色/白天三主题观感，用户机复核）。
- **Notes**: 烘焙产物是否入库或运行期生成是实施设计决策点（wasm 尺寸约束）。
- **Completion Evidence** (2026-11-20):
  - 数据面：`extract-environment.mjs` 扩展提取云烘焙（2048×640/seed 3/cov 0.46/
    10 strips）、PMREM（size 256/envK 0.66）、远景 cube（512/CubeCamera 2-30000）、
    平面反射四档（0/0.28/0.4/0.5）、海面 makeWaveTexture（256/11/44 波/wind 0.55）与
    GLSL_SWELL 三组 k/相位/速度/振幅（全部正则解析无手抄）进 `assets/environment.json`。
    `environment-crosscheck.mjs` 独立重解析 139 checks（+42 Task66）。
  - `env_bake.rs`（新模块）消费提取常量：`bake_wave_descriptors()` 用 `char/math.rs::Rng`
    （mulberry32）忠实复刻上游 makeWaveTexture 前 44 波；`bake.wave_sha256` 是 JS 参考实现
    + node crypto 生成的**跨语言 golden**，Rust 用 std-only SHA256（自检 abc 向量）按量化
    （1e-9）字节比对，golden 非自证。
  - 锚测：6 项 Task66 专项（bake 常量逐值、swell 三组全字段含振幅、波纹理字节确定、
    跨语言 SHA256 golden、SHA256 自检、主题 cloud/envK 有限性）+ environment-crosscheck
    139 checks + headless 四组合 + wasm32 check。
  - 九段门禁 `rust/tools/verify/logs/task66-gates-20261119T205957Z.log`：
    `FMT/CLIPPY/TEST/WASM/TREE/DETERM/XCHK/TRUNK/RELEASE_BUILD/HEADLESS/
    HEADLESS_BOSS_EXIT=0`，`DETERM=BYTE_IDENTICAL`，`JS_TREE_CLEAN`、`SIM_TREE_CLEAN`，
    `GATE_VERDICT=PASS`（TRUNK 真实通过）。独立评审首轮 P1（swell 振幅手抄未钉源、
    SHA256 golden 实为自证）已返工并复验，D-T66-1/2 登记于 REVIEW_CHECKLIST，未在沙箱
    伪验收 GPU 云/PMREM/反射/海面观感。

## Task 67: 3D alley 材质终稿
- **Status**: `completed`
- **Priority**: medium
- **Depends On**: Task 61
- **Description**: 3D 大厅湿地面 planar reflection、霓虹、蒸汽、PMREM（lobbySet*，
  Task 56 占位转正式）；占位几何保留，材质/反射升级。
- **Acceptance Criteria Addressed**: AC-M5-3/AC-M5-4
- **Test Requirements**:
  - `rule` TR-67.1: 材质参数提取锚测（ lobbyset.json 已有 QUALITY 三档）+ headless 冒烟。
  - `rule` TR-67.2: REVIEW_CHECKLIST 登记（反射清晰度/霓虹发光/蒸汽密度/lobby PMREM 光照，用户机复核）。
- **Notes**: planar reflection 的额外相机与 Task 61 分层交互需一并设计。
- **Completion Evidence** (2026-11-20):
  - `extract-lobbyset.mjs` 新增读 `lobbySet-mats.js`，正则提取 `materials` 段（无手抄）：
    reflection（envMapIntensity 1、Fresnel 0.02+0.98·pow(1-NdV,5)、k gDamp/gInk×0.5、
    mix(0.8,1.0,gWater)）、neon（core (0.1,1,0.35)、uI (7,7,1,5)、队色）、
    steam（3 卡/高 3.4/x-2.8/z-6/seed1.7/颜色/noise(1.6,3.0)/alpha smoothstep(0.4,0.85)·0.6）、
    lobby PMREM（fromScene 0.04/0.1/100/size128/pos、归一化 4/max、envIntensity 0.6、
    env 卡 k 5/4/3/2.5）。`lobbyset-crosscheck.mjs` 独立重解析 242 checks（+51）。
  - `showcase_state.rs` 新增 `WetReflectionSpec/NeonSpec/SteamSpec/LobbyPmremSpec/EnvCardK`
    数据面 + 2 项锚测（材质逐值、low reflection=0/steam=false 保底 + lobby PMREM 128 ≠
    world PMREM 256）；占位几何保留、主相机 layer0 零改动（Task61 守门测试通过）。
  - 九段门禁 `rust/tools/verify/logs/task67-gates-20261120T035622Z.log`：
    `FMT/CLIPPY/TEST/WASM/TREE/DETERM/XCHK/TRUNK/RELEASE_BUILD/HEADLESS/
    HEADLESS_BOSS_EXIT=0`，`DETERM=BYTE_IDENTICAL`（extract-lobbyset 双跑），
    `JS_TREE_CLEAN`、`SIM_TREE_CLEAN`，`GATE_VERDICT=PASS`（TRUNK 真实通过）。
    独立评审首轮 1 项 P1（复核登记/台账未收口）已返工，D-T67-1/2 登记于 REVIEW_CHECKLIST，
    未在沙箱伪验收 GPU 材质/反射/霓虹/蒸汽/PMREM 观感。

## Task 68: Boss 高保真模型
- **Status**: `completed`
- **Priority**: medium
- **Depends On**: M4 Task 46（占位档语义与 hitShapes 契约）
- **Description**: bossModelGeo 875 / bossAnim 859 / bossMats 567 / bossModelFx 143
  （≈2400 行，100-bone 蒙皮/≤150k tri 预算/5 材质+stencil atlas）替换 M4 体块剪影占位档；
  hitShapes 契约（M4 Task 41）不变——sim 几何已脱离渲染模型，本次只换呈现层。
- **Acceptance Criteria Addressed**: AC-M5-3
- **Test Requirements**:
  - `rule` TR-68.1: 模型提取确定性 + 骨树/材质槽锚测 + `['bm']` 时间线驱动动画的单测 +
    ≤150k tri 预算对账；headless boss 600 帧不 panic。
  - `rule` TR-68.2: REVIEW_CHECKLIST 登记（六招动画/weak 点可读性/telegraph，用户机复核）。
- **Notes**: 蒙皮走 M4 Task 54 的骨树/重放基建；体块剪影档保留为 `low` 档或资产缺失降级。
- **Completion Evidence** (2026-11-20):
  - `extract-boss.mjs` **import 真 `bossModelGeo.js`** 调 `buildRig()`/`buildParts()` 得 100 骨
    骨架（name/parent/量化 rest）、4 档真实 tri 预算（low 31830/medium 40283/high 59010/
    ultra 85977，全 ≤150k）、5 材质桶+1024 stencil atlas；正则提 HIT 10 项/DUR 六招×3/PH/
    clip 优先级。`boss-crosscheck.mjs` 独立重 import+正则重解析 721 checks（含每骨 rest_q）。
  - `boss_model.rs`（新模块）数据面 + 4 项锚测：SHA256 自检、**跨语言骨树 golden 真重算**
    （boss.json 存每骨量化 rest，Rust std-only SHA256 重算 canonical 串比对 JS 生成 golden，
    非自证）、HIT/tri 预算逐档、六招 DUR/clip 时间线纯函数（镜像 JS L484-495）。
    hitShapes 契约（M4 Task41/46）、sim 几何、`inkwave_sim` 依赖面零改动。
  - 九段门禁 `rust/tools/verify/logs/task68-gates-20261120T042437Z.log`：
    `FMT/CLIPPY/TEST/WASM/TREE/DETERM/XCHK/TRUNK/RELEASE_BUILD/HEADLESS/
    HEADLESS_BOSS_EXIT=0`，headless boss **600 帧**真执行，`DETERM=BYTE_IDENTICAL`
    （extract-boss 双跑），`JS_TREE_CLEAN`、`SIM_TREE_CLEAN`，`GATE_VERDICT=PASS`。
    独立评审首轮 2 项 P1（台账未收口、Rust golden 自证未重算）已返工，D-T68-1/2 登记于
    REVIEW_CHECKLIST，未在沙箱伪验收蒙皮/材质/动画/weak/telegraph GPU 观感。

## Task 69: locker 更衣室
- **Status**: `pending`
- **Priority**: medium
- **Depends On**: Task 58（kit 几何）；Task 61（分层）
- **Description**: character-outfit 更衣室（M4 wontport 项转正）：服装/皮肤选择 UI +
  3D 预览 + 与 loadout/profile 的联动（JS `locker.js` 面，开工时实读定稿）。
- **Acceptance Criteria Addressed**: AC-M5-3
- **Test Requirements**:
  - `rule` TR-69.1: outfit 数据提取锚测 + 选择状态机单测 + 网络同步面（若上游广播）对齐；
    headless 不 panic。
  - `rule` TR-69.2: REVIEW_CHECKLIST 登记（预览观感/切换流畅度）。
- **Notes**: 若上游 outfit 依赖贴图 atlas/ShaderMaterial（Task 53 顺延项），需一并评估。

## Task 70: M5 收尾——台账/文档/门禁矩阵/总评审
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 58–69
- **Description**: SYNC_LEDGER M5 段；PORT_MAP「M5 完成核对」+ 图例再扩档；
  rust/README M5 章节；九段门禁全矩阵（模板 `gate-task57.sh` 扩 M5 产物）；
  M5 总评审（独立只读评审员，P0/P1 闭环）；REVIEW_CHECKLIST 汇总复核清单；提交 + push origin/main。
- **Acceptance Criteria Addressed**: AC-M5-5
- **Test Requirements**:
  - `rule` TR-70.1: 文档内命令全部实跑留退出码；门禁日志落盘；总评审 P0/P1 闭环；
    TRUNK DEGRADED 按 D-T40-1 登记。
- **Notes**: 沿用 Task 57/27/40 收尾流程；预写结论一律以落盘日志 `*_VERDICT` 为准。

---

## 执行顺序与并行
- Phase A：58 → 60；59 独立可并行。
- Phase B：61 是 62/67 的前置；65 是 66 的前置；63/64 相互独立。
- Phase C：68 → 69（模型能力复用）；70 收尾。
- 默认串行 58 → 59 → 60 → 61 → 62 → 63 → 64 → 65 → 66 → 67 → 68 → 69 → 70。
