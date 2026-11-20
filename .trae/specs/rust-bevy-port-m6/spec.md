# INKWAVE Rust/Bevy 移植 — M6 本体接线与双轨验收闭环产品需求文档（PRD）

> 前置：M1（Task 1–16）、M2（17–27）、M3（28–40）、M4（41–57 + 技术债批次）、
> M5（`rust-bevy-port-m5/`，Task 58–70 + 收尾矩阵）已全部完成并 push origin/main
> （基线 workspace **1018+ passed**，末次 `M5_MATRIX_VERDICT=PASS`，日志
> `rust/tools/verify/logs/task70-m5-matrix-20261120T062621Z.log`）。本文档只描述 M6 增量；
> 既有架构/同步机制/网络栈/内容集/提取管线/双轨验收口径全部继续有效。
>
> **立项口径（2026-11-20）**：本文档为**计划**——所有任务 `pending`，不含任何实施结论；
> 每个任务开工时按 M5 惯例先实读 JS 锚点、在 `spec.md` 尾部补「TaskNN 实施设计」§0–§7，
> 再分批实施。范围以 M5 台账登记的顺延项为准（`rust/PORT_MAP.md`「M5 完成核对」、
> `.trae/specs/rust-bevy-port-m5/REVIEW_CHECKLIST.md`「M5 汇总复核清单」），不新增未登记范围。

## M6 决议（2026-11-20：本体接线优先，双轨验收闭环）
- **主线 = 把 M5 留下的「数据面已钉、本体未接」项接上**，并把已知 GPU/听感偏差收敛到
  「可复核」状态；沙箱只验无头可测部分，GPU/浏览器/听感项继续登记 REVIEW_CHECKLIST，
  不做伪验收。
  - **Phase A（Tasks 71–72）表现本体接线**：
    - Task 71：后处理链 GPU pass 本体——grade（12 uniform）+ screenfx composite 全屏 pass
      接线到 Bevy 渲染图，逐档可关、`low` 全关保底；GTAO/SMAA 因需 depth/normal prepass
      且沙箱无法验证观感，继续登记用户机复核（顺延至 Task 72 评估）。
    - Task 72：DOM/UI 桥与大厅呈现收口——名牌 DOM 桥（wasm web-sys）、
      安全区/kFront 实测入参、完整 `CharAnim` 预览替代 per-slot facade、
      hub⇄room cross-dissolve、GTAO/SMAA 选型落地评估。
  - **Phase B（Task 73）剩余音频与设置消费者**（严格不触碰 `inkwave_sim` / 上游树）：
    - 远端 18m 音频剔除接线（D-T60-5）、已登记有消费者的设置字段接线
      （`showFps`/`cameraShake` 等），以及不依赖新 `MatchEvent` 的 SFX 行为收敛。
    - 明确不接：muzzle 回写（D-T60-3，需改 sim 事件，违反零改动纪律）、
      footstep surface 字段（D-T60-2，需改事件 schema）、superjump/bomb explode FX
      （D-T64-4，需新 `MatchEvent`）——这些保持 M5 偏差登记，M6 不推翻非目标。
  - **Phase C（Task 74）M6 收尾**：台账/门禁矩阵/总评审/提交 push（沿用 Task 70 流程）。
- **延续 M5 双轨验收**（NFR-M5-4 → NFR-M6-4）：沙箱只验无头可测部分；GPU/听感项登记
  REVIEW_CHECKLIST 待用户机器复核，不做伪验收。
- **Task 74 = M6 收尾**（沿用 Task 70/57/27/40 流程）。

## Overview
- **Summary**: 把 Rust 版从「表现数据面已钉」推进到「表现本体接线」：后处理全屏 pass
  真正进入渲染图、DOM 叠层桥接、大厅 preview 完整化，同时收敛不触碰 sim 的剩余消费者。
- **Purpose**: M5 已把 grade/composite/GTAO/SMAA/环境/Boss/locker 的数值与状态机
  逐项钉源并通过九段门禁，但明确登记「GPU pass 本体未接、DOM 节点未渲染、观感待用户机」。
  M6 在**不破坏数据面钉源与 `low` 保底**的前提下接上可接线本体，把不可沙箱验证的部分
  收敛成可复核状态，维持沙箱与用户机双轨诚实口径。
