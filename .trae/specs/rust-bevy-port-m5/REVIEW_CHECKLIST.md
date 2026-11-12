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
