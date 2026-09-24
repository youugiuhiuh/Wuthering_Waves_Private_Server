# 设计：敏感内容转发 —— SimpleX 敏感文本转附件（修订 v2）

> 模式：**strict** ｜ 状态：**待批准**（v2） ｜ 日期：2026-09-23
> 触发：P2 真机验收暴露的不对称（记为 C）+ CodeGraph 复查对 v1 范围的修正

## 0. v2 修订摘要（相对 v1）

| 变更 | 依据 |
| --- | --- |
| **撤销 C2**（文件类分流 S-A/S-B） | CodeGraph 复查：`send_file`/`send_image`/`send_voice` 在**生产代码零调用点**（唯一调用者是 `BotAdapter` 默认委托与 `MatrixAdapter` 内部委托）→ 为一条不可达路径写路由+测试 = 死代码（YAGNI）。v1 §1 第 2 点所述「文件永远落 TG、绕过 secondary」在真机触发不到 |
| **新增 C1b** | 路由层同类漏判：`RoutingAdapter::send_message` 判的是**未渲染**的 `content.text`，而按钮 data 会被并入文本（`common/markup.rs:10`）→ 敏感内容只存在于按钮时按 primary 发出，落 TG |
| **新增 C1c（可选）** | `PlatformCapabilities::MATRIX` 常量与 `MatrixAdapter::capabilities()` 三个字段不一致 |
| **保留 C1** | 唯一真缺口，已真机复现（`chat_item 17`：`vless://…` 当**普通消息**落到 SimpleX） |

## 1. 现状（CodeGraph 复核，非推断）

### 1.1 敏感转发现有的两层

| 层 | 位置 | 行为 |
| --- | --- | --- |
| 路由层 | `common/routing.rs::RoutingAdapter::send_message`（`:58`） | `is_sensitive(&content.text)`（**未渲染**）命中 → 改投 secondary |
| 平台层 | `gateways/matrix/adapter.rs::MatrixAdapter::send_message`（`:120`） | `is_sensitive(&body_text)`（**已渲染**）命中 → 改写成附件 `batch_result.txt` |

`render_markup_buttons`（`common/markup.rs:4`）会把按钮并入正文：

```rust
lines.push(format!("{}. {} — send: `{}`", idx, btn.text, btn.data));   // markup.rs:10
```

### 1.2 SimpleX 能力矩阵（声明 / 实现 / 实际）

| 能力 | `PlatformCapabilities::SIMPLEX` | `SimplexAdapter` | 实际 |
| --- | --- | --- | --- |
| send_message / edit / delete | ✓ / ✓ / ✓ | ✓ `send_msg` / `update_msg` / `delete_msg` | ✓ |
| send_file | `can_send_file: true` | ✓ `File::new` + temp `0600` + 用后删 | ✓ |
| send_image | `can_send_image: true` | ✓ `Image::new` | ✓ |
| send_voice | `false` | ✗ 未实现 → 默认委托 `send_file("voice")` | 可用（发成文件） |
| send_reaction | `true` | ✓ `update_msg_reaction` | ✓ |
| send_typing | `false` | ✗ 默认 no-op | ✗ |
| inline keyboard | `false` | ✗ 默认 no-op；按钮降级为文本命令 | ✗ |
| thread | `false` | ✗ 默认回落 `send_message` | ✗ |
| download_file | `has_file_transfer: false` | ✗ 默认 `bail!` | ✗ 收文件不可用 |
| 联系人审批 | — | ✓ accept/reject | ✓ |

### 1.3 三条复核结论（决定 v2 范围）

1. **`send_file`/`send_image`/`send_voice` 生产调用点为 0** → C2 撤销。
2. **capabilities 字段在生产代码中无读取点**（`.capabilities()` 仅被 `RoutingAdapter` 透传 + mock/测试调用）→ 目前是「只写不读」的数据；C1c 只影响测试/文档，无线上风险。
3. **SimpleX 无法收文件**（`download_file` 未实现）。任何「让用户在 SimpleX 发文件给 bot」的流程都不可用——本设计不涉及，但记录在此避免误判。

## 2. 目标（v2）

- **G1**：命中 `is_sensitive` 的**文本**投到 SimpleX 时以**附件**（`.txt`）形式，而非可转发的普通消息。
- ~~**G2**：文件类发送分流~~ **【撤销，见 §0】**
- **G3**：不改变现有 Matrix 行为与纯 `--simplex` 行为（除 D-C2 裁决）。
- **G4（新增）**：**路由层与平台层的敏感判定对象统一为「渲染后文本」**，消除按钮 data 逃逸。

## 3. 设计

### 3.1 C1：SimpleX 敏感文本转附件

在 `SimplexAdapter::send_message` 内与 Matrix 对称处理（**判定对象为渲染后的 `text`**）：