- **Target Users**: 仓库维护者；最终验收口径 = Rust 客户端在用户 GPU/浏览器/音频机器上的
  画面/声音与 JS 上游不可区分（逐项对照 REVIEW_CHECKLIST）。

## Goals
- **G-1 后处理本体接线**：`assets/postprocess.json` / `assets/screenfx.json` 的 12-uniform
  grade 与 composite pass 参数真正绑定到 Bevy 全屏 pass；pass 顺序 ≡
  `render_hdr→gtao→bloom→grade_vignette→screenfx→output→smaa`；逐档可关，
  `low`/`post_enabled=false` 全关语义不破坏。
- **G-2 DOM/UI 桥与大厅收口**：伤害数字/名牌/小地图 FX 的 `CompositeOverlay` 经 wasm
  `web-sys` DOM 桥落到真实节点；安全区/kFront 取实测入参；per-slot 预览用完整
  `CharAnim` 骨树替换 facade；hub⇄room cross-dissolve 与 `covered` 清屏移植。
- **G-3 剩余消费者收敛（零 sim 改动）**：远端 18m 音频剔除接线；`showFps`/`cameraShake`
  等已有消费者字段接线；不新增 `MatchEvent`、不改 `inkwave_sim` 事件面。
- **G-4 GTAO/SMAA 选型落地评估**：基于 Bevy 0.19 的 depth/normal prepass 能力给出
  「接线 / 继续登记」的书面裁决；若接线，必须逐档可关且 `low` 不受影响。
- **G-5 门禁与纪律延续**：基线自 M5 单调递增；上游 JS 树/`server/` 零改动；
  `cargo tree -p inkwave_sim` 依赖面不变；九段门禁每任务全绿
  （`gate-task70.sh` 为模板，TRUNK DEGRADED 按 D-T40-1 登记）。

## Non-Goals（M6 明确不做）
- **改上游树**：`src/`、`server/`、`tools/`、`docs/` 零改动。
- **改 `inkwave_sim` 事件面**：muzzle 回写、footstep surface、superjump/bomb explode FX
  全部维持 M5 偏差登记，不以「顺手改 sim」方式绕过纪律。
- **手柄/移动端/发布部署**：延续 M5 Non-Goals，不做。
- **PROTO v1 / net-fixtures**：网络协议冻结，M6 不动。
- **新玩法模式/新地图**：内容集在 M4 已封顶（8 图 + turf/zones/boss）。
- **M4 遗留呈现资产**：atlas/ShaderMaterial、mural atlas 像素、旗帜 shader 动画、
  wasm32 烘焙 inert 路线——M4 明确「维持原状」，M6 不重开，顺延至后续里程碑裁决。
- **跨端 scenario C/D 首次真实执行 / trunk 用户机复跑**：属用户机验收项，M6 只准备矩阵，
  不在沙箱伪验收。

## Non-Functional Requirements
- **NFR-M6-1**：上游 JS 树/`server/` 零改动；`cargo check --target wasm32-unknown-unknown` 通过。
- **NFR-M6-2**：`cargo tree -p inkwave_sim` 依赖面不变（新依赖只进 `inkwave`）。
- **NFR-M6-3**：workspace 测试基线单调递增，每任务九段门禁落盘日志。
- **NFR-M6-4**：双轨验收——GPU/听感项登记复核，不伪验收。
- **NFR-M6-5**：`low` 档保底可玩路径不被破坏（所有新 pass/音频特性可关；SwiftShader
  下不 panic 的降级路径每任务 headless 冒烟）。
- **NFR-M6-6**：数据一律走提取管线（禁手抄），`source.commit` 钉上游 JS 提交。

## Acceptance Criteria
### AC-M6-1: 后处理本体接线
grade 12 uniform 与 composite pass 参数绑定到真实 Bevy 全屏 pass；pass 顺序可验证；
逐档开关 + `low` 全关 headless 600 帧不 panic；GPU 观感登记复核。

