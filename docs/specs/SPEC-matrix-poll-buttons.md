# SPEC: Matrix 原生可点菜单（MSC3381 poll 承载 inline keyboard）

- 状态：待批准（批准后进入 worktree + 计划）
- 目标平台：**仅 Matrix**（SimpleX 按用户决议排除）
- 前置确认：本改动只在 Matrix 入站开启时生效，见 §0

---

## 0. 前提确认（必须先认这一条）

`runtime.rs` 的 `secondary_handles_inbound(primary_enabled) = !primary_enabled`：

| 启动形态 | Matrix 入站 | 本 spec 是否生效 |
| --- | --- | --- |
| `--matrix` 单跑 | 开启 | **生效** |
| `--all`（TG + Matrix） | 关闭（Matrix 仅作出站落点） | **完全不生效** |
| `--tg-simplex`（不含 Matrix） | — | 不适用 |

若线上实际跑的是 `--all`，则本 spec 是死代码，需要另开 spec 处理入站门禁（属安全边界改动，需 `doubt-driven-development` 对抗覆核）。**批准本 spec 即表示确认线上 Matrix 走单跑形态。**

---

## 1. 意图与原因

Telegram 上菜单是真按钮：点一下 → `BotEvent::Callback` → 流程推进一步。Matrix 和 SimpleX 没有 inline keyboard（协议层就不存在：MSC3006 已 abandoned、MSC4139 仍是 draft、matrix-spec#1651 仍 open），当前只能把按钮降级成正文里的编号清单：

```
1. Help — send: `/help`
```

问题有三个：

1. 用户必须照抄命令，编号本身**不生效**（回 `1` 没有任何反应）；
2. 把内部 command 名（`/xray:menu:2` 这类）直接暴露在正文里；
3. 多步流程每步都要重抄一次完整命令，与 TG 的体验差一个量级。

MSC3381（Chat Polls）是 Matrix 里唯一被 Element 原生渲染成「可点选项」的机制，业界已在用它兜底（Moltis、openclaw 等把短动作列表发成原生 poll 并回灌同一个 interaction callback 路径）。本 spec 用它承载 inline keyboard，把 Matrix 的操作路径拉回「点一下」。

---

## 2. 目标 / 非目标 / 反目标

### 目标

- G1 Matrix 上带 markup 的消息，按钮渲染为**可点选项**（MSC3381 poll），而不是编号文本。
- G2 点选后走与 TG **完全相同**的 `BotEvent::Callback` 派发路径，`shared/handlers/*` 一行不改。
- G3 菜单刷新（58 处 `edit_message` 带 markup）自动获得新行为，**零调用点改动**。
- G4 无法承载的 markup（按钮过多等）自动回退到现有编号文本，不丢功能。

### 非目标

- N1 SimpleX（用户明确排除；其 bot 命令菜单还需 `simploxide-client` 上游支持）。
- N2 「回复数字选按钮」（A 方案，用户决定 B 优先，延后）。
- N3 `--all` / `--tg-simplex` 下放开 Matrix 入站门禁（安全边界改动，另开 spec）。
- N4 reaction 当按钮、poll 多选、poll 结果统计展示。
- N5 不改 `PlatformCapabilities.has_inline_keyboard`（全仓无读取点，改了只会误导）。

### 反目标

- 每次菜单刷新都往时间线里堆一个可点的旧 poll（会误触）——必须撤掉旧的。
- 让 `shared/` 层出现 `if platform == Matrix` 分支。

---

## 3. 现状证据（实现前已核对）

