# tuning.json — 字段模式（schema `inkwave.tuning.v2`）

由 `rust/tools/extract/extract-tuning.mjs` 从上游 `src/config.js` 提取，**不要手改**。
数值单位与 JS 一致：距离=米，时间=秒，速度=m/s，加速度=m/s²，角度=弧度（武器散布/弧判定/扇形/跃角字段为度），
HP/墨量为点（PLAYER.hp=100，PLAYER.inkMax=100）。坐标系 Y-up；Alpha 出生于 -Z，Bravo 出生于 +Z。
确定性输出：同一上游 commit 重跑逐字节一致（TR-3.4）。
v1→v2（Task 28 / FR-M3-1）：新增 `weaponOrder` `weapons` `weaponRange` `subOrder` `subs`
`specialOrder` `specials` `zones`，`difficulty` 由仅 easy 扩为三档；`spritzer` 保留为
`weapons.shooter` 的 M1 别名（Task 29 迁移消费端后移除）。
权威数值以本 JSON 为准（本文档只描述字段与语义，逐值锚定见 `inkwave_sim` 单测）。

## 顶层

| 字段 | 含义 |
| --- | --- |
| `schema` | 模式标识 `inkwave.tuning.v2`，Rust 侧由单测 `schema_and_provenance` 精确断言 |
| `source.commit` / `source.path` | 数据出处（git commit 与 JS 文件），用于上游同步审查 |
| `player` | `PLAYER`（config.js L21-90）：角色物理/手感全部常量，见下 |
| `spritzer` | `WEAPONS.shooter` 的 M1 别名（Task 29 移除；等值断言见测试） |
| `weaponOrder` | `WEAPON_ORDER`（L303）：12 武器出场顺序 |
| `weapons` | `WEAPONS`（L94-300）按 `weaponOrder` 投影出的 12 把武器；退役定义不输出（TR-28.2） |
| `weaponRange` | `weaponRange()`（L307-315）对每把武器的预计算有效射程（m），见下 |
| `subOrder` | `SUB_ORDER`（L407）：15 个副武器出场顺序 |
| `subs` | `SUBS`（L318-406）按 `subOrder` 投影 |
| `specialOrder` | `SPECIAL_ORDER`（L469）：19 个特殊武器出场顺序 |
| `specials` | `SPECIALS`（L411-468）按 `specialOrder` 投影 |
| `zones` | `ZONES`（L485-501）：Zone Control 规则数值，见下 |
| `match` | `MATCH`（L472-482）：涂地比赛参数 |
| `teamPalettes` | `TEAM_PALETTES`（L8-14）：5 套队伍墨色（hex），a=Alpha，b=Bravo |
| `difficulty` | `DIFFICULTY`（L503-508）：easy/normal/hard 三档 bot 参数，见下 |

## player（src/config.js `PLAYER`）

字段名与 JS 完全一致（camelCase）。关键值：`hp=100, runSpeed=6.0, swimSpeed=11.8,
respawnTime=5.5`。分组：

- 生存：`hp` `respawnTime` `spawnInvuln` `fallDeathY` `waterY`
  （`fallDeathY=-1.45` 触水即死；海面 `waterY=-1.6`）；`enemyInkDps=20`、
  `enemyInkDamageCap=40`；`regenDelay/regenRate/regenRateSwim`
- 移动：`runSpeed` `swimSpeed` `squidDrySpeed` `enemyInkSpeed` `climbSpeed`
  `accelGround/accelAir/accelSwim`，及 `run*/swim*/squid*/air*/enemyInk*` 一组手感曲线常量
  （S 曲线加速、刹车、转向、蹬墙等，注释见 JS 源文件）
- 跳跃/重力：`jumpVel` `swimJumpVel` `gravity=25` `maxFall` `jumpBuffer` `coyoteTime`
  `fallGravityMul` `apexGravityMul` `apexBand` `hardLand*`
