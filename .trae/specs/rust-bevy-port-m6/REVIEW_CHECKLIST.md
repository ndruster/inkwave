# M6 复核清单（本体接线与双轨验收闭环）

> **双轨验收口径（NFR-M5-4 → NFR-M6-4 延续）**：沙箱只验无头可测部分（数据确定性、
> 独立 crosscheck、锚定单测、headless 600 帧、wasm check、九段门禁）；GPU/浏览器 DOM/
> 听感项 **登记待用户机器复核**，不做伪验收。本文件随任务实施逐任务扩充，收尾时由
> Task 74 汇总成「M6 汇总复核清单」。

## 预登记：承接 M5 的复核基线

M5 汇总 7 组复核项（`.trae/specs/rust-bevy-port-m5/REVIEW_CHECKLIST.md`「M5 汇总复核清单」）
在 M6 对应任务落地前**仍然有效**；M6 任务把对应项转正式档或收敛后，在下方逐任务区登记，
并在收尾时标注「M5 项已被 M6 TaskNN 收敛」或「M5 项与 M6 无关、维持原状」。

## 预登记：M6 各任务待复核面（实施后扩充）

| Task | 预登记复核面 |
|---|---|
| 71 | grade 12 uniform 接线后色彩/暗角/受伤叠层；composite pass 与 bloom 顺序；`low` 档观感 |
| 72 | 名牌 DOM 节点/伤害数字/小地图 FX 落点与安全区；完整 CharAnim 预览流畅度；cross-dissolve 过渡 |
| 73 | 18m 远端剔除听感；`showFps`/`cameraShake` 等字段在实机的行为 |
| 74 | 用户机 trunk release、scenario C/D 首次真实执行、三主题 × 四档 headless 复跑 |

## 用户 GPU 机器复核顺序（沿用 M5 收尾清单格式，Task 74 定稿）

- [ ] `cd rust && trunk build --release` 一次通过（沙箱 D-T40-1 DEGRADED 持续登记）。
- [ ] 窗口态按 `rust/README.md` M6 章节操作，逐项勾上方表格。
- [ ] `low`/`ultra` 两档 + 三主题 headless 组合在用户机复跑。
- [ ] 网络可达时 `bash rust/tools/sync-upstream.sh check` 确认基线后无新上游提交。
- [ ] 跨端 scenario C/D 首次真实执行 + guest boss 快照 wasm seam 复核（D-T45-8/D-T39-1）。

> 本清单不构成「已通过」；收尾结论以 `tasks.md` Task 74 的 Completion Evidence
> 与 M6 总评审为准。

## Task 71 复核登记（2026-11-20，全屏 grade + composite pass）

无头可测部分已锚定：6 个 Task71 专项测试（identity/low 保底/grade 无条件/composite
转发/12 uniform 逐项/WGSL 接口结构）+ 九段门禁 PASS。以下为**待用户 GPU 机器复核**的观感项，
并登记本任务明确偏差（承接 M5 D-T64-2 / D-T65-3）：

- [ ] **D-M6-1 两 pass 合一**：上游 GradeShader 与 screenfx ShaderPass 是两个独立 pass，
  Rust 合并为一个 Bevy fullscreen pass（grade→composite 顺序在一支 shader 内执行）。
  逐帧观感（色彩分级后叠 hurt/flash/vignette）需对照 JS 确认无先后差异。
- [ ] **D-M6-2 GTAO/SMAA 未接**：仍需 depth/normal prepass，观感不可沙箱验证；
  本任务只接 grade+composite，GTAO/SMAA 保持 M5 D-T65-1/-3 状态，用户机裁决顺序。
- [ ] **D-M6-3 composite 子集**：只把 saturation / hurt / flash / vignette 放上 GPU 路径，
  其余 ~35 个 screenfx uniform 仍走 DOM/CSS 叠层数据面（Task 64）；边缘墨迹/镜头墨/速度线等
  观感待复核是否需要提升为 GPU pass。
- [ ] **D-M6-4 target 格式固定**：fragment target 写死 `Rgba8UnormSrgb`（Bevy tonemapping
  在本 set 之前完成）。用户机若启用 HDR 且出现色彩异常，按 Bevy v0.19.1 官方示例注释改
  `TextureFormat`；沙箱无 GPU 无法验证。
- [ ] **grade 观感**：sat/vib（队伍墨高饱和保护）/log-space 对比度/冷暖 split-tone/暗角曲线
  在 `low`/`medium`/`high`/`ultra` 四档 + 三主题下的实际画面。
- [ ] **pass 顺序**：本 pass 位于 `Core3dSystems::PostProcess`，与 Bevy 内建 Bloom 的先后
  关系（上游 grade 在 bloom 之后）需在用户机对照。
- [x] **D-M6-5（首轮评审 P0-2 闭环，已修）**：grade 为无条件 pass，不受 composite `any`
  门影响；空闲 composite 帧仍执行色彩分级，composite 项以中性值（sat=1/desat=hurt=flash=0）
  叠加。锚测 `task71_high_tier_idle_still_enables_grade` 守此不变量，无需用户机观感裁决。

> 沙箱未做 naga 编译（无 GPU / Bevy `#import` 需运行时预处理）；WGSL 接口只做结构锚测，
> 完整 shader 编译与像素结果是用户机复核项，不构成伪验收。

## Task 72 复核登记（2026-11-21，DOM/UI 桥与大厅呈现收口）

无头可测已锚定：16 项 task72 专项测试（DOM 操作序列 create/update/remove/behind/damage/
low-idle/reconcile、LobbyMeasure.safe_opt、SetFade 淡入淡出、set_fade_update 真系统、
CharAnim pose 确定性/全骨树/emote 改变姿态、G-4 AA 裁决）+ 九段门禁 PASS。以下为待用户
GPU/浏览器机器复核项与明确偏差：

