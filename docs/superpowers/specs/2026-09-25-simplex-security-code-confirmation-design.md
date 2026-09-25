# 设计：用户回贴安全码，Bot 自动验证（A 方案）

> 模式：**strict** ｜ 状态：**已批准** ｜ 日期：2026-09-25
> 前置 spec：`2026-09-24-simplex-share-link-and-security-code-design.md` §4.5「不自动标已验证」由本 spec 取代。
> 用户原话：「我想要功能是用户直接复制安全码到 TG 检查是不是一模一样的，下次就不会出现了」
> 用户确认：**A** —— 用户把自己客户端的安全码发到 TG，Bot 自动比对；一致后下次不再出现该提示。

## 0. 假设（不成立请纠正）

1. 用户从 **SimpleX 客户端的「验证安全码」界面**复制码，粘到 **TG** 私聊。
   —— 不是把 Bot 自己发的那条 TG 消息复制回来（那样恒等，无验证意义）。
2. 「下次就不会出现了」= **Bot 下次 TOTP 成功后不再发这条安全码 TG 消息**（持久化"已验证"）。
   —— 不是"让 SimpleX 客户端不再显示验证提示"（那要调 `/_verify`，见 §7，本次不做）。
3. 安全码稳定：同一 contactId 上 API 现取的码 == 用户客户端显示的码；重连换码由「现取比对」自然覆盖。
4. 只在 **TG 管理员已通过 TOTP** 后接受回贴（未授权消息在 `check_auth` 已被丢弃）。
5. 码格式跨行、分组长度不定（例见 §2），比对前**去掉全部空白**只比数字序列。

## 1. 目标

- **G1**：授权管理员的 TG 文本消息若等于当前 SimpleX 联系人安全码 → 标记「已验证」并回复成功。
- **G2**：标记持久化，重启后仍有效；`simplex_admin_id` 变更（重钉/换联系人）后自动回到未验证。
- **G3**：已验证后，TOTP 成功不再向 TG 发送 `simplex.security_code` 消息。
- **G4**：不命中安全码判据的普通消息行为**逐条不变**；纯 `--simplex`（无 TG 管理员）行为不变。

## 2. 事实（已核实，CodeGraph + 源码）

| 事实 | 位置/证据 |
| --- | --- |
| TOTP 成功后取码发 TG，**无任何确认/持久化** | `app/auth.rs:60-90` |
| 取码：`/_get code @<contactId>` → `resp.connectionCode` | `gateways/simplex/adapter.rs:208-228, 339-347` |
| 入站文本走 `dispatch_event`：destruct 拦截 → `check_auth` → `handle_message` | `shared/dispatch.rs:18-71, 129` |
| TOTP 判据：6 位 ASCII 数字 | `shared/dispatch.rs:106` |
| 持久化先例（明文 JSON + `save()`） | `bootstrap.rs:123-155` `BotSettings` |
| 配置装载 | `bootstrap.rs:134` `BotSettings::load()`；`main/config.rs:124,136`；`main.rs:124-140` |
| 超时设置会整体重写 BotSettings（需防覆盖新字段） | `shared/state_ops.rs:37-41` |
| 用户回贴示例（跨行、分组长度不定） | `54440 24092 64994 96911` / `44406 17421 40954 43544` / `07533 47951 41247 72875` / `13251 04366 70466 51` |

> 注：用户示例跨 4 行、每组 5 位（末行 3 位）；既有单测样例 20 位（`adapter.rs:784`）。因此**不假设固定长度**，只要求「纯数字+空白」且数字位数 >= 12。

## 3. 设计

### 3.1 纯函数（TDD 主战场，放 `shared/dispatch.rs`）

```rust
/// 归一化：仅保留 ASCII 数字，去掉空格/换行/制表符。两端都归一化后比较。
fn normalize_security_code(s: &str) -> String;

/// 是否是「疑似安全码」：归一化后非空、原文只含数字与空白、数字位数 >= 12。
/// 用于决定 mismatch 时是否回复，而非静默当作普通消息。
fn looks_like_security_code(s: &str) -> bool;
```

比较一律 `normalize(pasted) == normalize(fetched)`；常量 `MIN_CODE_DIGITS: usize = 12`。

### 3.2 持久化（`bootstrap.rs`）

`BotSettings` 增字段（向后兼容，旧文件缺字段取默认）：

```rust
#[serde(default)]
pub simplex_code_verified_for: Option<i64>, // 已验证的 contactId；None = 未验证
```

- 有效条件：`simplex_code_verified_for == state.simplex_admin_id()` 才算已验证；重钉后自然失配 → 回到未验证（G2），**无需显式清零**。
- `state_ops.rs:37-41` 的超时写入改为**读-改-写**（`load()` → 只改 `session_timeout_secs` → `save()`），避免抹掉新字段。
- 新增 `bootstrap::set_simplex_code_verified(config_dir, contact_id: Option<i64>)`（load→set→save），手法与 `set_simplex_admin_id` 一致但**不加密**（contactId 非机密，沿用 `BotSettings` 明文 JSON）。

### 3.3 运行时状态（`app/state.rs` + `main.rs`）

- `AppState` 增 `simplex_code_verified_for: AtomicI64`（0 = 未验证），`new()` 默认 0。
- 新增 `#[must_use] pub fn with_simplex_code_verified_for(mut self, id: Option<i64>) -> Self`。
- 新增 `pub fn simplex_code_verified(&self) -> bool`（读当前 `simplex_admin_id` 比对）与 `pub fn set_simplex_code_verified_for(&self, id: Option<i64>)`。
- `main.rs:124-140`：在 `.with_simplex_repin()` 旁追加 `.with_simplex_code_verified_for(app_config.bot_settings.simplex_code_verified_for)`。