| 事实 | 位置 |
| --- | --- |
| `MatrixAdapter` 只持有 `Room`，无内部状态 | `gateways/matrix/adapter.rs:20-27` |
| 按钮降级为编号清单 | `common/markup.rs` `render_markup_buttons` |
| `MessageContent { text, markup: Option<Markup> }`，`Markup { buttons: Vec<Vec<InlineButton>> }` | `common/trait.rs:19-27` |
| `BotAdapter::edit_message(&self, target, msg_id, content)` | `common/trait.rs:110-115` |
| `CallbackEvent { adapter, target, user_id, msg_id, data, callback_id, session_timeout_secs }` | `shared/types.rs:14-22` |
| 派发入口 `dispatch_event`（含 auth 门禁 + handler 重定向循环） | `shared/dispatch.rs:18` |
| Matrix 入站注册点（`OriginalSyncRoomMessageEvent`，room 匹配 + admin 门禁） | `runtime.rs:185-241` |
| 纯 `--matrix` 时 `state.adapter` 是 `Arc<dyn BotAdapter>`（**不是**具体类型） | `main/matrix.rs:18` `type MatrixHandle = (MatrixClient, Room, Arc<dyn BotAdapter>)`；`main/adapter.rs:32-35` |
| `ruma-events 0.36` 的 `pub mod poll` 受 `unstable-msc3381` 控制 | `ruma-events/src/lib.rs:179-180` |
| `matrix-sdk 0.20` 未透传 `unstable-msc3381` | `matrix-sdk-0.20.0/Cargo.toml` `[features]` |
| 依赖图中已固定 `ruma 0.18.0` / `ruma-events 0.36.0` | `rust/aegis/Cargo.lock:4041,4115` |
| `Room::send` / `Room::redact` 可用 | `matrix-sdk/src/room/mod.rs:2659,3483` |
| 58 处 `edit_message` 带 markup，8 处 `send_message` 带 markup | 全仓 grep |

---

## 4. 方案取舍

| 方案 | 取舍 | 结论 |
| --- | --- | --- |
| **A. 自建事件结构 + `EventContent` derive** | 零依赖变更，但 poll 的 JSON 形状（MSC1767 message block + MSC3381 block）要手写，Element 渲染不符只有真机才发现 | 备选（`unstable-msc3381` 编译不通时的回退） |
| **B. 直接依赖 `ruma@0.18` 并打开 `unstable-msc3381`** | 类型由 ruma 保证形状正确；只多一行依赖（同一份 ruma，不新增 crate 版本）；风险是 matrix-sdk 0.20 未验证过这个组合 | **采用** |
| C. `Room::send_raw("org.matrix.msc3381.poll.start", json!{})` | 零依赖，但响应侧要自己反序列化原始事件，matrix-sdk 0.20 无 `add_event_handler_raw` | 否 |
| D. reaction 当按钮（MSC2677） | 两平台都支持，但 emoji↔选项语义弱，选项数 >10 即崩 | 否（用户已选 B） |

**采用 B**，理由：平台侧本来就存在该机制，唯一成本是一行依赖；形状正确性交给 ruma 而不是手写 JSON。

---

## 5. 设计

### 5.1 依赖

```toml
ruma = { version = "0.18", default-features = false, features = ["unstable-msc3381", "events"] }
```

- 必须锁 `0.18`（与 `matrix-sdk 0.20` 要求一致），否则出现两份 ruma。
- 用 `cargo add` 添加，禁止手改 toml（AGENTS.md §3）。
- **Phase 0 硬门禁**：加完依赖后 `cargo build` 必须通过、且 `matrix_sdk::ruma::events::poll::unstable_start::UnstablePollStartEventContent` 可引用。不通过则整体回退到方案 A，功能目标不变。

### 5.2 `MatrixAdapter` 内部状态

```rust
struct PollBinding {
    anchor: String,          // 承载正文的 m.room.message event id
    buttons: Vec<InlineButton>, // 按 row-major 顺序拍平
    created: Instant,
    answered: bool,          // 消费一次即置位，防重复派发
}

pub struct MatrixAdapter {
    room: Room,
    polls: Mutex<HashMap<String /* poll event id */, PollBinding>>,
}
```