- 墨量：`inkMax=100` `inkRefillSwim=42` `inkRefillKid=9` `inkRefillDelay=0.9`
- 控制器几何：`radius=0.38` `height=1.45` `squidHeight=0.55` `footRadius=0.24`
  `stepUp=0.35` `stepDown=0.45` `squidStepUp` `squidBodyLift` `ledgeAssist=0.35`
- 朝向弹簧：`faceOmega/faceMaxRate/faceMaxAcc`（kid）、`squidFace*`、`swimFaceMaxRate`、
  `aimFace*`
- 爬墙：`climbAccel` `climbSideSpeed` `climbAttachDot` `climbDetachDot`
  `ledgePopClear` `ledgePopCarry`
- 射击联动：`emergeDelay=0.07`（鱿鱼→kid 后首发延迟）、`fireBuffer=0.16`
- 特殊武器计量：`specialChargeRate=0.8`、`specialKeepOnSplat=0.5`

## weapons（src/config.js `WEAPONS`，按 `WEAPON_ORDER` 输出 12 把）

每把的公共字段：`id/name/kind/class/blurb` 为展示字符串；`stats` 是 0..1 的配装界面条
（range/damage/rate/mobility/paint，非模拟数值）；`special/specialCost/sub` 是默认特殊/计量点/副武器
搭配；`anim`（仅部分武器）= 角色动画姿态族（kind 之外），存在于 bucket/spinner/twins/brush/
brolly/bow/blade/mitts。以下逐把列模拟字段（含义详见 config.js 行内注释）。

- **shooter**（L95-105，= `spritzer` 别名源）：`fireInterval` `damage` `inkPerShot` `projSpeed`
  `straightTime` `range` `spreadGround/spreadAir`（度）`impactRadius` `trailRadius/trailEvery`
  `moveSpeedFiring`。弹丸重力使用 `player.gravity`。
- **twins**（L158-168，anim shooter）：shooter 同族字段（`fireInterval/damage/inkPerShot/projSpeed/
  straightTime/range/spreadGround/spreadAir/impactRadius/trailRadius/trailEvery/moveSpeedFiring`）
  + 翻滚：`rollDist/rollTime/rollInk/rollCharges/rollLockout/rollReset`
  + 定身炮台：`turretInterval/turretSpread/turretProjSpeed/turretStraight/turretRange`。
- **brolly**（L180-198，anim shooter）：霰弹 `fireInterval/inkPerShot/pellets/pelletDamage/
  pelletDamageFar/falloffStart/falloffEnd/projSpeed/straightTime/pelletGrav/pelletDrag/pelletLife/
  pelletSize/pelletPaint/spreadDeg/spreadAir/range/moveSpeedFiring/moveSpeedShield`
  + 伞面：`canopyHp/openDelay/openTime/closeTime/launchHold/regrowTime/canopyRegen/canopyRegenDelay`
  + 伞墙：`launchSpeed/launchLife/launchPaint`。
- **blaster**（L126-135）：`fireInterval/directDamage/splashDamageMax/splashDamageMin/splashRadius/
  inkPerShot/projSpeed/range/impactRadius/burstRadius/moveSpeedFiring`（`burstRadius`=空中爆裂半径）。
- **spinner**（L148-157，anim charger）：`chargeTime/inkFull/burstMin/burstMax/fireInterval/damage/
  speedMin/speedMax/straightMin/straightMax/rangeMin/rangeMax/spreadGround/spreadAir/impactRadius/
  trailRadius/trailEvery/moveSpeedCharging/moveSpeedFiring`。
- **charger**（L117-125）：`chargeTime/rangeMin/rangeMax/damageMin/damageMax/inkFull/lineSplatEvery/
  lineRadius/impactRadius/moveSpeedFiring`（满蓄力穿透线）。
