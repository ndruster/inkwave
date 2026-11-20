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
- **Status**: `pending`
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

## Task 73: 剩余消费者收敛（零 sim 改动）
- **Status**: `pending`
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
