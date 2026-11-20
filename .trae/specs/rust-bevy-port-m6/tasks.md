# M6 任务清单（本体接线与双轨验收闭环）

> 状态口径沿用 M5：`pending` → `in_progress` → `completed`（含 Completion Evidence）。
> 每任务固定流程：实读 JS 锚点 → 本目录 `spec.md` 尾部「TaskNN 实施设计」§0–§7 →
> 分批实施 → 主上下文实读核验 → 独立评审（P0/P1/P2 当场修复复验）→ 九段门禁
> （模板 `rust/tools/verify/gate-task70.sh`）→ 勾选 + Completion Evidence →
> PORT_MAP/台账 → commit → 中文汇报。
> 所有条目当前为**计划**，行号/体量以开工时实读为准。

## Task 71: 后处理链 GPU pass 本体（grade + screenfx composite）
- **Status**: `completed`
- **Priority**: high
- **Depends On**: M5 Task 64/65
- **Description**: 把 M5 已钉源的 `assets/postprocess.json`（grade 12 uniform / pass 顺序 /
  四档开关）与 `assets/screenfx.json`（composite uniform / pass 阈值）真正绑定到
  Bevy 0.19 全屏 pass；`low` 与 `post_enabled=false` 全关保底不破坏；GTAO/SMAA
  因 depth/normal prepass 与观感不可沙箱验证，Task 71 只给选型书面裁决、不接线。
- **Acceptance Criteria Addressed**: AC-M6-1
- **Test Requirements**:
  - `rule` TR-71.1: grade uniform 绑定单测（12 项与 `Grade`/`postprocess` 同源）+ composite
    pass gate 与 `CompositeOverlay` 联动锚测 + `low`/post-off 全关回归；headless 600 帧不 panic。
  - `rule` TR-71.2: pass 顺序断言（`render_hdr→gtao→bloom→grade_vignette→screenfx→output→smaa`）
    + wasm32 check + 九段门禁。
- **Notes**: 沙箱无 GPU，shader 观感只能靠数据面锚测 + 用户机复核；实施设计需给
  Bevy 内建/自写 pass 的选型对比（承接 M5 Task 65 Notes）。
- **Completion Evidence（2026-11-20）**：
  - **本体接线**：新增 `fx/post_pass.rs`（本移植首份 render-app 代码）——`PostFxUniforms`
    （6×vec4 / 16B 对齐，镜像 WGSL struct）、`build_uniforms()` 纯函数、`PostFxPlugin`
    （`ExtractComponentPlugin`+`UniformComponentPlugin`+`RenderStartup` pipeline+
    `Core3dSystems::PostProcess` 节点），新增 `fx/post_shader.wgsl`（grade 12 uniform
    + composite saturation/hurt/flash/vignette 逐行移植上游 GLSL）。`main.rs` 仅非
    headless 分支注册插件与同步系统；`PostFxUniforms` 只挂 `MainCamera`（ShowcaseCamera
    不过后处理，D-T61-4 延续）。数据仍走 `postprocess.json`/`screenfx.json`，零手抄。
  - **独立评审闭环**：首轮 **2×P0 + 4×P1 + 5×P2**；P0-1（shader 缺 `var<uniform>` 绑定声明，
    真实 GPU 必编译失败）已补 binding 实例 + 全字段 `fx.` 前缀；P0-2（grade 被 composite
    `any` 门错误折叠，空闲帧丢色彩分级）已改为 `enabled = post_enabled && combo_on`，
    锚测覆盖；P1-1/P1-3（grade/composite 数学自创近似）已按 renderer.js L40-59 与
    screenfx.js L169/L200-203/L325-327 逐行重写；P1-2（D-M6-1..4 未登记）已落
    REVIEW_CHECKLIST「Task 71 复核登记」；P1-4（无 WGSL 校验）已加结构锚测
    （binding/6 vec4/字段顺序/fx. 前缀）。复验 9 点全 CLOSED，无新 P0/P1。
  - **确定性/门禁**：`gate-task71.sh`（模板 gate-task69.sh）→
    `tools/verify/logs/task71-gates-20261120T170213Z.log`：
    `FMT/CLIPLY/TEST/WASM/TREE/DETERM/XCHK(8 组含 task71×6)/RELEASE_BUILD/HEADLESS/
    HEADLESS_BOSS` 全 0，`DETERM=BYTE_IDENTICAL`（postprocess+screenfx 双跑），
    `JS_TREE_CLEAN`/`SIM_TREE_CLEAN`，`GATE_VERDICT=PASS`；TRUNK 本轮
    wasm-bindgen OOM SIGKILL → `TRUNK=DEGRADED`（D-T40-1 非阻断，前一轮
    `task71-gates-20261120T165034Z.log` `TRUNK_EXIT=0` 真实通过）。
  - **workspace**：inkwave 单测 448 → **452**（+6 task71，其中替换 4 旧锚为 6）；
    wasm32 check 通过；headless 四组合 + boss 600 帧不 panic；上游 JS/server/sim 零改动。
  - **已知偏差**：D-M6-1（两 pass 合一）/D-M6-2（GTAO/SMAA 未接）/D-M6-3（composite
    子集）/D-M6-4（target 格式/aspect 固定）/D-M6-5（grade 无条件，评审闭环）均登记
    `REVIEW_CHECKLIST.md`；GPU 观感/完整 naga 编译为用户机复核，未伪验收。