- **bow**（L199-219，anim charger）：三档（tap/ring/full）参数，二元数组= [ring, full]：
  `chargeTime/ring1/inkFull/inkMin/range`；`flightTap:[m,m] flightRing:[m,m] flightFull`
  `speedTap speedRing:[m/s,m/s] speedFull`；`straight/grav/diveGrav/diveDrag`；
  `damageTap:[14,20] damageRing:[30,38] damageFull sideMul sideFull`；`fanTap/fanRing/fanFull`（度）；
  `fuseRing/fuseFull`；`burstRadius:[m,m] burstInner burstDamage:[,] burstEdge:[,]`；
  `paintTap/paintStick/burstPaint:[,] trailEvery/trailRadius`；`cooldown/moveSpeedDrawing/moveSpeedFiring`。
- **roller**（L106-116）：滚动 `rollSpeed/rollWidth/rollInkPerMeter/rollDamage`
  + 甩墨 `flickInterval/flickWindup/flickInk/flickDrops/flickDamageNear/flickDamageFar/flickSpeed/
  flickSpreadDeg`（度）；`impactRadius/moveSpeedFiring`。
- **brush**（L169-178，anim roller）：冲刺 `brushSpeed/brushWidth/brushInkPerMeter/brushDamage/
  brushHitCd` + 横扫 `swipeInterval/swipeInk/swipeDrops/swipeSpeed/swipeSpreadDeg/swipeDamageNear/
  swipeDamageFar`；`impactRadius/moveSpeedFiring`。
- **blade**（L220-238，anim blade）：快斩 `tapInterval/tapInk/tapDrops/tapSpreadDeg/tapSpeed/
  tapStraight/tapDamageNear/tapDamageFar/tapDropRadius/tapMelee`
  + 蓄力斩 `chargeDelay/chargeTime/chargeStore/heavyInk/heavyInterval/heavyMelee/lungeDist/lungeTime/
  strokeRadius` + 墨浪 `waveDamage/waveDamageFar/waveSpeed/waveRange/waveWidth/waveHeight/
  wavePaintEvery/wavePaintRadius/waveEndRadius`
  + 刀身判定 `reach/meleeArcDeg/heavyArcDeg`（度）；`range/projSpeed/moveSpeedFiring/moveSpeedCharging`。
- **mitts**（L239-264，anim shooter）：拳弹 `punchInterval/inkPerPunch/punchSpread/fistSpeed/
  fistRange/fistSize/punchDamage/splashRadius/splashMax/splashMin/fistPaint`
  + 海绵格挡 `guardArmor/guardArc/guardAfter`（度）+ `botApproach`
  + 蓄力跃 `leapChargeTime/leapInkMin/leapInkMax/moveSpeedCharging/leapArmor/leapArmorGrace/
  leapSpeedMin/leapSpeedMax/leapAngle/leapAngleMin/leapAngleMax/leapPitchK/leapVyMax`（度）
  + 落地 `landRadius/landDamageMax/landDamageMin/landPaint/landInvuln/gloveRadius/gloveDamage`
  + 贴墙 `clingDrain`；`range/moveSpeedFiring`。
- **bucket**（L137-147，anim blaster）：`fireInterval/windup/inkPerShot/throwSpeed/lob/blobs/gravity/
  drag/reach/damage/impactRadius/belowFalloff/belowFalloffMax/moveSpeedFiring`（抛物线泼墨，无距离衰减，
  仅低于投掷者目标按 `belowFalloff`/m 减伤、上限 `belowFalloffMax`）。

退役定义 `dualies/slosher/splatling`（L266-299）**不进入产物**（TR-28.2，测试反向断言）。

## weaponRange（`weaponRange()` L307-315 的预计算表）

提取时逐武器求值（charger/spinner 取 `rangeMax`；roller=6；brush=5；bucket 取 `reach`；
其余取 `range`，缺省 12），Rust 侧直接消费 `HashMap<String, f32>`，不重复分支逻辑：
shooter 12.5 / twins 11 / brolly 7 / blaster 10.5 / spinner 21 / charger 27 / bow 25 /
roller 6 / brush 5 / blade 6.5 / mitts 5.3 / bucket 9.5（m）。

## subs（src/config.js `SUBS`，按 `SUB_ORDER` 输出 15 个）

