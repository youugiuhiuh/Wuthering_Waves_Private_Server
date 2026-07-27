# 一键部署预发行版 + mKCP 协议 — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One-click deploy defaults to Xray-core prerelease; adds mKCP+DNS ×5 and mKCP+WeChat ×5 to deploy flow.

**Architecture:** Extend `ReleaseResponse` with `prerelease` flag and add `fetch_prerelease()` to release API; wire it into `fetch_release` when no explicit tag is given. Insert two KCP batch calls into `handle_one_click` after Hysteria2 step, bumping step totals from 8 to 10. Add 4 i18n keys per locale.

**Tech Stack:** Rust, reqwest, serde, rust-i18n, YAML

## Global Constraints

- Xray-core owner/repo: `XTLS/Xray-core` (unchanged)
- mKCP mask codes: `mld` (DNS), `mlw` (WeChat) — already exist in `kcp_mask.rs`
- No new dependencies
- No new files
- `cargo fmt && cargo clippy -- -D warnings && cargo test` must pass before commit

---

### Task 1: Add `prerelease` field to ReleaseResponse + `fetch_prerelease()`

**Files:**
- Modify: `rust/aegis/src/core/network/release_api.rs`

**Interfaces:**
- Produces: `ReleaseResponse.prerelease: bool`, `pub async fn fetch_prerelease(client, bases, api_path, token) -> Result<ReleaseResponse>`

- [ ] **Step 1: Add `prerelease` field to `ReleaseResponse`**

```rust
#[derive(Debug, Deserialize)]
pub struct ReleaseResponse {
    pub tag_name: String,
    pub body: Option<String>,
    pub assets: Vec<ReleaseAsset>,
    #[serde(default)]
    pub prerelease: bool,
}
```

- [ ] **Step 2: Add `fetch_prerelease()` function after `fetch_json_from_mirrors()`**

After line 89 in release_api.rs, insert:

```rust
pub async fn fetch_prerelease(
    client: &reqwest::Client,
    bases: &[String],
    api_path: &str,
    token: Option<&str>,
) -> Result<ReleaseResponse> {
    let releases: Vec<ReleaseResponse> =
        fetch_json_from_mirrors(client, bases, api_path, token).await?;
    releases
        .into_iter()
        .find(|r| r.prerelease)
        .ok_or_else(|| anyhow!("No prerelease found"))
}
```

- [ ] **Step 3: Re-export `fetch_prerelease` from `core_upgrade.rs` imports**

No change needed — `core_upgrade.rs` already imports `fetch_json_from_mirrors` plus function-level imports. Need to add `fetch_prerelease` to the import block.

- [ ] **Step 4: Run tests to verify**

```bash
cargo test -p aegis --lib release_api
```

Expected: all 8 release_api tests pass.

- [ ] **Step 5: Commit**

```bash
git add rust/aegis/src/core/network/release_api.rs
git commit -m "feat: add prerelease field and fetch_prerelease to release API"
```

---

### Task 2: Use prerelease in fetch_release when no tag given

**Files:**
- Modify: `rust/aegis/src/core/system/core_upgrade.rs:5-6` (imports), `rust/aegis/src/core/system/core_upgrade.rs:217-223` (fetch_release body)

**Interfaces:**
- Consumes: `fetch_prerelease` from `release_api`
- Modifies: `WwpsCoreUpgradeManager::fetch_release` — when `tag` is `None`, fetches prerelease instead of `/releases/latest`

- [ ] **Step 1: Add `fetch_prerelease` to import**

Line 5-6, change:
```rust
use crate::core::network::release_api::{
    ReleaseAsset, ReleaseResponse, extract_sha256_from_body, fetch_json_from_mirrors,
    find_minisig_asset, parse_digest, parse_sha256_manifest,
};
```
To:
```rust
use crate::core::network::release_api::{
    ReleaseAsset, ReleaseResponse, extract_sha256_from_body, fetch_json_from_mirrors,
    fetch_prerelease, find_minisig_asset, parse_digest, parse_sha256_manifest,
};
```