### AC-M6-2: DOM/UI 桥与大厅收口
`CompositeOverlay` 经 wasm DOM 桥落节点（native 侧以 headless 数据面锚测替代）；
安全区/kFront、完整 `CharAnim` 预览、cross-dissolve 的有无与偏差逐项登记。

### AC-M6-3: 剩余消费者收敛（零 sim 改动）
18m 剔除与设置字段消费者有锚测；`inkwave_sim` 树零改动守卫通过；不改 sim 事件面。

### AC-M6-4: 门禁与纪律延续
M6 收尾矩阵全绿；上游树/sim 依赖面守卫通过；独立只读总评审 P0/P1 闭环；提交 + push origin/main。

---

## 执行顺序与并行
- Phase A：71 → 72（71 的 pass 顺序结论是 72 DOM/UI 叠层顺序的前提）。
- Phase B：73 可与 72 并行（音频/设置消费者与 UI 无依赖）；默认串行 72 → 73。
- Phase C：74 收尾。
- 默认串行 71 → 72 → 73 → 74。

---

## Task71 实施设计（2026-11-20，开工时实读定稿）

### §0 范围修正（开工实读结论）
- **Bevy 0.19.1 官方 `custom_post_processing.rs` 已实读**（v0.19.1 tag，含完整源码）：
  全屏 pass 的正确接法是 `ExtractComponentPlugin<T>` + `UniformComponentPlugin<T>`（主世界
  component → 渲染世界 uniform buffer 自动抽取）+ `RenderApp` 上 `RenderStartup` 初始化
  pipeline + `Core3d` schedule 的 `Core3dSystems::PostProcess` set 里 `ViewQuery` draw。
  shader 用 `FullscreenShader::to_vertex_state()`，`ViewTarget::post_process_write()` 拿
  source/destination 并翻转 main texture。**这是本仓库首次引入 render-app 代码**，无先前例。
- **`post_process.rs`/`composite.rs` 是纯数据面**（前者零 `impl Plugin`，后者仅一个
  `Update` 系统 `composite_overlay_update`）；本任务不改二者已钉源的数值/函数，只消费
  `grade_defaults()` / `pass_combo()` / `effective_combo()` / `pass_order()` 与
  `composite()` / `composite_pass_enabled()` / `CompositeOverlay`。
- **上游 pass 链实读**（renderer.js L111-153）：
  `RenderPass → GTAOPass(q.ao) → UnrealBloomPass(q.bloom&&settings.bloom) →
  ShaderPass(GradeShader, 无条件) → extraPass(screenfx, enabled) → OutputPass →
  SMAAPass(q.msaa&&!samples)`；与 `postprocess.json:pass_order` 完全一致。
- **screenfx composite** 由 `screenfx.js L563-566` 的 `setExtraPass()` 挂入，
  `L861-867` 的 `any` 表达式决定 `pass.enabled`；Rust 已在 `PassActivity::visible()` 镜像。
- **范围收敛**：本任务只接 **grade（12 uniform）+ composite 的 vignette/hurt/flash 三个
  可沙箱锚定的全屏效果**（两个上游 ShaderPass 合并为一个 Bevy fullscreen pass，
  D-M6-1 登记）。GTAO/SMAA 需要 depth/normal prepass 且观感不可沙箱验证，**本任务不接本体**，
  保留「选型落地评估」书面结论与用户机复核（D-T65-1/-3 延续）。

### §1 数据面（NFR-M6-6，零手抄）
- 复用 `assets/postprocess.json`（`source.commit` 已钉）的 `grade_defaults` 12 项；
  `assets/screenfx.json` 的 `composite.uniform_defaults`（vignette≡`sat`/`hurt`/`flash`
  等 7 项）与 `pass_terms` 阈值。新增 uniform 结构体字段**逐项取值于上述两个 JSON 与
  `Grade`/`EnvState`，不写死任何数字**。
- 新增 `rust/crates/inkwave/src/fx/post_shader.wgsl`：grade 链（sat/vib/contrast/tints/
  lift/exposure/vignette/hurt/hurt_color/flash/aspect）+ composite 三项（vignette 暗角、
  hurt 红闪、flash 白闪），采用 Bevy `FullscreenShader` 的 `fullscreen_vertex_shader`
  顶点入口约定；`embedded_asset!` 加载，沿用 `level_render.rs L503-512` 模式。

