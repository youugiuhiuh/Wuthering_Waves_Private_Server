# 用完即焚：totp_secret 与 matrix 恢复密钥内存清理设计

日期：2026-09-07
状态：已批准（chat approval, 2026-09-07）
范围：`rust/aegis` — 仅两条敏感链：TOTP secret 与 Matrix 恢复密钥
前置文档：`memory-secrets-investigation.md`（全量审计结论）

## 目标

这两个值一旦泄露即可被他人直接操作（TOTP 可过 bot 2FA → 管理命令/自毁；恢复密钥可接管 Matrix 交叉签名身份）。修复目标 = **内存中用完即焚**：

1. totp_secret：消除 boot 后全进程驻留、从未被读取的明文死字段。
2. matrix 恢复密钥：消除恢复过程中的裸明文中间拷贝（磁盘"成功即烧、失败保留"已实现，不动）。

## 现状（证据）

| 值 | 现状 | 问题 |
|---|---|---|
| totp_secret | boot 解密 → 裸 String 存 `DecryptedConfig.totp_secret`（`#[expect(dead_code)]`，config.rs:74-78）→ 全进程驻留；同时 `.clone()` 一份进 TotpManager | 1 死副本全进程明文 + 1 次多余 clone |
| matrix 恢复密钥 | decrypt → `to_vec()` 裸拷贝 → from_utf8 → trim → `SecretString`（matrix.rs:89-102）| 收尾受保护，中间 2-3 份裸拷贝不清零；磁盘成功恢复后已原子清除（matrix.rs:277） |

## 改动设计

### 1. `core/security/crypto.rs`：新增 `decrypt_secret` 助手

```rust
pub fn decrypt_secret(&self, data: &[u8]) -> Result<SecretString> {
    let vec = self.decrypt(data)?;                          // 已有: mlocked SecretVec
    let s = Zeroizing::new(String::from_utf8(
        vec.expose_secret().to_vec(),                       // 唯一一次明文拷贝
    ).map_err(|e| anyhow::anyhow!("密文包含无效 UTF-8: {}", e))?);
    Ok(SecretString::from(s.trim().to_string()))            // trim 结果进受保护区（drop 清零）
}
```

要点：全链仅 1 份明文拷贝，且从 `Zeroizing` 包住的 String trim 进 `SecretString`——中间无游离裸拷贝。`SecretString` drop 即清零。

### 2. `main/config.rs`：totp 用完即焚

- `DecryptedConfig` 删除 `totp_secret` 字段。
- `load_and_validate` 流程改为：
  1. 解密 totp 进**局部** `SecretString`（`decrypt_secret`）；
  2. 用局部值跑 `ConfigValidator::validate_decrypted_config`；
  3. `TotpManager::new(&local)`（去掉现有 `secret.clone()` 多余拷贝）；
  4. 局部 `SecretString` 出作用域 drop → 清零；
  5. AppConfig 只携带 `totp_manager`。
- 磁盘 `config.enc` 内 totp 密文**保留**（每次重启必需，用于重建 TotpManager——刻意不改）。

### 3. `main/matrix.rs`：恢复密钥中间态收口

- `try_recover_with_key` 改用 `decrypt_secret`，消除 `to_vec()`/`String` 中间裸拷贝。
- 磁盘清除逻辑（成功即烧、失败保留）与分层身份策略**不动**。

## 故意不动的

- `TotpManager` 内部 TOTP 密钥（2FA 全程需要，包在 totp_rs 内部，进程生命周期合法驻留）。
- totp 磁盘密文副本（重启重建必需）。
- matrix 恢复密钥磁盘"失败保留"语义（避免误烧，boot 重试）。

## 影响文件

- `rust/aegis/src/core/security/crypto.rs`（+助手 +测试）
- `rust/aegis/src/main/config.rs`（重构 + 删字段 + 测试适配）
- `rust/aegis/src/main/matrix.rs`（换助手）
- 连带：`main.rs` 等编译期适配（如引用 `decrypted.totp_secret` 处——预计无，死字段无读取方）

## 测试策略（TDD）

1. `decrypt_secret` 单测：roundtrip（encrypt→decrypt_secret 还原原文）；非法 UTF-8 密文报错。
2. config 回归：`load_and_validate` 后 `DecryptedConfig` 无 totp 字段（编译期保证）；totp_manager 可 verify。
3. matrix 恢复路径：现有 `after_recovery_action` 纯函数测试不动；`try_recover_with_key` 行为不变量保持（成功 true/失败 false）。

## 验证

- `cargo fmt` / `cargo clippy` / `cargo nextest`（rust-lint-format 技能强制门禁）
- 全仓 `cargo build` / 相关 integration tests（totp_trim、setup_roundtrip）

## 批次 2（2026-09-07 chat 批准）：matrix connect 的 2 个秘密局部用完即焚

范围：仅 `main/matrix.rs` `connect_matrix` 内 **`matrix_store_passphrase` + `matrix_pwd`** 两个秘密局部（用户裁定：公开的 homeserver/username/room_id 与 discord.rs 不在范围）。

现状：两者经 `decrypt_matrix` 闭包（decrypt → to_vec → 裸 String）创建，活到 connect_matrix 结束，drop 不清零；`matrix_store_passphrase` 只用一次（`sqlite_store` :177），`matrix_pwd` 有 3 处可能 hand-off（login :~200、bootstrap :278/:294）。

改动：
1. 两个秘密局部改用 `SecurityManager::decrypt_secret`（Task 1 助手）→ 类型 `SecretString`，消除裸中间拷贝，drop 即清零。
2. hand-off 处 `expose_secret()` 借出 &str（SDK 侧零改动——passphrase SDK 自留 Zeroizing 拷贝，已确认）。
3. `matrix_store_passphrase` 在 `.build()` 后立即 `drop()`（最后一次 hand-off 即弃）。
4. `matrix_pwd` 焚毁点 ruling：login 与 bootstrap 分支都可能用，无法在身份块中间提前焚——**自然 drop（connect_matrix 返回时 SecretString 清零）**，用户认可。

不变：`decrypt_matrix` 闭包保留给 3 个公开字段；错误文案差异（UTF-8 提示并入 decrypt_secret）可接受，与批次 1 一致；磁盘/会话逻辑零改动。