- [ ] **D-M6-6 DOM 桥观感**：native 是操作序列锚测；wasm `web-sys` sink 挂 `index.html`
  的 `#iw-overlay`（已补真实 mount），名牌定位（`kFront` px/metre）、伤害数字 scale、
  minimap FX 落点的实际 DOM 观感待浏览器复核。
- [ ] **D-M6-7 安全区/kFront 实测**：native 无 DOM → `safe_opt()=None`（≡ JS `!U`，边缘不
  剔除，与 M5 一致）；wasm 分支当前也未做 `_measureLobby` 实测，沿用 kFront=150 默认值。
  实测测量接入 + 遮挡/安全区观感待复核。
- [ ] **D-M6-8 CharAnim 预览**：骨树由 member 的 ready/emote 状态驱动（emote 首帧触发
  `CharAnim::trigger`），body/头仍是 facade 几何，完整骨树 mesh/skeleton 未生成；预览逐帧
  姿态与 upstream 对照待复核。
- [ ] **D-M6-9 cross-dissolve**：`covered = on && full_frame && k≥1`（≡ JS `_ffFrame`），
  淡入期 `opacity=eInOut(k)`（首轮评审 P0 已修，系统级锚测守此）；GPU 混合/黑屏清屏观感
  待复核。
- [ ] **D-M6-10 GTAO/SMAA 选型**：GTAO 无 Bevy 内建且需 depth/normal prepass → 保持未接；
  SMAA 虽有 Bevy 内建（`SmaaPlugin`+内嵌 LUT），但沙箱无 GPU + placeholder LUT 与 JS 真实
  area/search 纹理有差异 → 按 G-4「继续登记」分支暂不接线；两项待用户机裁决是否接线。
- [ ] **名牌遮挡/过渡观感**：8 名牌的 behind/safe 判定、hub⇄room 淡入淡出曲线在四档 +
  三主题下的实际画面。

## Task 73 复核登记（2026-11-21，剩余消费者收敛，零 sim 改动）

无头可测已锚定：8 项 task73 专项测试（audible 18m 边界 / 2-D cue 直通 / 有 camera 系统级
超距剔除 / 无 camera 全保留 / cameraShake 设置缩放 / showFps 文本门 / fps_hud_update 系统级
写标签 / low 档 post 全关且设置跟随用户）+ 九段门禁 PASS，
`inkwave_sim` 与上游 JS 均零改动。以下为用户机复核项与保持的偏差：

- [ ] **D-M6-11 showFps 观感**：`showFps` 设置门控数据面已接（HUD `LabelRole("fps")` 写
  "N FPS"/""），真实字形渲染/位置与 JS 对照待用户机复核；native headless 以系统级写入
  断言替代真实像素。
- [ ] **D-M6-12 18m 音频剔除**：`audible_at` 以相机 translation 为听点、18m 平方距离剔除
  spatial cue（2-D clock cue 直通，无 camera 时不剔除）；18m 是 D-T60-5 规定策略——实读上游
  `audio.js` 只有 3-D 距离 + panner `maxDistance=max(60,ref*20)` 平滑低通，**未发现 18m 硬截断
  常量**，故不声称与上游常量等价；实际听感（含 listener orientation/occlusion）用户机复核。
- [ ] **保持 M5 偏差（不因本任务改动）**：D-T60-2 footstep surface（数据模型无 surface 字段）、
  D-T60-3 muzzle 回写（需新 MatchEvent）、D-T64-4 superjump/bomb explode FX（需新事件）
  维持未接；本任务严格不改 `inkwave_sim` 事件面。

## M6 汇总复核清单（Task 74 定稿，2026-11-21）

沙箱无头可测部分已全部闭环（Task 71/72/73 九段门禁 PASS + `M6_MATRIX_VERDICT=PASS` +
全 10 crosscheck 回归 + run-all 双跑字节一致 + headless 4 组合/boss 600 帧），上游 JS/server
与 `inkwave_sim` 依赖面零改动（M6 全程零 sim 改动）。**以下为待用户 GPU/浏览器/听感机复核
的汇总项**（双轨验收，按任务索引到上方逐任务登记段的 D-M6 偏差，本汇总不构成「已通过」）：

| 复核组 | 任务 | 关键待复核面 | 偏差登记 |
|---|---|---|---|
| 后处理本体 | 71 | grade 12 uniform 色彩分级 / composite hurt-flash-vignette 观感、完整 naga 编译、两 pass 合一与 Bloom 先后、HDR target 格式 | D-M6-1..5 |
| DOM/大厅 | 72 | 名牌 DOM 定位/伤害数字/minimap FX 落点、安全区实测、CharAnim 骨树 mesh、hub⇄room cross-dissolve、SMAA/GTAO 接线裁决 | D-M6-6..10 |
| 消费者收敛 | 73 | showFps 字形渲染 / 18m 音频剔除听感（listener orientation/occlusion） | D-M6-11/12 |

**M5 偏差承接说明**：M6 全程未推翻 M5 Non-Goals——D-T60-2（footstep surface）、D-T60-3
（muzzle 回写）、D-T64-4（superjump/bomb FX）、D-T65-1/-3（GTAO/SMAA 本体）等保持 M5 状态；
M6 只在数据面/无头可测面上接线，未接的 GPU/听感本体继续登记。
**沙箱口径诚实声明**：M6「本体接线」在沙箱交付的是数据面 + 纯函数/状态机 + 无头可测行为；
WGSL 完整编译、DOM 真实节点观感、CharAnim mesh、GPU 混合、听感一致性均未伪验收，以
`M6 visual-review` 登记。用户机复核结论回填后，本清单方可整体勾选。
