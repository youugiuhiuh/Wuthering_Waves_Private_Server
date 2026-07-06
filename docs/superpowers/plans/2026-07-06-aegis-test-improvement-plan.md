# Aegis 测试改进计划

> 创建日期: 2026-07-06
> 范围: `rust/aegis` 测试覆盖提升

---

## 1. 当前测试覆盖盘点

### 1.1 已有良好测试覆盖的模块（无需增加）

| 模块 | 文件 | 测试数量 | 说明 |
|------|------|---------|------|
| ServiceAction | `core/cmd_action.rs` | 3 | roundtrip, 命令注入防御, Display |
| Async Cmd | `core/cmd_async.rs` | 7 | output, status, checked, stream, timeout |
| 加密 | `core/security/crypto.rs` | 12 | roundtrip, 空数据, 损坏数据, 内存锁定, 安全擦除 |
| 反调试 | `core/security/anti_debug.rs` | 3 | 无崩溃, TracerPid 解析 |
| UFW | `core/security/ufw.rs` | 6 | 安装检测, 端口解析 |
| Firewalld | `core/security/firewalld.rs` | 5 | 客户端存在性, 端口解析 |
| TLS Probe | `core/security/tls_probe.rs` | 6 | OID 映射, Sni 友好检测 |
| Hybrid2 | `core/singbox/hysteria2.rs` | 13 | 配置, JSON, 链接, 密码 |
| TUIC | `core/singbox/tuic.rs` | 8 | 配置, JSON, 链接, 密码 |
| SingBox 安装 | `core/singbox/installer.rs` | 5 | CPU 架构检测 |
| SingBox 错误 | `core/singbox/error.rs` | 6 | 错误变体 |
| SNI Selector | `core/sni/selector.rs` | 8 | roundtrip, 国家代码, 无重复 |
| SNI State | `core/sni/state.rs` | 4 | 状态机, 序列化 |
| KCP Mask | `core/xray/kcp_mask.rs` | 25+ | 变体, 分类, 兼容性, 解析, 排序 |
| 端口分配器 | `core/xray/port_allocator.rs` | 6 | 范围查找, 释放, 锁定 |
| Xray 路由 | `core/xray/routing.rs` | 4 | 规则定义, JSON 输出 |
| AppState | `app/state.rs` | 18 | 认证, 锁定, 自毁流程, 调度, 超时 |
| Error | `core/error.rs` | 9 | 错误变体, From 实现 |
| Paths | `core/paths.rs` | 9 | 所有路径常量 |
| Types | `core/types.rs` | 7 | BatchResult, IpVersion |
| Utils | `core/utils.rs` | 13 | 格式化, 解析, 报告进度 |
| TOTP | `core/totp.rs` | 8 | 生成, 验证, Base32 |
| I18n | `core/i18n.rs` | 12 | 语言切换, 时区映射 |
| GeoIP | `core/network/geoip.rs` | 3 | 国家代码, 字段缺失 |
| Release API | `core/network/release_api.rs` | 7 | SHA256 解析, 摘要解析 |
| Warp API | `core/network/warp_api.rs` | 3 | 序列化/反序列化 |
| Minisign | `core/crypto/minisign.rs` | 4 | 注释解析 |
| System/Upgrade | `core/system/upgrade.rs` | 5 | 仓库解析, 进度格式 |
| System/Operations | `core/system/operations.rs` | 18 | 发行版, 配置, 清理, 超时 |
| System/Upgrade Core | `core/system/core_upgrade.rs` | 8 | CPU 架构, 配置验证 |
| System/Scheduler | `core/system/scheduler/mod.rs` | 8 | 任务验证, CRON, 序列化 |
| System/Scheduler Types | `core/system/scheduler/task_types.rs` | 8 | 任务类型, 序列化, 显示 |
| Xray Config | `core/xray/config.rs` | 10 | 出站配置, KCP Mask, 端口提取 |
| Xray 安装 | `core/xray/installer.rs` | 3 | ProgressState |
| Xray KCP | `core/xray/kcp.rs` | 7 | KCP FinalMask |
| Reality | `core/xray/reality.rs` | 1 | PQ 验证 |
| Bootstrap | `bootstrap.rs` | 14 | ConfigValidator 校验函数 |
| Routing | `adapters/common/routing.rs` | 11 | is_sensitive 检测 |
| System/Monitor | `core/system/monitor.rs` | 4 | 负载, 网络流量聚合 |
| System/Maintenance | `core/system/maintenance.rs` | 7 | BBR, CPU 级别, 地理数据 |