- 容量上限 `MAX_POLL_BINDINGS = 64`，超出按 `created` 淘汰最旧（不引入 TTL 配置，避免动 `AppState`）。
- 单次查找/淘汰复杂度对 64 条以内无所谓，不做索引。
- `MatrixAdapter::new(room)` 签名不变，测试可直接构造。

### 5.3 发送路径（`send_message` / `edit_message`）

一个纯函数承担判定，IO 留在方法里（便于单测）：

```rust
/// 返回 (正文文本, 可选 poll 规格)
fn plan_send(content: &MessageContent) -> (String, Option<PollSpec>)

struct PollSpec { question: String, answers: Vec<(String /*id*/, String /*label*/)> }
```

规则：

1. `markup` 为空 → 原行为（正文原样，无 poll）。
2. `markup` 非空且 `1 <= 按钮总数 <= MAX_POLL_ANSWERS(20)` → 正文 = `content.text` **不拼编号清单**；`PollSpec` 的 answers 按 row-major 拍平，`id = "0","1",...`，`text = 按钮 label`（超过 64 字符截断，空 label 用 `data` 兜底）。
3. 超过 20 个按钮 → 回退现有 `render_markup_buttons`（编号清单），不发 poll。理由：poll 选项过多在 Element 里不可用。
4. `is_sensitive(&text)` 命中时，正文走现有「改成 `.txt` 附件」分支不变；poll 照发。

`send_message`：发正文 → 若存在 `PollSpec`，`room.send(poll_content)` → 记录 binding（key = poll event id，anchor = 正文 event id）。

`edit_message`：若存在 `PollSpec` →
1. 编辑 anchor 正文（不带编号清单）；
2. 按 anchor 找到旧 binding → `room.redact(old_poll_id, Some("superseded"), None)` → 从表中移除；
3. 发新 poll 并登记新 binding。
无 `PollSpec` 时完全走原逻辑。

`send_message_threaded` 带 markup：暂不承载（poll 有 thread relation 未建模）→ 回退编号清单。记为已知限制。

`delete_message`：删掉 anchor 时顺带移除其 binding（不 redact，消息本身已删）。

### 5.4 入站路径（`runtime.rs`）

**前提修正（T4 后发现）**：`runtime.rs` 拿到的 adapter 是 `Arc<dyn BotAdapter>`（`type MatrixHandle = (MatrixClient, Room, Arc<dyn BotAdapter>)`，`main/matrix.rs:18`），**不能**直接调 `MatrixAdapter::polls()`。因此在 `BotAdapter` trait 上加一个带默认 `None` 的平台专属方法（与既有 `accept_contact_request` / `contact_security_code` 同一风格）：

```rust
/// Matrix：取一次 poll 点选结果并标记已消费。默认不支持。
async fn take_poll_answer(&self, _poll_id: &str, _index: usize) -> Option<(MessageId, String)> {
    None
}
```

`MatrixAdapter` 实现它，内部转调 `self.polls.take_unanswered(poll_id, index)`。`shared/` 层不做任何平台判断。

在现有 Matrix 段（`secondary_handles_inbound` 内）注册一个 `UnstablePollResponseEvent` handler：

门禁与现有文本 handler 完全一致：room id 必须等于目标、`state.is_admin_user(user_id)`、否则静默丢弃并留一条 `log::debug!`。

处理：

1. 从响应里取 `selected_answer_ids`（MSC3381 的 `org.matrix.msc3381.poll.response`）；取第一个 id → `usize` 索引。
2. 以 poll event id 查 binding：查不到（被淘汰/非本 bot）→ 丢弃。
3. `answered == true` → 丢弃（幂等，用户改票不重复推进流程）。
4. 置 `answered = true`，按下标取 `buttons[idx].data`。
5. 合成事件并派发：

