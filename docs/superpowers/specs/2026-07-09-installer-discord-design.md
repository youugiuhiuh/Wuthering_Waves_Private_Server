# Installer Discord 部署支援設計

## 目標

在 installer 的 3 條 setup 路徑（interactive / `--setup-keyval` / `--setup-stdin`）中添加 Discord bot 部署支援，與 Matrix 段完全對稱。Discord 僅需 2 個輸入（bot token + admin user ID），比 Matrix 的 5 個欄位更簡單。

## 異動清單

| 文件 | 動作 | 內容 |
|---|---|---|
| `main.go` `buildSetupPayload` | 修改 | 簽名加 `discordToken, discordAdminID string`；非空時 append JSON |
| `main.go` `setupConfig` struct | 修改 | 加 `DiscordToken` + `DiscordAdminID` 字段 |
| `main.go` `parseKeyVal` | 修改 | 加 `case "discord_token"` + `case "discord_admin_id"` |
| `main.go` `installFromKeyVal` | 修改 | `buildSetupPayload` 調用加 2 個新參數 |
| `main.go` `firstTimeSetup` | 修改 | Matrix 段後加 Discord 段（y/n → token + admin_id + 警告） |
| `main_test.go` | 修改 | 4 處 `buildSetupPayload` 調用更新簽名 + Discord subtests |
| `i18n/zh.json` | 修改 | 加 ~16 個 `firsttime.discord_*` key |
| `i18n/en.json` | 修改 | 同上 |
| `i18n/ja.json` | 修改 | 同上 |

### 不變

- `installFromStdin` — 直接透傳 JSON 給 aegis，aegis 已支援（PR #163）
- `runAegisSetup`、`finishDeploy`、`writeSystemdService` 不變

## buildSetupPayload 改動

```go
func buildSetupPayload(token, adminID, totpSecret []byte,
    matrixHS, matrixUser, matrixRoom string, matrixPass, matrixStorePassphrase []byte,
    discordToken, discordAdminID string,
) []byte {
    // 現有 matrix 邏輯不變...
    if discordToken != "" {
        payload = append(payload, ',')
        payload = append(payload, `"discord_token":`...)
        payload = appendJSONEscaped(payload, []byte(discordToken))
    }
    if discordAdminID != "" {
        payload = append(payload, ',')
        payload = append(payload, `"discord_admin_id":`...)
        payload = appendJSONEscaped(payload, []byte(discordAdminID))
    }
    payload = append(payload, '}')
    return payload
}
```

## firstTimeSetup Discord 段

Matrix 段後插入（~line 990 後），流程：

```
========== Discord Bot 部署（可选）==========
💡 Discord 可作为独立平台运行（--discord 启动）。
   注意: 与 Telegram 不可同时运行。
是否配置？(y/n): _

  y → token（readSecureInput, memguard 加密）
  → admin_id（readSecureInputStr）
  → ⚠ Privileged Intent 警告（MESSAGE_CONTENT，开发者门户手动开启）
  → ⚠ 共享 guild 警告（bot 与 admin 须共处一 guild）
```

## i18n keys（~16 個，三語）

| key | 中文值（示例） |
|---|---|
| `firsttime.discord_section` | `\n========== Discord Bot 部署（可选）==========` |
| `firsttime.discord_desc1` | `💡 Discord 可作为独立平台运行。` |
| `firsttime.discord_desc2` | `   注意: 与 Telegram 不可同时运行。` |
| `firsttime.discord_prompt_yn` | `是否配置 Discord Bot？(y/n): ` |
| `firsttime.discord_token_title` | `\n🤖 Discord Bot Token` |
| `firsttime.discord_token_help_step1` | `  1. 打开 Developer Portal` |
| `firsttime.discord_token_help_step2` | `  2. 创建 Application → Bot → Reset Token` |
| `firsttime.discord_token_help_format` | `  格式如: MTIzNDU2Nzg5.Gabc...` |
| `firsttime.discord_token_prompt` | `请输入 Discord Bot Token: ` |
| `firsttime.discord_admin_title` | `\n👤 Discord 管理员用户 ID` |
| `firsttime.discord_admin_help_step1` | `  1. Discord 设置 → 高级 → 开发者模式` |
| `firsttime.discord_admin_help_step2` | `  2. 右键头像 → 复制用户 ID` |
| `firsttime.discord_admin_help_format` | `  格式如: 123456789012345678` |
| `firsttime.discord_admin_prompt` | `请输入 Discord 管理员用户 ID: ` |
| `firsttime.discord_intent_warning` | `⚠ 部署前必须开启 MESSAGE CONTENT Intent` |
| `firsttime.discord_guild_warning` | `⚠ 机器人须与管理员共处一个服务器` |

## 測試

| 範圍 | 方法 |
|---|---|
| `buildSetupPayload` with Discord | 新 subtest：帶值 → JSON 含兩字段 |
| `buildSetupPayload` without Discord | 現有 subtest 加 2 空參數 → JSON 不含 discord 字段 |
| `parseKeyVal` with Discord | 新 subtest：`discord_token=xxx\ndiscord_admin_id=123` |
| 回歸 | `go test ./...` 全綠 |

## 已知限制

1. Privileged Intent + 共享 guild 為運維前提，installer 僅文字警告
2. Discord 與 TG 配置可共存於 config.enc，運行時由 `--discord` / `--tg` flag 決定