### 1.2 已有基本但可补充的模块

| 模块 | 文件 | 现有测试 | 需补充 |
|------|------|---------|--------|
| System/LogAudit | `core/system/log_audit.rs` | 2 | 更多格式测试 |
| Fail2Ban | `core/security/fail2ban.rs` | 6 | 防火墙后端检测, 配置格式 |
| Firewall | `core/security/firewall.rs` | 3 | 后端变体 |
| Firewall Scanner | `core/security/firewall_scanner.rs` | 5 | 端口解析 |
| SelfDestruct | `core/security/self_destruct.rs` | 2 | Executor trait |
| Matrix 适配器 | `adapters/matrix/adapter.rs` | 4 | platform, message_id |
| Discord 适配器 | `adapters/discord/adapter.rs` | 1 | convert_markup |
| Telegram 适配器 | `adapters/telegram/adapter.rs` | 1 | convert_markup |

### 1.3 无测试覆盖的高优先级模块

| 模块 | 文件 | 风险等级 |
|------|------|---------|
| **batch_handler** | `app/batch_handler.rs` | **高** — 发送批处理结果 |
| **destruct_flow** | `app/destruct_flow.rs` | **高** — 自毁流程核心逻辑 |
| **auth** | `app/auth.rs` | **中** — TOTP 认证流程 |
| **build_adapter** | `main/adapter.rs` | **中** — 适配器构造决策 |
| **load_and_validate** | `main/config.rs` | **高** — 配置加载/解密 |
| **has_matrix_config** | `main/matrix.rs` | **中** — Matrix 配置检测 |
| **connect_matrix** | `main/matrix.rs` | **低** — 需要网络, 可模拟 |
| **runtime** | `main/runtime.rs` | **低** — 重度 teloxide/matrix 集成 |

### 1.4 集成测试覆盖

| 文件 | 测试内容 | 状态 |
|------|---------|------|
| `tests/cli_config_key_missing.rs` | 缺少 key 时 CLI 输出 | ✅ |
| `tests/cli_generate_totp_stdout.rs` | TOTP 生成 + 版本输出 | ✅ |
| `tests/cli_setup_stdin.rs` | stdin 安装流程 | ✅ |
| `tests/cli_setup_stdout.rs` | setup 输出格式 | ✅ |
| `tests/cli_verify_integrity_no_dir.rs` | 缺少目录时验证 | ✅ |
| `tests/integration_security.rs` | 加密/解密 roundtrip | ✅ |
| `tests/integration_setup_roundtrip.rs` | 安装→解密→TOTP | ✅ |
| `tests/integration_totp_trim.rs` | TOTP 尾部空白处理 | ✅ |
| `tests/test_self_destruct.rs` | 自毁状态机 | ✅ |
| `tests/test_self_destruct_e2e.rs` | 自毁 E2E 文件擦除 | ✅ |

---

## 2. 测试改进任务清单

### Priority 0 (紧急) — 无测试的关键路径

#### P0-1: `app/batch_handler.rs` — `send_singbox_batch_result`

```rust
// 需要对以下行为添加单元测试:
// - 发送 header message → 期望调用 adapter.send_message(header_msg)
// - 发送 combined_links → 期望 adapter.send_message(links)
// - 发送 result message → 期望 adapter.send_message(result_msg)
// - 60s 后自动删除 → 期望 adapter.delete_message()
// - 空 links 时跳过 → 不发送 links 消息
// - adapter.send_message 失败时不影响后续发送
```

**方案**: 使用 `MockBotAdapter`（mockall 或手动实现）断言调用顺序和次数。

```rust
#[tokio::test]
async fn batch_handler_sends_all_messages() {
    let mock = Arc::new(MockBotAdapter::new());
    mock.expect_send_message().times(3).returning(|_, _| Ok(MessageId("1".into())));
    mock.expect_delete_message().returning(|_, _| Ok(()));

    let result = BatchCreationResult {
        created_count: 2,
        links: vec!["vless://...".into(), "vless://...".into()],
        config_file: Some("/tmp/test.json".into()),
    };

    send_singbox_batch_result(mock, ChatId(1), "hy2", &result).await.unwrap();
    // 被动: 3 send_message + 1 delete_message 在 60s 后
}
```

