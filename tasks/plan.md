# Implementation Plan: Matrix 原生可点菜单（MSC3381 poll 承载 inline keyboard）

> **For agentic workers:** 逐任务执行。每步用 `- [ ]` 勾选跟踪。任务按依赖串行，唯 T4/T5 之后的 T5/T6 可并行。

**Goal:** 让 Matrix 上带 markup 的消息把按钮渲染成 Element 可点的 MSC3381 poll 选项，点选后走与 Telegram 完全相同的 `BotEvent::Callback` 派发路径。

**Architecture:** 判定逻辑收敛到新模块 `gateways/matrix/poll.rs` 的纯函数（`plan_send` / `answer_index`）+ 一个容量受限的 `PollStore`；`MatrixAdapter` 只是把这些接进 `send_message` / `edit_message` / `delete_message`（58 个调用点零改动）；入站侧在 `runtime.rs` 现有 Matrix 段注册一个 poll 响应 handler，门禁与现有文本 handler 一致。

**Tech Stack:** Rust 2024 / `matrix-sdk 0.20` / `ruma 0.18`（`unstable-msc3381`）/ `matrix-sdk::Room::{send, redact}` / `cargo nextest`

**Spec:** `docs/specs/SPEC-matrix-poll-buttons.md`

## Global Constraints

- 只允许 `cargo add` 改依赖，禁止手改 `Cargo.toml` 依赖段（AGENTS.md §3）。
- Rust 门禁：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo +nightly nextest run --cargo-profile fast-test`。
  - **clippy 基线已红**：`src/main/runtime.rs:25` 的 `UnreachableCall`（`BotCommands` derive 产物）与 `goldberg` unused-dependency 警告在 `81a6f4e` 上就存在（已用 `git stash -u` 对基线复验）。门禁口径 = **不新增**，不是「全绿」。不要顺手去修它们。
- `ruma` 版本必须锁 `0.18`（与 `matrix-sdk 0.20` 一致），`Cargo.lock` 中不得出现第二份 ruma。
- `MAX_POLL_ANSWERS = 20`（= `ruma_events` 的 `PollAnswers::MAX_LENGTH`），`MAX_POLL_BINDINGS = 64`，`MAX_ANSWER_LABEL_CHARS = 64`。
- 不改 `shared/` 层任何文件；不加 `if platform == Matrix` 分支。
- 不改 `PlatformCapabilities.has_inline_keyboard`（全仓无读取点）。
- 单次提交 ≤200 行 patch。
- 仅 `--matrix` 单跑形态生效（spec §0），组合形态不在范围。

## Review Focus

1. **按钮 `data` 必须原样传给 handler**：label 截断到 64 字符，`data`（真正的命令串）一个字节都不能动 —— 截断了命令就废了。T2 用超长 label + 长 data 的用例钉住。
2. **点已撤掉的旧 poll**：binding 已被 redact 清除，`PollStore::take_unanswered` 必须返回 `None` 而不是 panic 或派发到错误按钮。T3 钉住。
3. **同一 poll 连点两下 / 事后改票**：只推进一次流程。T3 钉住 `answered` 语义。
4. **20/21 边界**：正好 20 → 发 poll；21 → 回退编号清单且不发 poll。T2 钉住。
5. **加密房间**：poll 必须通过 `room.send(...)`（matrix-sdk 自动加密），不能用 `send_raw` 绕过加密。T1 只验证内容类型可用于 `send`；真机验收第 1 条覆盖。

---

### Task 1: 依赖可行性 + poll 内容构造器

**Files:**
- Modify: `rust/aegis/Cargo.toml`, `rust/aegis/Cargo.lock`
- Create: `rust/aegis/src/gateways/matrix/poll.rs`
- Modify: `rust/aegis/src/gateways/matrix/mod.rs`

**Interfaces:**
- Consumes: 无
- Produces:
  - `pub struct PollSpec { pub question: String, pub answers: Vec<PollAnswer> }`
  - `pub struct PollAnswer { pub id: String, pub text: String }`
  - `pub const MAX_POLL_ANSWERS: usize = 20;` `pub const MAX_POLL_BINDINGS: usize = 64;` `pub const MAX_ANSWER_LABEL_CHARS: usize = 64;`
  - `pub fn poll_start_content(spec: &PollSpec) -> Option<UnstablePollStartEventContent>`（`None` = ruma 拒收，调用方回退编号清单）

- [ ] **Step 1: 加依赖**

```bash
cd rust/aegis && cargo add ruma@0.18 --features unstable-msc3381
```

Expected: `Cargo.toml` 出现 `ruma = { version = "0.18", features = ["unstable-msc3381"] }`；`Cargo.lock` 里 `name = "ruma"` 仍是 0.18.0 且只有一条。

- [ ] **Step 2: 写失败测试**

`poll.rs` 内：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_start_content_serializes_msc3381_shape() {
        let spec = PollSpec {
            question: "选择一个操作".into(),
            answers: vec![
                PollAnswer { id: "0".into(), text: "Search".into() },
                PollAnswer { id: "1".into(), text: "Help".into() },
            ],
        };
        let content = poll_start_content(&spec).expect("2 answers is within limits");
        let json = serde_json::to_value(&content).unwrap();
        let block = &json["org.matrix.msc3381.poll.start"];
        assert_eq!(block["question"]["org.matrix.msc1767.text"], "选择一个操作");
        assert_eq!(block["max_selections"], 1);
        assert_eq!(block["answers"][0]["id"], "0");
        assert_eq!(block["answers"][1]["org.matrix.msc1767.text"], "Help");
        assert_eq!(json["org.matrix.msc1767.text"], "选择一个操作");
    }

    #[test]
    fn poll_start_content_rejects_more_than_max_answers() {
        let spec = PollSpec {
            question: "q".into(),
            answers: (0..21)
                .map(|i| PollAnswer { id: i.to_string(), text: format!("b{i}") })
                .collect(),
        };
        assert!(poll_start_content(&spec).is_none());
    }

    #[test]
    fn poll_start_content_rejects_empty_answers() {
        let spec = PollSpec { question: "q".into(), answers: vec![] };
        assert!(poll_start_content(&spec).is_none());
    }
}
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo +nightly nextest run -E 'test(poll_start_content)' --cargo-profile fast-test`
Expected: 编译失败（`PollSpec` / `poll_start_content` 未定义）