```rust
// gateways/simplex/adapter.rs  impl BotAdapter for SimplexAdapter::send_message
let chat_id = parse_chat_id(target)?;
let text = match &content.markup {
    Some(markup) => render_markup_buttons(content.text, markup),
    None => content.text,
};
if is_sensitive(&text) {
    // 复用既有 write_temp_file(0600) → File::new → 发送后删除
    return self.send_file(target, "batch_result.txt", text.into_bytes(), "text/plain").await;
}
let resp = self.bot.send_msg(chat_id, text).await.context("发送 SimpleX 消息失败")?;
Ok(MessageId(first_item_id(&resp)?.to_string()))
```

- `is_sensitive` 已是 `pub(crate)`（`routing.rs:36`），Matrix 已在用。
- `SimpleXAdapter::send_file` 已存在（`adapter.rs:268`），无新增依赖、无新增临时文件生命周期。
- **不要**判 `content.text`：按钮 data 会被 `render_markup_buttons` 并入正文，判未渲染文本会漏判（v1 草图的缺陷）。

> 备选（不推荐）：在 `RoutingAdapter` 里改调 `secondary.send_file(...)`。路由层被迫知道平台的 mime/文件名，且纯 `--simplex`（无 RoutingAdapter）不生效。

### 3.2 C1b：`RoutingAdapter` 判定改用渲染后文本

```rust
// common/routing.rs  RoutingAdapter::send_message
let routed_text = match &content.markup {
    Some(markup) => render_markup_buttons(content.text.clone(), markup),
    None => content.text.clone(),
};
match &self.secondary {
    Some(secondary) if is_sensitive(&routed_text) => { /* 原分流逻辑不变 */ }
    _ => self.primary.send_message(target, content).await,
}
```

- 风险：`render_markup_buttons` 依赖 `rust_i18n::t!("matrix.markup_header")`。若 i18n 未初始化，渲染结果可能退化——单测需覆盖。
- `send_message_primary`（`routing.rs:182`）本就是绕过分流的专用通道，不受影响。

### 3.3 C1c（可选）：对齐 `PlatformCapabilities::MATRIX`

| 字段 | 常量现值 | `MatrixAdapter::capabilities()` | 应改为 |
| --- | --- | --- | --- |
| can_send_typing | `false` | `true` | `true` |
| can_send_reaction | `false` | `true` | `true` |
| can_thread | `false` | `true` | `true` |

纯常量改动，零运行时影响（见 §1.3 第 2 点）。

### 3.4 与 P2.1 的关系

P2.1 已修：`RoutingAdapter.secondary_target` 动态化（重钉即时生效）+ 敏感发送失败记 `log::error`。本设计**依赖 P2.1**（否则落点仍是启动快照）。

## 4. 边界与风险

1. **纯 `--simplex` 无 secondary**：C1 的适配器级转换对该形态**也生效**（敏感文本→附件），但目标就是管理员本身。是否接受由 D-C2 裁决。
2. **`is_sensitive` 是启发式**：误判 → 变附件（体验下降，不泄露）；漏判 → 普通消息发出（**主要残余风险**，与 Matrix 一致）。
3. **SimpleX 附件**：bot 需以 `--create-bot-allow-files` 启动（安装器单元已含）；接收方需能收文件。
4. **C1b 会改变分流结果**：原本判不出（按钮 data）而发往 TG 的消息，改为发往 secondary。需确认「安全通知」路径仍走 `send_message_primary`（已实现，`routing.rs:182`）。
5. **测试**：C1 可单测（敏感文本 → 断言走 `send_file` 的 mock 期望）；C1b 用带敏感按钮 data 的 `MessageContent` 单测分流；真机验收需 SimpleX 客户端确认收到 `.txt` 附件。

## 5. 交付拆分（v2）

| # | 内容 | 依赖 | 状态 |
| --- | --- | --- | --- |
| C1 | `SimplexAdapter::send_message` 敏感文本转 `.txt` 附件（判定用渲染后文本） | P2.1 | 待批 |
| C1b | `RoutingAdapter::send_message` 判定改用渲染后文本 | — | 待批 |
| C1c | 对齐 `PlatformCapabilities::MATRIX` 常量 | — | 待批（可选） |
| ~~C2~~ | ~~文件类分流 S-A/S-B~~ | — | **撤销（零调用点）** |
| C3 | 单测 + 真机验收 + 文档勘误 | C1/C1b | 待批 |

## 6. 待裁决

| # | 决策 | 默认 |
| --- | --- | --- |
| D-C1 | C1b（路由层判定改渲染后文本）是否纳入本次 | **是** |
| D-C2 | 纯 `--simplex` 是否也做「敏感文本→附件」 | **是**（与适配器行为一致，最简单） |
| D-C3 | 附件文件名 / mime | `batch_result.txt` / `text/plain`（与 Matrix 一致） |
| D-C4 | 判定对象统一为**渲染后文本**（路由层+平台层） | **是** |
| D-C5 | C1c 常量对齐是否顺手做 | **是** |
| ~~D-C6~~ | ~~文件类分流策略 S-A/S-B~~ | **随 C2 撤销** |