公共字段：`id/name/kind/blurb/inkCost`（墨耗，点）。按形态分组：

- 投掷爆炸族（`throwSpeed` 初速 m/s、`radius/damageMax/damageMin` 爆炸、`paintRadius` 泼墨）：
  **bomb**（`fuse`）、**sticky**（贴墙，`fuse` 2.4）、**burst**（触地即爆：`directDamage/splashDamage`，
  无 fuse）、**mist**（`fuse/radius` + 毒雾 `mistTime/slow/inkDrain`）、
  **seeker**（爬行者：`speed/seekRange/life/trailRadius/triggerDist/turnRate`（rad/s）`commitDist/creep`）、
  **shaker**（摇三下：`chargeTime/moveBoost/jumpBoost/shakeBoost/maxRate/fuse/gap/hopSpeed/hopUp/
  radius/damageMax/damageMin/paintRadius/trailEvery/trailRadius`）、
  **waddle**（落地感应 `senseRadius/senseUp`，追人 `speed/turnRate/life/maxTravel/triggerDist`，
  `radius/damageMax/damageMin/paintRadius/hp`）、
  **torpedo**（空中锁定：`feetPaint/armTime/lockRange/flyLife/unfoldTime/launchSpeed0/launchSpeed/
  launchAccel/turnRate/launchLife/hp/radius/coreRadius/damageMax/damageMin/paintRadius/fallbackPaint/
  drops/dropDamage/dropHits/dropPaint`）、
  **boomerang**（`range/outTime/hover/hoverRadius/tickRate/tickDamage/hoverPaint/returnSpeed/orbit/
  orbitRadius/orbitSpin/orbitDamage/orbitHitCd/radius/damageMax/damageMin/paintRadius/contact/hitFuse/
  hitRadius/hitDamageMax/hitDamageMin/hitPaintRadius`）。
- 侦测/场地族：**scan**（`fuse/radius/cloudTime/trackTime`，无伤害）、
  **curtain**（墨幕 `width/height/hp/decay/shotMul`）、
  **sprinkler**（`hp/pulse/drops/sprayRadius/sprayFade/dropDamage`）、
  **mist** 见上。
- 放置族（`placed=true`、`max` 同时上限）：**mine**（`triggerRadius/armTime/delay/radius/damage/
  trackTime/paintRadius`）、**beacon**（`uses/hp`）。
- 弹道族：**tracer**（`speed/range/pitchDown`（度）`floorExit/maxBounces/size/puddleRadius/
  directDamage/directMark/trailLife/trailRadius/trailDamage/trailMark`）。

## specials（src/config.js `SPECIALS`，按 `SPECIAL_ORDER` 输出 19 个）

公共字段：`id/name/blurb`；`duration`（s，计时类）；开始施放回满墨（JS 注释语义，无数值字段）。
JSON 键为蛇形（`barrage_sticky` 等），Rust 侧显式 rename。

- **slam**：`rise/hang/radius/killRadius/damageMax/damageMin`。
- **storm**：`duration/radius/dps/throwSpeed/driftSpeed`。
- **barrage ×5**（`barrage`/`barrage_sticky`/`barrage_burst`/`barrage_seeker`/`barrage_mist`，
  `kind:"barrage"`）：`bomb`（对应 sub id）`/duration/gap`（最短投掷间隔 s）。
- **bubbler**：`duration/radius/knockPerDamage/knockMax/shareRange`。
- **sonar**：`duration/slow/inkMul/cast`。
- **strike**：`aimTime/flight/radius/duration/dps/pull`。
- **zooka**：`duration/interval/speed/range/damage/height/radius/paintEvery/paintRadius`。
- **wail**：`charge/blast/radius/range/dps/holdTime/holdSpeed`。
- **kraken**：`duration/speed/hopVel/attackVel/attackFwd/radius/damage/knockPerDamage/cooldown/paintRadius`。
- **blower**：`duration/max/inflate/rMin/rMax/drift/life/popDamage/blastMul/damageMax/damageMin`。
- **jetpack**：`duration/height/speed/accel/boost/boostGap/interval/projSpeed/range/directDamage/
  splashMax/splashMin/splashRadius/paintRadius`。
