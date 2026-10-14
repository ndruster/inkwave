# Task 24 实施计划：极简大厅 UI（`inkwave::ui::online`）

## Context

M2 里程碑按 tasks.md 逐任务推进；Task 17–23 已闭环（wire codec / Transport / 协议消息 / 会话层 / replication / playback / 事件时间线）。Task 24 是首个把 `inkwave_sim::net::session::NetSession` 接入 Bevy app 的任务：交付极简联网大厅（bevy_ui，无 showcase 场景），并落地 "starting→载入对局、结算→回大厅" 的 app 侧粘合，满足 AC-M2-6（create→join→切队/ready→host start→对局→结算→回大厅闭环）。

硬约束（project_memory.md）：逻辑逐字对齐 JS（menus.js `_scr_online`/`_scr_lobby`、session.js、main.js `startNetMatch`/`netMatchEnd`/`netMatchAborted`），注释写 JS 行号锚点；测试基线只增（当前 243 passed）；JS 树（src/server/tools/docs）零改动；sim 依赖面不变（glam/serde/serde_json）；全门禁含 wasm32（口径 `-p inkwave -p inkwave_sim`）与 trunk build。

## 范围决策（如实记录）

- **完整 app 客户端**：大厅 UI + 会话事件流 + 网络对局接线全部落地（Task 26 浏览器冒烟依赖 app 能打联网局）。
- **TR-24.2 双窗口人工试玩**：沙箱无 GPU/窗口，无法真跑。等价验证 = inkwave app 内 FakeRelay 双 `NetSession` 全流程集成测试（走与 UI 相同的 `project`/`run_cmd` 纯映射 + `build_net_match`）；真实双窗口试玩（create/join/ready/start/回大厅/断线提示 6 步）作为"待用户机器复核"条目登记进 tasks.md Task 24 段（REVIEW_CHECKLIST 文件按既定安排由 Task 26 建立，本任务不新建文档文件）。
- **偏差声明**（写进模块 docs，JS 锚点注释）：① Actor 无名字/武器字段，roster name/weapon 仅 UI 显示；② JS `_warmCharacters` 着色器预热无对应物；③ JS 建局停 `init`、go 才 intro，Rust 构造即 intro，guest 靠 host `{k:'st'}` 重入对齐；④ bot 脑一律 `tuning.difficulty.easy`（Tuning 仅 easy 档）；⑤ 局 seed = fnv1a(cfg.id)（JS 用 Math.random，Rust 确定性）；⑥ 名输入框省略（Profile 固定 'Player'，JS 空名回退语义一致）；⑦ leave 确认用两段式 banner（JS 为三文案模态框）；⑧ 手柄码输入模式（`st.input === 'pad'`，menus.js L2516 ghost 字母 + L2526 `↑↓ pick a letter · A to confirm` 提示）省略：M2 Rust hub 无手柄焦点路径，仅投影 L2526 键盘分支提示。
- **补记（实施中发现，计划文档初稿未载）**：JS `type()` L2554 第 5 字符落末格时武装 320 ms `st.autoT` 自动加入定时器（回调守卫 `st.alive && st.mode === 'entry' && full()` 再 `doJoin`）；Rust 以 `UiState.auto_join` 每帧倒计时逐字移植（含 JS quirk：无效字符与 Cancel 均不清定时器，backspace/Enter/JOIN 按钮/离开 hub 清零）。

## 实施步骤

### Step 1 — 会话资源（`crates/inkwave/src/net/mod.rs` + `main.rs`）
- `AppTransport` 类型别名（native=`NativeTransport` / wasm=`WasmTransport`）+ `default_relay_base()`（native：`NativeTransport::resolve_relay_base(cli)`；wasm：`relay_url_from_location()`，两函数已存在）。
- `#[derive(Resource)] OnlineSession { sess: NetSession<AppTransport, Box<dyn FnMut()->AppTransport>>, busy: bool }`；工厂闭包捕获 relay base `String`；`Profile::default()`。启动不 connect。
- `main.rs` `parse_args` 增 `--relay=` → `RelayBase` 资源（仅非 headless 分支 insert）。