### §2 Rust 数据结构（主世界 component → render 世界 uniform）
- 新增 `fx/post_pass.rs`：`#[derive(Component, Clone, Copy, ExtractComponent, ShaderType)]`
  的 `PostFxUniforms`，字段按 WGSL struct 顺序对齐（全部 f32 或 Vec3/Vec4 对齐 16 字节），
  覆盖 §1 的 12+3 项 + `u_pass_enabled`（低档门）。用 `#[extract_app(RenderApp)]` 不需要
  （v0.19.1 示例无此属性），照抄 v0.19.1 派生集合。
- 主世界每帧系统 `post_fx_uniform_sync(Res<EnvState>, Res<Grade查询>, Res<CompositeOverlay>,
  Query<&mut PostFxUniforms, With<MainCamera>>)`：从 `EnvState.params.tier` + `post_enabled`
  调 `effective_combo()` 得到是否启用；从 `Grade::for_theme()` 读 12 uniform 当前值
  （主题 overlay 已含）；从 `CompositeOverlay.activity` 读 hurt/flash/sat。
  该函数拆成**纯函数** `build_uniforms(...) -> PostFxUniforms` 以便 headless 锚测。

### §3 渲染插件 `PostFxPlugin`
- `impl Plugin`：`app.add_plugins((ExtractComponentPlugin::<PostFxUniforms>::default(),
  UniformComponentPlugin::<PostFxUniforms>::default()))`； `app.get_sub_app_mut(RenderApp)`
  上 `add_systems(RenderStartup, init_pipeline)` + `add_systems(Core3d,
  post_process_node.in_set(Core3dSystems::PostProcess))`。
- `init_pipeline`：`BindGroupLayoutEntries::sequential(ShaderStages::FRAGMENT,
  (texture_2d(filterable), sampler(filtering), uniform_buffer::<PostFxUniforms>(true)))`；
  `pipeline_cache.queue_render_pipeline(...)`，fragment target `Rgba8UnormSrgb`
  （官方示例同款；HDR→LDR 转换由 Bevy tonemapping 在前完成）。
- `post_process_node`：照 v0.19.1 示例的 `ViewQuery` + `post_process_write()` + bind group
  cache + `begin_render_pass` + `draw(0..3,0..1)`；额外读 `u_pass_enabled`，为 0 时**直接 return
  不翻转 main texture**（low 档零成本）。

### §4 装配与相机限定
- `main.rs` 仅**非 headless 分支** `add_plugins(fx::post_pass::PostFxPlugin)`；headless
  无 `RenderApp`，插件的 `get_sub_app_mut` 早退，但 component/系统注册仍安全（不触碰 GPU）。
- uniform component **只挂到 `MainCamera` 实体**（world.rs `build_world` 的 cam spawn 处），
  `ShowcaseCamera` 不挂 → 第二相机无后处理（与 Task 61 Bloom 限定一致，D-T61-4 延续）。

### §5 锚测（TR-71.1/TR-71.2）
- `post_pass.rs` 内 `#[cfg(test)]`：`build_uniforms` 纯函数锚测——grade 12 项与
  `grade_defaults()`/`Grade::defaults()` 同源、`low`/`post_enabled=false` 时
  `u_pass_enabled=0`、`high`+activity 时 =1、composite 三项阈值边界、`source_commit` 长度。
- `postprocess-crosscheck`/`screenfx-crosscheck` 保持全绿（数据面未改）；headless 四组合
  600 帧不 panic（headless 无 RenderApp，验证插件不破坏调度器路径）；wasm32 check 通过。
- `low` 档断言 `effective_combo().msaa/bloom/gtao/smaa` 全 0 且 `u_pass_enabled=0`。

### §6 已知偏差登记（写入 REVIEW_CHECKLIST Task 71 段）
- D-M6-1：两个上游 ShaderPass（grade + screenfx）合并为**一个** Bevy fullscreen pass；
  视觉等价性待用户机逐帧对照，沙箱只锚定 uniform 数值与门控。
