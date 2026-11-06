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

### Task 52 ink 着色 + 地表材质（2026-11-06 登记；沙箱无 GPU，以下均需用户机器视觉复核）
- **湿润/光泽对比档**（TR-52.2）：inkShading 9 chunk 全量（INK_PARS/COLOR/ROUGH/GEL/SLOPE/EMISSIVE/LIGHTS/LIGHT_MAPS/SHADE）替换 M1 简化 ink.wgsl——高度位移/队伍色/tone jitter/光泽/湿润 roughness 调制/墨缘斜面法线，对照 JS 浏览器版重点=落墨瞬间 wet 高光衰减与 slope 边缘观感。
- **46 层材质观感**（D-T52-7①）：CPU 烘焙替代 GPU MRT——与 JS GPU 输出**逐像素不对等**（同参数 Rust 自洽逐字节已由 golden 钉死）；逐层抽看 concrete/pavers/hazard/grate/gelcoat + 至少 2 个 stage 层（kelpline/lockgate/terraces/cargo 槽位）的 albedo/normal/orm 观感。
- **mip 远景闪烁**（D-T52-2/5）：box-filter mip 链 + mip-linear + aniso16 sampler；远景 GRID/HEX 防重复采样模式（texlib_sample 三模式+sym 位掩码）是否闪烁/走样，对照 JS textureGrad 行为。
- **烘焙耗时**（D-T52-7① 性能复核）：沙箱实测 43 层×32px=190ms、**43 层×512px≈47.2s**（JS GPU 冷启动≈40ms）；后台线程烘焙不阻塞主循环，但首秒内地面为中性占位纹理——用户机器留意冷启动观感与三附件 512 档 ~193MB 显存。
- **1174 draw call**（D-T52-4 性能项）：批次键含 fsz（面尺寸入批键，pattern uniform 每批一份的代价），tidewater 实测 1174 批——帧率复核归 Task 55 性能项。
- **上传路径偏离预设**（D-T52-4）：spec 预设「`Queue::write_texture` 手写 render-world system，不可行则 LOD0 回退」——实际 Bevy 0.19 `GpuImage::prepare_asset` 原生支持多 mip 数组纹理（`create_texture_with_data` 按 mip_level_count），未走手写路径；**边角复核**：`texlib_bake_poll` 原地改写占位 Image 的 `texture_descriptor`（1×1/mip1→512×512×46/mip10）触发 GPU texture 重建的时序（首帧是否有 descriptor/data 不一致窗口）。
- **naga≠Bevy 编译**：level.wgsl 经 naga-cli 独立校验通过（binding 契约 20-42），但未经真实 wgpu 设备编译/渲染——首次上机运行留意 shader 编译错误与 binding 布局。
- **INK_LIGHTS/LIGHT_MAPS 近似**（D1 登记）：无 Bevy 灯钩子→折入 emissive/diffuse 近似；灯墨面反光（uLamps 表已接、`u_lamp_n=0` 恒关=Task 53 props 数据源）复核。
- **质量档位锚错误**（E1 登记）：spec 锚 `rendererQuality==='low'` 在 JS 全树 **0 命中**；真实门=main.js L121 `q.paintAtlas>=4096?512:256`（**medium 实际也 256**）——Rust `quality.rs` 按真实门实现（Low=256/Med-High=512 的 texlib 档位为 Rust 侧设计），JS 对照口径以此为准。
- **偏差留档**（D-T52-7）：②clearcoat 近似（roughness/法线调制，Task 55 重议）③faceData/faceFlags/faceTan→批 uniform（box 面内恒定等价；lamp/lightKind 位恒 0）④lightUv→paintUv 复用（无 lightmap atlas）⑤gel 13 槽/mural 表 shader 面平移但数据恒失活（Task 53 接）⑥bevel/圆角几何不入本任务⑦showcase ripple=Task 57⑧f32 超越函数跨后端位差不承诺（确定性口径=同机同二进制逐字节）。
- **wasm32 烘焙 inert**（E2 登记）：`std::thread` wasm 不可用→浏览器构建保持中性占位纹理（地面有材质无图案）；`wasm_thread`/`Asyncify` 烘焙=Task 55 候选。
- **swimWake 收口**（D-T51-7⑦ 闭环）：`SwimWakeState` 四缓冲→LevelExt binding 33-36 每帧上传已接（钉测=getter 逐值断言）；游动尾迹观感随本任务视觉复核。
- **FloodJet y 语义**（E2 拿不准项→评审已核销）：独立评审实核 `ink_ripple.rs` FloodJet.y=zone.center[1] ≡ zones.js L166 调用点，与 paint.js L729 消费口径一致；喷墨涟漪高度贴合仍随本任务视觉复核。
- **独立评审处置**（2026-11-06）：**0 P0 / 0 P1 / 6 P2 → 判定可提交**。逐式抽查 7 层（concrete/pavers/hazard/grate/brick/herringbone/stucco）+ gen.rs 噪声原语 + bake.rs≡GEN_MAIN + texlib_sample 槽表 + face_frame + RippleTable + E2 上传路径（bevy_render gpu_image.rs 全链上传实证）+ 口径纪律（JS 树零改动/提取器钉源/329 测试）全部通过。P2 处置：
  - **P2-1 ✅ 当场修复**：`level.wgsl` L430 `texlib_detail` 原 `vec4` splat 连 `.a` 缩放（JS texlib.js L1291 仅乘 rgb）→改 `(*s).albedo.rgb = (*s).albedo.rgb * vec3<f32>(...)`。当前无调用点（死代码），先于 Task 53/55 接线消除 grate 覆盖/mask tint alpha 污染隐患；WGSL 语法与同函数 L432/433 既有指针解引用+swizzle 用法一致，naga-cli 沙箱不可复得→全量编译复核并入「naga≠Bevy 编译」条目。
  - **P2-2 留档**：quality.rs Medium=512 与 JS 真实门（main.js L121，medium→256）不符——即上方「质量档位锚错误」条目，Rust 三档设计口径，Task 55 复核。
  - **P2-3 ✅ 台账更正**：任务简报 texlib.json sha256 失实（`42833d0459f5…69ec`）→实际 `42833d04a9eb9ec3d247f0c1f0ec1fbfa1bad8120cf6372b2500a556b20a7edc`；仓内无文件钉该哈希、重跑提取 git diff 为空，工件无恙，仅台账用实测值。
  - **P2-4 留档**：`extract-texlib.mjs` L178 `sizeLow:256, sub:2` 手抄字面量（非提取），L176-177 assertContains 钉源兜底；后续如需可改提取式，不阻塞。
  - **P2-5 留档**：`level_render.rs` L424 `cams.iter().next()` 依赖单相机前提（world.rs L943 仅一个 Camera3d，当前充分）；多相机场景（Task 57 showcase）需加 `is_active` 过滤。
  - **P2-6 ✅ 处置**：`rust/tools/verify/logs/task38-ladder-*.log` 系 Task 38 scratch 产物（未跟踪），不纳入 Task 52 提交面；本仓门禁日志惯例逐任务提交、scratch 日志不入库。

