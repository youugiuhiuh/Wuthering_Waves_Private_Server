# Design: 一键部署成功消息 i18n 化

**Date:** 2026-07-27
**Status:** Approved

---

## 背景

`handle_one_click()` 中所有步骤成功消息均为硬编码中文 `format!()`，无法根据用户 locale 切换语言。

`IpVersion::label()` 保持返回英文技术标签（`IPv4`/`IPv6`/`IPv4/IPv6`/`IPv6/IPv4`），作为参数嵌入翻译模板。

## 变更文件

| 文件 | 改动 |
|---|---|
| `src/shared/handlers/ops.rs` | 6 处 `format!` → `t!` 调用 |
| `src/resources/i18n/zh.yml` | 新增 6 个 key |
| `src/resources/i18n/en.yml` | 新增 6 个 key |
| `src/resources/i18n/ja.yml` | 新增 6 个 key |

## 1. ops.rs 变更

### Skip 消息（2 处：xray_init 和 singbox 跳过）

```rust
// from
format!("{} - ⏩ 已安装，跳过", t!("ops.deploy_step_xray_init"))
// to
t!("ops.deploy_skip", "0" => t!("ops.deploy_step_xray_init"))
```

### 成功消息（5 处）

```rust
// XHTTP (有 IP label + count + file, 3 参数)
format!("✅ XHTTP Reality ({}) 已创建 {} 个配置\n📁 {}",
    ip_version.label(), result.created_count,
    result.config_file.as_deref().unwrap_or("?"))
// →
t!("ops.deploy_created_xhttp",
    "0" => ip_version.label(),
    "1" => format!("{}", result.created_count),
    "2" => result.config_file.as_deref().unwrap_or("?"))

// Vision (同上 3 参数) → ops.deploy_created_vision
// Hysteria2 (同上 3 参数) → ops.deploy_created_h2

// mKCP DNS (2 参数: count + file)
format!("✅ mKCP+DNS伪装 已创建 {} 个配置\n📁 {}", ...)
// → ops.deploy_created_kcp_dns

// mKCP WeChat (2 参数: count + file)
// → ops.deploy_created_kcp_wechat
```

## 2. i18n 键

| Key | zh | en | ja |
|---|---|---|---|
| `ops.deploy_skip` | `%{0} - ⏩ 已安装，跳过` | `%{0} - ⏩ already installed, skipping` | `%{0} - ⏩ インストール済み、スキップ` |
| `ops.deploy_created_xhttp` | `✅ XHTTP Reality (%{0}) 已创建 %{1} 个配置\n📁 %{2}` | `✅ XHTTP Reality (%{0}) created %{1} config(s)\n📁 %{2}` | `✅ XHTTP Reality (%{0}) %{1} 個の設定を作成しました\n📁 %{2}` |
| `ops.deploy_created_vision` | `✅ Reality Vision (%{0}) 已创建 %{1} 个配置\n📁 %{2}` | `✅ Reality Vision (%{0}) created %{1} config(s)\n📁 %{2}` | `✅ Reality Vision (%{0}) %{1} 個の設定を作成しました\n📁 %{2}` |
| `ops.deploy_created_h2` | `✅ Hysteria2 (%{0}) 已创建 %{1} 个配置\n📁 %{2}` | `✅ Hysteria2 (%{0}) created %{1} config(s)\n📁 %{2}` | `✅ Hysteria2 (%{0}) %{1} 個の設定を作成しました\n📁 %{2}` |
| `ops.deploy_created_kcp_dns` | `✅ mKCP+DNS伪装 已创建 %{0} 个配置\n📁 %{1}` | `✅ mKCP+DNS obfuscation created %{0} config(s)\n📁 %{1}` | `✅ mKCP+DNS 偽装 %{0} 個の設定を作成しました\n📁 %{1}` |
| `ops.deploy_created_kcp_wechat` | `✅ mKCP+微信伪装 已创建 %{0} 个配置\n📁 %{1}` | `✅ mKCP+WeChat obfuscation created %{0} config(s)\n📁 %{1}` | `✅ mKCP+WeChat 偽装 %{0} 個の設定を作成しました\n📁 %{1}` |

Keys 插入位置：`ops.deploy_fail_security` 之后。

## 3. 不去做的事

- 不改 `IpVersion::label()` — 保持英文技术标签
- 不改其他 handler 中的硬编码 — 只修 `handle_one_click` 一键部署流程
