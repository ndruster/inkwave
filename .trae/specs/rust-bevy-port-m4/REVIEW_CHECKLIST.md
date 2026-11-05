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

### Task 47 威胁 AI：tr38 ladder survival 端点门曾翻转（2026-11-05 登记，批次 D 后已恢复门过；待用户评审）
- **现象（批次 B+C 时）**：`tr38_difficulty_ladder_monotonic` 的 hard≥easy survival 端点断言失败：开启态 hard 1593.4s < easy 1603.0s（HEAD 基线 hard 1658.8 / easy 1600.3，裕量 +58.5s）。kills 门仍单调（55/75/78）。日志：`rust/tools/verify/logs/task38-ladder-1793819794.log`。
- **A/B 实验**（同 8 种子，`set_threat_ai(false)` 对照，临时测试已回收）：OFF 态 hard 1608.6 > easy 1567.2（裕量 +41.4，门通过）→ 第 7 预抽 `r_thr_scan` 的 RNG 流移单独不翻转门；翻转源自威胁 AI 行为本身。
- **保真复查**：evade 方向（背离设备）、sidestep（垂直设备速度、偏己侧）、drift、scan 的 team+live 过滤、shoot 分支移动改写（站定/kite/×0.25）、aim 误差与 fire 门、`it.sub/bombAim` 压制、flank/steer/break、stamp steer、tail snappy——逐分支与 JS 一致，未发现 bug。
- **统计论证**：ON vs OFF 逐 seed：easy 净 +35.6s（弱 bot 生存收益，符合设计意图）、hard 净 -15.2s，其中 seed 777 单点 -30.6s（死亡螺旋 15→22 死），其余 7 seed 净 +15.4s。8 种子端点门噪声底实测 ±17s 以上（HEAD→OFF 裕量漂移），单个离群 seed 足以翻转 -9.6s 裕量。
- **最终事实（批次 D + 评审 P0-1 修复后）**：端点门已恢复通过——easy 1599.1s / normal 1631.7s / hard 1602.4s，hard≥easy 裕量 +3.3s（日志 `rust/tools/verify/logs/task38-ladder-1793831523.log`；P0-1 `BotCtx.time` 接 `projectiles.time` 的修复对 ladder 逐位无影响，ladder 装具无 blade）。严格单调 hard≥normal≥easy 仍 false（normal 高于两端），与 HEAD 基线同态，沿用 spec §5「违规记录并评审」口径，非新增违例。
- **复核项**：确认端点门恢复属实、±17s 噪声底下的裕量 +3.3s 是否需要扩种子（8→16）复测由用户定夺。

### Task 47 偏差登记复核（2026-11-05 评审轮登记；spec.md D-T47-11/12）
- **D-T47-11 zones catch_up 时延补偿**：`match_/zones.rs` 的 `'zo` field2 / `'zs` field10 时钟戳 + follower `catch_up` 回放补偿是 JS 无对应的新逻辑（tr37 frame 6913 `pen` 差 0.083=5/60 复现证据、修复方式、互操作安全结论见 spec.md 条目）。**专项评审已完成（2026-11-05）**：机制正确性 6 项核对通过；F-1 保真缺口（catch_up 缺 `_tick` tie-point 边沿检测）已修复并复验（sim 592/net-harness 20 全绿，spec.md D-T47-11a）；F-2（`+FIXED_DT` 时序假设）留档。**用户裁定（2026-11-05）：①接受 sim 侧网络路径偏差（JS 树零改动纪律不变，「JS 同步补补偿」否决）②兼容超集定位（JS 按下标读、尾部字段忽略；Rust 按 `len` 门降级；字节等价门只对 fixture 重放路径生效）③性质=网络一致性强化。条目闭环**，定位全文见 spec.md D-T47-11a 与 PORT_MAP zones catch_up 行。
- **D-T47-12 rebuild_shields 每帧重建**：JS 仅 scan 时刷新成员、Rust 每帧重建（canopy 出现/消失至多早一个 scan 周期 ≤0.25s）。评审已定级 P1 可接受，留档备查。

### Task 49 小地图（2026-11-05 登记；沙箱无 GPU，以下均需用户机器复核）
- **清晰度档位**：`MinimapTier{Low=4, Med=7(JS 默认), High=10}` pxPerM——三档下 turf 双线性边缘、emboss 立体感、5m 网格可读性对照 JS 基线（`minimap.rs` draw_base/draw_ink）。
- **采样率**：ink 刷新 timer=0.15s（≤6.7Hz）+ BANDS=3 分带（单帧约 1/3 工作量）——观感上墨块更新是否有可感滞后/撕裂（`MinimapClock`）。
- **base 层视觉**：海面波带+码头泡沫、块色提亮向纸色、hillshade/投影阴影/AO/暗边 rim、grate 斜 hatch、sunset 暖色板切换（theme 经 `map_theme(id,"day")`，golden≠sunset=现状）——与 JS `_drawBase` 对照（hex 直读 sRGB 偏差 D-T49-5③，±1 舍入）。
- **点标可读性**：player dots=12px 方块色标占位（无圆形/描边/朝向箭头；self 旋转仅方块角度、dead .7 防御分支、tracked 的 CSS ping 环动画未移植）；boss 点=16px 红方块（Rust 扩展，JS 无对应）。
- **zone 标记**：非活动虚线幽灵/活动持有色 fill+双层描边/中立跑马虚线——虚线相位与脉冲节奏对照 `_drawZones`；多 part zone 逐 part even-odd 填充 vs JS nonzero（parts 重叠会双重混合，当前 8 图无此形态）。
- **HUD 布局**：角标左下 `fit(14.5u)`、TAB 展开 `fit(min(H*0.78,W*0.6))` 居中偏上、弹簧 k=190/c=21（`map_layout`/`MapSpring` 公式已单测，观感需复核）；`is-inside` 图例避让、strike 态复用大地图、beacon 面板/死亡标记/live 矢量层（fx 环/云/炸弹/subs/跳跃线）顺延 Task 51/56（D-T49-5①②）。
- **flip**：viewerTeam===1 双轴镜像+全量重建（重建帧 ink 立即全量 vs JS 等 timer ≤0.15s，一次性时序差 D-T49 评审 P2-3）。