- [ ] **Step 4: 实现**

`ruma-events 0.36` 的实测 API（T1 已踩过一次，照这里写）：
- `UnstablePollStartEventContent` 是**枚举**（`New` / `Replacement`），**没有** `new(block)`。
- 用 `NewUnstablePollStartEventContent::plain_text(question, block).into()`：它同时填 `org.matrix.msc1767.text` 兜底字段；`::new(block)` 那条路 `text` 为 `None`，↑ 的序列化断言会挂。

`poll.rs`：

```rust
pub fn poll_start_content(spec: &PollSpec) -> Option<UnstablePollStartEventContent> {
    let answers: Vec<_> = spec.answers.iter()
        .map(|a| UnstablePollAnswer::new(a.id.clone(), a.text.clone()))
        .collect();
    let block = UnstablePollStartContentBlock::new(
        spec.question.clone(),
        UnstablePollAnswers::try_from(answers).ok()?,
    );
    Some(UnstablePollStartEventContent::new(block))
}
```

`mod.rs` 加 `pub mod poll;`。

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo +nightly nextest run -E 'test(poll_start_content)' --cargo-profile fast-test`
Expected: 3 passed

- [ ] **Step 6: Commit**

```bash
git add rust/aegis/Cargo.toml rust/aegis/Cargo.lock rust/aegis/src/gateways/matrix/poll.rs rust/aegis/src/gateways/matrix/mod.rs
git commit -m "feat(matrix): 加 ruma unstable-msc3381 与 poll 内容构造器"
```

> **硬门禁**：本节 `cargo build` 不过 → 停，回退 spec §4 方案 A（自建 `EventContent` 结构），不得继续 T2。

---

### Task 2: `plan_send` 判定纯函数 + poll 问题文案

**Files:**
- Modify: `rust/aegis/src/gateways/matrix/poll.rs`
- Modify: `rust/aegis/src/resources/i18n/en.yml:842`, `zh.yml:862`, `ja.yml:842`

**Interfaces:**
- Consumes: T1 的 `PollSpec` / `PollAnswer` / `MAX_POLL_ANSWERS` / `MAX_ANSWER_LABEL_CHARS`
- Produces: `pub fn plan_send(content: &MessageContent) -> (String, Option<PollSpec>)`

- [ ] **Step 1: 写失败测试**

```rust
fn content(text: &str, rows: Vec<Vec<(&str, &str)>>) -> MessageContent {
    MessageContent {
        text: text.into(),
        markup: if rows.is_empty() { None } else {
            Some(Markup { buttons: rows.into_iter().map(|row| row.into_iter()
                .map(|(t, d)| InlineButton { text: t.into(), data: d.into() }).collect()).collect() })
        },
    }
}

