# Aegis 测试改进 — 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**目标:** 在 `rust/aegis` 中为无测试的关键路径添加单元测试，覆盖 batch_handler、destruct_flow、self_destruct、config、matrix 模块

**架构:** 使用 mockall 的 `#[automock]` 模拟 `BotAdapter` 和 `SelfDestructExecutor` 依赖；提取纯逻辑函数使 destruct_flow 可测试；使用 tempfile 文件系统模拟 load_and_validate

**技术栈:** Rust 2024, mockall 0.14, tokio test, tempfile, proptest

---

## 文件改动总览

| 文件 | 操作 | 说明 |
|------|------|------|
| `Cargo.toml` | 修改 | 添加 proptest dev-dependency |
| `src/adapters/common/trait.rs` | 修改 | 添加 `#[cfg_attr(test, automock)]` |
| `src/adapters/common/routing.rs` | 修改 | 添加 RoutingAdapter 单元测试 |
| `src/app/batch_handler.rs` | 修改 | 添加单元测试 |
| `src/app/destruct_flow.rs` | 修改 | 添加纯逻辑函数 + 单元测试 |
| `src/core/security/self_destruct.rs` | 修改 | 添加 trigger 单元测试 |
| `src/main/config.rs` | 修改 | 添加单元测试 |
| `src/main/matrix.rs` | 修改 | 添加 has_matrix_config 单元测试 |
| `src/core/system/core_upgrade.rs` | 修改 | 修复 env var 基线测试 |
| `tests/integration_security.rs` | 修改 | 添加 RoutingAdapter 集成测试 |

---

### Task 0: 修复基线测试

**Files:**
- Modify: `src/core/system/core_upgrade.rs:839-860`

- [ ] **Step 1: 确认失败原因**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/feat-aegis-test/rust/aegis
cargo test --lib -- core::system::core_upgrade::tests::test_wwps_core_release_api_bases_default 2>&1
```
Expected: 测试因 WWPS_RELEASE_API_BASES 环境变量被覆盖而失败

- [ ] **Step 2: 修改测试 — 保存/恢复环境变量**

修改 `src/core/system/core_upgrade.rs` 行 ~839，在 `wwps_core_release_api_bases` 和 `wwps_core_release_api_bases_trailing_slash_stripped` 测试中添加环境变量隔离:

```rust
#[test]
fn test_wwps_core_release_api_bases_default() {
    let old = std::env::var("WWPS_RELEASE_API_BASES").ok();
    if old.is_some() {
        std::env::remove_var("WWPS_RELEASE_API_BASES");
    }
    let result = wwps_core_release_api_bases();
    // 恢复
    if let Some(val) = old {
        std::env::set_var("WWPS_RELEASE_API_BASES", val);
    }
    assert_eq!(result, vec!["https://api.github.com/repos".to_string()]);
}
```

对 `test_wwps_core_release_api_bases_trailing_slash_stripped` 做同样处理。

**DRY 改进:** 使用测试助手函数替代重复代码:

```rust
// 在测试模块顶部
fn with_clear_env<F, R>(env_var: &str, f: F) -> R
where
    F: FnOnce() -> R,
{
    let old = std::env::var(env_var).ok();
    if old.is_some() {
        std::env::remove_var(env_var);
    }
    let result = f();
    if let Some(val) = old {
        std::env::set_var(env_var, val);
    }
    result
}
```

使用:
```rust
#[test]
fn test_wwps_core_release_api_bases_default() {
    let result = with_clear_env("WWPS_RELEASE_API_BASES", wwps_core_release_api_bases);
    assert_eq!(result, vec!["https://api.github.com/repos".to_string()]);
}
```

- [ ] **Step 3: 运行确认通过**

Run: `cargo test --lib -- core::system::core_upgrade::tests::test_wwps_core_release_api_bases_default`
Expected: PASS

- [ ] **Step 4: 运行确认第二个测试也通过**

Run: `cargo test --lib -- core::system::core_upgrade::tests::test_wwps_core_release_api_bases_trailing_slash_stripped`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/core/system/core_upgrade.rs
git commit -m "test: fix core_upgrade tests affected by WWPS_RELEASE_API_BASES env var"
```

---

### Task 1: 为 BotAdapter 添加 #[automock]