- D-M6-2：GTAO/SMAA GPU pass 本体**仍未接**（需 depth/normal prepass，观感不可沙箱验证），
  本任务只更新选型书面结论；用户机复核前维持 D-T65-1/-3 状态。
- D-M6-3：screenfx composite 的 ~35 个 uniform 中只有 grading-type saturation / hurt /
  flash / vignette 四项进全屏 pass，其余仍由 DOM/CSS 叠层数据面承载（Task 64 口径），
  观感待用户机复核。
- D-M6-4：fragment target 固定 `Rgba8UnormSrgb`；用户机若启用 HDR 且出现异常需按官方示例
  注释改 `TextureFormat`（沙箱无 GPU 无法验证）；`uAspect` 取自提取表固定 1.7，resize 时
  上游按 w/h 重写，1920×1080 下暗角形状有观感偏差（P2，用户机复核）。
- D-M6-5（独立评审 P0-2 闭环）：上游 GradeShader 的 `ShaderPass` 无条件执行，只有 screenfx
  `extraPass` 受 `any` 门控；两 pass 合并后 composite 项作为中性叠加量（sat=1/desat=hurt=
  flash=0）折叠进同一 draw，**不再用 composite any 门 gate grade**，避免空闲帧丢色彩分级。

### §7 九段门禁
- 复制 `gate-task69.sh`（常规模板）→ `gate-task71.sh`，保留 11 段
  （FMT/CLIPLY/TEST/WASM/TREE/DETERM/XCHK/TRUNK/RELEASE_BUILD/HEADLESS/HEADLESS_BOSS），
  日志落 `tools/verify/logs/task71-gates-<ts>.log`，末行 `GATE_VERDICT=PASS`。
  - DETERM：`extract-postprocess.mjs` + `extract-screenfx.mjs` 双跑字节一致。
  - XCHK：task71 专项锚测（含 WGSL 接口结构锚测，≥6 项）+ postprocess/screenfx/
    environment/lobbyset/boss/locker/props crosscheck（8 组，常规模板口径；
    非收尾矩阵的全 10 组，全 10 组留待 Task 74 收尾矩阵）。
  - 上游 JS/sim 树零改动守卫照旧。
  - 沙箱无 GPU，WGSL 只做结构锚测（binding/struct/字段顺序），完整 naga 编译 +
    像素观感登记 D-M6-4 用户机复核，不伪验收。

---

## Task73 实施设计（2026-11-21，开工时实读定稿）

### §0 范围（零 sim 改动，纯消费者收敛）
- **18m 远端音频剔除（D-T60-5）**：实读 `audio.rs` L515-591 的 `audio_sfx` —— cues 已携带
  `Option<[f32;3]>` 世界源（Task63），但**未做距离剔除**，所有 splat/shot 无论远近都进 gate+
  spawn。剔除策略 18m 由 D-T60-5 任务规定（上游 `audio.js` 实读只有 3-D 距离 + panner
  `maxDistance=max(60,ref*20)` 平滑低通，**未找到 18m 硬截断常量**；听感等价性留 D-M6-12
  用户机复核，不声称来自上游常量）。接线点：`for (c,source) in cues` 前对每个 `Some(p)` 计算
  与 camera.translation 的距离，`p > 18m` 跳过（2-D `None` 直通）；无 MainCamera（headless
  夹具）⇒ 无听点 ⇒ 不剔除（D-M6-12）。纯函数 `audible_at(Option<[f32;3]>,camera,max_dist)->bool`
  + 系统级剔除锚测（有 camera 超距剔除 / 近距 spawn / 无 camera 保留）。
- **showFps**：实读 `settings.rs` —— `Settings.show_fps: bool`（提取自 environment.json
  `defaultSettings.showFps`）无消费者。接线：`ui/fps.rs` 的 `FpsDisplay` EMA + HUD
  `LabelRole("fps")` 节点 + `fps_hud_update`（`.after(ui_update)` 防覆盖），headless 以系统级
  写标签锚测，真实字形观感用户机复核（D-M6-11）。
- **cameraShake**：实读 `world.rs camera_rig` —— shake **无条件**应用；`Settings.camera_shake:
  f64`（0..1，提取自 environment.json）未被读取。接线：`shake_applied(camera_shake: f64,
  amount: f64) -> f64`（乘性缩放，0 禁用、1 全量），`camera_rig` 用其结果抖动；headless 锚测。