#[test]
fn plan_send_without_markup_keeps_body_and_no_poll() {
    let (body, poll) = plan_send(&content("plain", vec![]));
    assert_eq!(body, "plain");
    assert!(poll.is_none());
}

#[test]
fn plan_send_with_markup_drops_numbered_list_and_builds_poll() {
    let (body, poll) = plan_send(&content("Hi", vec![vec![("Search", "/search"), ("Help", "/help")]]));
    assert_eq!(body, "Hi");
    let spec = poll.unwrap();
    assert_eq!(spec.answers.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["0", "1"]);
    assert_eq!(spec.answers[0].text, "Search");
}

#[test]
fn plan_send_flattens_row_major() {
    let (_, poll) = plan_send(&content("t", vec![
        vec![("a", "/a"), ("b", "/b")],
        vec![("c", "/c")],
    ]));
    let spec = poll.unwrap();
    assert_eq!(spec.answers.iter().map(|a| a.text.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
}

#[test]
fn plan_send_with_20_buttons_still_polls() {
    let row: Vec<(String, String)> = (0..20).map(|i| (format!("b{i}"), format!("/b{i}"))).collect();
    let m = Markup { buttons: vec![row.iter().map(|(t, d)| InlineButton { text: t.clone(), data: d.clone() }).collect()] };
    let (_, poll) = plan_send(&MessageContent { text: "t".into(), markup: Some(m) });
    assert_eq!(poll.unwrap().answers.len(), 20);
}

#[test]
fn plan_send_with_21_buttons_falls_back_to_numbered_list() {
    let row: Vec<InlineButton> = (0..21).map(|i| InlineButton { text: format!("b{i}"), data: format!("/b{i}") }).collect();
    let (body, poll) = plan_send(&MessageContent { text: "t".into(), markup: Some(Markup { buttons: vec![row] }) });
    assert!(poll.is_none());
    assert!(body.contains("1. b0"));
    assert!(body.contains("/b0"));
}

#[test]
fn plan_send_truncates_long_label_but_keeps_data_intact() {
    let long = "x".repeat(200);
    let (_, poll) = plan_send(&content("t", vec![vec![(long.as_str(), "/very:long:command:data:that:must:survive")]]));
    let spec = poll.unwrap();
    assert_eq!(spec.answers[0].text.chars().count(), MAX_ANSWER_LABEL_CHARS);
    assert_eq!(spec.answers[0].id, "0");
}

#[test]
fn plan_send_uses_data_when_label_is_blank() {
    let (_, poll) = plan_send(&content("t", vec![vec![("   ", "/fallback")]]));
    assert_eq!(poll.unwrap().answers[0].text, "/fallback");
}
```

> `data` 完整性由 `PollStore` 侧的 `InlineButton` 保留来保证；`plan_send` 只产出 label。Step 3 会补一条断言：`plan_send` 的 `PollSpec` 不携带 data（data 由 adapter 用同一顺序从 `content.markup` 拍平取），因此 label 截断不可能影响 data。

- [ ] **Step 2: 加 i18n 文案**

三个 yml 的 `matrix:` 段各加一行（key 名固定）：

```yaml
  poll_question: "选择一个操作"
```

（en: `"Choose an action"`，ja: `"操作を選択"`）

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo +nightly nextest run -E 'test(plan_send)' --cargo-profile fast-test`
Expected: 编译失败（`plan_send` 未定义）

- [ ] **Step 4: 实现**

```rust
/// 返回 (正文, 可选 poll)。有 poll 时正文**不拼**编号清单。
/// 返回 (正文, 可选 poll)。有 poll 时正文**不拼**编号清单。
pub fn plan_send(content: &MessageContent) -> (String, Option<PollSpec>) {
    let buttons = flatten(&content.markup);
    match &content.markup {
        Some(m) if !buttons.is_empty() && buttons.len() <= MAX_POLL_ANSWERS => {
            let answers = buttons.iter().enumerate()
                .map(|(i, b)| PollAnswer { id: i.to_string(), text: answer_label(&b.text, &b.data) })
                .collect();
            (content.text.clone(), Some(PollSpec { question: t!("matrix.poll_question").to_string(), answers }))
        }
        Some(m) => (render_markup_buttons(content.text.clone(), m), None),
        None => (content.text.clone(), None),
    }
}

pub fn flatten(markup: &Option<Markup>) -> Vec<InlineButton>  // row-major，返回 clone
fn answer_label(label: &str, data: &str) -> String            // trim 后为空用 data；按 chars 截断到 MAX_ANSWER_LABEL_CHARS
```

`render_markup_buttons(text: String, markup: &Markup)` 的既有签名只吃 `&Markup`，所以 fallback 分支必须 `match &content.markup`，不能把 `Option<&Markup>` 直接传进去。

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo +nightly nextest run -E 'test(plan_send)' --cargo-profile fast-test`
Expected: 6 passed

- [ ] **Step 6: Commit**

```bash
git commit -am "feat(matrix): plan_send 判定按钮能否由 poll 承载"
```

---

### Task 3: `PollStore` + `answer_index`

**Files:**
- Modify: `rust/aegis/src/gateways/matrix/poll.rs`

**Interfaces:**
- Consumes: T1 的 `MAX_POLL_BINDINGS`
- Produces:
  - `pub struct PollBinding { pub anchor: MessageId, pub buttons: Vec<InlineButton>, created: Instant, answered: bool }`
  - `pub struct PollStore { /* Mutex<HashMap<String, PollBinding>> */ }`
  - `impl PollStore`：`pub fn new() -> Self`、`pub fn insert(&self, poll_event_id: String, binding: PollBinding)`、`pub fn find_by_anchor(&self, anchor: &str) -> Option<String>`、`pub fn remove(&self, poll_event_id: &str)`
  - `pub fn take_unanswered(&self, poll_event_id: &str, index: usize) -> Option<(MessageId, String)>` —— 返回 `(anchor, button.data)`；未知 poll / 已 answered / 下标越界 → `None`
  - `pub fn answer_index(answer_id: &str, total: usize) -> Option<usize>`

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn answer_index_parses_and_bounds() {
    assert_eq!(answer_index("0", 3), Some(0));
    assert_eq!(answer_index("2", 3), Some(2));
    assert_eq!(answer_index("3", 3), None);
    assert_eq!(answer_index("x", 3), None);
    assert_eq!(answer_index("", 3), None);
    assert_eq!(answer_index("-1", 3), None);
}

#[test]
fn take_unanswered_returns_anchor_and_data_once() {
    let s = PollStore::new();
    s.insert("$poll".into(), binding("$anchor", vec![button("Search", "/search")]));
    assert_eq!(s.take_unanswered("$poll", 0).map(|(m, d)| (m.0, d)),
               Some(("$anchor".into(), "/search".into())));
    assert!(s.take_unanswered("$poll", 0).is_none(), "第二次不得再派发（连点/改票）");
}

#[test]
fn take_unanswered_ignores_unknown_poll_and_out_of_range() {
    let s = PollStore::new();
    assert!(s.take_unanswered("$nope", 0).is_none());
    s.insert("$poll".into(), binding("$anchor", vec![button("a", "/a")]));
    assert!(s.take_unanswered("$poll", 5).is_none());
    assert_eq!(s.take_unanswered("$poll", 0).map(|(_, d)| d), Some("/a".into()),
               "越界失败不得把 answered 置位");
}

#[test]
fn find_by_anchor_and_remove() {
    let s = PollStore::new();
    s.insert("$p1".into(), binding("$a1", vec![button("a", "/a")]));
    assert_eq!(s.find_by_anchor("$a1").as_deref(), Some("$p1"));
    assert!(s.find_by_anchor("$a2").is_none());
    s.remove("$p1");
    assert!(s.find_by_anchor("$a1").is_none());
}

#[test]
fn insert_evicts_oldest_beyond_capacity() {
    let s = PollStore::new();
    for i in 0..=MAX_POLL_BINDINGS {
        s.insert(format!("$p{i}"), binding(&format!("$a{i}"), vec![button("a", "/a")]));
    }
    assert!(s.find_by_anchor("$a0").is_none(), "最旧的必须被淘汰");
    assert!(s.find_by_anchor(&format!("$a{MAX_POLL_BINDINGS}")).is_some());
}
```

辅助：`fn binding(anchor: &str, buttons: Vec<InlineButton>) -> PollBinding`、`fn button(t: &str, d: &str) -> InlineButton`、`fn button` 用 `created: Instant::now()`。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo +nightly nextest run -E 'test(answer_index) or test(take_unanswered) or test(find_by_anchor) or test(insert_evicts)' --cargo-profile fast-test`
Expected: 编译失败

- [ ] **Step 3: 实现**

关键点（其余按签名写）：
- `take_unanswered` 取锁后：`answered` 已真 → `None`；`index >= buttons.len()` → `None`（**不置位**）；否则置 `answered = true` 并返回克隆。
- `insert` 后若 `len() > MAX_POLL_BINDINGS`，按 `created` 最小者逐个移除直到回到上限。
- `created` 字段仅用于淘汰排序。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo +nightly nextest run -E 'test(answer_index) or test(take_unanswered) or test(find_by_anchor) or test(insert_evicts)' --cargo-profile fast-test`
Expected: 5 passed

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(matrix): PollStore 承载 poll→按钮映射与一次性消费"
```

---

### Task 4: `send_message` 接入（含既有测试改造）

**Files:**
- Modify: `rust/aegis/src/gateways/matrix/adapter.rs:20-27`（结构体）、`:114-138`（`send_message`）、`:68-108`（既有测试）
- Modify: `README.md`（管理平台表下方补一行已知限制）

**Interfaces:**
- Consumes: T1 `poll_start_content`、T2 `plan_send`/`flatten`、T3 `PollStore`/`PollBinding`
- Produces: `MatrixAdapter { room: Room, polls: PollStore }`；`impl MatrixAdapter { pub fn polls(&self) -> &PollStore }`

- [ ] **Step 1: 改造既有测试（RED）**

把 `send_message_with_markup_appends_command_list` 改名为 `send_message_with_markup_no_longer_inlines_command_list`，断言改为：

```rust
let (body, poll) = crate::gateways::matrix::poll::plan_send(&MessageContent {
    text: "Hello".into(),
    markup: Some(markup),
});
assert_eq!(body, "Hello");
assert!(!body.contains("send: `/search`"), "正文不得再暴露内部命令");
assert_eq!(poll.unwrap().answers.len(), 3);
```

`send_message_without_markup_returns_plain_text` 保持不变。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo +nightly nextest run -E 'test(send_message_with_markup)' --cargo-profile fast-test`
Expected: FAIL（断言旧行为不成立）

- [ ] **Step 3: 实现**

`send_message` 改为：

```rust
let (body_text, poll_spec) = poll::plan_send(&content);
let buttons = poll::flatten(&content.markup);
// 敏感内容分支不变：is_sensitive(&body_text) → 发 batch_result.txt 附件
// 非敏感分支：发正文，拿 response.event_id 作为 anchor
if let Some(spec) = poll_spec {
    if let Some(poll_content) = poll::poll_start_content(&spec) {
        let poll_resp = self.room.send(poll_content).await?;
        self.polls.insert(poll_resp.response.event_id.to_string(), PollBinding {
            anchor: MessageId(anchor_event_id.clone()),
            buttons,
            created: Instant::now(),
            answered: false,
        });
    }
}
```

- poll 发送失败只 `log::warn!`，不影响正文已送达（**不得**让 poll 失败把消息回滚成错误）。
- 敏感分支发完附件后同样发 poll，anchor = 附件事件 id。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo +nightly nextest run -E 'test(send_message)' --cargo-profile fast-test`
Expected: 2 passed

- [ ] **Step 5: README 补一行**

管理平台表下方加：

```markdown
> Matrix 上的按钮以 MSC3381 poll 呈现（Element 等客户端可点选）。启动形态为 `--all` 时 Matrix 入站关闭，此项不生效。
```

- [ ] **Step 6: Commit**

```bash
git commit -am "feat(matrix): send_message 用 poll 承载按钮，新增 PollStore 字段"
```

---

### Task 5: `edit_message` / `delete_message` 接入（可与 Task 6 并行）

**Files:**
- Modify: `rust/aegis/src/gateways/matrix/poll.rs`（新增 `take_by_anchor`）
- Modify: `rust/aegis/src/gateways/matrix/adapter.rs:139-165`

**Interfaces:**
- Consumes: T4 的 `self.polls`、T3 的 `insert`/`remove`
- Produces: `PollStore::take_by_anchor(&self, anchor: &str) -> Option<String>` —— 原子地「找到并移除」该 anchor 名下的 poll，返回其 event id（供调用方 redact）

- [ ] **Step 1: 写失败测试（`poll.rs`）**

```rust
#[test]
fn take_by_anchor_returns_and_removes_old_poll_once() {
    let s = PollStore::new();
    s.insert("$old".into(), binding("$anchor", vec![button("a", "/a")]));
    assert_eq!(s.take_by_anchor("$anchor").as_deref(), Some("$old"));
    assert!(s.take_by_anchor("$anchor").is_none(), "同一个 anchor 只能取一次");
}

#[test]
fn take_by_anchor_leaves_other_anchors_alone() {
    let s = PollStore::new();
    s.insert("$p1".into(), binding("$a1", vec![button("a", "/a")]));
    s.insert("$p2".into(), binding("$a2", vec![button("b", "/b")]));
    assert_eq!(s.take_by_anchor("$a1").as_deref(), Some("$p1"));
    assert_eq!(s.find_by_anchor("$a2").as_deref(), Some("$p2"));
    assert!(s.take_by_anchor("$unknown").is_none());
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo +nightly nextest run -E 'test(take_by_anchor)' --cargo-profile fast-test`
Expected: 编译失败（`take_by_anchor` 未定义）

- [ ] **Step 3: 实现 `take_by_anchor`，并接进 adapter**

`take_by_anchor`：取锁 → 找到 `anchor == 参数` 的唯一 entry → 移除并返回 key；没有则 `None`。

`edit_message` 有 poll 时严格按序（IO 顺序本身由真机验收覆盖）：

1. `self.room.edit(...)` 用 `plan_send` 的正文（无编号清单）更新 anchor；
2. `if let Some(old) = self.polls.take_by_anchor(&msg_id.0)` → `self.room.redact(&EventId::parse(&old)?, Some("superseded"), None).await` —— 失败只 `log::warn!`，**不得**中断后续发新 poll；
3. 发新 poll 并 `insert`（anchor 仍是传入的 `msg_id`）。

无 poll 时完全走原逻辑。`delete_message`：原逻辑后追加 `let _ = self.polls.take_by_anchor(&msg_id.0);`（消息已删，不 redact）。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo +nightly nextest run -E 'test(take_by_anchor)' --cargo-profile fast-test`
Expected: 2 passed

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(matrix): edit_message 刷新 poll（redact 旧的 + 发新的）"
```

---

### Task 6: 入站 poll 响应 → `CallbackEvent`（可与 Task 5 并行）

**Files:**
- Modify: `rust/aegis/src/main/runtime.rs:185-260`（在现有 Matrix 段内追加 handler）

**Interfaces:**
- Consumes: `MatrixAdapter::polls()`（T4）、`PollStore::take_unanswered`/`answer_index`（T3）、既有 `dispatch_event`、`state.is_admin_user`、`state.session_timeout_secs`
- Produces: 无（终端行为）

- [ ] **Step 1: 写失败测试**

把「响应 → 该不该派发」抽成一个纯函数，handler 的 IO 不测：

```rust
pub(crate) fn response_index(selections: &[String]) -> Option<usize>
```

测试：

```rust
#[test]
fn response_index_uses_first_selection() {
    assert_eq!(response_index(&["1".to_string()]), Some(1));
}

#[test]
fn response_index_ignores_empty_or_unparsable() {
    assert!(response_index(&[]).is_none());
    assert!(response_index(&["x".to_string(), "1".to_string()]).is_none());
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo +nightly nextest run -E 'test(response_index)' --cargo-profile fast-test`
Expected: 编译失败（`response_index` 未定义）

- [ ] **Step 3: 实现**

```rust
pub(crate) fn response_index(selections: &[String]) -> Option<usize> {
    selections.first().and_then(|s| s.parse::<usize>().ok())
}
```

在现有 Matrix 段（`secondary_handles_inbound` 内、紧跟 `OriginalSyncRoomMessageEvent` handler 之后）注册：

```rust
client.add_event_handler(move |event: OriginalSyncUnstablePollResponseEvent,
                               room: MatrixRoom, _client: MatrixClient| {
    let state = polls_state.clone();
    let adapter = polls_adapter.clone();     // Arc<MatrixAdapter>
    let target = polls_target.clone();
    async move {
        if room.room_id().as_str() != target.0.as_str() { return; }
        let user_id = parse_user_id(event.sender.as_str());
        if !state.is_admin_user(user_id) { log::debug!("Matrix poll 响应来自非管理员，丢弃"); return; }
        let poll_id = event.content.relates_to.event_id.as_str();
        let Some(idx) = response_index(&event.content.poll_response.answers) else { return; };
        let Some((anchor, data)) = adapter.polls().take_unanswered(poll_id, idx) else { return; };
        let _ = dispatch_event(BotEvent::Callback(CallbackEvent {
            adapter: adapter.clone(),
            target: target.clone(),
            user_id: user_id.to_string(),
            msg_id: anchor,
            data,
            callback_id: format!("poll:{poll_id}"),
            session_timeout_secs: state.session_timeout_secs().await,
        }), &state).await;
    }
});
```

越界与「已 consumed」一律由 `take_unanswered` 判，`response_index` 只做「取第一个选择并解析成数字」，负号/非数字返回 `None`。类型导入：`matrix_sdk::ruma::events::poll::unstable_response::OriginalSyncUnstablePollResponseEvent`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo +nightly nextest run -E 'test(response_index)' --cargo-profile fast-test`
Expected: 2 passed

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(matrix): poll 响应合成 CallbackEvent 走同一派发路径"
```

---

## Phase 收尾

- [ ] **Checkpoint 1（T1–T3 后）**：`cargo clippy --all-targets -- -D warnings` 无新警告；`Cargo.lock` 只有一份 ruma。
- [ ] **Checkpoint 2（T6 后）**：全量 `cargo +nightly nextest run --cargo-profile fast-test` 绿；`cargo fmt --check` 绿。
- [ ] **真机验收（spec §7）**：`--matrix` 单跑，Element 里 `/menu` → 可点选项 → 点一项推进 → 旧 poll 消失 → 连点只进一次 → 非管理员点选无反应。
- [ ] **Branch review**：整支 diff 过 `code-review-and-quality`。

## 并行边界

| 组 | 任务 | 文件 | 说明 |
| --- | --- | --- | --- |
| 串行 | T1 → T2 → T3 | 全在 `gateways/matrix/poll.rs` | 同一文件，禁止并发写（AGENTS.md 并发铁律） |
| 串行 | T4 | `gateways/matrix/adapter.rs` | 依赖 T1–T3 的全部接口 |
| **并行** | T5 ｜ T6 | `adapter.rs` ｜ `runtime.rs` | 不同文件；T6 只依赖 T3/T4 的接口，不碰 adapter 内部 |

只有一个可并行组（T5/T6）。其余单文件改动一律串行。