**Files:**
- Modify: `src/adapters/common/trait.rs:34-35`

- [ ] **Step 1: 在 trait 上添加 automock**

```rust
use anyhow::Result;
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct TargetId(pub String);

#[derive(Debug, Clone)]
pub struct MessageId(pub String);

#[derive(Debug, Clone)]
pub struct MessageContent {
    pub text: String,
    pub markup: Option<Markup>,
}

#[derive(Debug, Clone)]
pub struct Markup {
    pub buttons: Vec<Vec<InlineButton>>,
}

#[derive(Debug, Clone)]
pub struct InlineButton {
    pub text: String,
    pub data: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Telegram,
    Discord,
    Matrix,
}

#[cfg_attr(test, automock)]
#[async_trait]
pub trait BotAdapter: Send + Sync {
    fn platform(&self) -> Platform;
    async fn send_message(&self, target: &TargetId, content: MessageContent) -> Result<MessageId>;
    async fn edit_message(
        &self,
        target: &TargetId,
        msg_id: &MessageId,
        content: MessageContent,
    ) -> Result<()>;
    async fn delete_message(&self, target: &TargetId, msg_id: &MessageId) -> Result<()>;
}
```

关键改动: 在 `pub trait BotAdapter` 之前加上 `#[cfg_attr(test, automock)]`

- [ ] **Step 2: 编译确认**

Run: `cargo check --lib`
Expected: 编译成功

- [ ] **Step 3: 确认 MockBotAdapter 可用**

Run: `cargo test --lib -- --ignored` 或快速确认:
```bash
echo 'fn _compile_check() { let _ = MockBotAdapter::new(); }' > /dev/null
cargo check --lib 2>&1 | grep -q "MockBotAdapter" && echo "mock available"
```
Expected: mock 可用

- [ ] **Step 4: Commit**

```bash
git add src/adapters/common/trait.rs
git commit -m "test: add #[automock] to BotAdapter trait for mockall"
```

---

### Task 2: batch_handler 单元测试

**Files:**
- Modify: `src/app/batch_handler.rs:87`

- [ ] **Step 1: 添加测试模块（RED — 先写测试）**