### Task 51 fx（2026-11-06 登记；沙箱无 GPU，以下均需用户机器视觉复核）
- **粒子/impostor 观感**（D-T51-7②）：六池 instanced billboard 近似材质占位档——JS 逐像素 GLSL（液滴 elongation 拉伸、glow kind G_SOFT/STAR/BUBBLE/HALO、motes 浮尘、shells crown）精度顺延视觉复核；重点=命中 splat/beam/laserDot 的观感对照 JS 浏览器版。
- **flood 呈现**（D-T51-7①）：screenfx 效果状态机（12 事件→uniform 值）本任务平移，**全屏 composite pass 不接=Task 55**——受击 chroma 边缘/击杀闪/urg 脉冲当前无全屏 shader 合成可见，登记顺延。
- **zone 带绘**（D-T51-7⑥）：zoneMarks 单 mesh ShaderMaterial→简化边界带 mesh 占位档（flood 波峰/contested/active/sweep/flash 数据层已对账）；虚线相位/跑马动画/波峰渐显观感需复核。
- **swimWake**（D-T51-7⑦）：swimmer 筛选（≤4/本地优先/相机距离）+轨迹 ring-buffer 数据层落地；`uWake` 组 uniform 上传点 ink.wgsl 无对应→登记顺延（Task 52 着色面接）。
- **事件→fx 触发对账**（TR-51.1）：`fx/hooks.rs` polling 推断（actor:\* 总线 Rust 无→≡JS fallback，D-T51-7③）+SimEvent/TickEvent 消费；触发计数 `this.count` 逐项单测；缺口如实登记（bomb:explode/weapon:impact/damage 无 MatchEvent）。
- **粒子预算**（TR-51.1 性能门限）：满池 cap 单测（drops/sprites/rings 达上限即 swap 回收不超发）；用户机器留意大混战帧率。
- **确定性**（D-T51-7④）：Math.random→固定种子 PRNG（≡Task 50 口径），同参数逐字节可复现。