## Task 72: DOM/UI 桥与大厅呈现收口
- **Status**: `completed`
- **Priority**: high
- **Depends On**: Task 71（pass 顺序结论是 DOM 叠层顺序前提）；M5 Task 61/62/64
- **Description**: `CompositeOverlay`（名牌/伤害数字/小地图 FX）经 wasm `web-sys` DOM 桥
  落到真实节点；安全区/kFront 取实测入参（D-T62-3）；per-slot 预览用完整 `CharAnim`
  骨树替换 facade（D-T62-1）；hub⇄room cross-dissolve 与 `covered` 清屏（D-T61-2）；
  GTAO/SMAA 选型落地评估（G-4）。
- **Acceptance Criteria Addressed**: AC-M6-2
- **Test Requirements**:
  - `rule` TR-72.1: DOM 桥数据面锚测（native headless 以 overlay 状态机与节点操作序列
    替代真实 DOM）+ 完整 `CharAnim` 预览逐帧比对 + `low` 档全关回归。
  - `rule` TR-72.2: REVIEW_CHECKLIST 登记（名牌遮挡/安全区/cross-dissolve 过渡观感）。
- **Notes**: wasm DOM 节点操作无法在沙箱验证观感，登记用户机复核，不伪验收。
- **Completion Evidence（2026-11-21）**：
  - **DOM 桥本体**：新增 `ui/dom_bridge.rs`——平台中立 `DomKey`/`DomOp`、`build_overlay_ops()`
    纯函数（名牌/伤害数字/minimap FX → 幂等节点操作序列，按 key diff）、`RecordingDomSink`
    （native headless 锚测）、`#[cfg(wasm32)]` web-sys sink（真实节点，扩 Cargo.toml 的
    Document/Element/Node/HtmlElement feature）；`index.html` 补 `#iw-overlay` 真实 mount。
  - **安全区/kFront**：新增 `LobbyMeasure` 资源（`safe_opt()`：native 未测量 → None ≡ JS
    `!U` 无剔除，与 M5 行为一致；kFront 默认 150 ≡ JS 初始），`showcase_nameplates_update`
    从硬编码 `None/1.0` 改读此资源，D-T62-2 数据面闭环。
  - **CharAnim 预览**：新增 `ui/preview_anim.rs`（`OnceLock<Tables>` 提升 'static、
    `step_preview_pose` 纯函数、`PreviewAnim` 组件）；`showcase_members_update` 用
    `member_anim_terms`+`preview_anim_state`+emote 首帧 trigger 驱动真实骨树替代恒定 idle。
  - **cross-dissolve**：新增 `SetFade` 状态机（k 按 dt/SET_FADE 升降、`eInOut` quintic 曲线、
    `covered=on&&full_frame&&k>=1` ≡ JS `_ffFrame`），`set_fade_update` 每帧 advance。
  - **G-4 裁决**：新增 `fx/aa_decision.rs`——实读 Bevy 0.19.1（SMAA 内建存在、GTAO 无内建），
    裁决两者「继续登记、暂不接线」（G-4 允许分支），门控纯函数已锚测。
  - **独立评审闭环**：首轮 **2×P0 + 4×P1 + 6×P2**；P0-1（covered 钉死 opacity=1 杀淡入）
    已修为 advance 后算 `covered=k>=1` 并加系统级锚测；P0-2（wasm `#iw-overlay` 根节点缺失
    → sink 静默空操作）已补 index.html mount；P1-1（D-M6-6..10 未登记）已落 REVIEW_CHECKLIST；
    P1-2（safe=(0,0,0,0) 全剔除回归）已改 `safe_opt()=None`；P1-3（CharAnim 恒定 idle 伪接线）
    已改 member 状态驱动 + emote trigger + bone_hash 锚测；P1-4（SMAA 设计/登记冲突）已统一
    "继续登记"。P2（稳定 key/帧上限/冗余注释/文案）亦全部修复。复验无新 P0/P1。
  - **确定性/门禁**：`gate-task72.sh` → `tools/verify/logs/task72-gates-20261120T175933Z.log`：
    `FMT/CLIPLY/TEST/WASM/TREE/DETERM(三提取器 re-run 字节一致)/XCHK(8 组含 task72×16)/
    TRUNK_EXIT=0/RELEASE_BUILD/HEADLESS(四组合)/HEADLESS_BOSS` 全绿，`GATE_VERDICT=PASS`；
    `JS_TREE_CLEAN`/`SIM_TREE_CLEAN`。workspace 单测 452 → **470**（+18），wasm32 check 通过。
  - **已知偏差**：D-M6-6..10 全登记 `REVIEW_CHECKLIST.md`「Task 72 复核登记」；DOM 观感/
    安全区实测/CharAnim mesh/cross-dissolve GPU 混合/SMAA-GTAO 接线均用户机复核，未伪验收。

