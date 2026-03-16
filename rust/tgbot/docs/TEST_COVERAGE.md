# 测试覆盖说明

## 结论：是的，存在覆盖缺口并导致了回归

当前测试套件**没有完全覆盖**关键路径，尤其是 **CLI 与进程边界**、**bootstrap 与 main 的调用顺序**，因此出现了「TOTP 整段 stdout 被当密钥」的回归。补上 CLI 端到端测试并文档化缺口后，同类回归可被拦住。

---

## 覆盖缺口与回归风险一览

| 区域 | 已有覆盖 | 缺口 | 回归风险 |
|------|----------|------|----------|
| **CLI 输出边界** | 有 | `--generate-totp-secret` 的 stdout 未断言 | ✅ 已补：`cli_generate_totp_stdout.rs` |
| **CLI 其它** | 有 | `-v`/`--version`、`--setup`、`--setup-stdin` 的 stdout/行为 | ✅ 已补：`cli_generate_totp_stdout`（-v）、`cli_setup_stdout`、`cli_setup_stdin`、`cli_verify_integrity_no_dir` |
| **main 执行顺序** | 有 | 谁先跑（verify_integrity vs CLI 分支）无测试 | ✅ 通过「stdout 仅一行」间接保障 |
| **bootstrap** | 有 | `run_setup` / `run_setup_from_stdin` / `verify_integrity` | ✅ 已补：setup 往返、setup-stdin E2E、verify_integrity 目录不存在 |
| **config 与 key 配对** | 有 | 「config.enc 存在且 .key 不存在则 bail」 | ✅ 已补：`cli_config_key_missing.rs` |
| **TotpManager** | 有 | 换行/trim/base32 边界已覆盖 | 低 |
| **SecurityManager** | 有 | 加解密往返、同 key 解密已覆盖 | 低 |
| **自毁流程** | 有 | 状态机、擦除、E2E 已覆盖 | 低 |
| **调度器** | 有 | cron 校验、任务校验、add 失败不持久化 | 低 |
| **Config/Reality** | 有 | 部分单元测 | 中：批量创建、SNI、多协议路径仍可加 |

---

## 为何「TOTP 整段 stdout 被当密钥」没被测试捕获？

### 问题回顾

- 安装器执行 `tgbot --generate-totp-secret` 并把**整段 stdout** 当作 TOTP 密钥写入配置。
- 当时 main 先执行 `verify_integrity()`（向 stdout 打印 `Binary Integrity Hash: ...`），再处理 `--generate-totp-secret`，导致 stdout 有两行，写入的是无效「密钥」。
- 启动时解密得到带前缀的字符串，base32 解码失败 → 「无效的 TOTP 密钥」。

### 已有测试为何没发现

| 测试 | 覆盖范围 | 未覆盖到的 |
|------|----------|------------|
| **integration_totp_trim.rs** | 给定字符串（如 `secret + "\n"`）trim 后 TotpManager 能否接受 | 不跑二进制、不检查**真实 stdout** 是否只有一行 |
| **logic/totp 单元测试** | `TotpManager::new`、`generate_new_secret`、带换行/非法 base32 的边界 | main() 的执行顺序、CLI 输出边界 |
| **integration_security.rs** | SecurityManager 加解密往返 | 与 TOTP/CLI 无关 |

也就是说：**没有任何测试会真正执行 `tgbot --generate-totp-secret` 并断言 stdout 内容**。bug 出在「进程边界 + main 里调用顺序」，只有对二进制做 CLI 端到端测试才能发现。

### 补上的测试

- **tests/cli_generate_totp_stdout.rs**：在集成测试里用 `CARGO_BIN_EXE_tgbot` 执行 `tgbot --generate-totp-secret`，断言：
  - stdout **仅有一行**；
  - 该行不为空、不含 `Binary Integrity Hash`、为合法 Base32（A–Z2–7）、长度 ≥ 16。

这样以后若有人把 `verify_integrity()` 或其它打印挪到 `--generate-totp-secret` 之前，或往该路径加 stdout 输出，测试会失败。

### 可补充的其它方向

- **安装器端**：Go 安装器里对 `runCmdOutputBytes(binaryPath, "--generate-totp-secret")` 的结果做「仅取最后一行 / 仅取合法 base32 行」的断言或单测（当前已用 `extractBase32Secret` 做兼容，可再为它写单元测试）。
- **setup 往返**：若需要可加「临时目录 + run_setup + 启动时加载 config 并建 TotpManager」的集成测试，覆盖「写入的密钥能被正常解密并初始化 TOTP」；当前主要靠 CLI stdout 测试和 trim 集成测试覆盖解密后的行为。

---

## 建议优先补的测试（降低回归）

1. **CLI `-v` / `--version`**：执行 `tgbot -v`，断言 stdout 仅一行且包含 `tgbot 0.0.x`，避免版本号与其它日志混入 stdout。  
   → **已补**：`tests/cli_generate_totp_stdout.rs` 中 `cli_version_stdout_is_single_line`。
2. **config.enc 存在且 .key 不存在**：在临时目录只写 config.enc、不写 .key，运行二进制（不传 --setup），断言进程失败且 stderr 包含「.key 不存在」类提示。  
   → **已补**：`tests/cli_config_key_missing.rs`（通过环境变量 `TGBOT_CONFIG_DIR` 指定临时目录）。
3. **setup 往返**：临时目录内执行 `run_setup`（或 `tgbot --setup ...`），再在同一目录用 SecurityManager 解密并 trim 后建 TotpManager，断言成功。  
   → **已补**：`tests/integration_setup_roundtrip.rs` 中 `setup_roundtrip_decrypt_and_totp_manager_succeeds`。
4. **安装器 extractBase32Secret**：Go 侧为 `extractBase32Secret` 写表驱动单测（多行含 hash、仅一行、空、无合法行等），保证解析逻辑不倒退。  
   → **已补**：`go/installer/main_test.go` 中 `TestExtractBase32Secret`。

---

## 后续已补的测试（CLI / bootstrap 细化）

- **`--setup` 成功时 stdout 仅一行**：`tests/cli_setup_stdout.rs` → `cli_setup_success_stdout_is_single_line_with_success_message`，防止 setup 输出污染管道。
- **`--setup-stdin` 端到端**：`tests/cli_setup_stdin.rs` → `cli_setup_stdin_creates_key_and_config_enc`，用 `TGBOT_CONFIG_DIR` + stdin JSON 断言生成 `.key` 与 `config.enc`。
- **verify_integrity 目录不存在**：`tests/cli_verify_integrity_no_dir.rs` → `cli_verify_integrity_fails_with_stderr_when_config_dir_missing`，断言 exit(1) 且 stderr 含「配置文件目录不存在」类提示。
