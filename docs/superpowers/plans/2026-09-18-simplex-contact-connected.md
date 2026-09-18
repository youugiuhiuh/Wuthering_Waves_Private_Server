# 计划：处理 SimpleX `ContactConnected`，让管理员连上即拿到 contactId

> 模式：**normal**（1 文件、~10 行、无架构/DB/安全语义变更）
> 分支：`fix/simplex-contact-connected` ｜ 基线 main `40890cf`（P1–P5 已合并）
> 依据：真实主机已确认的缺陷（见「问题」），设计与用户逐轮确认后定案

## 1. 问题（真实主机已复现）

管理员点开 bot 分享链接连上后**没有任何权限**。定位过程（测试机 `murky-pull`）：

1. journal 出现 `[WARN] SimpleX 未授权联系人 contactId=3 尝试发消息，已忽略` → 管理员 contactId 是 `3`
2. `未配置 simplex_admin_id` 出现 0 次 → `simplex_admin_id` **字段存在，只是值不是 3**
3. `--set-simplex-admin 3` + 重启 → 管理员身份生效（`/start` 有回应）
4. 重置 TOTP 后全部功能可用

**根因（两段）**：

| 段 | 位置 | 现象 |
| --- | --- | --- |
| 配置值对不上 | `config.enc` | 只需 `--set-simplex-admin` 补填（已在本轮手工修好，**不是本次代码改动**） |
| **连上时 bot 收不到通知** | `rust/aegis/src/main/runtime.rs:284-286` | 非 `NewChatItems` 的事件被全部丢弃，**含 `ContactConnected`** |

第二段才是「邀请链接之后的后续步骤没有了」：官方文档明说
`Contact connection events — Bots must use these events to process connecting users`
（`bots/api/EVENTS.md`），而 aegis 把该事件丢了，于是管理员**必须再发一条消息**、
再从「未授权联系人」告警里反推自己的 contactId —— 一个绕路。

### 已核实的类型事实

- `Event::ContactConnected(Arc<ContactConnected>)` — `simploxide-api-types-0.12.0/src/events.rs:11`
- `ContactConnected { user, contact: Contact, user_custom_profile, undocumented }` — 同文件 `:475-488`
- `Contact.contact_id: i64`、`Contact.local_display_name: String` — `:3622-3630`
- `Event` 带 `#[non_exhaustive]`（`:8`）→ 匹配**必须**有 `_` 分支
- `EventKind::ContactConnected` 是 `EventKind` 的第一个变体（`:281`）

## 2. 目标

管理员连上 bot 的那一刻，journal 里就出现他/她的 `contactId`，无需再先发消息试探。

非目标：
- 不改权限模型（连接 ≠ 管理员，管理员仍只由 `simplex_admin_id` 决定）
- 不自动补填 `simplex_admin_id`（等于「谁先连谁拿 admin」，公开地址 + 自动接受下危险）
- 不新增 TOTP 机制（已是第二因子：`dispatch.rs:35` → `state.rs:148`）
- 不处理其它事件（`ContactUpdated` / `ReceivedContactRequest` / 群事件等）
- 不改「撤销地址 / 关闭自动接受」（`Bot::delete_address` / `configure_address`）——留给独立改动

## 3. 设计

`runtime.rs` 把「非 NewChatItems 一律丢弃」的 `let-else` 换成显式 `match`，
新增 `ContactConnected` 分支打印一行，其余仍忽略：

```rust
let items = match ev {
    simploxide_client::events::Event::NewChatItems(items) => items,
    simploxide_client::events::Event::ContactConnected(ev) => {
        log::info!(
            "{}",
            contact_connected_log_line(ev.contact.contact_id, &ev.contact.local_display_name)
        );
        continue;
    }
    _ => continue, // Event 是 #[non_exhaustive]
};
```

日志文本抽成纯函数以便单测：

```rust
fn contact_connected_log_line(contact_id: i64, display_name: &str) -> String {
    format!("SimpleX 新联系人连接: contactId={contact_id} display_name={display_name:?}")
}
```

**`{:?}` 而非 `{}` 是刻意的**：地址是公开的、自动接受的，任何人都能连接并**自设显示名**；
`{:?}` 转义控制字符，避免用换行伪造日志行（日志注入）。

### 诚实边界（写进 PR）

- 事件循环跑在 `tokio::spawn` 里、依赖活的 WebSocket，**「分支是否存在」无法用单测覆盖**
  （与 P1 日志器缺陷同类的「缺失型」回归）。单测只覆盖日志文本构造；
  端到端只能靠真实主机 journal 断言（§5）。这条必须披露，不能声称已覆盖。

## 4. 任务（TDD）

- [ ] T1 在 `runtime.rs` 加 `contact_connected_log_line` + `#[cfg(test)] mod tests`（**先红**）
- [ ] T2 跑测试确认失败（函数不存在 → 编译失败即 RED）
- [ ] T3 实现函数与 `match` 分支
- [ ] T4 跑测试确认通过
- [ ] T5 Rust 质量门：`fmt --check` / `clippy -D warnings` / `nextest --cargo-profile fast-test` / `test --doc`
- [ ] T6 commit + push + 开 PR

## 5. 验收

| 层 | 断言 |
| --- | --- |
| 单测 | `contact_connected_log_line` 6 例（含转义） |
| 环境 | `export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target` |
| 基线 | Rust **926 passed, 1 skipped** → 预期 **927~930** |
| 真实主机（硬断言） | 部署本分支构建 → **不先发消息**，只连接 → `journalctl -u wwps-aegis \| grep '新联系人连接'` 必须出现且含 `contactId=3` |

## 6. 已知非本次问题（记录，勿混入）

- `simplex_admin_id` 值对不上：需 `--set-simplex-admin`，本轮已在测试机手工修好
- TOTP 是 **SHA512 / 6 位 / 30 秒**；验证器若默认 SHA1 会算出不同的码（已实测：
  同一密钥 SHA512→`720184`、SHA1→`406058`、SHA256→`053238`）
- 地址是**公开长期地址**（`UserContactLink`），不是一次性邀请链接；
  「删掉聊天里的链接」无效，撤销需 `Bot::delete_address()`