- **明确不接**（保持 M5 偏差，不推翻 Non-Goals）：muzzle 回写（D-T60-3，需新 MatchEvent）、
  footstep surface（D-T60-2，数据模型无 surface 字段）、superjump/bomb explode FX（D-T64-4，
  需新事件）。本任务只接不依赖新 MatchEvent 的消费者。
- **严格零 sim 改动**：`inkwave_sim` 事件面/任何文件不动；门禁 `SIM_TREE_CLEAN` 守。

### §1 18m 音频剔除
- 新增纯函数 `audible_at(source: [f32;3], camera: [f32;3], max_dist: f32) -> bool`（距离平方
  比较避免 sqrt），`MAX_AUDIBLE_DIST = 18.0`（上游常量，零手抄——从 audio.js 提取，若
  audio.js 是硬编码则注明来源 commit）。
- `audio_sfx` 在遍历 cues 时对 `Some(pos)` 过滤：`if !audible_at(pos, cam_pos, 18.0) { continue }`；
  `None`（2-D clock cue）保留。headless 锚测：`audible_at` 边界（17.9 true / 18.1 false）、
  `None` cue 不被过滤、muted 仍然静音。

### §2 showFps / cameraShake 设置接线
- Fps HUD：在 HUD 资源/系统里加 `FpsDisplay` 状态（bool+value），由 `Settings.show_fps` 门控；
  native 无文本渲染锚点，以「开关切换是否产生 fps 状态更新」数据面锚测；真实 FPS 文本渲染
  用户机复核（D-M6-11）。
- cameraShake：`camera_rig` 加 `settings_game: Res<Settings>`（或现有 Settings 资源），
  抖动条件 `settings_game.camera_shake && shake.amount > 0.0`。锚测：纯条件函数
  `shake_applied(camera_shake: bool, amount: f32) -> bool`。

### §3 SFX 行为收敛（不依赖新事件）
- 实读 audio.rs 的 `sfx_cue`/`phase_cue`/`splat_cue` —— 这些纯函数已覆盖所有现有 MatchEvent
  的 SFX 映射；本任务只做「18m 剔除 + 设置字段接线」，不新增 cue 映射（保持 D-T60-2/3/64-4）。

### §4 偏差登记（REVIEW_CHECKLIST Task 73 段）
- D-M6-11：showFps 为数据面开关锚测，真实 FPS 文本观感用户机复核。
- D-M6-12：18m 剔除以相机 translation 为听点（≡ JS）；上游可能用 listener orientation/occlusion，
  观感用户机复核。
- 重申不接项：D-T60-2（footstep surface）、D-T60-3（muzzle 回写）、D-T64-4（superjump/bomb FX）
  保持 M5 状态，不因本任务改动。

### §5 门禁与锚测
- `gate-task73.sh`（模板 gate-task69.sh）：DETERM 双跑 postprocess+screenfx；XCHK task73
  专项（≥6 锚测：audible 边界/2D cue/mute/shake 条件/fps 开关/低档仍全关）+ 8 组 crosscheck。
- headless 四组合 + boss 600 帧不 panic；wasm32 check；九段全绿；SIM_TREE_CLEAN；上游 JS 零改动。


---

## Task72 实施设计（2026-11-21，开工时实读定稿）

### §0 范围（实读结论：AC-M6-2 的「接线 + 逐项登记」双口径）
- **DOM 桥**：实读确认零 DOM 节点操作（`Document`/`Element` feature 都未开）；`CompositeOverlay`/
  `LobbyNameplates` 数据面已就绪。上游 `showcase.js _measureLobby`(L2677-2694) 的 UI 安全矩形
  与 `_lobFrame`(L2670) 的 `kFront` 都是**运行时 DOM 测量**，非 JSON 字段。本任务接一个
  平台中立的 **DOM 操作序列桥**：`DomSink` trait（create/set_text/set_style/append/remove）
  + native `RecordingDomSink`（headless 锚测节点操作序列）+ wasm `web-sys` sink（真实节点，
  用户机复核）。native 侧以操作序列断言替代真实 DOM（AC-M6-2 明确口径）。
