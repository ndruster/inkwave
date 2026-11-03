# M3 复核清单（实施期逐任务登记；收尾时并入总评审）

> 格式沿用 M2。"已证" 必须附落盘日志/测试输出路径；沙箱不可达项标注"待用户机器复核"。

## 待登记条目（规划期预置）
1. **跨端真机开黑**（AC-M3-4 / TR-39.2）：JS 客户端 ↔ Rust 客户端（浏览器 + 原生）同房间 turf/zones 各一局——沙箱以 headless 实跑为基线；真人双端开黑待用户机器复核。
2. **新图视觉复核**（Task 37）：saltpan/crossmarket 体块档在 GPU 环境的可玩性（出生点/遮挡/光照简化档）。
3. **mitts leap/cling 手感**（Task 33）：冲量/贴墙/落地溅射的本地手感复核（数值以测试断言为准）。
4. **特殊武器呈现占位档**（Task 35 / OQ-3）：19 特殊的 ghost 轮廓可辨识度复核。
5. **zone 视觉标记**（Task 36）：zone 边界/中心标记体块档 + HUD 计数复核。
6. **bot 档位体感**（Task 38）：normal/hard 对战强度单调性人工试玩。

## 逐任务复核记录

### Task 39（2026-11-03）——跨端完整局待用户机器复核（沙箱降级，D-T39-1）
- **已证（沙箱落盘）**：loadout knobs 端到端解析（`[nettest] boot … weapon:Some("brolly") …`，dump `rust/tools/verify/logs/net-smoke-2026-11-03T05-58-41-451Z.json`）；JS host 建房成功（PZMLU/UUH26/6KENC）；roster 全链路进程内闭环（harness `tr39_loadout_end_to_end`，guest 端 sub/special 部署断言）；九段门禁 `task39-gates-20261103T073400Z.log` 全绿（553 passed）。
- **待用户机器复核**：① `node rust/tools/verify/net-smoke.mjs --scenario C`（turf 完整局：断言双端 roster 互认、终局 coverage 差<0.5%、winner 一致、Rust probe sub≥1 sp≥1、无 err）；② `--scenario D`（zones：zone 终局 counts/penalty/reason/overtime 逐项一致，300s+加时长预算）；③ scenario A 在真实 GPU 环境应恢复 PASS（沙箱因 SwiftShader PBR 崩溃 + 内存压力退化为 DEGRADED，非代码回归——Task 26 曾 PASS 同栈）。
- **复核口径**：任一 FAIL 先查 `[nettest] loadout`/`result` 行与 retry 行（1a/1b 各 ≤3 次 × 5s），再对照 spec.md D-T39-4/5 字面契约。

### Task 40（2026-11-03）——M3 收尾登记
- **规划期预置条目状态**：#2 新图视觉、#3 mitts leap/cling 手感、#4 特殊呈现占位档、#5 zone 视觉标记、#6 bot 档位体感——均为 GPU/真人环境复核项，沙箱不可达，**维持待用户机器复核**（数值/行为对等已由对应任务无头测试证明，见各 Completion Evidence）。
- **沙箱内存压力注记**：M3 收尾窗口 `wasm-bindgen` 需 ~3.9GB 而沙箱 avail ~4.0GB，trunk release 构建间歇性被 OOM SIGKILL（dmesg 实证，Task 39/40 窗口多次）；用户机器无此约束，trunk/net-smoke 应一次通过。
- **总评审结论**：M3（Task 28-39）独立总评审 APPROVE（0 Blocker/0 Major/3 P2，全部当场闭环或记录，见 tasks.md Task 40 Review 附录）。
