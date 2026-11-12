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
- **Status**: `pending`
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

## Task 60: kit SFX 与生产消费者接线
- **Status**: `pending`
- **Priority**: high
- **Depends On**: M4 Task 50/54；与 Task 58 可并行
- **Description**: 42 名 kit 系 SFX（`kits/*-sfx.js` 自注册，Task 50 事件缺口）接线；
  footstep 队列接音频 hook（M4 D-T54-5⑫ 测试自 drain → 生产消费）；
  `getMuzzle/getHeadPosition/aimReady` 回写射击/FX 消费者（替换 sim 射击原点直用）。
- **Acceptance Criteria Addressed**: AC-M5-1
- **Test Requirements**:
  - `rule` TR-60.1: 42 名 SFX 表提取锚测（≡JS 自注册顺序/参数）+ 事件→cue 映射单测 +
    footstep 消费节奏锚测；`low` 档音频关闭路径回归。
  - `rule` TR-60.2: REVIEW_CHECKLIST 听感登记（kit SFX 触发时机/音量平衡，用户机复核）。
- **Notes**: 不新增 crate 依赖（D-T50 选型延续）；SfxGate voice cap 语义复用。

## Task 61: RenderLayers 分层与第二相机
- **Status**: `pending`
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

## Task 62: 3D 大厅终稿（角色 preview / 名牌 / emote）
- **Status**: `pending`
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

## Task 63: 音频总线终稿（混响/压限/3D panner）
- **Status**: `pending`
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

## Task 64: fx composite pass 与 DOM 叠层
- **Status**: `pending`
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

## Task 65: 后处理链（GTAO/grade/SMAA/tilt-shift）
- **Status**: `pending`
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

## Task 66: 环境终稿（云/PMREM/远景反射/海面）
- **Status**: `pending`
- **Priority**: medium
- **Depends On**: Task 65（pass 顺序）
- **Description**: 云烘焙、PMREM 环境反射、远景反射、海面 swell·marina·泡沫
  （environment.js/renderer.js，D-T55-⑦ 顺延）；主题三档数据已有（Task 55）。
- **Acceptance Criteria Addressed**: AC-M5-4
- **Test Requirements**:
  - `rule` TR-66.1: 云/PMREM 烘焙确定性 + 参数提取锚测 + headless 四组合冒烟。
  - `rule` TR-66.2: REVIEW_CHECKLIST 登记（黄昏/金色/白天三主题观感，用户机复核）。
- **Notes**: 烘焙产物是否入库或运行期生成是实施设计决策点（wasm 尺寸约束）。

## Task 67: 3D alley 材质终稿
- **Status**: `pending`
- **Priority**: medium
- **Depends On**: Task 61
- **Description**: 3D 大厅湿地面 planar reflection、霓虹、蒸汽、PMREM（lobbySet*，
  Task 56 占位转正式）；占位几何保留，材质/反射升级。
- **Acceptance Criteria Addressed**: AC-M5-3/AC-M5-4
- **Test Requirements**:
  - `rule` TR-67.1: 材质参数提取锚测（ lobbyset.json 已有 QUALITY 三档）+ headless 冒烟。
  - `rule` TR-67.2: REVIEW_CHECKLIST 登记（反射清晰度/霓虹发光/蒸汽密度，用户机复核）。
- **Notes**: planar reflection 的额外相机与 Task 61 分层交互需一并设计。

## Task 68: Boss 高保真模型
- **Status**: `pending`
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
