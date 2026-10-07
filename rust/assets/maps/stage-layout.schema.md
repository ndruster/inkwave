# tidewater.json — 关卡几何模式（schema `inkwave.stage_layout.v1`）

由 `rust/tools/extract/extract-layout.mjs` 从上游
`src/world/stages/tidewater/layout.js`（含 mapkit.js 的 B/R/O/OCT/ARC 与本关
arcBand/arcSub/chainBand/chainSub/chainSections 求值结果）提取。
**不要手改**；上游同步后重跑 `rust/tools/extract/run-all.sh`。

提取器复刻 `src/world/level.js` 的 `mirrorDef`（绕 Y 轴 180°：(x,z)→(-x,-z)），
图元顺序 = `[...single, ...half, ...half镜像]`，与 JS `Level._build` 中 block id 顺序一致。
坐标单位米，Y-up；Alpha -Z、Bravo +Z。

## 顶层

| 字段 | 含义 |
| --- | --- |
| `schema` | `inkwave.stage_layout.v1` |
| `source` | commit / layout 文件路径 |
| `stage` | `id`、`bounds {minX,maxX,minZ,maxZ}`、`spawnPads`（每队一个 `[x,y,z]`）、`spawnBarrier`（出生屏障半径 4.2 m）、`waterY`（海面高度，取自 PLAYER.waterY=-1.6） |
| `primitives` | 全部 block 原始描述（box/obox/ramp），顺序即 block id |
| `meta` | node 侧统计：`primitiveCounts {box,obox,ramp,total}`、`sourceCounts {single,half,halfMirrored}`、bounds/spawnPads/spawnBarrier/waterY 回显、`surfaceSlots` |

Rust 解析器（`inkwave_sim::geometry`）以 `meta` 对解析结果做交叉校验（TR-3.3）。

## 图元共有字段（flag 语义对齐 level.js `_addBlock`）

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `tag` | string\|null | 玩法/渲染分组标签（`square`、`terrace-rail` 等） |
| `color` | hex string | 图元本色，缺省 `#dddddd` |
| `pattern` | u16 | PATTERN 纹理槽位 id；0=plain；28/29/30 为本关预留槽 herringbone/terrazzo/stucco |
| `paint` | bool | 面可否上墨（`paint!==false && !grate && !rail`；roof/perch 顶面在 Face 阶段再排除） |
| `solid` | bool | 是否参与碰撞（缺省 true） |
| `grate` | bool | 格栅/栏杆：kid 可行走，鱿鱼/弹丸/墨汁穿过，不可上墨（`!!d.grate \|\| !!d.rail`） |
| `rail` | bool | 栏杆：grate 的纯碰撞子类（可见栏杆是 prop），hidden=true |
| `roof` | bool | 禁区顶（塔楼等）：落上去会滑下，顶面不上墨 |
| `perch` | bool | 可站立但不上墨的顶面 |
| `noNav` | bool | bot 寻路不走其顶面 |
| `hidden` | bool | 纯碰撞体（prop 碰撞/栏杆），不生成面 |
| `bevel` | number\|null | 渲染倒角覆盖（碰撞不使用） |
| `noPaint` | `[nx,ny,nz][]` | 不上墨的世界空间法线列表（随镜像翻转 x,z） |
| `mural` | `{id,n:[nx,ny,nz]}[]` | 墙面装饰 id + 贴合法线（随镜像翻转 x,z） |
| `oct` | `[cx,cz,R]\|null` | 八角台来源标记（缩略图/统计用） |

## box

`{kind:'box', min:[x0,y0,z0], max:[x1,y1,z1], ...共有}` — 轴对齐盒。

## obox

`{kind:'obox', center:[x,y,z], size:[w,h,d], rotY, ...共有}` — 绕 Y 轴旋转盒。
`rotY` 为度；Rust 碰撞局部基向量（复刻 level.js）：
`axis0=(cos,0,-sin)`、`axis1=(0,1,0)`、`axis2=(sin,0,cos)`。

## ramp

`{kind:'ramp', low:[x,y,z], high:[x,y,z], width, thickness, thin, ...共有}` —
倾斜厚板，顶面从 low 边中到 high 边中。`thin=true` 时保持给定 `thickness`（薄板/跳板），
否则厚度=`max(thickness, rise*cosT+0.35)` 并向低端延伸 0.6 m 到地面（Task 4 复刻 level.js
`_addBlock` 的 ramp 分支：center/half/axes=(side,n,s)）。

## meta.surfaceSlots

Tidewater 保留纹理槽：`herringbone=28`（方形铺砖）、`terrazzo=29`（散步道水磨石）、
`stucco=30`（建筑灰泥），定义于 `src/world/stages/tidewater/surfaces.js`。
