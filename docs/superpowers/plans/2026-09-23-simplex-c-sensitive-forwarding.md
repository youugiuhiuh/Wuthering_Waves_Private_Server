# 计划：C —— SimpleX 敏感文本转附件 + 路由层判定统一（v2）

> 依据：`docs/superpowers/specs/2026-09-23-simplex-sensitive-file-forwarding-design.md`（v2，已批准）
> 模式：strict ｜ 分支：`docs/simplex-c-revision` ｜ 门禁：见 §3

## 1. 任务分解（原子、可独立验证）

| # | 任务 | 精确文件 | 验收标准 | 验证 |
| --- | --- | --- | --- | --- |
| **C1** | SimpleX 敏感文本改投 `.txt` 附件，判定对象为**渲染后文本** | `rust/aegis/src/gateways/simplex/adapter.rs` | `send_message` 命中 `is_sensitive(outgoing_text(content))` 时走 `send_file("batch_result.txt", …, "text/plain")`，否则照旧发文本 | 新增 4 个纯函数单测（见 §2）；`cargo nextest run -p aegis simplex` |
| **C1b** | `RoutingAdapter::send_message` 判定改用渲染后文本 | `rust/aegis/src/common/routing.rs` | 敏感内容**只存在于按钮 data** 时也改投 secondary | 新增 1 个 mockall 单测（RED 证明现状漏判）；既有分流测试保持绿 |
| **C1c** | 对齐 `PlatformCapabilities::MATRIX` 常量 | `rust/aegis/src/common/trait.rs` | `can_send_typing`/`can_send_reaction`/`can_thread` 改为 `true`，与 `MatrixAdapter::capabilities()` 一致 | `cargo nextest run -p aegis trait` + 全量 |
| **C3a** | 全量门禁 | — | fmt / clippy / nextest / doc 全绿 | 见 §3 |
| **C3b** | 真机验收 | murky-pull | SimpleX 客户端收到 `.txt` **附件**（非普通消息）；`chat_items` 里敏感链接不再是 text item | 部署 + `chat_items` 只读查询 |
| **C3c** | 文档勘误 | 设计文档 | 实现后回填「已实现/差异」 | — |

**已撤销**：C2（文件类分流）—— `send_file`/`send_image`/`send_voice` 生产调用点为 0。

## 2. 测试设计（TDD：先 RED）

### C1（纯函数，因为 SimpleX 发送路径需活体 simplex-chat，仓库既有测试也只覆盖纯函数）

| 测试 | 断言 |
| --- | --- |
| `outgoing_text_renders_markup_buttons_into_body` | 按钮 `data` 出现在渲染结果里 |
| `sensitive_plain_text_is_flagged` | `is_sensitive(vless://…) == true` |
| `sensitive_only_in_button_data_is_flagged` | **关键**：`content.text` 干净、按钮 `data` 为 `vless://…` → 判定为敏感 |
| `normal_text_is_not_flagged` | 普通文本不误判 |

### C1b（mockall：`MockBotAdapter` 已就绪）

| 测试 | 断言 |
| --- | --- |
| `sensitive_only_in_button_data_uses_secondary` | secondary 被调用 1 次、primary 0 次 |
| 既有 `sensitive_uses_secondary_target_override` 等 | 保持绿 |

## 3. 门禁（每个任务完成后必跑）

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo nextest run --cargo-profile fast-test
cargo test --doc
```

## 4. 已知覆盖缺口（记录，不在本计划修）

- `SimpleXAdapter::send_message` 的**发送接线**无单测（需活体 WS）。C1 用纯函数覆盖判定逻辑，接线由真机验收（C3b）兜底。
- `PlatformCapabilities` 字段在生产代码中无读取点（只写不读），故 C1c 无运行时影响。

## 5. 提交切分

1. `docs(simplex)`: 计划（本文件）
2. `fix(simplex)`: C1 —— 敏感文本转附件（含 RED→GREEN 单测）
3. `fix(routing)`: C1b —— 判定改用渲染后文本
4. `chore(capabilities)`: C1c —— MATRIX 常量对齐
5. PR → 全量门禁 → 真机验收（C3b）
