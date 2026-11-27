# FIX: 单 Matrix（及单 SimpleX）交互式安装不生成 TOTP，部署后 `/auth` 永远失败

- 状态：待实施（本文只描述修复，不改代码）
- 触发环境：`go/installer` 交互式安装（`firstTimeSetup`），平台选择**不含 Telegram**
- 发现时间：2026-11-27，单 Matrix 部署实测

---

## 1. 症状

VPS 上装好、服务 active、Matrix 登录成功，但在 Element 里：

```
/auth 487209
❌ Invalid TOTP code. (1/5 attempts)
/auth 019204
❌ Invalid TOTP code. (4/5 attempts)
```

任何码都错，5 次后进冷却。`/menu` 这类命令也全部无响应（`check_auth` 要求 `is_authorized`，而授权必须先 `/auth`）。

## 2. 根因

**TOTP 密钥生成被错误地锁在 `enableTG` 门禁里。**

`go/installer/main.go`，函数 `firstTimeSetup`（1774–1963）：

```go
var totpSecretEnclave *memguard.Enclave

if enableTG {                                    // ← main.go:1809，元凶
    totpSecretOutput, err := runCmdOutputBytes(binaryPath, "--generate-totp-secret")
    ...
    totpSecretEnclave = memguard.NewEnclave(totpSecretRaw)
    ...                                          // 展示 secret / otpauth URL / QR
}
```

`enableTG == false` 时 `totpSecretEnclave` 保持 `nil` → 后面的 `tSecretBytes` 为 `nil` → `buildSetupPayload(...)`（`main.go:554`）因 `len(totpSecret) == 0` **不写 `totp_secret` 字段** → Rust 侧 `EncryptedConfig.totp_secret == None` → `TotpManager` 为 `None` → `TotpManager`/`verify_totp` 恒返回 `false`：

```rust
// rust/aegis/src/app/state.rs
pub fn verify_totp(&self, code: &str) -> bool {
    self.totp_manager.as_ref().map(|t| t.verify(code)).unwrap_or(false)  // None → 永远 false
}
```

线上 `config.enc` 的实证（逐字段加密，`None` 会序列化为 JSON `null`，无需解密即可判断）：

```
token            None
admin_id         None          ← 见 §4
totp_secret      None          ← 根因
matrix_*         set (5 个)
```

## 3. 为什么这是 bug 而不是设计

同一个安装器里，**另外两条入口路径都是无条件生成的**，只有交互式路径被门禁包住：

| 入口 | 位置 | 行为 |
| --- | --- | --- |
| `installFromStdin`（`--setup-stdin` 管道式） | `main.go:1576-1579` | `totp_secret` 缺失或为空 → 直接 `generateTOTPSecret(destPath)` ✅ |
| `installFromKeyVal`（`key=value` 配置式） | `main.go:1686-1688` | `cfg.TOTPSecret == ""` → 直接 `generateTOTPSecret` ✅ |
| `firstTimeSetup`（交互式） | `main.go:1809` | **仅 `if enableTG`** ❌ |

也就是说：同一份配置，用 `--setup-stdin` 装就有 TOTP，用交互式问答装就没有。这不是取舍，是漏写。

## 4. 连带的第二个缺口：`admin_id` 同样被 `enableTG` 门禁

`main.go:1786` 的 `if enableTG { ... }` 块里包含 `adminIDEnclave = readSecureInput(...)`（TG 管理员的数字 ID）。非 TG 形态不填 `admin_id`，于是 Rust 侧 `admin_id == None`。

这带来一个**危险行为**，必须在修 TOTP 时一并认清（修复动作在 Rust 侧，不在 installer）：

```rust
// rust/aegis/src/main/runtime.rs
fn parse_user_id(s: &str) -> i64 {
    s.trim_start_matches('@').split(':').next()
        .and_then(|n| n.parse().ok()).unwrap_or(0)   // "@us-sanjose:matrix.org" → 0
}

// rust/aegis/src/app/state.rs
pub fn is_admin_user(&self, user_id: i64) -> bool {
    user_id == self.admin_id.unwrap_or(0)            // None → 0 == 0 → true
        || self.simplex_admin_id() == Some(user_id)
}
```

非数字 MXID 一律解析成 `0`，`admin_id` 为 `None` 时 `0 == 0` 成立 → **房间里任何人发消息都被当作管理员**，唯一拦着的是 TOTP 会话门禁。所以：

- 修完 TOTP 后，`--matrix` 形态**默认是「房间内人人皆管理员」**，安全边界实际只剩「谁在这个房间」。
- 这个缺陷清单 Matrix 部署务必使用**私有房间**，且不要把它当成分权机制。

## 5. 修复方案

### 方案 A（推荐）：TOTP 生成无条件执行，与另两条路径对齐

把 `main.go:1809` 的 `if enableTG { ... }` 整块**去掉条件**，TOTP 的生成 + 展示（secret / otpauth URL / QR / 清理提示）对所有平台形态都执行。

理由：
1. TOTP 是 **aegis 所有形态**的命令授权前置（`check_auth` → `is_authorized`），不只是 TG 的需要；`--simplex` 单跑同样需要它（SimpleX 的 6 位码登录靠它，新 contactId 自证也靠它）。
2. 与 `installFromStdin` / `installFromKeyVal` 行为一致，消除「同一份配置因入口不同而结果不同」。
3. 改动最小：一个 `if` 条件。

