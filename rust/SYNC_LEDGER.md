# 上游同步台账（SYNC LEDGER）

Rust 代码无法用 `git merge` 合并上游 JS 改动。工作流：

```bash
rust/tools/sync-upstream.sh check              # 列出基线之后的上游新提交
# 对照 rust/PORT_MAP.md 逐项移植到 Rust，并在下表登记
rust/tools/sync-upstream.sh update-baseline    # 移植完成后推进基线
```

- **上游仓库**：https://github.com/jaydendavisnc/inkwave（remote 名 `upstream`，只读 HTTPS）
- **当前基线**：`3e9b5505ea900cb41851b4d75f16d488552b1794`（2026-09-29，上游 main，等同 M1 开工时 fork HEAD；存储于 `.upstream-baseline`）
- **状态图例**：`pending`（未移植）· `porting`（移植中）· `done`（已移植，含证据）· `wontport`（Rust 侧不适用，注明原因）

## 逐提交移植记录

| Upstream commit | 日期 | 摘要 | 受影响 JS 文件 | Rust 对应 | 状态 | 证据/备注 |
|---|---|---|---|---|---|---|
| _基线之后暂无新提交_ | | | | | | |

## M1 初始移植说明

M1 为**基线快照重写**（Tidewater + Spritzer + 涂地 + easy bot 的垂直切片），不是逐提交移植；
基线之前的所有 JS 内容整体视为“待移植参考”，其模块映射见 [PORT_MAP.md](./PORT_MAP.md)。
上游此后的每个提交才在上表逐行登记。

### M1 基线核对（Task 16 收尾）

`tools/sync-upstream.sh check` 于 2026-10-05 复核：本地 `upstream/main` = 基线
`3e9b5505ea900cb41851b4d75f16d488552b1794`，**基线之后零新提交**，无需逐行登记。
M1 已移植范围见 PORT_MAP 的 `M1` 行；未移植项全部标 `M2+`/`skip`。

### 同步分类原则（移植 triage）

| 上游改动类型 | 移植方式 |
|---|---|
| 调参/常量（`src/config.js`）、关卡布局数据（`stages/*/layout.js`） | 优先重跑 `rust/tools/extract/` 提取管线，生成物更新后跑测试；一般零手工代码 |
| 玩法逻辑（actor/physics/weapons/match/bots） | 对照 PORT_MAP 手工移植到 `inkwave_sim`，同步补确定性测试 |
| 渲染/着色器/角色表现 | 手工移植到 Bevy 侧；M1 简化项可标记 `wontport` 并注明 M2+ 再做 |
| `server/` relay | 协议保持兼容；除非协议变更否则不动 Rust，协议变更需同步更新 `inkwave_sim::net` 文档与类型 |
| `tools/`、`electron/`、构建脚本 | 一般 `wontport`（Rust 有等价工具链：`--headless`、trunk、cargo） |