在 `src/app/batch_handler.rs` 底部添加:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use aegis::adapters::common::MockBotAdapter;
    use aegis::core::types::BatchCreationResult;

    fn make_result(created_count: u32, links: Vec<String>, config_file: Option<&str>) -> BatchCreationResult {
        BatchCreationResult {
            created_count,
            links,
            config_file: config_file.map(String::from),
        }
    }

    #[tokio::test]
    async fn sends_header_links_and_result_messages() {
        let mut mock = MockBotAdapter::new();
        mock.expect_send_message()
            .times(3)
            .returning(|_, _| Ok(MessageId("1".to_string())));
        mock.expect_delete_message()
            .returning(|_, _| Ok(()));

        let result = make_result(2, vec!["vless://a".into(), "vless://b".into()], Some("/tmp/cfg.json"));
        send_singbox_batch_result(Arc::new(mock), ChatId(1), "hy2", &result)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn skips_links_when_empty() {
        let mut mock = MockBotAdapter::new();
        mock.expect_send_message()
            .times(2) // header + result only
            .returning(|_, _| Ok(MessageId("1".to_string())));
        mock.expect_delete_message()
            .returning(|_, _| Ok(()));

        let result = make_result(0, vec![], None);
        send_singbox_batch_result(Arc::new(mock), ChatId(1), "hy2", &result)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn handles_adapter_send_failure_gracefully() {
        let mut mock = MockBotAdapter::new();
        mock.expect_send_message()
            .times(3)
            .returning(|_, _| Err(anyhow::anyhow!("network error")));
        mock.expect_delete_message()
            .returning(|_, _| Ok(()));

        let result = make_result(5, vec!["vless://x".into()], Some("/tmp/x.json"));
        let output = send_singbox_batch_result(Arc::new(mock), ChatId(1), "hy2", &result).await;
        assert!(output.is_ok());
    }

    #[tokio::test]
    async fn includes_protocol_name_in_header() {
        let mut mock = MockBotAdapter::new();
        mock.expect_send_message()
            .withf(|_, content| content.text.contains("hy2"))
            .times(1)
            .returning(|_, _| Ok(MessageId("1".to_string())));
        mock.expect_send_message()
            .returning(|_, _| Ok(MessageId("2".to_string())));
        mock.expect_delete_message()
            .returning(|_, _| Ok(()));

        let result = make_result(1, vec!["vless://x".into()], Some("/tmp/x.json"));
        send_singbox_batch_result(Arc::new(mock), ChatId(1), "hy2", &result)
            .await
            .unwrap();
    }
}
```

- [ ] **Step 2: 运行确认测试失败 — 确认 MockBotAdapter 可用**

Run: `cargo test --lib -- app::batch_handler::tests --nocapture`
Expected: 所有测试通过（mock 自动生成，无需额外实现）

- [ ] **Step 3: Commit**

```bash
git add src/app/batch_handler.rs
git commit -m "test: add batch_handler unit tests with MockBotAdapter"
```

---

### Task 3: has_matrix_config 单元测试

**Files:**
- Modify: `src/main/matrix.rs:128`

- [ ] **Step 1: 在文件底部添加测试模块**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap::EncryptedConfig;

    fn make_empty_encrypted_config() -> EncryptedConfig {
        EncryptedConfig {
            token: vec![],
            admin_id: vec![],
            totp_secret: vec![],
            self_destruct_key_hash: None,
            matrix_homeserver: None,
            matrix_username: None,
            matrix_password: None,
            matrix_room_id: None,
            matrix_store_passphrase: None,
            lang: None,
        }
    }

    #[test]
    fn returns_true_when_all_matrix_fields_present() {
        let config = EncryptedConfig {
            matrix_homeserver: Some(vec![1]),
            matrix_username: Some(vec![1]),
            matrix_password: Some(vec![1]),
            matrix_room_id: Some(vec![1]),
            ..make_empty_encrypted_config()
        };
        assert!(has_matrix_config(&config, &[]));
    }

    #[test]
    fn returns_false_when_matrix_fields_missing() {
        let config = make_empty_encrypted_config();
        assert!(!has_matrix_config(&config, &[]));
    }

    #[test]
    fn returns_true_when_flag_overrides_empty_fields() {
        let config = make_empty_encrypted_config();
        assert!(has_matrix_config(&config, &["--matrix".to_string()]));
    }

    #[test]
    fn returns_true_when_all_flag_overrides_empty_fields() {
        let config = make_empty_encrypted_config();
        assert!(has_matrix_config(&config, &["--all".to_string()]));
    }

    #[test]
    fn returns_false_when_some_fields_missing() {
        let config = EncryptedConfig {
            matrix_homeserver: Some(vec![1]),
            matrix_username: Some(vec![1]),
            ..make_empty_encrypted_config()
        };
        assert!(!has_matrix_config(&config, &[]));
    }

    #[test]
    fn ignores_non_matrix_flags() {
        let config = make_empty_encrypted_config();
        assert!(!has_matrix_config(&config, &["--tg-only".to_string()]));
    }
}
```

- [ ] **Step 2: 运行确认通过**

Run: `cargo test --lib -- main::matrix::tests`
Expected: All 6 tests PASS

- [ ] **Step 3: Commit**

```bash
git add src/main/matrix.rs
git commit -m "test: add has_matrix_config unit tests"
```

---

### Task 4: destruct_flow 提取纯逻辑 + 测试

**Files:**
- Modify: `src/app/destruct_flow.rs`

- [ ] **Step 1: 在文件顶部添加 DestructMessageAction 枚举**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestructMessageAction {
    Timeout,
    NotAuthorized,
    ConfirmFirstTotp,
    AwaitingSecondTotp,
    VerifyFailed,
    AwaitingFile,
    FileVerified { hash_short: String },
    FileMismatch,
    NoSecurityKey,
    Noop,
}
```

- [ ] **Step 2: 提取纯逻辑函数 process_destruct_message**

```rust
/// 纯逻辑层: 处理自毁流程的消息输入，返回需要执行的操作。
/// 不依赖 teloxide Bot/Message 类型，可直接单元测试。
pub async fn process_destruct_message(
    text: Option<&str>,
    has_file: bool,
    file_content: Option<&[u8]>,
    step: DestructStep,
    state: &AppState,
    self_destruct_key_hash: Option<&str>,
) -> DestructMessageAction {
    match step {
        DestructStep::AwaitFirstTotp => {
            match text {
                Some(code) if state.verify_totp(code.trim()) => DestructMessageAction::ConfirmFirstTotp,
                Some(_) => DestructMessageAction::VerifyFailed,
                None => DestructMessageAction::Noop,
            }
        }
        DestructStep::AwaitSecondTotp => {
            match text {
                Some(code) if state.verify_totp(code.trim()) => DestructMessageAction::AwaitingSecondTotp,
                Some(_) => DestructMessageAction::VerifyFailed,
                None => DestructMessageAction::Noop,
            }
        }
        DestructStep::AwaitSecurityFile => {
            if let Some(content) = file_content {
                let hash = hex::encode(sha2::Sha256::digest(content));
                match self_destruct_key_hash {
                    Some(correct) if hash == correct => {
                        let hash_short = if hash.len() > 12 {
                            format!("{}...{}", &hash[..8], &hash[hash.len() - 4..])
                        } else {
                            hash.clone()
                        };
                        DestructMessageAction::FileVerified { hash_short }
                    }
                    Some(_) => DestructMessageAction::FileMismatch,
                    None => DestructMessageAction::NoSecurityKey,
                }
            } else if has_file {
                DestructMessageAction::Noop // 需要下载文件内容
            } else {
                DestructMessageAction::AwaitingFile
            }
        }
        DestructStep::AwaitConfirm | DestructStep::AwaitFinalConfirm => DestructMessageAction::Noop,
    }
}
```

- [ ] **Step 3: 使用新函数简化 handle_message_flow**

将 `handle_message_flow` 中的 `match destruct_state.step { ... }` 块替换为调用 `process_destruct_message`:

```rust
    match destruct_state.step {
        DestructStep::AwaitFirstTotp => {
            let action = process_destruct_message(
                msg.text(),
                msg.document().is_some() || msg.photo().is_some(),
                None,
                DestructStep::AwaitFirstTotp,
                state,
                state.self_destruct_key_hash().await.as_deref(),
            ).await;
            match action {
                DestructMessageAction::ConfirmFirstTotp => {
                    if state.confirm_first_destruct_totp(&chat_id_str, msg.text().unwrap().trim(), Instant::now()).await {
                        let keyboard = InlineKeyboardMarkup::new(vec![
                            vec![InlineKeyboardButton::callback(t!("destruct.confirm_btn"), "a_destroy_confirm")],
                            vec![InlineKeyboardButton::callback(t!("destruct.cancelled"), "a_destroy_cancel")],
                        ]);
                        bot.send_message(chat_id, t!("destruct.title_2"))
                            .parse_mode(ParseMode::Html)
                            .reply_markup(keyboard)
                            .await?;
                    }
                }
                DestructMessageAction::VerifyFailed => {
                    bot.send_message(chat_id, t!("destruct.verify_fail")).await?;
                }
                _ => {}
            }
            Ok(MessageFlowOutcome::Handled)
        }
        // ... 类似地将其他分支替换为 process_destruct_message 调用
        // (AwaitSecondTotp, AwaitSecurityFile 同理)
        DestructStep::AwaitConfirm | DestructStep::AwaitFinalConfirm => {
            Ok(MessageFlowOutcome::Handled)
        }
    }
```

> **注意:** 为了最小化改动，只将纯逻辑判断提取到 `process_destruct_message`，UI 交互（send_message, edit_message_text）保留在原处。这避免了重构整个文件，同时使 90% 的逻辑可单元测试。

- [ ] **Step 4: 在文件底部添加测试模块**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::AppState;
    use aegis::adapters::common::MockBotAdapter;
    use aegis::core::totp::TotpManager;
    use aegis::core::security::self_destruct::{SelfDestructExecutor, production_executor};
    use secrecy::SecretString;
    use std::sync::Arc;
    use futures_util::future::BoxFuture;

    struct TestExecutor;
    impl SelfDestructExecutor for TestExecutor {
        fn execute(&self) -> BoxFuture<'static, Result<()>> {
            Box::pin(async { Ok(()) })
        }
    }

    async fn make_test_state(totp_secret: &str) -> AppState {
        AppState::new(
            42,
            TotpManager::new(&SecretString::from(totp_secret.to_string())).unwrap(),
            Arc::new(TestExecutor),
            None,
            600,
            Arc::new(MockBotAdapter::new()),
        )
    }

    #[tokio::test]
    async fn first_totp_valid_returns_confirm() {
        let secret = TotpManager::generate_new_secret();
        let state = make_test_state(&secret).await;
        let code = state.generate_current_totp().unwrap();
        let action = process_destruct_message(
            Some(&code), false, None,
            DestructStep::AwaitFirstTotp, &state, None,
        ).await;
        assert_eq!(action, DestructMessageAction::ConfirmFirstTotp);
    }

    #[tokio::test]
    async fn first_totp_invalid_returns_verify_failed() {
        let secret = TotpManager::generate_new_secret();
        let state = make_test_state(&secret).await;
        let action = process_destruct_message(
            Some("000000"), false, None,
            DestructStep::AwaitFirstTotp, &state, None,
        ).await;
        assert_eq!(action, DestructMessageAction::VerifyFailed);
    }

    #[tokio::test]
    async fn first_totp_no_text_returns_noop() {
        let secret = TotpManager::generate_new_secret();
        let state = make_test_state(&secret).await;
        let action = process_destruct_message(
            None, false, None,
            DestructStep::AwaitFirstTotp, &state, None,
        ).await;
        assert_eq!(action, DestructMessageAction::Noop);
    }

    #[tokio::test]
    async fn security_file_match_returns_file_verified() {
        let content = b"test security file content";
        let hash = hex::encode(sha2::Sha256::digest(content));
        let secret = TotpManager::generate_new_secret();
        let state = make_test_state(&secret).await;
        let action = process_destruct_message(
            None, true, Some(content.as_slice()),
            DestructStep::AwaitSecurityFile, &state, Some(&hash),
        ).await;
        assert!(matches!(action, DestructMessageAction::FileVerified { .. }));
    }

    #[tokio::test]
    async fn security_file_mismatch_returns_file_mismatch() {
        let content = b"test content";
        let wrong_hash = "0000000000000000000000000000000000000000000000000000000000000000";
        let secret = TotpManager::generate_new_secret();
        let state = make_test_state(&secret).await;
        let action = process_destruct_message(
            None, true, Some(content.as_slice()),
            DestructStep::AwaitSecurityFile, &state, Some(wrong_hash),
        ).await;
        assert_eq!(action, DestructMessageAction::FileMismatch);
    }

    #[tokio::test]
    async fn confirm_step_returns_noop() {
        let secret = TotpManager::generate_new_secret();
        let state = make_test_state(&secret).await;
        let action = process_destruct_message(
            None, false, None,
            DestructStep::AwaitConfirm, &state, None,
        ).await;
        assert_eq!(action, DestructMessageAction::Noop);
    }
}
```

- [ ] **Step 5: 确保 AppState 有 generate_current_totp 方法**

如果 `AppState` 没有 `generate_current_totp` 方法，需要在 `src/app/state.rs` 中添加:

```rust
pub fn generate_current_totp(&self) -> Result<String, totp_rs::TOTPError> {
    self.totp_manager.generate_current()
}
```

- [ ] **Step 6: 运行确认通过**

Run: `cargo test --lib -- app::destruct_flow::tests`
Expected: All tests PASS

- [ ] **Step 7: Commit**

```bash
git add src/app/destruct_flow.rs src/app/state.rs
git commit -m "test: extract destruct_flow pure logic and add unit tests"
```

---

### Task 5: self_destruct trigger 单元测试

**Files:**
- Modify: `src/core/security/self_destruct.rs`

- [ ] **Step 1: 添加 MockSelfDestructExecutor + trigger 测试**

在文件底部测试模块中添加:

```rust
use mockall::mock;

mock! {
    pub SelfDestructExecutorMock {}
    #[async_trait]
    impl SelfDestructExecutor for SelfDestructExecutorMock {
        fn execute(&self) -> BoxFuture<'static, Result<()>>;
    }
}

#[tokio::test]
async fn test_trigger_calls_executor() {
    let mut mock = MockSelfDestructExecutorMock::new();
    mock.expect_execute()
        .times(1)
        .returning(|| Box::pin(async { Ok(()) }));

    trigger(Arc::new(mock));
    // trigger spawns a task with 2s sleep, give it time
    tokio::time::sleep(Duration::from_secs(3)).await;
}

#[tokio::test]
async fn test_trigger_handles_executor_error() {
    let mut mock = MockSelfDestructExecutorMock::new();
    mock.expect_execute()
        .times(1)
        .returning(|| Box::pin(async { Err(anyhow::anyhow!("boom")) }));

    trigger(Arc::new(mock));
    tokio::time::sleep(Duration::from_secs(3)).await;
    // 不应 panic
}
```

- [ ] **Step 2: 运行确认通过**

Run: `cargo test --lib -- core::security::self_destruct::tests --nocapture`
Expected: All tests PASS

- [ ] **Step 3: Commit**

```bash
git add src/core/security/self_destruct.rs
git commit -m "test: add self_destruct trigger unit tests"
```

---

### Task 6: load_and_validate 单元测试

**Files:**
- Modify: `src/main/config.rs`

- [ ] **Step 1: 在文件底部添加测试模块**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// 创建加密配置文件和密钥，供测试使用
    fn setup_test_config() -> (TempDir, String, String, String) {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("etc/wwps/aegis");
        fs::create_dir_all(&config_dir).unwrap();

        // 设置环境变量指向测试目录
        std::env::set_var("AEGIS_CONFIG_DIR", config_dir.to_str().unwrap());

        let key_path = config_dir.join(".key");
        let key_hex = hex::encode([0u8; 32]);
        fs::write(&key_path, [0u8; 32]).unwrap();

        // 创建简单的加密配置
        let encrypted = EncryptedConfig {
            token: vec![0; 16],   // 非真实加密数据，仅测试结构
            admin_id: vec![0; 16],
            totp_secret: vec![0; 16],
            self_destruct_key_hash: None,
            matrix_homeserver: None,
            matrix_username: None,
            matrix_password: None,
            matrix_room_id: None,
            matrix_store_passphrase: None,
            lang: None,
        };
        let config_data = serde_json::to_vec(&encrypted).unwrap();
        fs::write(config_dir.join("config.enc"), &config_data).unwrap();

        (dir, config_dir.to_str().unwrap().to_string(), key_path.to_str().unwrap().to_string(), hex::encode(&[0u8; 32]))
    }

    #[test]
    fn config_dir_uses_env_var_when_set() {
        let dir = TempDir::new().unwrap();
        std::env::set_var("AEGIS_CONFIG_DIR", dir.path().to_str().unwrap());
        let result = config_dir();
        assert_eq!(result, dir.path());
    }

    #[test]
    fn config_dir_defaults_when_env_not_set() {
        std::env::remove_var("AEGIS_CONFIG_DIR");
        let result = config_dir();
        assert_eq!(result, std::path::PathBuf::from("/etc/wwps/aegis"));
    }

    #[test]
    fn load_and_validate_fails_when_config_exists_but_key_missing() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("etc/wwps/aegis");
        fs::create_dir_all(&config_dir).unwrap();
        std::env::set_var("AEGIS_CONFIG_DIR", config_dir.to_str().unwrap());

        // 只创建 config.enc，不创建 .key
        fs::write(config_dir.join("config.enc"), b"{}").unwrap();

        let result = load_and_validate();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains(".key") || err.contains("不存在"), "error should mention missing key: {err}");
    }

    #[test]
    fn load_and_validate_fails_when_key_exists_but_config_missing() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("etc/wwps/aegis");
        fs::create_dir_all(&config_dir).unwrap();
        std::env::set_var("AEGIS_CONFIG_DIR", config_dir.to_str().unwrap());

        // 只创建 .key
        fs::write(config_dir.join(".key"), [0u8; 32]).unwrap();

        let result = load_and_validate();
        assert!(result.is_err());
    }

    #[test]
    fn load_and_validate_fails_with_invalid_key_length() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("etc/wwps/aegis");
        fs::create_dir_all(&config_dir).unwrap();
        std::env::set_var("AEGIS_CONFIG_DIR", config_dir.to_str().unwrap());

        // 写入过短的密钥
        fs::write(config_dir.join(".key"), [0u8; 16]).unwrap();
        fs::write(config_dir.join("config.enc"), b"{}").unwrap();

        let result = load_and_validate();
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: 运行确认通过**