#### P0-2: `app/destruct_flow.rs` — `handle_message_flow`

当前 `destruct_flow.rs` 的 `handle_message_flow` 逻辑包含了完整的自毁状态机（复杂度 26），且完全依赖 teloxide 的 `Bot` + `Message` 类型，无法直接单元测试。需要解耦：

**方案**: 提取纯逻辑函数，将 `Bot` 和 `Message` 依赖抽象化：

```rust
// 提取纯逻辑层
pub(crate) fn process_destruct_message(
    text: Option<&str>,
    step: DestructStep,
    state: &AppState,
) -> DestructMessageAction {
    match step {
        DestructStep::AwaitFirstTotp => {
            let code = text.unwrap_or("").trim();
            if state.verify_totp(code) {
                DestructMessageAction::ConfirmFirstTotp
            } else {
                DestructMessageAction::VerifyFailed
            }
        }
        DestructStep::AwaitSecondTotp => {
            // ...
        }
        DestructStep::AwaitSecurityFile => {
            // ...
        }
        _ => DestructMessageAction::Noop,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_totp_valid_returns_confirm() {
        let state = AppState::new(/* ... */);
        let result = process_destruct_message(
            Some("111111"),
            DestructStep::AwaitFirstTotp,
            &state,
        );
        assert_eq!(result, DestructMessageAction::ConfirmFirstTotp);
    }

    #[test]
    fn first_totp_invalid_returns_fail() {
        let state = AppState::new(/* ... */);
        let result = process_destruct_message(
            Some("000000"),
            DestructStep::AwaitFirstTotp,
            &state,
        );
        assert_eq!(result, DestructMessageAction::VerifyFailed);
    }

    #[test]
    fn no_text_in_first_totp_returns_noop() {
        let state = AppState::new(/* ... */);
        let result = process_destruct_message(
            None,
            DestructStep::AwaitFirstTotp,
            &state,
        );
        assert_eq!(result, DestructMessageAction::Noop);
    }
}
```

#### P0-3: `main/config.rs` — `load_and_validate`

```rust
// 需要对以下行为添加单元测试:
// - 正常解密流程
// - key 缺失但 config 存在的错误分支
// - token 解密失败
// - admin_id 格式无效
// - 配置校验失败
```

**方案**: 提取纯函数 `validate_decrypted_data`，将文件操作和加密操作通过 trait 注入。

### Priority 1 (高) — 高调用次数但覆盖不足

#### P1-1: `core/security/self_destruct.rs` — `trigger` + `SelfDestructExecutor`

当前只有 `test_production_executor_creation` 和 `test_executor_trait_available`，缺少 `trigger` 的实际测试。

```rust
#[tokio::test]
async fn trigger_calls_executor_execute() {
    let mut mock = MockSelfDestructExecutor::new();
    mock.expect_execute()
        .times(1)
        .returning(|| Box::pin(async { Ok(()) }));

    trigger(Arc::new(mock));
    // 确保 trigger 异步等待 executor 完成
}
```

#### P1-2: `main/adapter.rs` — `build_adapter`

```rust
// 测试以下 4 条分支:
// 1. Telegram + Matrix → RoutingAdapter
// 2. Telegram only → TelegramAdapter
// 3. Matrix only → MatrixAdapter
// 4. None → bail!
```

**方案**: 使用 mockall 模拟 `Bot` 构造并验证返回类型。

#### P1-3: `main/matrix.rs` — `has_matrix_config`

```rust
#[test]
fn has_matrix_config_all_fields_present_returns_true() {
    let config = EncryptedConfig {
        matrix_homeserver: Some(vec![]),
        matrix_username: Some(vec![]),
        matrix_password: Some(vec![]),
        matrix_room_id: Some(vec![]),
        ..Default::default()
    };
    assert!(has_matrix_config(&config, &[]));
}

#[test]
fn has_matrix_config_missing_fields_returns_false() {
    let config = EncryptedConfig::default();
    assert!(!has_matrix_config(&config, &[]));
}

#[test]
fn has_matrix_config_flag_overrides_empty_fields() {
    let config = EncryptedConfig::default();
    assert!(has_matrix_config(&config, &["--matrix"]));
}
```