- [ ] **Step 2: Change fetch_release None path**

Lines 217-227, change:
```rust
pub async fn fetch_release(&self, tag: Option<&str>) -> Result<WwpsCoreReleaseInfo> {
    let config = &self.config;
    let path = if let Some(t) = tag {
        format!("{}/{}/releases/tags/{}", config.owner, config.repo, t)
    } else {
        format!("{}/{}/releases/latest", config.owner, config.repo)
    };
    let bases = vec![WWPS_CORE_RELEASE_API_BASE.to_string()];

    let release: ReleaseResponse =
        fetch_json_from_mirrors(&self.client, &bases, &path, self.github_token.as_deref())
            .await?;
```
To:
```rust
pub async fn fetch_release(&self, tag: Option<&str>) -> Result<WwpsCoreReleaseInfo> {
    let config = &self.config;
    let bases = vec![WWPS_CORE_RELEASE_API_BASE.to_string()];

    let release: ReleaseResponse = if let Some(t) = tag {
        let path = format!("{}/{}/releases/tags/{}", config.owner, config.repo, t);
        fetch_json_from_mirrors(&self.client, &bases, &path, self.github_token.as_deref())
            .await?
    } else {
        let path = format!("{}/{}/releases?per_page=20", config.owner, config.repo);
        fetch_prerelease(&self.client, &bases, &path, self.github_token.as_deref()).await?
    };
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p aegis
```

Expected: all 582 tests pass.

- [ ] **Step 4: Commit**

```bash
git add rust/aegis/src/core/system/core_upgrade.rs
git commit -m "feat: default to Xray-core prerelease when no tag given"
```

---

### Task 3: Add i18n keys for three languages

**Files:**
- Modify: `rust/aegis/src/resources/i18n/zh.yml`
- Modify: `rust/aegis/src/resources/i18n/en.yml`
- Modify: `rust/aegis/src/resources/i18n/ja.yml`

- [ ] **Step 1: Add keys to zh.yml**

After line 189 (`deploy_fail_security`), insert:

```yaml
  deploy_step_kcp_dns: "正在创建 mKCP+DNS伪装 (×5)..."
  deploy_step_kcp_wechat: "正在创建 mKCP+微信伪装 (×5)..."
  deploy_fail_kcp_dns: "mKCP+DNS伪装 创建步骤失败"
  deploy_fail_kcp_wechat: "mKCP+微信伪装 创建步骤失败"
```

- [ ] **Step 2: Add keys to en.yml**

After line 179 (`deploy_fail_security`), insert:

```yaml
  deploy_step_kcp_dns: "Creating mKCP+DNS obfuscation (×5)..."
  deploy_step_kcp_wechat: "Creating mKCP+WeChat obfuscation (×5)..."
  deploy_fail_kcp_dns: "mKCP+DNS obfuscation step failed"
  deploy_fail_kcp_wechat: "mKCP+WeChat obfuscation step failed"
```

- [ ] **Step 3: Add keys to ja.yml**

After line 179 (`deploy_fail_security`), insert:

```yaml
  deploy_step_kcp_dns: "mKCP+DNS 偽装 (×5) を作成中..."
  deploy_step_kcp_wechat: "mKCP+WeChat 偽装 (×5) を作成中..."
  deploy_fail_kcp_dns: "mKCP+DNS 偽装ステップが失敗しました"
  deploy_fail_kcp_wechat: "mKCP+WeChat 偽装ステップが失敗しました"
```

- [ ] **Step 4: Verify YAML is parseable**

```bash
python3 -c "import yaml; yaml.safe_load(open('rust/aegis/src/resources/i18n/zh.yml')); yaml.safe_load(open('rust/aegis/src/resources/i18n/en.yml')); yaml.safe_load(open('rust/aegis/src/resources/i18n/ja.yml')); print('OK')"
```

Expected: `OK`

- [ ] **Step 5: Commit**

```bash
git add rust/aegis/src/resources/i18n/zh.yml rust/aegis/src/resources/i18n/en.yml rust/aegis/src/resources/i18n/ja.yml
git commit -m "feat: add mKCP deploy step i18n keys"
```