- **安全区/kFront**：新增 `LobbyMeasure` 资源（safe rect + kFront），native 默认 =(0,0,W,H)/
  150（≡ JS `kFront:150` 初始），wasm 侧按 `_measureLobby` 从 DOM 实测。`showcase_nameplates_update`
  从硬编码 `None/1.0`(showcase_scene.rs L405) 改为读此资源 → 消除 D-T62-2 缺口。
- **CharAnim 预览**：实读确认完整 `CharAnim` 骨树为**纯数据、零 GPU 句柄**
  （`Tables::load()` include_str! → `CharAnim::new` → `update(dt,&AnimState)` → 写 `bones`/`kid`），
  pinned fixture 已证明 headless 可比。facade `ShowcasePreview`(showcase_scene.rs L63-82) 不碰
  CharAnim。本任务给预览实体挂 `PreviewAnim`（持有 CharAnim + 确定性 style/seed），
  在 `showcase_members_update` 里逐帧 `update` 并把 `kid`/body 的 pose 变换写到 `Transform`
  （emote hop/dance 仍由 `advance_preview` 的弧线叠加），逐帧 pose 可 headless 锚测。
- **cross-dissolve/covered**：实读 `_updateSet`(showcase.js L2093-2121) 与 render()(L3055-3071)：
  `k += dt/SET_FADE`(SET_FADE=0.45 从 lobbyset.json 提取)，on=mode∈{hub,lobby}&&ready，
  `covered`=全帧无下层绘制→透明度直接 1。新增纯数据 `SetFade` 状态机（k/gone/ready/on/
  covered + `eInOut`），headless 锚测；Bevy 侧合成不透明度接线为数据面（真实 GPU 混合
  观感登记用户机复核）。
- **GTAO/SMAA 选型裁决（G-4）**：实读 Bevy 0.19.1 registry——**SMAA 已内建**
  （`bevy_anti_alias::Smaa`+`SmaaPlugin`，`smaa_luts` feature 内嵌 area/search LUT，
  `AntiAliasPlugin` 默认注册）；GTAO 无内建（仅 SSAO）。裁决：SMAA 用 Bevy 内建 `Smaa`
  组件按档接线（`msaa>0&&!multisampled` 由 `postprocess.json` 门控，`low` 不挂）；GTAO
  维持数据面 + 用户机复核（需 depth/normal prepass，无内建，沙箱不可验观感）。

### §1 DOM 桥数据面（NFR-M6-6，零手抄）
- 新增 `ui/dom_bridge.rs`：`DomSink` trait + `DomOp` 枚举（文本/样式/子节点操作，携带
  stable key `slot`/`kind`）；`build_overlay_ops(&CompositeOverlay, &LobbyNameplates) -> Vec<DomOp>`
  纯函数把名牌/伤害数字/minimap FX 映射为幂等操作序列（按 key diff 上一帧）。
- native：`RecordingDomSink` 记录每帧 ops 到 `Vec<Vec<DomOp>>`，锚测「roster 变化→ops 序列」、
  「low 档→全空 ops」、伤害数字合并只更新文本不重建节点。
- wasm：`WebDomSink` 用 `web-sys`（需给 Cargo.toml 加 `Document`/`Element`/`Node`/`HtmlElement`
  feature，`#[cfg(target_arch="wasm32")]` 门控）按 key 维护 `#iw-overlay` 下的节点。观感用户机复核。

### §2 安全区/kFront 接线
- `LobbyMeasure { safe: (f32,f32,f32,f32), k_front: f32 }` Resource；native 默认 `k_front=150`
  （≡ JS 初始）、safe=(0,0,W,H)（无实测 DOM，等价于当前 None 行为）。lambda 入参替换
  `showcase_nameplates_update` L405 的 `None/1.0` → `Some(measure.safe)/measure.k_front`。

### §3 CharAnim 预览接线
- `PreviewAnim<'static>` 不能作 Component（生命周期）——改为 `PreviewAnim` 持 `Tables` 的
  `'static` 引用（`Tables::load()` 可用 `OnceLock<Tables>` 提升为静态）+ 内部 `CharAnim<'static>`、
  style[6]、seed。`Tables` 由 `include_str!` 解析，`OnceLock` 模式沿用 `level_render`/`post_pass`
  的静态资源先例。