```rust
BotEvent::Callback(CallbackEvent {
    adapter: adapter.clone(),
    target: target.clone(),
    user_id: user_id.to_string(),
    msg_id: MessageId(binding.anchor.clone()), // 关键：指向正文消息，edit_message 才能原地改
    data: button.data.clone(),
    callback_id: format!("poll:{poll_id}"),    // Matrix 的 answer_callback 是 no-op
    session_timeout_secs: state.session_timeout_secs().await,
})
```

6. `dispatch_event(event, &state).await` —— 与 TG 回调走同一入口（auth 门禁、handler 重定向循环全部复用）。

### 5.5 与现有降级的关系

- `render_markup_buttons` 保留（超限回退 + SimpleX 仍在用）。
- `has_inline_keyboard` 保持 `false`，不改。

---

## 6. 已知限制（写进代码注释与 README）

1. 组合形态（`--all`）下不生效（§0）。
2. 每步菜单刷新会「改正文 + 撤旧 poll + 发新 poll」，时间线里 poll 会往下跳一次；这是 Matrix 没有内联键盘的固有代价。
3. 用户在 Element 里改票不会二次推进流程（`answered` 只消费一次）。
4. 线程内（thread）的 markup 仍回退编号清单。
5. 仅 Element 等实现了 MSC3381 的客户端可点；不支持 poll 的客户端会看到「问题 + 选项」文本，比现状仍不差。
6. 进程重启后房间里的旧 poll 会失效（映射表是内存态），需重新发送 `/menu`。

---

## 7. 测试计划（TDD，红→绿）

### 单元测试（不碰网络）

| 用例 | 断言 |
| --- | --- |
| `plan_send` 无 markup | 正文原样、无 poll |
| `plan_send` 1 个按钮 | 正文不含编号清单、poll 有 1 个答案、id 顺序正确 |
| `plan_send` 20 个按钮 | 有 poll，答案 20 个 |
| `plan_send` 21 个按钮 | 无 poll，正文含编号清单（回退） |
| `plan_send` 空 label / 超长 label | 用 data 兜底 / 截断到 64 字符 |
| `plan_send` row-major 拍平 | 两行两列 → 顺序 `[0,0],[0,1],[1,0],[1,1]` |
| binding 表 | 插入/按 anchor 查/按 poll id 查/`answered` 只消费一次/超 64 淘汰最旧 |
| 响应索引解析 | `"1"` → 1；非数字/超界/空 selections → `None` |

### 改造既有测试（会先红）

`gateways/matrix/adapter.rs` 里 `send_message_with_markup_appends_command_list` 当前断言正文含 `1. Search` —— 改为断言「正文不含编号清单」+「产生 poll 规格」。这是本 spec 的 RED 起点。

### 集成/真机验收（人工）

1. `--matrix` 单跑，在 Element 里 `/menu` → 出现可点选项。
2. 点第一项 → 流程推进一步，旧 poll 消失，新 poll 出现。
3. 点旧 poll（已撤）→ 无反应。
4. 连续点两下同一项 → 只推进一步。
5. 非管理员账号点选 → 无反应且日志有 debug 记录。

### 门禁

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo +nightly nextest run --cargo-profile fast-test`
- Phase 0 的依赖可行性构建

---

## 8. 发布与回滚

- 无配置项、无 DB/schema 变更、无 i18n 破坏性变更（新增 1 个 poll 问题文案 key）。
- 回滚 = revert 单个分支；无存量数据需要迁移。
- 影响面：`rust/aegis/src/gateways/matrix/{adapter.rs, poll.rs(新)}`、`rust/aegis/src/main/runtime.rs`、`rust/aegis/Cargo.toml`、`Cargo.lock`、`README.md`（平台说明补一行）、`locales`。

---

## 9. 留给实现者的判断

- poll 问题文案的具体措辞与 i18n key 命名。
- `PollSpec` 与 `PollBinding` 的确切字段命名。
- 是否把 `plan_send` 拆成独立 `poll.rs` 模块（倾向是，`adapter.rs` 已有 281 行含测试）。
- `MAX_POLL_ANSWERS` 取 20 还是更小。