## Task 73: 剩余消费者收敛（零 sim 改动）
- **Status**: `completed`
- **Priority**: medium
- **Depends On**: M5 Task 59/60/64
- **Description**: 远端 18m 音频剔除接线（D-T60-5）；`showFps`/`cameraShake` 等已有消费者
  的设置字段接线；不依赖新 `MatchEvent` 的 SFX 行为收敛。明确不接 muzzle 回写（D-T60-3）、
  footstep surface（D-T60-2）、superjump/bomb explode FX（D-T64-4）——保持 M5 偏差登记。
- **Acceptance Criteria Addressed**: AC-M6-3
- **Test Requirements**:
  - `rule` TR-73.1: 18m 剔除边界锚测 + 设置字段消费者锚测 + `inkwave_sim` 零改动守卫 +
    `low` 档音频关闭路径回归。
  - `rule` TR-73.2: REVIEW_CHECKLIST 登记听感复核项。
- **Notes**: 严格不改 `inkwave_sim` 事件面，不与 M5 Non-Goals 冲突。
- **Completion Evidence（2026-11-21）**：
  - **18m 剔除（D-T60-5）**：新增 `audio.rs::audible_at`（平方距离，2-D cue 直通、非有限拒绝）
    与 `MAX_AUDIBLE_DIST=18.0`，`audio_sfx` 循环按相机听点剔除超距 spatial cue（无 MainCamera
    时无听点不剔除，D-M6-12）。18m 是任务规定策略（实读 audio.js 无 18m 硬截断常量，不伪称
    上游等价）。
  - **设置接线**：`camera_rig` 用 `shake_applied(camera_shake:f64, amount:f64)` 缩放 boss 抖动
    （0 禁用/1 全量，来自 environment.json）；`show_fps` 经新 `ui/fps.rs`（`FpsDisplay` EMA +
    `fps_text`）写 HUD `LabelRole("fps")` 节点（`fps_hud_update.after(ui_update)` 防覆盖）。
  - **独立评审闭环**：首轮 **0×P0 + 3×P1 + 5×P2**；P1-1（缺生产路径剔除锚测）已加系统级
    `task73_audio_sfx_culls_far_cue_with_camera`（超距剔除/近距 spawn）；P1-2（测试走"无相机
    跳过"通道）已加 `task73_no_camera_retains_all_cues` 显式声明有意设计；P1-3（低档 shake
    空转断言）已改为"post 全关 + 设置跟随用户"诚实不变量。P2（系统顺序 `.after`、spec 签名
    回写、18m 出处澄清、死字段移除、crosscheck 措辞）全修。复验无新 P0/P1。
  - **确定性/门禁**：`gate-task73.sh` → `tools/verify/logs/task73-gates-<ts>.log`：8 项 task73
    锚测 + 7 组 crosscheck；FMT/CLIPLY/TEST/WASM/TREE/DETERM/XCHK/RELEASE_BUILD/HEADLESS
    (四组合)/HEADLESS_BOSS 全绿，`GATE_VERDICT=PASS`；TRUNK 环境 DEGRADED（D-T40-1 wasm-bindgen
    OOM SIGKILL，非阻断）；`JS_TREE_CLEAN`/`SIM_TREE_CLEAN`（零 sim 改动）。workspace 单测
    470 → **478**（+8 task73）。
  - **已知偏差**：D-M6-11（showFps 观感）/D-M6-12（18m 剔除听感）登记 `REVIEW_CHECKLIST.md`；
    D-T60-2（footstep surface）/D-T60-3（muzzle 回写）/D-T64-4（superjump/bomb FX）在 spec §4
    与 REVIEW_CHECKLIST 重申不接，未因本任务改动。

## Task 74: M6 收尾——台账/文档/门禁矩阵/总评审
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 71–73
- **Description**: SYNC_LEDGER M6 段；PORT_MAP「M6 完成核对」+ 图例再扩档；
  rust/README M6 章节；九段门禁全矩阵（模板 `gate-task70.sh` 扩 M6 产物）；
  M6 总评审（独立只读评审员，P0/P1 闭环）；REVIEW_CHECKLIST 汇总复核清单；
  提交 + push origin/main。
- **Acceptance Criteria Addressed**: AC-M6-4
- **Test Requirements**:
  - `rule` TR-74.1: 文档内命令全部实跑留退出码；门禁日志落盘；总评审 P0/P1 闭环；
    TRUNK DEGRADED 按 D-T40-1 登记。
  - `rule` TR-74.2: M6 矩阵含 M5 全 10 crosscheck 回归 + 新增 M6 守卫（如有）。
- **Notes**: 沿用 Task 70/57/27/40 收尾流程；预写结论一律以落盘日志 `*_VERDICT` 为准。

---

## 执行顺序与并行
- Phase A：71 → 72。
- Phase B：73 可与 72 并行；默认串行 72 → 73。
- Phase C：74 收尾。
- 默认串行 71 → 72 → 73 → 74。