### Step 2 — `ui.rs` 平移为 `ui/mod.rs`（git mv，内容零改动），新建 `ui/online.rs` 纯映射层（TR-24.1）
无 Bevy 依赖的纯函数 + 单测先行：
- `SessionCmd{Create,Join(String),Leave,Cancel,ToggleReady,SetTeam(TeamSel),SetDuration(f64),SetBots(bool),SetDifficulty(String),Start}`；`run_cmd(&mut NetSession<T,F>, &mut OnlineFlags, cmd)` 泛型化（FakeTransport 可测）。
- `OnlineFacts{state,busy,is_host,code,lobby,can_start,start_block,error,code_draft,leave_confirm}` → `project(&OnlineFacts) -> OnlineView{hub/create/join/settings/start 各 enabled、ready_label(host 在 Lobby→"START!" 语义 menus.js L3048-3049)、rows:Vec<RosterRow{name,team,ready,host,ping,you}>、banner、screen:Hub|Lobby|MatchLoading|None}`。
- `error_copy`（ERRMAP menus.js L49-54）、`code_char_feedback`（L2544 四分支文案）、`code_key_to_char`（A-Z/0-9/Backspace 手工映射，无 IME）、`list_names`（L3073 `&`/`+N`）、`waiting_banner`（L3066 startBlock ?? 'Waiting for … to ready up' ?? 'Not ready to start yet'）。

### Step 3 — 屏幕树（`ui/mod.rs`）
- `Screen` 增 `OnlineHub`/`OnlineLobby`；`UiState` 增 `net_mode`/`leave_confirm`；`Action` 增 `Online(OnlineAction)` 与 `NetBackToLobby`。
- 主菜单加 "ONLINE" 按钮 → OnlineHub。`build_ui` 追加两棵树：hub（CREATE ROOM / 5 格码显示 / JOIN / BACK / banner）、lobby（房码大字、8 行名册 LabelRole 定槽、TEAM ALPHA/AUTO/BRAVO、READY?|START!、host 设置行 LENGTH 90/180 + SKILL 循环 + BOTS 开关、LEAVE、banner）。沿用等值写模式避免每帧脏布局。
- `ui_state_machine`：Online* 屏守卫（Enter 不触发离线 Play；Esc 两段式 leave 确认；R=ready；码输入逐键）；`apply_action` net_mode 下拒绝 Play/Restart、Results 按钮语义切换（NetBackToLobby / Menu=leave）。

### Step 4 — 对局接线（新文件 `net/match_link.rs` + 小改 `ink_render.rs`/`actors.rs`/`world.rs`）
- `#[derive(Resource)] NetMatchState { rep: Option<Replication>, local_idx, cfg: Option<StartCfg>, result_sent, end_sent, go_pending }`（与 DemoSim/OnlineSession 三资源分离，规避 borrow 冲突）。
- `build_net_match(cfg,seed,sim,t,mc) -> Match`：逐 roster entry `Actor::new(team,slot)` + `SPAWN_RING_*` 环公式（match_.rs L74-78 pub）+ `spawn_at` + runner push，roster 序 = 平行数组契约；不动 sim。
- `net_tick` 系统（Update 链，`ui_state_machine` 后、`ink_upload` 前）：
  1. `session.update(dt)`（dt=delta_secs().min(0.1)）→ `poll_events()` 一次取完；
  2. 事件分派：`Match{Start}`→build+bind+attach_bot(host bot 位)+prune_loading_leavers(noBots)+替换 `demo.m`+`mark_loaded`；`Match{Go}`→解锁；`Match{End}`→清 rep 回 Lobby 屏；`Aborted`/`Error`→回 hub+banner（JS `_roomGone`/`netMatchAborted` L844）；`Host`/`Leave`→`set_host` 后 `on_leave`（契约 replication.rs L200-202）；`GameMessage`→`on_message`，`Some(MatchEnd)`→`send_end`+`end_match`；
  3. rep 在场时每 FIXED_DT 子步：`drive_bots`→本地输入写 `inputs[local_idx]`（`ink_render.rs` slot0 硬编码改 `demo.player_slot`，离线默认 0）→`step_masked(Some(remote_mask))`→`rep.update(...,session.transport_mut())`→rest 事件回灌 `m.events`（HUD/audio/feed 零改动复用）；
  4. host 局终：`send_result` 一次；Results 停留→`send_end`+`end_match`→guest 收 `{k:'end'}` 后 `end_match`（R5：guest 按钮仅本地过渡显示）。