---

### Task 4: Insert mKCP steps into one-click deploy

**Files:**
- Modify: `rust/aegis/src/shared/handlers/ops.rs`

**Interfaces:**
- Consumes: `ConfigManager::batch_create_kcp(5, ip_version, &["mld"|"mlw"])`, new i18n keys
- Modifies: `handle_one_click()` — step totals 8→10, adds steps 8 & 9

- [ ] **Step 1: Change all step totals from 8 to 10**

Find every `send_progress` call in `handle_one_click` (lines 546, 556, 560, 563, 576, 602, 638, 675, 683, 695, 756): change the second parameter `8` to `10`.

For each:
```rust
// from
send_progress(&tx, N, 8, msg)

// to
send_progress(&tx, N, 10, msg)
```

- [ ] **Step 2: Insert mKCP DNS step after Hysteria2 (after line 730, before the `all_links` block)**

After line 730 (closing `}` of if !failed for h2), insert:

```rust
        if !failed {
            let _ = adapter
                .send_message(
                    &target,
                    MessageContent {
                        text: t!("ops.deploy_step_kcp_dns").into_owned(),
                        markup: None,
                    },
                )
                .await;
        }
        if !failed {
            send_progress(
                &tx,
                8,
                10,
                t!("ops.deploy_step_kcp_dns"),
            );
            match ConfigManager::batch_create_kcp(5, ip_version, &["mld"]).await {
                Ok(result) => {
                    all_links.extend(result.links);
                    let _ = adapter
                        .send_message(
                            &target,
                            MessageContent {
                                text: format!(
                                    "✅ mKCP+DNS伪装 已创建 {} 个配置\n📁 {}",
                                    result.created_count,
                                    result.config_file.as_deref().unwrap_or("?")
                                ),
                                markup: None,
                            },
                        )
                        .await;
                }
                Err(e) => {
                    let _ = tx.send(
                        t!("ops.deploy_fail",
                            "0" => format!("{}: {}", t!("ops.deploy_fail_kcp_dns"), e)
                        )
                        .to_string(),
                    );
                    failed = true;
                }
            }
        }
```

- [ ] **Step 3: Insert mKCP WeChat step after mKCP DNS step**

Immediately after the closing `}` of the mKCP DNS block, insert:

```rust
        if !failed {
            send_progress(
                &tx,
                9,
                10,
                t!("ops.deploy_step_kcp_wechat"),
            );
            match ConfigManager::batch_create_kcp(5, ip_version, &["mlw"]).await {
                Ok(result) => {
                    all_links.extend(result.links);
                    let _ = adapter
                        .send_message(
                            &target,
                            MessageContent {
                                text: format!(
                                    "✅ mKCP+微信伪装 已创建 {} 个配置\n📁 {}",
                                    result.created_count,
                                    result.config_file.as_deref().unwrap_or("?")
                                ),
                                markup: None,
                            },
                        )
                        .await;
                }
                Err(e) => {
                    let _ = tx.send(
                        t!("ops.deploy_fail",
                            "0" => format!("{}: {}", t!("ops.deploy_fail_kcp_wechat"), e)
                        )
                        .to_string(),
                    );
                    failed = true;
                }
            }
        }
```

- [ ] **Step 4: Update security step number from 8 to 10**

Line 756, change:
```rust
send_progress(&tx, 8, 8, t!("ops.deploy_step_security"));
```
To:
```rust
send_progress(&tx, 10, 10, t!("ops.deploy_step_security"));
```

- [ ] **Step 5: Build and lint check**

```bash
cargo build -p aegis 2>&1
```

Expected: build succeeds, 0 errors.

- [ ] **Step 6: Run tests**

```bash
cargo test -p aegis
```

Expected: all 582 tests pass.

- [ ] **Step 7: Commit**

```bash
git add rust/aegis/src/shared/handlers/ops.rs
git commit -m "feat: add mKCP DNS×5 and mKCP WeChat×5 to one-click deploy"
```