### 3.4 出站门控（`app/auth.rs`）

TOTP 成功块里的安全码发送追加判据：`&& !state.simplex_code_verified()`。已验证 → 完全跳过取码与发送（G3）。

### 3.5 入站确认（`shared/dispatch.rs`）

`dispatch_event` 的 `BotEvent::Message` 分支，在 `handle_message` **之前**插入：

```rust
if try_confirm_security_code(&msg, state).await? { return Ok(()); }
```

`try_confirm_security_code` 逻辑：

1. `state.simplex_code_verified()` 为真 → 返回 false（放行普通处理）。
2. `state.simplex_admin_id()` / `state.admin_id()` 任一为 None → false（纯 `--simplex` 不介入，G4）。
3. 取 `msg.text`：`looks_like_security_code` 为假 → false（普通消息不变，G4）。
4. `state.adapter.contact_security_code(sx_admin)` 现取；失败 → `log::warn!` 后返回 false（fail-open）。
5. `normalize(text) == normalize(fetched)`：
   - **一致**：`state.set_simplex_code_verified_for(Some(sx_admin))` + `bootstrap::set_simplex_code_verified(...)`（落盘失败仅 `log::error!`，内存态保留）；回复 `simplex.code_verified`；返回 true。
   - **不一致**：回复 `simplex.code_mismatch`，`log::warn!`；返回 true（**不**落盘）。
6. 回复一律强制 `send_message_primary`（与既有安全码路径一致，避免被敏感分流改投 SimpleX）。

### 3.6 i18n

`simplex.security_code` 文案由「逐段比对」改为「请在客户端复制后直接发到这里自动验证」（zh/en/ja）。
新增 `simplex.code_verified`、`simplex.code_mismatch`（zh/en/ja）。
`tests/hy2_i18n_parity.rs` 把三语文件当契约，三语必须齐全。

## 4. 边界与风险

1. **fail-open**：取码失败、落盘失败都不阻塞 TOTP 验证结果，也不打断普通消息处理。
2. **不误吞消息**：只有「未验证 + 有 TG 管理员 + 纯数字空白 + >=12 位」才进入比对；其余逐条不变。
3. **不调 `/_verify`**：客户端提示是否消失不在本次范围（用户选 A）。
4. **无速率限制**：mismatch 不额外限流（沿用既有全局 TOTP 限流模型）；如需再加。
5. **不新增 CLI flag、不改 `RoutingAdapter` 能力表、不改 `BotAdapter` 签名。**

## 5. 交付拆分（strict）

| # | 内容 | 依赖 | 文件 |
| --- | --- | --- | --- |
| S1 | `BotSettings.simplex_code_verified_for` + 读改写修复 + 单测 | — | `bootstrap.rs`, `shared/state_ops.rs` |
| S2 | `AppState` 字段/方法 + `main.rs` 接线 | S1 | `app/state.rs`, `main.rs` |
| S3 | 出站门控（已验证不再发码）单测 | S2 | `app/auth.rs` |
| S4 | `normalize_security_code` / `looks_like_security_code` 纯函数 + 单测 | — | `shared/dispatch.rs` |
| S5 | `try_confirm_security_code` 入站确认 + 单测（一致/不一致/普通消息/无 TG） | S2,S4 | `shared/dispatch.rs` |
| S6 | i18n 三语（改 `security_code`，加 2 key） | S3,S5 | `resources/i18n/{zh,en,ja}.yml` |
| S7 | 质量门 + 真机验收 + REVIEW | 全部 | 无（验证） |

S1 与 S4 互相独立，可并行；S3/S5 依赖 S2；S6 依赖 S3/S5。

## 6. 验收标准

**命令**
```bash
cargo fmt && \
cargo clippy --all-targets --all-features -- -D warnings && \
cargo nextest run --cargo-profile fast-test && \
cargo test --doc
```

**可测条件**
- [ ] `normalize_security_code("54440 24092\n64994") == "544402409264994"`；`looks_like_security_code` 对纯数字/跨行真，对含字母/命令/6 位 TOTP(阈值外)/空假。
- [ ] 授权管理员 + 未验证 + 回贴等于 API 码 → `simplex_code_verified()==true`、落盘字段写入、回复 `simplex.code_verified`、后续普通处理被短路。
- [ ] 回贴不等 → 不落盘、回复 `simplex.code_mismatch`、`simplex_code_verified()` 仍 false。
- [ ] 已验证后再次 TOTP 成功：`contact_security_code` 调用次数 == 0、`send_message_primary` 安全码调用次数 == 0。
- [ ] `simplex_admin_id` 从 5 改为 7 后，`simplex_code_verified()` == false（重钉即失效）。
- [ ] 超时设置保存后 `simplex_code_verified_for` 不被抹掉（读改写回归）。
- [ ] 纯 `--simplex`（`admin_id == None`）不进入确认路径。

**真机**
- [ ] `--tg-simplex`：TOTP 后 TG 收到安全码 → 从 SimpleX 客户端复制码贴回 TG → 收到"已验证"。
- [ ] 重启后再次 TOTP 成功，TG **不再**收到安全码消息。
- [ ] 贴错码收到不一致提示，且下次仍会正常发码。

## 7. 未决 / 后续

- 是否让 SimpleX 客户端提示消失：需 `/_verify code @<id> <code>`，其响应格式未在本地 vendored 源码确认；确认后可作为独立 spec（B 方案）。
- mismatch 是否需要独立限流：待真机观察攻击面后再定。