- **stamp**：`duration/interval/reach/radius/damage/lunge/moveSpeed/throwSpeed/throwRadius/
  throwDamageMax/throwDamageMin/turnRate/turnRateStill/strafe/guardTime/deflectArc`（度）
  `deflectR/bombClearR/flipTime/flipReach/flipRadius/flipBackReach/flipBackRadius`。
- **booyah**：`charge/cheer/cheerSpecial/autoThrow/moveSpeed/throwSpeed/fuse/radius/killRadius/
  damageMax/damageMin`。
- **zipcaster**：`duration/range/speed/hang/cooldown/impactDirect/impactSplash/impactDirectR/impactRadius`。
- **crab**：`duration/hp/speed/rollSpeed/turnRate/gunInterval/gunDamage/gunSpeed/gunRange/gunSpread/
  cannonGap/cannonSpeed/cannonRadius/cannonDamageMax/cannonDamageMin/rollArmor`。

## zones（src/config.js `ZONES`，Zone Control 规则）

| 字段 | 值 | 含义 |
| --- | --- | --- |
| `duration` | 300 s | 常规时限（5 分钟 + 加时） |
| `count` | 100 | 双方各自倒数 |
| `rotateMin` / `rotateMax` | 30 / 60 s | 目标点在中场与边区之间轮换的间隔 |
| `finalCentre` | 30 s | 剩余时间低于此值（及整个加时）仅中场有效 |
| `warn` | 0.30 | 对方占有区墨量份额达此值时触发「即将翻转」警报 |
| `control` / `contest` | 0.80 / 0.40 | 夺取 / 中和对方所占区的墨量份额 |
| `rateCenter` | 1 | 占中场每秒扣减点数 |
| `rateHome` / `rateAway` | 0.5 / 2 | 占己方 / 对方半场边区的扣减速率 |
| `penaltyK` | 0.75 | 结算罚时 = ROUND(0.75 × (start − end))（start=100 再 +1） |
| `gaugeHeld` | 4.5 | 未占目标方特殊计量点/s |
| `gaugeNeutral` | 1.5 | 无人占领时落后方特殊计量点/s |
| `overtimeGrace` | 10 s | 落后方离开目标后加时宽限 |
| `overtimeMax` | 300 s | 加时上限 |
| `sampleHz` | 5 | 每秒覆盖采样次数 |

## match（src/config.js `MATCH`，涂地模式）

`durations=[90,180]` 秒可选；`defaultDuration=maxDuration=180`；`finalCountdown=10`；
`teamSize=4`；`pointsPerM2=1.0`（每平方米新涂地=1 分）；
`deathMarkLife=5`、`deathMarkFade=1.3`。

## teamPalettes

`{id, a, b, names:[Alpha名,Bravo名]}`；颜色为 sRGB hex 字符串，渲染侧按原值使用。

## difficulty（src/config.js `DIFFICULTY`，三档）

字段：`id/name/reaction/aimError/fireDiscipline/awareness/aimOmega/aimTurn`。
`reaction` 反应时间 s；`aimError` 瞄准误差；`fireDiscipline` 开火纪律 0..1；
`awareness` 感知半径 m；`aimOmega` 瞄准弹簧刚度 rad/s；`aimTurn` 转向速率上限 rad/s。

| 档位 | name | reaction | aimError | fireDiscipline | awareness | aimOmega | aimTurn |
| --- | --- | --- | --- | --- | --- | --- | --- |
| easy | Chill | 0.55 | 0.11 | 0.55 | 16 | 9 | 7 |
| normal | Fresh | 0.32 | 0.06 | 0.8 | 21 | 13 | 10 |
| hard | Fierce | 0.17 | 0.03 | 0.95 | 26 | 18 | 14 |
