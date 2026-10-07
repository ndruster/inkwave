# tuning.json — 字段模式（schema `inkwave.tuning.v1`）

由 `rust/tools/extract/extract-tuning.mjs` 从上游 `src/config.js` 提取，**不要手改**。
数值单位与 JS 一致：距离=米，时间=秒，速度=m/s，加速度=m/s²，角度=弧度（武器散布例外，为度），
HP/墨量为点（PLAYER.hp=100，PLAYER.inkMax=100）。坐标系 Y-up；Alpha 出生于 -Z，Bravo 出生于 +Z。
确定性输出：同一上游 commit 重跑逐字节一致（TR-3.4）。

## 顶层

| 字段 | 含义 |
| --- | --- |
| `schema` | 模式标识，Rust 侧解析前校验前缀 |
| `source.commit` / `source.path` | 数据出处（git commit 与 JS 文件），用于上游同步审查 |
| `player` | `PLAYER`：角色物理/手感全部常量，见下 |
| `spritzer` | `WEAPONS.shooter`：M1 唯一武器 Spritzer |
| `match` | `MATCH`：涂地比赛参数 |
| `teamPalettes` | `TEAM_PALETTES`：5 套队伍墨色（hex），a=Alpha，b=Bravo |
| `difficulty.easy` | `DIFFICULTY.easy`：M1 bot 参数 |

## player（src/config.js `PLAYER`）

字段名与 JS 完全一致（camelCase）。关键值：`hp=100, runSpeed=6.0, swimSpeed=11.8,
respawnTime=5.5`。分组：

- 生存：`hp` `respawnTime` `spawnInvuln` `fallDeathY` `waterY`
  （`fallDeathY=-1.45` 触水即死；海面 `waterY=-1.6`）；`enemyInkDps=20`、
  `enemyInkDamageCap=40`；`regenDelay/regenRate/regenRateSwim`
- 移动：`runSpeed` `swimSpeed` `squidDrySpeed` `enemyInkSpeed` `climbSpeed`
  `accelGround/accelAir/accelSwim`，及 `run*/swim*/squid*/air*/enemyInk*` 一组手感曲线常量
  （S 曲线加速、刹车、转向、蹬墙等，Task 5 角色控制器逐项消费，注释见 JS 源文件）
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

## spritzer（src/config.js `WEAPONS.shooter`）

`id/name/kind/class/blurb` 为展示字符串；`stats` 是 0..1 的配装界面条（非模拟数值）。
模拟字段（TR-3.2 锁定）：

| 字段 | 值 | 含义 |
| --- | --- | --- |
| `fireInterval` | 0.1 s | 射击间隔 |
| `damage` | 36 | 每发命中伤害（3 发击杀 100 HP） |
| `inkPerShot` | 0.95 | 每发墨耗 |
| `projSpeed` | 34 m/s | 弹丸初速 |
| `straightTime` | 0.13 s | 直线飞行时长，之后受重力坠落 |
| `range` | 12.5 m | 有效射程（bot/瞄准辅助） |
| `spreadGround` / `spreadAir` | 5.5 / 11 度 | 地面/空中散布 |
| `impactRadius` | 0.85 m | 命中泼墨半径 |
| `trailRadius` / `trailEvery` | 0.44 m / 1.05 m | 飞行拖尾半径/间距 |
| `moveSpeedFiring` | 4.6 m/s | 开火时移动速度 |
| `special/specialCost/sub` | zooka/190/bomb | M1 不实现，仅保留出处 |

弹丸重力使用 `player.gravity=25`（JS 端由调用方传入）。

## match（src/config.js `MATCH`，涂地模式）

`durations=[90,180]` 秒可选；`defaultDuration=maxDuration=180`；`finalCountdown=10`；
`teamSize=4`；`pointsPerM2=1.0`（每平方米新涂地=1 分）；
`deathMarkLife=5`、`deathMarkFade=1.3`。

## teamPalettes

`{id, a, b, names:[Alpha名,Bravo名]}`；颜色为 sRGB hex 字符串，渲染侧按原值使用。

## difficulty.easy（src/config.js `DIFFICULTY.easy`）

`reaction=0.55` s 反应时间；`aimError=0.11` 瞄准误差；`fireDiscipline=0.55`；
`awareness=16` m 感知半径；`aimOmega=9` rad/s 瞄准弹簧刚度；`aimTurn=7` rad/s 转向速率上限。
