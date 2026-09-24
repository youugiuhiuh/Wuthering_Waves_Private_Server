# 设计：TG+SimpleX 的分享链接与安全码输出

> 模式：**strict** ｜ 状态：**已按用户指示定稿** ｜ 日期：2026-09-24
> 用户原话：「就 Bot 当是 TG+simplex 自动在 TG 发送分享链接，之后用户连接，TOTP 允许验证之后自动输出安全码 TG」

## 1. 背景（已核实，CodeGraph + 依赖源码）

| 事实 | 位置/证据 |
| --- | --- |
| TG+SimpleX 下 SimpleX 分支**跳过**启动/调度块（防重复），TG 分支的启动通知只发 upgrade/online/bbr3 | `main/runtime.rs:240`（`simplex_admin.filter(\|_\| !enable_telegram)`）、`:486-508` |
| 因此**分享链接从未发到 TG** | 同上；地址只在日志与 `simplex_address` 文件里（`main/simplex.rs:53` `record_address`） |
| TOTP 成功钩子已存在（SimpleX 来源才重钉 + 刷新敏感落点） | `app/auth.rs:34-57` |
| aegis 能发 raw 命令取安全码（零新依赖） | `ws::Bot = Bot<Client>`；`Bot::client()`；`impl ClientApi for Client`（`ws.rs:199`）；`ClientApi` 由 crate root 公开 |
| 命令与响应 | `/_get code @<contactId>`（simplex-chat `Commands.hs:5580`），`resp.type=contactCode`、`resp.connectionCode` |
| 自动标记已验证的命令（**本次不做**） | `/_verify code @<id> <code>`（`Commands.hs:5582`） |

## 2. 目标

- **G1**：`--tg-simplex` 启动时，向 **TG 管理员**发送 **SimpleX 分享链接**（bot 地址），管理员点开即可连接。
- **G2**：TOTP 验证成功且**来源为 SimpleX** 时，把该联系人的 **SimpleX 安全码**发到 **TG**，管理员在自己客户端「验证安全码」逐段比对。
- **G3**：不改变纯 `--simplex`（无 TG）与 Matrix-only 行为。

## 3. 设计

### 3.1 分享链接（G1）

- `SimplexHandle` 增加 `address: Option<String>`（`connect_simplex` 里 `bot.address()` 成功后填入；沿用既有 `record_address` 的取值）。
- `main/runtime.rs`：在 `run()` 顶部（SimpleX 分支消费 handle 之前）取 `let simplex_address = simplex_handle.as_ref().and_then(|h| h.address.clone());`
- TG 分支的启动 `tokio::join!` 里追加一次发送（仅当 `admin_id` 与地址都存在）：

```rust
notify_simplex_share_link(&*adapter_for_init, &target_for_init, &address).await
```

### 3.2 安全码（G2）

- `BotAdapter` 新增默认方法（默认 `bail!`，只有 SimpleX 实现）：

```rust
async fn contact_security_code(&self, _contact_id: i64) -> Result<String> {
    anyhow::bail!("当前平台不支持获取连接安全码")
}
```

- `SimplexAdapter` 实现：`self.bot.client().send_raw(format!("/_get code @{contact_id}"))` → 交给纯函数 `parse_contact_code`。
- 纯函数（TDD 主战场）：

```rust
fn parse_contact_code(raw: &str) -> Result<String>  // 取 resp.connectionCode；缺失/空/非 contactCode 都报错
```

- `app/auth.rs` 在既有重钉块**之后**追加（同一 `if state.verify_totp(code)` 内）：

```rust
if adapter.platform() == Platform::Simplex
    && let Some(tg_admin) = state.admin_id()
{
    match adapter.contact_security_code(user_id).await {
        Ok(code) => { /* state.adapter.send_message_primary(TargetId(tg_admin), ...) */ }
        Err(e) => log::warn!("获取 SimpleX 安全码失败（不影响验证）: {e}"),
    }
}
```

### 3.3 发送通道

一律 `send_message_primary`（绕过敏感分流）：安全码/链接若走 `send_message`，可能被 `is_sensitive` 改投到 SimpleX，管理员反而在 TG 看不到。

### 3.4 i18n

两条新文案走 `rust_i18n`，**三语齐全**（zh/en/ja）——`tests/hy2_i18n_parity.rs` 把 locale 文件当契约。

## 4. 边界与风险

1. **fail-open**：取码/发链失败只 `warn`，不阻塞启动、不影响 TOTP 验证结果。
2. **纯 `--simplex`**：`state.admin_id()` 为 `None` → 不发（G3）。
3. **启动时联系人可能尚未连接**：不取码（只在 TOTP 成功后取），避免无谓失败。
4. **分享链接是公开地址**：任何人都能连接（既有设计），发到 TG 私聊不扩大暴露面（地址本就写在 `simplex_address` 与日志里）。
5. **不自动标已验证**：`/_verify` 存在但用户要求自己验证。

## 5. 交付拆分

| # | 内容 | 依赖 |
| --- | --- | --- |
| S1 | `parse_contact_code` 纯函数 + `BotAdapter::contact_security_code` + `SimplexAdapter` 实现（TDD） | — |
| S2 | `app/auth.rs` TOTP 成功后发码到 TG（TDD，mockall） | S1 |
| S3 | `SimplexHandle.address` + 启动发分享链接（TDD 判据函数） | — |
| S4 | i18n 三语 key | S2/S3 |
| S5 | 门禁 + PR + 真机验收 | 全部 |

## 6. 验收标准（真机）

1. `--tg-simplex` 重启后，TG 管理员收到含 SimpleX 地址的消息。
2. 在 SimpleX 发 TOTP 码 → TG 管理员收到该联系人的安全码。
3. 该安全码与 SimpleX 客户端「验证安全码」显示**逐段一致**。
4. 纯 `--simplex` 不产生任何 TG 发送。