- 在 `spawn_preview` 挂 `PreviewAnim`，`showcase_members_update` 每帧：`c.update(dt,&AnimState::from(member))`
  → 用 `c.kid.pos/quat/scale` 写预览 root 的 `Transform`（body/头由 facade 几何承担，骨树 pose
  驱动 root 的位移/旋转/缩放 + emote 弧线）。逐帧锚测：同一 `(name,style,seed)` 序列两次运行 pose
  数值逐帧相同；`low` 档无预览实体。

### §4 cross-dissolve 状态机
- `SetFade { k: f32, gone: f32, ready: bool, covered: bool }` + `fn advance(&mut self, dt, on: bool)`：
  `k = if on&&ready { min(1,k+dt/set_fade) } else { max(0,k-dt/set_fade) }`；`set_fade` 从
  `pedestal().set_fade`(lobbyset.json) 读取，零手抄。
- `fn opacity(&self) -> f32`：`covered ? 1.0 : eInOut(k)`（eInOut ≡ JS `eInOut` smoothstep）。
  挂到 `ShowcaseState` 或独立 Resource；`showcase_scene_update` 每帧 advance。

### §5 G-4 SMAA/SMAA 选型裁决（开工实读更正 → 继续登记）
- 开工实读 Bevy 0.19.1 registry：**SMAA 确有内建**（`bevy_anti_alias::smaa::{Smaa,
  SmaaPlugin}`，`AntiAliasPlugin` 默认注册，area/search LUT 内嵌于 `smaa_luts` feature）；
  GTAO 无内建（仅 SSAO）。初稿 §5 拟按档挂 `Smaa` 组件。
- **实读裁决改为「继续登记」（G-4 允许分支，见 `fx/aa_decision.rs`）**：本相机管线已有
  Bloom + Task 71 自写 post-pass，沙箱无 GPU 无法验证再叠加一个内建全屏 pass 的观感；
  且默认 `lut_placeholder` 与 JS 真实 area/search 纹理不同，不能声称像素等价。G-4 的
  SMAA 门（`msaa>0 && !multisampled`）以纯函数锚定，接线待用户机裁决（D-M6-10）。

### §6 偏差登记（写入 REVIEW_CHECKLIST Task 72 段）
- D-M6-6：native DOM 桥是**操作序列锚测**；wasm web-sys sink 挂到 `index.html` 的
  `#iw-overlay`（已补真实 mount），节点渲染观感用户机复核。
- D-M6-7：安全区/kFront 在 native 无 DOM 实测 → `safe_opt()=None`（≡ JS `!U`，无边缘剔除，
  与 M5 行为一致）；wasm 分支目前也未做 `_measureLobby` 实测，沿用默认 kFront=150，
  实测接入与观感用户机复核。
- D-M6-8：CharAnim 预览由 member 的 ready/emote 状态驱动骨树 root pose（body/头仍是 facade
  几何），emote 首帧触发一次性 `CharAnim::trigger`；完整骨树 mesh 渲染观感用户机复核。
- D-M6-9：cross-dissolve 的 `covered = on && full_frame && k>=1`（≡ JS `_ffFrame`），淡入期
  `opacity = eInOut(k)`；GPU 混合/清屏观感用户机复核，数据面 k/opacity 已锚测。
- D-M6-10：GTAO 保持未接（无内建+需 depth/normal prepass）；SMAA 虽有 Bevy 内建但按 G-4
  裁决**继续登记、暂不接线**（沙箱不可验观感 + placeholder LUT 差异），两项用户机复核。

### §7 门禁与锚测
- `gate-task72.sh`（模板 gate-task69.sh）：DETERM 加 lobbyset 双跑；XCHK task72 专项
  （≥6 锚测：DOM ops/安全区/CharAnim pose/cross-dissolve/SMAA 门控/low 全关）+ 8 组 crosscheck。
- headless 四组合 + boss 600 帧不 panic；wasm32 check；九段全绿；上游 JS/sim 零改动。