### 方案 B：显式列为平台条件

`if enableTG || enableMatrix || enableSimplex { ... }`。语义等价，但把「哪种形态需要 TOTP」写成一个需要维护的判断，反而给未来平台留了再次漏写的口子。**不采用。**

### i18n

无需新增文案：`firsttime.totp_section` / `totp_key_label` / `totp_qr_scan` / `totp_no_qr` / `totp_installing_qr` / `totp_manual_url` / `totp_clear_hint` / `totp_separator` 在 `en/zh/ja` 三语里都已存在。

### 展示时机

现在 QR 展示夹在 TG 问答中间。去掉门禁后它会在平台选择**之后立即出现**（Matrix 问答之前）。建议：
- 先问平台 → 立即生成并展示 TOTP（用户此时就能扫码/存 secret）
- 再问各平台细节（Matrix MXID / 密码 / 房间 / recovery key）

这样即使用户中途 Ctrl-C，已经拿到的 TOTP 也不会丢——现在的顺序（TG 问答 → TOTP）在中断时同样会丢，属顺手改善，不是必需。

## 6. 测试计划

Go 侧没有单测框架（`go/installer` 只有 `main_test.go` 覆盖少量纯函数），所以要靠**可测的纯函数抽取** + 端到端验证。

**建议抽取纯函数**（放 `main.go`，便于 `main_test.go` 覆盖）：

```go
// TOTP 是否需要在 setup payload 里生成
func needsGeneratedTOTP(enableTG, enableMatrix, enableSimplex bool) bool
```

- 方案 A 下它就是 `return true`，但保留函数让「所有形态都需要」成为**被测试断言的行为**而不是隐含约定。
- 表驱动用例：`{TG}`、`{Matrix}`、`{SimpleX}`、`{Matrix,SimpleX}`、全开、全关 → 全部 `true`。

**payload 断言**（`buildSetupPayload` 已有、是纯函数）：

- `totp_secret` 为空 → 序列化结果**不含** `totp_secret` 键（现状，锁住回归面）
- `totp_secret` 非空 → 含该键且值被正确 JSON 转义

**端到端（必须做，本次 bug 就是端到端才暴露的）**：

1. 干净的 `--matrix` 交互式安装（`enableTG=false`）
2. 装完检查：`python3 -c 'import json;print(json.load(open("/etc/wwps/aegis/config.enc"))["totp_secret"] is not None)'` → `True`
3. 安装器输出里应出现 otpauth URL，且**含 `algorithm=SHA512&digits=6&period=30`**（`buildOtpAuthURL` 已正确，`main.go:652`，别弄丢）
4. Element 里 `/auth <码>` → `✅ Authentication successful`
5. `/menu` → 有响应

## 7. 验收判据

- `go test ./...`（`go/installer`）全绿，新增表驱动用例覆盖 §6 的 `needsGeneratedTOTP`
- `--matrix` 干净安装后 `config.enc.totp_secret != null`
- 安装日志里能拿到 otpauth URL，导入后 `/auth` 通过
- `--tg-simplex` 安装行为**不变**（回归检查：TOTP 仍然只生成一次、仍然展示、payload 仍含该字段）

## 8. 不在本修复范围内

1. **`--matrix` 的 admin 身份问题**（§4）：需要在 Rust 侧改 `parse_user_id`，让非数字 MXID 得到稳定的非零标识（例如对完整 MXID 做截断哈希），并让 `admin_id` 支持「MXID 形式」。这是独立改动，属安全边界，需另开 spec 并做对抗覆核（`doubt-driven-development`）。
2. `--setup` / `--setup-stdin` 的**全量覆盖**语义（`run_setup` 直接构造新 `EncryptedConfig`，未指定的字段变 `None`）。当前重配置必须把所有字段重传一遍，易踩。是否改成「按字段合并」是独立设计决策。
3. SHA512 的第三方兼容性：`buildOtpAuthURL` 已带 `algorithm=SHA512`，但**Google Authenticator 忽略该参数**（固定 SHA1），导入会算出错误的码。建议在安装器输出里加一句提示（「不支持 SHA512 的 App 请改用 Aegis / Bitwarden / 2FAS」）——文案工作，可并入本修复顺手做。

## 9. 临时绕过（已实测可用，用于当下救急）

在已部署的机器上手工补 `totp_secret`，**不重跑 setup**（重跑会清空其它字段）：

1. 生成密钥：`/etc/wwps/aegis/aegis --generate-totp-secret`（输出即 base32 密钥，stdout 单行）
2. 用 `config.enc` 同目录的 `.key`（32 字节）按 AES-256-GCM 加密该字符串，密文格式 `nonce(12) || ciphertext+tag`，整体作为 JSON 数字数组写回 `config.enc` 的 `totp_secret` 键
3. `systemctl restart wwps-aegis`
4. 用 `otpauth://totp/wwps:admin?secret=<SECRET>&issuer=wwps&algorithm=SHA512&digits=6&period=30` 导入

注意：
- 只能**改这一个字段**，不要用 `--setup` 重配（见 §8.2）
- `.key` 一旦重新生成，旧 `config.enc` 的所有字段就永久解不开——备份必须 `.key` + `config.enc` **成对**