### Task 50 音频（2026-11-05 登记；沙箱无声卡，以下均需用户机器听感复核）
- **音色对 JS 基线**（TR-50.2）：148 SFX 全表烘焙档（`audio/sfx_table.rs`）逐组抽听——UI/Weapons/Ink/Squid/World/Combat/Status/Specials/Match/Zones/Boss；重点=pink 噪声音色（splat/bomb/storm/boss_roar，评审 P0 修复后钉值）、pulse/stroke 谐波近似（f64 朴素求和 vs JS Float32Array f32——精度差声明）、jitter→播放速率近似（D-T50-6①）。
- **混响/总线缺失声明**（D-T50-6②）：convolver 混响 send、glue compressor、limiter、3D panner/距离 lowpass、音乐 delay/reverb 湿信号**未烘**（dry 占位档）；听感干/贴耳属预期，JS 对照=浏览器版。
- **音乐状态机**：9 曲（title/menu/battle/battle_final/results_win/results_lose/boss/boss_2/boss_3）synth 编译烘焙整曲 loop——file-backed 轨本仓无资产恒 false（占位声明）；fade/remap/escalation 切换听感（battle→battle_final 升级、boss phase 换轨）；loopFrom 跳 intro 不生效（整缓冲回绕偏差①）；loop 起播 60ms fade-in 未烘（偏差⑨声明）；setIntensity 恒 1.0（偏差⑥）。
- **bossAudio**：Task 46 cue 记录表→真实播放（14 事件映射+remap+duck 0.7/3）；LoopStop 硬停无 0.25s 淡出、`<name>_end` 不播、StopAll/music(null) fade 塌缩 0.6（偏差④）。
- **设置面**：`set_volumes({master,music,sfx})`≡L165-172（taper^1.5/τ=0.04）——**无 UI 调用点**（M5 设置屏接，`#[allow(dead_code)]` 在案）；静音回归=无头测已钉（mute→无 spawn+GlobalVolume 归零），用户机器复核切屏恢复。
- **事件接线缺口对账**（TR-50.1 部分）：JS 字面量调用 115 名中 **42 名 kit 系**（blade_/bow_/boomerang_/brolly_/mitts_/shaker_/torpedo_ 等，定义在 `src/game/kits/*-sfx.js` 非 audio.js）超本任务范围——登记顺延（kit 武器 SFX 随各 kit 深战术/Task 51 事件面补）；`judge_drumroll`（hud.js L577）与 title 屏 music('title') 缺口在案。
- **烘焙性能**：惰性首用烘焙（victory_fanfare 单条 debug 10.7s/release 2.1s 实测）——用户机器留意首触发卡顿；全曲烘焙冒烟测 `#[ignore]`（分钟级）。

### Task 49 小地图（2026-11-05 登记；沙箱无 GPU，以下均需用户机器复核）
- **清晰度档位**：`MinimapTier{Low=4, Med=7(JS 默认), High=10}` pxPerM——三档下 turf 双线性边缘、emboss 立体感、5m 网格可读性对照 JS 基线（`minimap.rs` draw_base/draw_ink）。
- **采样率**：ink 刷新 timer=0.15s（≤6.7Hz）+ BANDS=3 分带（单帧约 1/3 工作量）——观感上墨块更新是否有可感滞后/撕裂（`MinimapClock`）。
- **base 层视觉**：海面波带+码头泡沫、块色提亮向纸色、hillshade/投影阴影/AO/暗边 rim、grate 斜 hatch、sunset 暖色板切换（theme 经 `map_theme(id,"day")`，golden≠sunset=现状）——与 JS `_drawBase` 对照（hex 直读 sRGB 偏差 D-T49-5③，±1 舍入）。
- **点标可读性**：player dots=12px 方块色标占位（无圆形/描边/朝向箭头；self 旋转仅方块角度、dead .7 防御分支、tracked 的 CSS ping 环动画未移植）；boss 点=16px 红方块（Rust 扩展，JS 无对应）。
- **zone 标记**：非活动虚线幽灵/活动持有色 fill+双层描边/中立跑马虚线——虚线相位与脉冲节奏对照 `_drawZones`；多 part zone 逐 part even-odd 填充 vs JS nonzero（parts 重叠会双重混合，当前 8 图无此形态）。
- **HUD 布局**：角标左下 `fit(14.5u)`、TAB 展开 `fit(min(H*0.78,W*0.6))` 居中偏上、弹簧 k=190/c=21（`map_layout`/`MapSpring` 公式已单测，观感需复核）；`is-inside` 图例避让、strike 态复用大地图、beacon 面板/死亡标记/live 矢量层（fx 环/云/炸弹/subs/跳跃线）顺延 Task 51/56（D-T49-5①②）。
- **flip**：viewerTeam===1 双轴镜像+全量重建（重建帧 ink 立即全量 vs JS 等 timer ≤0.15s，一次性时序差 D-T49 评审 P2-3）。
