---
name: tgbot-testing
description: >
  rust/tgbot 与 go/installer 的测试规范与覆盖说明。在编写或补充 CLI、bootstrap、SecurityManager、TOTP、安装器相关测试时使用。包含 TGBOT_CONFIG_DIR 用法、测试文件分布与回归防护要点。
---

# tgbot 测试规范与覆盖

本 Skill 说明 **rust/tgbot** 与 **go/installer** 的测试约定、已覆盖路径与补充测试时的做法。

## 何时查阅

- 要补「CLI 或 bootstrap」相关测试时，先看本文与 `rust/tgbot/docs/TEST_COVERAGE.md`。
- 要跑/调试某类测试时，用下文「测试命令」。
- 要保证「进程边界 / stdout / 配置目录」行为不倒退时，按「回归防护要点」加断言。

## 环境变量：TGBOT_CONFIG_DIR

- **用途**：集成测试中让 `tgbot` 使用临时目录作为配置目录，避免写 `/etc/wwps/tgbot`。
- **实现**：`rust/tgbot/src/bootstrap.rs` 中 `config_dir()` 优先读 `TGBOT_CONFIG_DIR`，未设置则用常量 `CONFIG_DIR`。
- **用法**：在测试里 `Command::new(bin).env("TGBOT_CONFIG_DIR", temp_dir.path()).args([...]).output()`。
- **注意**：仅测试时使用；生产不设置该变量。

## 测试文件分布（rust/tgbot/tests）

| 文件 | 覆盖内容 |
|------|----------|
| **cli_config_key_missing.rs** | config.enc 存在、.key 不存在 → 进程失败，stderr 含「.key」或「不存在」 |
| **cli_generate_totp_stdout.rs** | `--generate-totp-secret` stdout 仅一行合法 base32；`-v` stdout 仅一行含 tgbot |
| **cli_setup_stdout.rs** | `--setup` 成功时 stdout 仅一行且含「Setup completed」或「✅」 |
| **cli_setup_stdin.rs** | `--setup-stdin` 从 stdin 读 JSON，断言生成 .key 与 config.enc |
| **cli_verify_integrity_no_dir.rs** | 配置目录不存在时 exit(1)，stderr 含「配置文件目录不存在」类提示 |
| **integration_security.rs** | SecurityManager 创建 .key、加解密往返、同 key 解密 |
| **integration_setup_roundtrip.rs** | `tgbot --setup` 后同目录解密并 trim，建 TotpManager 成功 |
| **integration_totp_trim.rs** | TOTP 密钥带换行/空格时 trim 后 TotpManager 接受 |
| **test_self_destruct.rs** / **test_self_destruct_e2e.rs** | 自毁状态机、擦除、E2E |

## Go 安装器测试

- **go/installer/main_test.go**：`TestExtractBase32Secret` 表驱动单测，覆盖多行含 hash、仅一行 base32、空、无合法行、trim、不足 16 位等，防止 TOTP 输出解析逻辑倒退。

## Reality / XHTTP 双栈分离测试要点

- **IpVersion 模式**：
  - `IPv4` / `IPv6`：只使用单栈 IP 生成 Reality + XHTTP 配置与链接。
  - `SplitStackV6Primary`：主地址使用 IPv6，`extra.downloadSettings.address` 使用 IPv4，对应“v6 上 v4 下”。
  - `SplitStackV4Primary`：主地址使用 IPv4，`extra.downloadSettings.address` 使用 IPv6，对应“v4 上 v6 下”。
- **建议增加的测试（示例）**：
  - 单元测 `resolve_public_hosts`：对两种 split 模式断言 `(primary, secondary)` 顺序正确。
  - 链接生成测试：检查 XHTTP 链接中 `remote-host` 与 `extra.downloadSettings.address` 是否符合上述语义。

## 回归防护要点

1. **CLI 仅输出模式**：凡会进入「仅 stdout 输出」的路径（如 `-v`、`--generate-totp-secret`、`--setup` 成功），都应断言 **stdout 行数（通常为 1）与关键内容**，避免混入 `Binary Integrity Hash` 等导致安装器或管道误用。
2. **config 与 .key 配对**：若只存在 config.enc 而无 .key，必须 bail 并提示；已有 `cli_config_key_missing.rs` 覆盖。
3. **setup 写入可被正常加载**：`run_setup` / `tgbot --setup` 写入的 config.enc 能被 SecurityManager 解密且 trim 后建 TotpManager；已有 `integration_setup_roundtrip.rs` 与 `cli_setup_stdin` 覆盖。
4. **verify_integrity**：配置目录不存在时应 exit(1) 且 stderr 有明确提示；已有 `cli_verify_integrity_no_dir.rs`。

## 测试命令

```bash
# 运行所有 tgbot 测试
cd rust/tgbot && cargo test

# 仅 CLI 相关
cargo test cli_

# 仅集成（security / setup / totp）
cargo test integration_

# Go 安装器 extractBase32Secret
cd go/installer && go test -v -run TestExtractBase32Secret
```

## 参考文档

- **rust/tgbot/docs/TEST_COVERAGE.md**：覆盖缺口表、建议优先补的测试（均已补）、后续已补列表与原因说明。
