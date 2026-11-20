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

## M6 汇总复核清单（Task 74 定稿时填写）

_待 Task 74 收尾时按实际实施结果填写。_
