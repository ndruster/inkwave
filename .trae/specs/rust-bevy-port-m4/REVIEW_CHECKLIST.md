# M4 复核清单（实施期逐任务登记；收尾时并入总评审）

> 格式沿用 M2/M3。"已证" 必须附落盘日志/测试输出路径；沙箱不可达项标注"待用户机器复核"。

## 待登记条目（规划期预置）
1. **Boss 战跨端完整局**（AC-M4-2 / TR-45.2）：JS host ↔ Rust guest boss 局 scenario E——沙箱 headless 实跑为基线（握手/roster/快照到达可断言），真人开黑待用户机器复核。
2. **Boss 战视觉呈现**（Task 46）：体块剪影可辨识度/telegraph 可读性/HUD boss 条（GPU 环境）。
3. **小地图清晰度**（Task 49）：采样率/点标可读性。
4. **音频听感**（Task 50）：过程化合成音色对 JS 基线的听感复核；音乐状态机切换。
5. **fx 视觉**（Task 51）：粒子/墨团/受击方向指示/zoneMarks flood 呈现。
6. **ink 湿润/光泽与地表材质**（Task 52）：与 JS 渲染对照的视觉档位。
7. **props/murals 装饰**（Task 53）：三图+五新图装饰完整度。
8. **角色姿态手感**（Task 54）：弹簧绑定形态切换/武器手持。
9. **环境与后处理**（Task 55）：黄昏主题/GTAO/bloom/SMAA/tilt-shift 逐档；低配全关保底可玩路径。
10. **showcase 大厅**（Task 56）：3D 大厅场景与 loadout 联动。
11. **沙箱内存约束延续**：trunk release 构建受 OOM SIGKILL 间歇影响（D-T40-1 断言已入门禁脚本）；用户机器应一次通过。

## 逐任务复核记录

### Task 46 Boss 呈现占位档（2026-11-04 登记；沙箱无 GPU，以下均需用户机器复核）
- **剪影轮廓**：壳/双钳/眼柄/6 腿布局可辨识度、朝向与移动一致；腿步频随 speed、静止停摆；钳举→slam 俯冲连贯；stunned 下垂；dead 压平 0.35；phase≥3 红移 hint + hurt 白闪强度（`boss_render.rs`，参照 bossModel.js L86-101 简化档）。
- **telegraph 可读性**：slam 圈渐显 + 双环扩散（带宽随半径增长=占位简化）；barrage 落圈脉冲 + 桶抛物线；sweep 扇形与 beam 扫角同步（beamLen 常数 17、无墙裁边——D-T46-7）；charge 车道随推进收缩/随 yaw 旋转；frenzy 全圆渐显；标记 unlit+blend 与地面/雾的绘制次序（参照 bossHazards.js L357-600 HazardFX）。
- **crablet pop**：肿胀 1+0.45u²/0.14s → 消失节奏（参照 bossModel.js L222-234）。
- **HUD boss 条**：top 布局、chip 白条/红 HP 条叠放宽度、stun 条、OPEN!/callout 定位、title card/banner/ending 全屏文本、`is-zero` 闪与 hover cue 未移植（DOM 效应，D-T46-9）——均为文本/矩形占位档（参照 hud-boss.js 全 524 行）。
- **结算 boss 行**：PHASE 列 + per-player dmg/weak/splats + Σ 总计行对齐（参照 main.js L960-961、menus.js L4031-4040）。
- **bossAudio 占位**：本档仅事件→cue 记录表（无真合成，听感验收属 Task 50）；cue 触发序/去重门已由单测钉值，无需 GPU 复核。