Run: `cargo test --lib -- main::config::tests`
Expected: All tests PASS

- [ ] **Step 3: Commit**

```bash
git add src/main/config.rs
git commit -m "test: add load_and_validate unit tests with tempfile"
```

---

### Task 7: RoutingAdapter 集成测试

**Files:**
- Modify: `src/adapters/common/routing.rs`
- Modify: OR `tests/integration_security.rs`

- [ ] **Step 1: 在 routing.rs 底部添加单元测试**

```rust
#[cfg(test)]
mod routing_tests {
    use super::*;
    use aegis::adapters::common::MockBotAdapter;

    #[tokio::test]
    async fn sends_sensitive_to_secondary() {
        let mut primary = MockBotAdapter::new();
        primary.expect_platform().returning(|| Platform::Telegram);
        primary.expect_send_message().never();

        let mut secondary = MockBotAdapter::new();
        secondary.expect_platform().returning(|| Platform::Matrix);
        secondary.expect_send_message()
            .times(1)
            .returning(|_, _| Ok(MessageId("1".to_string())));

        let routing = RoutingAdapter::new(Arc::new(primary), Some(Arc::new(secondary)));
        routing.send_message(
            &TargetId("1".to_string()),
            MessageContent { text: "vless://abc123".into(), markup: None },
        ).await.unwrap();
    }

    #[tokio::test]
    async fn sends_normal_to_primary() {
        let mut primary = MockBotAdapter::new();
        primary.expect_platform().returning(|| Platform::Telegram);
        primary.expect_send_message()
            .times(1)
            .returning(|_, _| Ok(MessageId("1".to_string())));

        let mut secondary = MockBotAdapter::new();
        secondary.expect_platform().returning(|| Platform::Matrix);
        secondary.expect_send_message().never();

        let routing = RoutingAdapter::new(Arc::new(primary), Some(Arc::new(secondary)));
        routing.send_message(
            &TargetId("1".to_string()),
            MessageContent { text: "正常系统消息".into(), markup: None },
        ).await.unwrap();
    }

    #[tokio::test]
    async fn sends_normal_to_primary_when_no_secondary() {
        let mut primary = MockBotAdapter::new();
        primary.expect_platform().returning(|| Platform::Telegram);
        primary.expect_send_message()
            .times(1)
            .returning(|_, _| Ok(MessageId("1".to_string())));

        let routing = RoutingAdapter::new(Arc::new(primary), None);
        routing.send_message(
            &TargetId("1".to_string()),
            MessageContent { text: "vless://should-not-matter".into(), markup: None },
        ).await.unwrap();
    }
}
```

