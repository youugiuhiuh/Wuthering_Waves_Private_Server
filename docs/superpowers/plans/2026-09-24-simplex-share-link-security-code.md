# 计划：TG+SimpleX 分享链接 + 安全码输出

> 依据：`docs/superpowers/specs/2026-09-24-simplex-share-link-and-security-code-design.md`
> 分支：`feat/simplex-link-security-code` ｜ 门禁见 §3

## 1. 任务

| # | 任务 | 文件 | 验收标准 | 验证 |
| --- | --- | --- | --- | --- |
| **S1** | `parse_contact_code` 纯函数 + `BotAdapter::contact_security_code` 默认方法 + `SimplexAdapter` 实现（走 `client().send_raw("/_get code @<id>")`） | `common/trait.rs`、`gateways/simplex/adapter.rs` | 合法 JSON 取出 `resp.connectionCode`；错误响应/缺字段/坏 JSON 均报错 | 4 个纯函数单测（RED→GREEN） |
| **S2** | TOTP 成功后（来源=SimpleX 且存在 TG 管理员）把安全码发到 TG（`send_message_primary`） | `app/auth.rs` | TG 收到含安全码的消息；纯 simplex（无 TG admin）不发；取码失败只 warn | mockall 单测 ×2 |
| **S3** | `SimplexHandle.address` + 启动时向 TG 发分享链接 | `main/simplex.rs`、`main/runtime.rs` | TG+SimpleX 启动后 TG 收到含 SimpleX 地址的消息；纯 simplex 不发 | 判据函数单测 |
| **S4** | 两条文案 i18n 三语齐全 | `resources/i18n/{zh,en,ja}.yml` | parity 测试通过 | `cargo nextest run --test hy2_i18n_parity` |
| **S5** | 门禁 + PR + 真机验收 | — | 见 SPEC §6 | 见 §3 |

## 2. 测试设计（先 RED）

### S1（纯函数）

| 测试 | 断言 |
| --- | --- |
| `parse_contact_code_extracts_connection_code` | 取出 `"52075 05398 …"` |
| `parse_contact_code_rejects_error_response` | `chatCmdError` → Err |
| `parse_contact_code_rejects_missing_code` | 无 `connectionCode` / 空串 → Err |
| `parse_contact_code_rejects_invalid_json` | 非 JSON → Err |

### S2（mockall）

| 测试 | 断言 |
| --- | --- |
| `totp_from_simplex_sends_security_code_to_tg` | `state.adapter` 收到 `send_message_primary(target=tg_admin)` 且文本含安全码 |
| `no_tg_admin_means_no_code_send` | 同上场景但 `admin_id=None` → `send_message_primary` 调用 0 次 |

### S3（判据纯函数）

| 测试 | 断言 |
| --- | --- |
| `share_link_only_when_tg_and_simplex` | `(true, Some(addr))` → true；`(true, None)` / `(false, _)` → false |

## 3. 门禁

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo nextest run --cargo-profile fast-test
cargo test --doc
```

## 4. 提交切分

1. `docs(simplex)`: SPEC + 计划
2. `feat(simplex)`: S1 安全码取值
3. `feat(simplex)`: S2 TOTP 后输出安全码到 TG
4. `feat(simplex)`: S3 启动分享链接
5. `chore(i18n)`: S4 三语文案
6. PR → 门禁 → 真机验收