### Priority 2 (中) — 边界覆盖补充

在已有测试的模块中补充边界条件：

| 模块 | 需补充的测试 |
|------|-------------|
| `core/security/ufw.rs` | 空输入、大量端口、重复端口 |
| `core/security/firewalld.rs` | 重复端口、非数字端口过滤 |
| `core/sni/selector.rs` | 所有国家代码映射、空 protobuf |
| `core/xray/kcp_mask.rs` | 空 slice、非法 Code 字符 |
| `core/utils.rs` | 大文件尺寸、零分割、极端超时值 |
| `core/totp.rs` | 空白 Base32、特殊字符 |
| `core/system/operations.rs` | 未知发行版 fallback |

---

## 3. 测试基础设施改进

### 3.1 引入 mockall 对关键模块做模拟

```toml
# Cargo.toml 已有 mockall
[dev-dependencies]
mockall = "0.14"
```

需对 `BotAdapter` trait 添加 `#[automock]`：

```rust
// adapters/common/trait.rs
#[cfg_attr(test, automock)]
#[async_trait]
pub trait BotAdapter: Send + Sync {
    fn platform(&self) -> Platform;
    async fn send_message(&self, target: &TargetId, content: MessageContent) -> Result<MessageId>;
    async fn edit_message(&self, target: &TargetId, msg_id: &MessageId, content: MessageContent) -> Result<()>;
    async fn delete_message(&self, target: &TargetId, msg_id: &MessageId) -> Result<()>;
}
```

### 3.2 引入 proptest

```toml
[dev-dependencies]
proptest = "1.4"
```

适用于:
- `core/totp.rs`: 所有合法/非法的 Base32 输入
- `core/xray/kcp_mask.rs`: 所有 Code 组合
- `core/security/crypto.rs`: 任意明文 → 加密 → 解密 → 原值
- `core/utils.rs`: 所有 IP 版本字符串
- `core/paths.rs`: 路径不包含非法字符

示例:
```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn encrypt_decrypt_roundtrip(plaintext in prop::collection::vec(any::<u8>(), 0..1024)) {
        let data = SecurityManager::encrypt(&manager, &plaintext).unwrap();
        let decrypted = SecurityManager::decrypt(&manager, &data).unwrap();
        assert_eq!(&plaintext, decrypted.expose_secret().as_slice());
    }

    #[test]
    fn kcp_mask_code_roundtrip(code in "[A-Za-z0-9_+-]{1,20}") {
        // Code → KcpFinalMask → code → 相等
    }
}
```

### 3.3 引入 criterion 基准测试

```toml
[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }

[[bench]]
name = "benchmark"
harness = false
```

适用于:
- TOTP 验证（高频率调用）
- SecurityManager 加密/解密
- KCP Mask 解析
- SNI 选择器随机抽取

---

## 4. 实施顺序

```
阶段 1 ─── P0-1 batch_handler 单元测试     [~30分钟]
         └── P0-3 load_and_validate 提取+测试 [~30分钟]

阶段 2 ─── P1-1 self_destruct trigger 测试   [~15分钟]
         └── P1-2 build_adapter 测试          [~15分钟]
         └── P1-3 has_matrix_config 测试       [~10分钟]

阶段 3 ─── P0-2 destruct_flow 纯逻辑提取+测试  [~45分钟]

阶段 4 ─── 边界覆盖 (Priority 2)              [~45分钟]

阶段 5 ─── proptest 引入                      [~30分钟]
         └── criterion 基准测试                [~30分钟]

阶段 6 ─── BotAdapter #[automock] + 集成测试   [~20分钟]
```

**预期效果**: 模块覆盖率从 ~65% 提升至 ~85%，关键路径覆盖率达 100%。

---

## 5. 运行验证

```bash
cd rust/aegis

# 所有测试
cargo test

# 只看集成测试
cargo test --test integration_security

# 只看单元测试
cargo test --lib

# 只看特定模块
cargo test --lib -- core::security::crypto::tests

# 覆盖率 (需安装 cargo-llvm-cov)
cargo llvm-cov --lib

# 快速检查编译
cargo check
```