- [ ] **Step 2: 运行确认通过**

Run: `cargo test --lib -- adapters::common::routing::routing_tests`
Expected: All tests PASS

- [ ] **Step 3: Commit**

```bash
git add src/adapters/common/routing.rs
git commit -m "test: add RoutingAdapter integration tests with MockBotAdapter"
```

---

### Task 8: 最终集成验证

- [ ] **Step 1: 运行全部测试**

Run: `cargo test --lib 2>&1 | tail -20`
Expected: All tests pass (环境变量影响的测试除外)

- [ ] **Step 2: 运行全部集成测试**

Run: `cargo test --tests 2>&1 | tail -20`
Expected: All integration tests pass

- [ ] **Step 3: 检查编译无所用警告**

Run: `cargo check 2>&1`
Expected: No warnings

---

## 任务依赖图

```
Task 0: 修复基线测试 ─────────────┐
                                  ├── 无依赖
Task 1: #[automock] ──────────────┤
                                  │
Task 2: batch_handler 测试 ──── 依赖 Task 1
Task 3: has_matrix_config 测试 ── 无依赖
Task 4: destruct_flow 提取+测试 ── 无依赖
Task 5: self_destruct 测试 ──── 无依赖
Task 6: load_and_validate 测试 ── 无依赖
Task 7: RoutingAdapter 测试 ──── 依赖 Task 1
```

并行执行: Task 1 + Task 3 + Task 4 + Task 5 + Task 6 可并行

---

## 执行方式选择

计划已保存。两种执行方式:

1. **Subagent-Driven（推荐）** — 每个任务分发独立子 Agent，两阶段审查
2. **Inline Execution** — 在当前会话按批次执行，设置检查点

请选择执行方式。
