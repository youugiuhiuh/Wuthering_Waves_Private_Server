# Design: 一键部署使用预发行版 + 新增 mKCP 协议

**Date:** 2026-07-27
**Status:** Approved

---

## 背景与目标

1. **Xray-core 初始化默认使用预发行版**：当前 `fetch_release(None)` 走 `/releases/latest` API，获取最新稳定版。改为走 `/releases?per_page=20`，筛选 `prerelease: true` 的第一条（最新预发行版）
2. **一键部署新增 mKCP 协议**：在部署流程中添加 mKCP+DNS伪装 ×5 和 mKCP+微信伪装 ×5

## 变更文件

| 文件 | 改动 |
|---|---|
| `src/core/system/core_upgrade.rs` | `fetch_release(None)` 调用改为请求预发行版 |
| `src/core/network/release_api.rs` | `ReleaseResponse` 添加 `prerelease` 字段；新增 `fetch_prerelease()` |
| `src/shared/handlers/ops.rs` | 嵌入 mKCP DNS/微信 步骤 |
| `src/resources/i18n/zh.yml` | 新增 4 个 key |
| `src/resources/i18n/en.yml` | 新增 4 个 key |
| `src/resources/i18n/ja.yml` | 新增 4 个 key |

## 1. 预发行版获取

### release_api.rs

`ReleaseResponse` 添加字段：
```rust
pub prerelease: bool,
```

新增方法：
```rust
pub async fn fetch_prerelease(&self, owner: &str, repo: &str) -> Result<ReleaseResponse> {
    let url = format!("{}/repos/{}/{}/releases?per_page=20", self.base_url, owner, repo);
    let resp = self.client.get(&url).send().await?;
    let releases: Vec<ReleaseResponse> = resp.json().await?;
    releases.into_iter()
        .find(|r| r.prerelease)
        .ok_or_else(|| anyhow::anyhow!("No prerelease found"))
}
```

### core_upgrade.rs

`fetch_release(None)` 调用路径改为调用 `release_api.fetch_prerelease(owner, repo)`。

## 2. 一键部署流程（新）

```
1. VPS 调优
2. Xray-core（预发行版）
3. PQ 密钥
4. XHTTP Reality ×20
5. Vision Reality ×20
6. Sing-box 初始化
7. Hysteria2 ×3
8. mKCP + DNS伪装 ×5    ← 新增
9. mKCP + 微信伪装 ×5    ← 新增
10. 安全加固
```

### ops.rs handle_one_click()

从 8 步扩展为 10 步。在步骤 7(Hysteria2) 后插入：

**步骤 8 — mKCP + DNS伪装 ×5：**
```rust
msg.edit_text(t!("ops.deploy_step_kcp_dns")).await?;
ConfigManager::batch_create_kcp(5, &IpVersion::Both, &["mld"]).await
    .map_err(|e| anyhow::anyhow!("{}: {e}", t!("ops.deploy_fail_kcp_dns")))?;
```

**步骤 9 — mKCP + 微信伪装 ×5：**
```rust
msg.edit_text(t!("ops.deploy_step_kcp_wechat")).await?;
ConfigManager::batch_create_kcp(5, &IpVersion::Both, &["mlw"]).await
    .map_err(|e| anyhow::anyhow!("{}: {e}", t!("ops.deploy_fail_kcp_wechat")))?;
```

### 无需改动

- `kcp_mask.rs` — `mld`、`mlw` 码点已存在
- `kcp.rs` — `batch_create_kcp()` 接受任意 mask 数组
- `installer.rs` — 已调用 `install_wwps_core()`

## 3. i18n

| Key | zh | en | ja |
|---|---|---|---|
| `ops.deploy_step_kcp_dns` | 正在创建 mKCP+DNS伪装 (×5)... | Creating mKCP+DNS obfuscation (×5)... | mKCP+DNS 偽装 (×5) を作成中... |
| `ops.deploy_step_kcp_wechat` | 正在创建 mKCP+微信伪装 (×5)... | Creating mKCP+WeChat obfuscation (×5)... | mKCP+WeChat 偽装 (×5) を作成中... |
| `ops.deploy_fail_kcp_dns` | mKCP+DNS伪装 创建步骤失败 | mKCP+DNS obfuscation step failed | mKCP+DNS 偽装ステップが失敗しました |
| `ops.deploy_fail_kcp_wechat` | mKCP+微信伪装 创建步骤失败 | mKCP+WeChat obfuscation step failed | mKCP+WeChat 偽装ステップが失敗しました |

## 4. 不去做的事

- 不加 Noise/Salamander/Sudoku 混淆层 — mkcp-legacy 自带 XOR 加密 + DNS/微信协议头伪装已足够
- starter text / 进度切分：只做最简 — 两步各自一条消息，不变动消息状态机，标记位和展示逻辑堆在 `handle_one_click` 里不改架构