- `sync_actor_count` 系统（`sync_actors` 前）：actor 数变化时 despawn/补齐实体树（`spawn_actors` 主体抽 `build_actor_tree`）。

### Step 5 — 测试
- TR-24.1 单测（`ui/online.rs` tests + `net/match_link.rs` tests）：`tr24_project_hub_gating`、`tr24_project_start_enable_needs_all_ready`、`tr24_start_block_banner`、`tr24_error_copy_matches_errmap`、`tr24_code_char_feedback`、`tr24_list_names_and_waiting_banner`、`tr24_run_cmd_maps_to_session`（FakeTransport 真 session）、`tr24_leave_confirm_two_step`、`tr24_net_mode_results_buttons`、`tr24_build_net_match_roster_order`。
- TR-24.2 等价集成测试：`tr24_lobby_flow_two_clients`（FakeRelay 双 session 全流程：create→码→join→名册→切队/ready→start_enabled→Match{Start}→build 对齐→Go→N 帧 rep.update+step_masked→host 终局 res→guest result 一致→end_match→双端 Lobby）、`tr24_match_end_returns_lobby`。范式抄 session.rs L1951-2017（Box<dyn FnMut> 工厂 + 双端时钟助手）。

### Step 6 — 门禁与收尾（固定流程）
1. 全门禁：`cargo fmt --all --check`、`clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`（≥243）、`cargo check -p inkwave -p inkwave_sim --target wasm32-unknown-unknown`、`trunk build --release`（cwd=rust）、headless 冒烟（`--headless --frames 120`）、`cargo tree -p inkwave_sim --depth 1`、JS 树 `git status` 零改动。
2. 独立审查 agent（逐行对照 menus.js/session.js/main.js 语义）→ 修复 → 新起复审 agent 读盘核对 → 重跑门禁。
3. tasks.md Task 24 段写 Completion Evidence + Review 附录 + TR-24.2 人工复核登记，Status=completed。
4. git 提交（`-c user.name='ndruster' -c user.email='ndruster@users.noreply.github.com'`，标题 "M2 Task 24: …"）→ `git show HEAD` 复核落盘 → memory topics.md 追加条目。

## 关键文件

- 改：`rust/crates/inkwave/src/main.rs`、`net/mod.rs`、`ui.rs`→`ui/mod.rs`、`ink_render.rs`、`actors.rs`、`world.rs`
- 新：`rust/crates/inkwave/src/ui/online.rs`、`rust/crates/inkwave/src/net/match_link.rs`
- 只读契约：`inkwave_sim/src/net/{session,replication,transport}.rs`、`match_.rs`；JS 基准 `src/ui/menus.js`、`src/net/session.js`、`src/main.js`

## 验证

- `cargo test --workspace` 全绿（基线 243 + 新增 tr24_*）；`tr24_lobby_flow_two_clients` 即大厅闭环实证。
- 全门禁清单（Step 6 第 1 条）逐项通过；headless 冒烟行为不变（无 net 局快路径）。
- 双窗口真实试玩 → tasks.md 登记"待用户机器复核"。

## 风险

- R1 borrow：三资源分离 + poll_events 一次取完再借 transport_mut。
- R2 sim 零改动：app 侧 builder 用已 pub 常量/方法。
- R3 ui.rs 平移：git mv 纯移动，42 个既有测试不破坏。
- R6 wasm：cfg 分叉只允许在 net/mod.rs 与 main.rs；ui/online.rs、match_link.rs 保持 target 无关。
