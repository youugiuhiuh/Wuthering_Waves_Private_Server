# 内存中敏感数据保护调查结论（rust/aegis）

日期：2026-09-07
范围：`rust/aegis`（Telegram/Discord/Matrix 管理机器人）秘密值在内存中的生命周期与保护
方法：CodeGraph + 源码走查 + zeroize/secrecy 官方文档对照

## 结论摘要

1. at-rest 与进程加固做得正确；**运行期内存保护链在 `SecurityManager::decrypt()` 返回处断裂**——mlock 只罩住从未被使用的解密缓冲，真正被使用的裸 `String` 全程无保护。
2. 项目自己选择了 mlock 威胁模型（防 swap/内存刮擦），却在每个调用方把秘密拷入可自由 swap 的普通 `String`，**mlock 被系统性架空**——"稳态明文 String 豁免"的论证因此不成立。
3. `DecryptedConfig` 存在 3 个 `#[expect(dead_code)]` 明文死字段，整进程驻留却从不被读取。

## 已正确处理的环节（不受影响）

| 环节 | 证据 |
|---|---|
| at-rest 配置加密（AES-256-GCM） | `config.enc` / `core/security/crypto.rs` |
| 主密钥内存 | `key: Zeroizing<[u8; 32]>`（crypto.rs:16） |
| 解密返回缓冲 | `SecretVec<u8>` + `mlock()` + drop 清零（crypto.rs:69-106） |
| TOTP 运行时 | `TotpManager` 内部 `secrecy::SecretString` |
| setup stdin 输入 | `SetupInput` derive `Zeroize, ZeroizeOnDrop` + `Zeroizing<String>`（bootstrap.rs:59,293） |
| 进程加固 | `RLIMIT_CORE=0` + `PR_SET_DUMPABLE=0`（bootstrap.rs `harden_process`） |
| 自毁密钥 | 只存 hash（`self_destruct_key_hash`） |

## 断裂点：所有调用方同一行代码

```rust
// main/config.rs、main/matrix.rs、main/discord.rs 同款
String::from_utf8(vec.expose_secret().to_vec())  // mlocked 缓冲 → 普通 Vec 拷贝
    .trim().to_string()                          // → 又一份普通 String
```

`SecurityManager::decrypt()` 返回 mlocked `SecretVec` 后，mlock 的页**从未被读取**；调用方立刻 `expose_secret().to_vec()` 拷出普通拷贝，随后以裸 `String` 驻留或传入客户端库。

## 未处理的清单（已逐处确认）

| # | 数据 | 位置 | 生命周期 |
|---|---|---|---|
| 1 | Telegram token | `main/config.rs` 解密 → `DecryptedConfig.token` → `runtime.rs:348` 交 teloxide | 明文 String 全进程驻留 |
| 2 | totp_secret | `main/config.rs:74` → `DecryptedConfig.totp_secret`（`#[expect(dead_code)]`） | 明文驻留整进程，boot 后从未读取（纯浪费） |
| 3 | discord_token / discord_admin_id | `main/config.rs:87,99` → 同 struct 死字段 | 同上；且 `discord.rs:88` 会自己再解密一份 |
| 4 | Matrix 5 项（homeserver/user/pwd/room/passphrase） | `main/matrix.rs:165-171` connect 时全量解密 | 明文喂 matrix-sdk（sqlite store passphrase 亦裸传） |
| 5 | setup 期 Matrix 5 项 | `bootstrap.rs:304-315` `MatrixSetupConfig`（普通 String，无 Drop） | 从 `ZeroizeOnDrop` 的 `SetupInput` take 出后无人管 |
| 6 | Matrix 恢复密钥中间拷贝 | `main/matrix.rs:89-102` | 收尾进 `SecretString`；中间 `to_vec()`/`trim()` 裸拷贝不清零 |

## 威胁模型矛盾（核心发现）

zeroize 官方文档明确 mlock/mprotect 类机制 out-of-scope、"often overkill (RAM scraping / swap access)"——即**若威胁模型不含 swap/内存刮擦，稳态明文 String 可豁免**。但本项目 `decrypt()` 自行对解密缓冲 mlock（crypto.rs:97），等于**主动选择了含 swap 的威胁模型**，随后又把同一秘密放进可 swap 的普通 `String` 常驻。二者必居其一：

- 要么承认稳态裸 String 是真实暴露（含 swap 威胁模型下），需要把稳态也纳入保护或消除驻留；
- 要么认为 mlock 属仪式性加固（不含 swap 威胁模型），则 `decrypt()` 内的 mlock + 调用方逐份拷贝均无必要，可简化。

## 建议修复方向（未实施，待决策）

1. **消除死字段**：#2/#3 —— boot 用毕即焚（totp 就地构建 `TotpManager` 后丢弃；discord 两字段由 `discord.rs` 自行解密，`config.rs` 无需驻留）。
2. **瞬时拷贝收口**：#1/#4/#6 —— 解密→`SecretString` 一步到位，不再经裸 `String` 中间态；matrix-sdk/teloxide 需要 `&str` 处仍须 expose，属框架边界。
3. **setup 暂存**：#5 —— `MatrixSetupConfig` 补 `ZeroizeOnDrop` 或直接存 `SecretString`。
4. **稳态 token**：#1 是否改 `SecretString` 需先定威胁模型（见上）；若走"稳态豁免"路线，应同时移除 `decrypt()` 中 mlock，消除仪式性复杂度。
