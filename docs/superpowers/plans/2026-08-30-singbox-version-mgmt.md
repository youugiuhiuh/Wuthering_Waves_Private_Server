# Sing-box Management & Xray Service Controls Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add sing-box version management (mirroring wwps-core), add wwps-core restart/status buttons, and switch sing-box default install channel to prerelease.

**Architecture:** A new `SingBoxUpgradeManager` (core/singbox/upgrade.rs) reuses `release_api.rs` helpers (`fetch_json_from_mirrors`, `fetch_prerelease`, `ReleaseResponse`) and `SingBoxInstaller`'s `download_file`/`extract_archive` (promoted to `pub(crate)`). The settings menus gain buttons: sing-box menu gets "upgrade to latest prerelease" + "pick version"; wwps-core menu gets "restart" + "status". `installer.rs::fetch_latest_version` switches from `releases/latest` to the first `prerelease=true` among `releases?per_page=20`, matching wwps-core's existing channel.

**Tech Stack:** Rust (edition 2024), reqwest, serde_json, rust_i18n (zh/en/ja), teloxide adapter, cargo nextest.

**Spec:** `docs/superpowers/specs/2026-08-30-singbox-version-mgmt-design.md`

## Global Constraints

- **No binary backup** on sing-box upgrade (spec decision 2) — replace only after download+extract succeed in a temp dir.
- **Prerelease is the default channel** (spec decision 3) — `install()` and `run_upgrade(None)` both resolve the newest `prerelease=true` release.
- **Settings menus only** — `m_singbox_mgmt` and `m_xray_mgmt` user menus are untouched (spec non-goal).
- **Version list shows 5 tags** (spec decision 5), same as wwps-core.
- Reuse `release_api.rs` helpers; do not re-implement HTTP/mirror logic.
- **i18n**: every new user-facing string exists in `zh.yml`, `en.yml`, `ja.yml`. Reuse `menu.version_tags`, `menu.no_version_found`, `menu.version_tag_empty` where the text matches.
- **Quality gate** (rust-lint-format), from `rust/aegis` before every commit and at the end:
  `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run && cargo test --doc`
- Rust skill rules: `err-no-unwrap-prod` (defensive `unwrap_or` in parsing), `test-arrange-act-assert`, `test-descriptive-names` in `#[cfg(test)] mod tests`.

---

### Task 1: Pure helper functions for `SingBoxUpgradeManager` (TDD)

**Files:**
- Create: `rust/aegis/src/core/singbox/upgrade.rs` (functions + tests first)

**Interfaces:**
- Consumes: `ReleaseResponse` from `crate::core::network::release_api`.
- Produces (pure, unit-testable):
  - `pub fn parse_version_from_output(output: &str) -> Option<String>` — first line starting `sing-box version ` → the version token (no `v` prefix, e.g. `1.14.0-rc.4`).
  - `pub fn build_download_url(version: &str, arch: &str) -> String` — `https://github.com/SagerNet/sing-box/releases/download/v{version}/sing-box-{version}-linux-{arch}.tar.gz`.
  - `pub fn tag_names(releases: &[ReleaseResponse]) -> Vec<String>` — raw `tag_name`s in order.

- [ ] **Step 1: Write the failing tests**

Create `rust/aegis/src/core/singbox/upgrade.rs` with only the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::network::release_api::ReleaseResponse;

    #[test]
    fn test_parse_version_from_output_typical() {
        let out = "sing-box version 1.14.0-rc.4\n\nEnvironment: go1.25.12 linux/amd64\n";
        assert_eq!(parse_version_from_output(out), Some("1.14.0-rc.4".to_string()));
    }

    #[test]
    fn test_parse_version_from_output_stable() {
        let out = "sing-box version 1.13.20\n";
        assert_eq!(parse_version_from_output(out), Some("1.13.20".to_string()));
    }

    #[test]
    fn test_parse_version_from_output_empty() {
        assert_eq!(parse_version_from_output(""), None);
        assert_eq!(parse_version_from_output("not a version line\n"), None);
    }

    #[test]
    fn test_build_download_url() {
        assert_eq!(
            build_download_url("1.14.0-rc.4", "amd64"),
            "https://github.com/SagerNet/sing-box/releases/download/v1.14.0-rc.4/sing-box-1.14.0-rc.4-linux-amd64.tar.gz"
        );
    }

    #[test]
    fn test_tag_names_maps_in_order() {
        let releases = vec![
            ReleaseResponse { tag_name: "v1.14.0-rc.4".to_string(), body: None, assets: vec![], prerelease: true },
            ReleaseResponse { tag_name: "v1.13.20".to_string(), body: None, assets: vec![], prerelease: false },
        ];
        assert_eq!(tag_names(&releases), vec!["v1.14.0-rc.4", "v1.13.20"]);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run core::singbox::upgrade`
Expected: compile error — module `upgrade` not found (not declared in `mod.rs`). Register it in `src/core/singbox/mod.rs`:

```rust
pub mod upgrade;
```

Re-run: Expected: compile errors for the three undefined functions.

- [ ] **Step 3: Implement the functions**

Add above the test module in `upgrade.rs`:

```rust
use crate::core::network::release_api::ReleaseResponse;

/// Parse the version token from `wwps-box version` output
/// (first line `sing-box version <ver>`).
pub fn parse_version_from_output(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let line = line.trim();
        let rest = line.strip_prefix("sing-box version ")?;
        let version = rest.trim();
        (!version.is_empty()).then(|| version.trim_start_matches('v').to_string())
    })
}

/// Build the sing-box release tarball download URL for a version and arch.
pub fn build_download_url(version: &str, arch: &str) -> String {
    format!(
        "https://github.com/SagerNet/sing-box/releases/download/v{}/sing-box-{}-linux-{}.tar.gz",
        version, version, arch
    )
}

/// Map GitHub release responses to their raw tag names, in order.
pub fn tag_names(releases: &[ReleaseResponse]) -> Vec<String> {
    releases.iter().map(|r| r.tag_name.clone()).collect()
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run core::singbox::upgrade`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add rust/aegis/src/core/singbox/upgrade.rs rust/aegis/src/core/singbox/mod.rs
git commit -m "feat: singbox upgrade pure helpers (version parse, url, tag map)"
```

---

### Task 2: `SingBoxUpgradeManager` core methods

**Files:**
- Modify: `rust/aegis/src/core/singbox/upgrade.rs`
- Modify: `rust/aegis/src/core/singbox/installer.rs` (promote helpers to `pub(crate)`)
- Modify: `rust/aegis/src/core/singbox/mod.rs` (re-export)

**Interfaces:**
- Consumes: Task 1 helpers; `fetch_json_from_mirrors`, `fetch_prerelease`, `extract_sha256_from_body` from `release_api.rs`; `SingBoxInstaller::{download_file, extract_archive, detect_arch, restart_service}` (all promoted `pub(crate)`); `singbox::{BIN, DIR, CONF_DIR}` paths; `human_readable_size` from `crate::core::utils`.
- Produces:
  - `pub struct SingBoxReleaseInfo { pub tag_name: String, pub download_url: String, pub size: Option<u64> }`
  - `pub struct SingBoxUpgradeManager { client: reqwest::Client, github_token: Option<String> }` with:
    - `pub fn new() -> Result<Self>`
    - `pub async fn fetch_recent_tags(&self, limit: usize) -> Result<Vec<String>>`
    - `pub async fn fetch_release(&self, tag: Option<&str>) -> Result<SingBoxReleaseInfo>` (None → prerelease)
    - `pub async fn current_version() -> Option<String>`
    - `pub async fn run_upgrade(tag: Option<String>, adapter: &dyn BotAdapter, target: &TargetId) -> Result<()>`

- [ ] **Step 1: Promote installer helpers**

In `src/core/singbox/installer.rs`, change three private items to `pub(crate)`:

```rust
    pub(crate) async fn download_file(url: &str, path: &str) -> Result<()> {
```

```rust
    pub(crate) async fn extract_archive(archive: &str, dest: &str) -> Result<()> {
```

```rust
    pub(crate) fn detect_arch() -> Result<&'static str> {
```

- [ ] **Step 2: Implement the manager**

Append to `src/core/singbox/upgrade.rs` (above the `#[cfg(test)]` module), keeping the Task 1 functions:

```rust
use crate::common::{BotAdapter, MessageContent, TargetId};
use crate::core::network::release_api::{
    ReleaseResponse, fetch_json_from_mirrors, fetch_prerelease,
};
use crate::core::paths::singbox;
use crate::core::singbox::installer::SingBoxInstaller;
use crate::core::utils::human_readable_size;
use anyhow::{Context, Result, anyhow};
use rust_i18n::t;
use std::path::Path;
use tokio::fs;

const SINGBOX_RELEASE_OWNER: &str = "SagerNet";
const SINGBOX_RELEASE_REPO: &str = "sing-box";
const SINGBOX_RELEASE_API_BASE: &str = "https://api.github.com/repos";
const SINGBOX_UPGRADE_TEMP_DIR: &str = "/tmp/sing-box-upgrade";

#[derive(Debug, Clone)]
pub struct SingBoxReleaseInfo {
    pub tag_name: String,
    pub download_url: String,
    pub size: Option<u64>,
}

pub struct SingBoxUpgradeManager {
    client: reqwest::Client,
    github_token: Option<String>,
}

impl SingBoxUpgradeManager {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .context("构建 HTTP 客户端失败")?;
        let token = std::env::var("GITHUB_TOKEN").ok().filter(|v| !v.is_empty());
        Ok(Self { client, github_token: token })
    }

    pub async fn fetch_recent_tags(&self, limit: usize) -> Result<Vec<String>> {
        if limit == 0 {
            return Ok(vec![]);
        }
        let path = format!(
            "{}/{}/releases?per_page={}",
            SINGBOX_RELEASE_OWNER, SINGBOX_RELEASE_REPO, limit
        );
        let bases = vec![SINGBOX_RELEASE_API_BASE.to_string()];
        let releases: Vec<ReleaseResponse> = fetch_json_from_mirrors(
            &self.client,
            &bases,
            &path,
            self.github_token.as_deref(),
        )
        .await?;
        Ok(tag_names(&releases).into_iter().take(limit).collect())
    }

    pub async fn fetch_release(&self, tag: Option<&str>) -> Result<SingBoxReleaseInfo> {
        let bases = vec![SINGBOX_RELEASE_API_BASE.to_string()];
        let release: ReleaseResponse = if let Some(t) = tag {
            let path = format!(
                "{}/{}/releases/tags/{}",
                SINGBOX_RELEASE_OWNER, SINGBOX_RELEASE_REPO, t
            );
            fetch_json_from_mirrors(&self.client, &bases, &path, self.github_token.as_deref())
                .await?
        } else {
            let path = format!(
                "{}/{}/releases?per_page=20",
                SINGBOX_RELEASE_OWNER, SINGBOX_RELEASE_REPO
            );
            fetch_prerelease(&self.client, &bases, &path, self.github_token.as_deref()).await?
        };

        let version = release.tag_name.trim_start_matches('v');
        let arch = SingBoxInstaller::detect_arch()?;
        let download_url = build_download_url(version, arch);
        let tarball_name = format!("sing-box-{}-linux-{}.tar.gz", version, arch);
        let size = release
            .assets
            .iter()
            .find(|a| a.name == tarball_name)
            .and_then(|a| a.size);

        Ok(SingBoxReleaseInfo {
            tag_name: release.tag_name,
            download_url,
            size,
        })
    }

    pub async fn current_version() -> Option<String> {
        let output = tokio::process::Command::new(singbox::BIN)
            .arg("version")
            .output()
            .await
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        parse_version_from_output(&text)
    }

    pub async fn run_upgrade(
        tag: Option<String>,
        adapter: &dyn BotAdapter,
        target: &TargetId,
    ) -> Result<()> {
        let status_msg_id = adapter
            .send_message(
                target,
                MessageContent {
                    text: t!("menu.singbox_upgrade_checking").to_string(),
                    markup: None,
                },
            )
            .await?;

        let manager = SingBoxUpgradeManager::new()?;

        let _ = adapter
            .edit_message(
                target,
                &status_msg_id,
                MessageContent {
                    text: t!("menu.singbox_upgrade_fetching").to_string(),
                    markup: None,
                },
            )
            .await;

        let release = manager.fetch_release(tag.as_deref()).await?;

        let size_str = release
            .size
            .map(human_readable_size)
            .unwrap_or_else(|| t!("menu.singbox_upgrade_unknown_size").to_string());
        let info_text = t!(
            "menu.singbox_upgrade_download_info",
            "0" => release.tag_name.as_str(),
            "1" => size_str.as_str()
        )
        .to_string();
        let _ = adapter
            .edit_message(
                target,
                &status_msg_id,
                MessageContent {
                    text: info_text,
                    markup: None,
                },
            )
            .await;

        fs::create_dir_all(SINGBOX_UPGRADE_TEMP_DIR).await?;
        let archive_path = format!("{}/sing-box.tar.gz", SINGBOX_UPGRADE_TEMP_DIR);
        SingBoxInstaller::download_file(&release.download_url, &archive_path).await?;

        let _ = adapter
            .edit_message(
                target,
                &status_msg_id,
                MessageContent {
                    text: t!("menu.singbox_upgrade_extracting").to_string(),
                    markup: None,
                },
            )
            .await;
        SingBoxInstaller::extract_archive(&archive_path, SINGBOX_UPGRADE_TEMP_DIR).await?;

        let version = release.tag_name.trim_start_matches('v');
        let arch = SingBoxInstaller::detect_arch()?;
        let unpacked_bin = format!(
            "{}/sing-box-{}-linux-{}/sing-box",
            SINGBOX_UPGRADE_TEMP_DIR, version, arch
        );
        if !Path::new(&unpacked_bin).exists() {
            anyhow::bail!("未找到解压后的 sing-box 二进制: {}", unpacked_bin);
        }

        let _ = adapter
            .edit_message(
                target,
                &status_msg_id,
                MessageContent {
                    text: t!("menu.singbox_upgrade_replacing").to_string(),
                    markup: None,
                },
            )
            .await;
        fs::copy(&unpacked_bin, singbox::BIN)
            .await
            .context("复制 sing-box 二进制失败")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(singbox::BIN).await?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(singbox::BIN, perms).await?;
        }

        let _ = adapter
            .edit_message(
                target,
                &status_msg_id,
                MessageContent {
                    text: t!("menu.singbox_upgrade_restarting").to_string(),
                    markup: None,
                },
            )
            .await;
        SingBoxInstaller::restart_service().await?;

        let _ = adapter
            .edit_message(
                target,
                &status_msg_id,
                MessageContent {
                    text: t!("menu.singbox_upgrade_success", "0" => release.tag_name.as_str())
                        .to_string(),
                    markup: None,
                },
            )
            .await;

        let _ = fs::remove_dir_all(SINGBOX_UPGRADE_TEMP_DIR).await;
        Ok(())
    }
}
```

Register the re-export in `src/core/singbox/mod.rs`:

```rust
pub use upgrade::SingBoxUpgradeManager;
```

- [ ] **Step 3: Verify compilation and existing tests**

Run: `cargo nextest run`
Expected: compiles; all existing tests pass. (Manager methods are network-backed — covered by Task 1 pure-function tests; no new unit tests.)

- [ ] **Step 4: Commit**

```bash
git add rust/aegis/src/core/singbox/upgrade.rs rust/aegis/src/core/singbox/installer.rs rust/aegis/src/core/singbox/mod.rs
git commit -m "feat: SingBoxUpgradeManager fetch/upgrade/current-version"
```

---

### Task 3: Sing-box default install = prerelease (TDD)

**Files:**
- Modify: `rust/aegis/src/core/singbox/installer.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: `fn find_prerelease_tag(json: &serde_json::Value) -> Option<String>` (version WITHOUT `v`, e.g. `1.14.0-rc.4`); `fetch_latest_version()` now returns the newest prerelease tag.

- [ ] **Step 1: Write the failing test**

In `installer.rs` (check for an existing `#[cfg(test)] mod tests`; create one at the end if absent):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_find_prerelease_tag_picks_first_prerelease() {
        let json = json!([
            { "tag_name": "v1.13.20", "prerelease": false },
            { "tag_name": "v1.14.0-rc.4", "prerelease": true },
            { "tag_name": "v1.14.0-rc.2", "prerelease": true }
        ]);
        assert_eq!(find_prerelease_tag(&json), Some("1.14.0-rc.4".to_string()));
    }

    #[test]
    fn test_find_prerelease_tag_none_when_no_prerelease() {
        let json = json!([{ "tag_name": "v1.13.20", "prerelease": false }]);
        assert_eq!(find_prerelease_tag(&json), None);
    }

    #[test]
    fn test_find_prerelease_tag_empty_or_non_array() {
        assert_eq!(find_prerelease_tag(&json!([])), None);
        assert_eq!(find_prerelease_tag(&json!({})), None);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run core::singbox::installer`
Expected: compile error — `find_prerelease_tag` not defined.

- [ ] **Step 3: Implement**

Add near `fetch_latest_version` in `installer.rs`:

```rust
    /// Find the newest prerelease tag (version without leading `v`) in a
    /// GitHub `releases?per_page=N` JSON array.
    fn find_prerelease_tag(json: &serde_json::Value) -> Option<String> {
        let releases = json.as_array()?;
        for release in releases {
            let prerelease = release
                .get("prerelease")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if prerelease {
                return release
                    .get("tag_name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim_start_matches('v').to_string());
            }
        }
        None
    }
```

Rewrite `fetch_latest_version`:

```rust
    async fn fetch_latest_version() -> Result<String> {
        let output = tokio::process::Command::new("curl")
            .args([
                "-s",
                "https://api.github.com/repos/SagerNet/sing-box/releases?per_page=20",
            ])
            .output()
            .await
            .context("获取版本信息失败")?;

        let json: serde_json::Value =
            serde_json::from_slice(&output.stdout).context("解析版本信息失败")?;

        Self::find_prerelease_tag(&json).ok_or_else(|| anyhow::anyhow!("未找到预发行版本"))
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run core::singbox::installer`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add rust/aegis/src/core/singbox/installer.rs
git commit -m "feat: sing-box installer defaults to latest prerelease"
```

---

### Task 4: i18n keys (zh/en/ja)

**Files:**
- Modify: `rust/aegis/src/resources/i18n/zh.yml`
- Modify: `rust/aegis/src/resources/i18n/en.yml`
- Modify: `rust/aegis/src/resources/i18n/ja.yml`

**Interfaces:**
- Produces keys consumed by Tasks 5/6: `menu.singbox_upgrade_latest`, `menu.singbox_upgrade_checking`, `menu.singbox_upgrade_fetching`, `menu.singbox_upgrade_download_info` (args 0=version,1=size), `menu.singbox_upgrade_extracting`, `menu.singbox_upgrade_replacing`, `menu.singbox_upgrade_restarting`, `menu.singbox_upgrade_success` (arg 0=version), `menu.singbox_upgrade_fail` (arg 0=err), `menu.singbox_upgrade_unknown_size`, `menu.wwps_core_restart`, `menu.wwps_core_status`, `menu.wwps_core_restart_success`, `menu.wwps_core_restart_fail` (arg 0=err), `menu.wwps_core_status_text` (arg 0=status), `menu.wwps_core_status_running`, `menu.wwps_core_status_stopped`, `menu.wwps_core_status_fail` (arg 0=err). Reuses existing `menu.version_tags`, `menu.no_version_found`, `menu.version_tag_empty`.

- [ ] **Step 1: zh.yml**

Append inside `menu:` (e.g. after the `singbox_status` key at line 36):

```yaml
  singbox_upgrade_latest: "⬆️ 升级到最新（预发行）"
  singbox_upgrade_tags_title: "🏷️ 选择版本\n当前版本: %{0}"
  singbox_upgrade_checking: "🛰️ 正在检查 Sing-box 环境..."
  singbox_upgrade_fetching: "📦 正在获取 Sing-box 版本信息..."
  singbox_upgrade_download_info: "📦 准备下载 Sing-box %{0}\n文件大小: %{1}"
  singbox_upgrade_extracting: "🗜️ 正在解压 Sing-box..."
  singbox_upgrade_replacing: "♻️ 正在替换 Sing-box 二进制..."
  singbox_upgrade_restarting: "🔁 正在重启 wwps-box 服务..."
  singbox_upgrade_success: "✅ Sing-box 已更新至 %{0}！"
  singbox_upgrade_fail: "❌ Sing-box 升级失败: %{0}"
  singbox_upgrade_unknown_size: "未知"
  wwps_core_restart: "🔄 重启 wwps-core"
  wwps_core_status: "📊 wwps-core 状态"
  wwps_core_restart_success: "✅ wwps-core 已重启"
  wwps_core_restart_fail: "❌ wwps-core 重启失败: %{0}"
  wwps_core_status_text: "⚙️ wwps-core 状态: %{0}"
  wwps_core_status_running: "🟢 运行中"
  wwps_core_status_stopped: "🔴 未运行"
  wwps_core_status_fail: "❌ wwps-core 状态获取失败: %{0}"
```

- [ ] **Step 2: en.yml**

Append inside `menu:` after the `singbox_status` key:

```yaml
  singbox_upgrade_latest: "⬆️ Upgrade to Latest (Pre-release)"
  singbox_upgrade_tags_title: "🏷️ Select Version\nCurrent version: %{0}"
  singbox_upgrade_checking: "🛰️ Checking Sing-box environment..."
  singbox_upgrade_fetching: "📦 Fetching Sing-box version info..."
  singbox_upgrade_download_info: "📦 Ready to download Sing-box %{0}\nSize: %{1}"
  singbox_upgrade_extracting: "🗜️ Extracting Sing-box..."
  singbox_upgrade_replacing: "♻️ Replacing Sing-box binary..."
  singbox_upgrade_restarting: "🔁 Restarting wwps-box service..."
  singbox_upgrade_success: "✅ Sing-box updated to %{0}!"
  singbox_upgrade_fail: "❌ Sing-box upgrade failed: %{0}"
  singbox_upgrade_unknown_size: "Unknown"
  wwps_core_restart: "🔄 Restart wwps-core"
  wwps_core_status: "📊 wwps-core Status"
  wwps_core_restart_success: "✅ wwps-core restarted"
  wwps_core_restart_fail: "❌ wwps-core restart failed: %{0}"
  wwps_core_status_text: "⚙️ wwps-core Status: %{0}"
  wwps_core_status_running: "🟢 Running"
  wwps_core_status_stopped: "🔴 Not running"
  wwps_core_status_fail: "❌ wwps-core status check failed: %{0}"
```

- [ ] **Step 3: ja.yml**

Append inside `menu:` after the `singbox_status` key:

```yaml
  singbox_upgrade_latest: "⬆️ 最新版にアップグレード（プレリリース）"
  singbox_upgrade_tags_title: "🏷️ バージョンを選択\n現在のバージョン: %{0}"
  singbox_upgrade_checking: "🛰️ Sing-box 環境を確認中..."
  singbox_upgrade_fetching: "📦 Sing-box バージョン情報を取得中..."
  singbox_upgrade_download_info: "📦 Sing-box %{0} のダウンロード準備\nサイズ: %{1}"
  singbox_upgrade_extracting: "🗜️ Sing-box を展開中..."
  singbox_upgrade_replacing: "♻️ Sing-box バイナリを置換中..."
  singbox_upgrade_restarting: "🔁 wwps-box サービスを再起動中..."
  singbox_upgrade_success: "✅ Sing-box を %{0} に更新しました！"
  singbox_upgrade_fail: "❌ Sing-box のアップグレードに失敗: %{0}"
  singbox_upgrade_unknown_size: "不明"
  wwps_core_restart: "🔄 wwps-core を再起動"
  wwps_core_status: "📊 wwps-core ステータス"
  wwps_core_restart_success: "✅ wwps-core を再起動しました"
  wwps_core_restart_fail: "❌ wwps-core の再起動に失敗: %{0}"
  wwps_core_status_text: "⚙️ wwps-core ステータス: %{0}"
  wwps_core_status_running: "🟢 稼働中"
  wwps_core_status_stopped: "🔴 停止中"
  wwps_core_status_fail: "❌ wwps-core ステータス取得に失敗: %{0}"
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check`
Expected: compiles.

- [ ] **Step 5: Commit**

```bash
git add rust/aegis/src/resources/i18n/zh.yml rust/aegis/src/resources/i18n/en.yml rust/aegis/src/resources/i18n/ja.yml
git commit -m "feat: i18n keys for singbox upgrade and wwps-core service controls"
```

---

### Task 5: Sing-box handler callbacks (`sb_upgrade_latest` / `sb_upgrade_tags` / `sb_tag:`)

**Files:**
- Modify: `rust/aegis/src/shared/handlers/singbox.rs`

**Interfaces:**
- Consumes: `SingBoxUpgradeManager` (Task 2), i18n keys (Task 4).
- Produces callback data strings: `sb_upgrade_latest`, `sb_upgrade_tags`, `sb_tag:{tag}` (menu.rs buttons from Task 6).

- [ ] **Step 1: Add the import**

```rust
use crate::core::singbox::SingBoxUpgradeManager;
```

- [ ] **Step 2: Add the three callback arms**

Append after the `sb_install` arm (all arms end with `Ok(HandlerAction::Done)`):

```rust
        "sb_upgrade_latest" => {
            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("menu.singbox_upgrade_checking").into_owned()),
                )
                .await?;
            let adapter = event.adapter.clone();
            let target = event.target.clone();
            tokio::spawn(async move {
                if let Err(err) =
                    SingBoxUpgradeManager::run_upgrade(None, adapter.as_ref(), &target).await
                {
                    let _ = adapter
                        .send_message(
                            &target,
                            MessageContent {
                                text: t!("menu.singbox_upgrade_fail", "0" => err.to_string())
                                    .into_owned(),
                                markup: None,
                            },
                        )
                        .await;
                }
            });
            Ok(HandlerAction::Done)
        }

        "sb_upgrade_tags" => {
            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("menu.version_tags").into_owned()),
                )
                .await?;

            let reply = async {
                let manager = SingBoxUpgradeManager::new()?;
                let current = SingBoxUpgradeManager::current_version()
                    .await
                    .unwrap_or_else(|| t!("menu.singbox_upgrade_unknown_size").to_string());
                let tags = manager.fetch_recent_tags(5).await?;
                Ok::<_, anyhow::Error>((tags, current))
            }
            .await;

            match reply {
                Ok((tags, current)) if !tags.is_empty() => {
                    let mut buttons = Vec::new();
                    for tag in &tags {
                        buttons.push(vec![InlineButton {
                            text: format!("⬆️ {}", tag),
                            data: format!("sb_tag:{}", tag),
                        }]);
                    }
                    buttons.push(vec![InlineButton {
                        text: t!("menu.back_settings").into(),
                        data: "a_wwps_box_menu".into(),
                    }]);
                    event
                        .adapter
                        .edit_message(
                            &event.target,
                            &event.msg_id,
                            MessageContent {
                                text: t!(
                                    "menu.singbox_upgrade_tags_title",
                                    "0" => &current
                                )
                                .into_owned(),
                                markup: Some(Markup { buttons }),
                            },
                        )
                        .await?;
                }
                Ok(_) => {
                    event
                        .adapter
                        .edit_message(
                            &event.target,
                            &event.msg_id,
                            MessageContent {
                                text: t!("menu.no_version_found").into_owned(),
                                markup: None,
                            },
                        )
                        .await?;
                }
                Err(err) => {
                    event
                        .adapter
                        .edit_message(
                            &event.target,
                            &event.msg_id,
                            MessageContent {
                                text: t!("menu.singbox_upgrade_fail", "0" => err.to_string())
                                    .into_owned(),
                                markup: None,
                            },
                        )
                        .await?;
                }
            }

            Ok(HandlerAction::Done)
        }

        d if d.starts_with("sb_tag:") => {
            let tag = d.strip_prefix("sb_tag:").unwrap_or("").to_string();
            if tag.is_empty() {
                event
                    .adapter
                    .answer_callback(
                        &event.target,
                        &event.callback_id,
                        Some(t!("menu.version_tag_empty").into_owned()),
                    )
                    .await?;
                return Ok(HandlerAction::Done);
            }

            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("menu.singbox_upgrade_checking").into_owned()),
                )
                .await?;

            let adapter = event.adapter.clone();
            let target = event.target.clone();
            tokio::spawn(async move {
                if let Err(err) =
                    SingBoxUpgradeManager::run_upgrade(Some(tag), adapter.as_ref(), &target).await
                {
                    let _ = adapter
                        .send_message(
                            &target,
                            MessageContent {
                                text: t!("menu.singbox_upgrade_fail", "0" => err.to_string())
                                    .into_owned(),
                                markup: None,
                            },
                        )
                        .await;
                }
            });
            Ok(HandlerAction::Done)
        }
```

No external async-runner crate is needed: the `sb_upgrade_tags` arm runs its work in an inline `async { ... }.await` block.

- [ ] **Step 3: Verify**

Run: `cargo nextest run`
Expected: compiles; all tests pass.

- [ ] **Step 4: Commit**

```bash
git add rust/aegis/src/shared/handlers/singbox.rs
git commit -m "feat: singbox upgrade callbacks in telegram handler"
```

---

### Task 6: Menu buttons + wwps-core restart/status callbacks

**Files:**
- Modify: `rust/aegis/src/shared/handlers/menu.rs`

**Interfaces:**
- Consumes: `WwpsCoreUpgradeManager` (already imported), `monitor::check_service_status`, i18n keys (Task 4).
- Produces button data: `sb_upgrade_latest`, `sb_upgrade_tags` (handled in Task 5), `a_wwps_core_restart`, `a_wwps_core_status`.

- [ ] **Step 1: Add sing-box menu buttons**

In `a_wwps_box_menu`, replace the buttons block:

```rust
            let markup = Markup {
                buttons: vec![
                    vec![InlineButton {
                        text: t!("menu.singbox_upgrade_latest").into(),
                        data: "sb_upgrade_latest".into(),
                    }],
                    vec![InlineButton {
                        text: t!("menu.version_tags").into(),
                        data: "sb_upgrade_tags".into(),
                    }],
                    vec![InlineButton {
                        text: t!("ops.singbox_restart").into(),
                        data: "a_wwps_box_restart".into(),
                    }],
                    vec![InlineButton {
                        text: t!("menu.singbox_status").into(),
                        data: "a_wwps_box_status".into(),
                    }],
                    vec![InlineButton {
                        text: t!("menu.back_settings").into(),
                        data: "m_settings".into(),
                    }],
                ],
            };
```

- [ ] **Step 2: Add wwps-core menu buttons**

In `a_wwps_core_menu`, replace the buttons block:

```rust
            let markup = Markup {
                buttons: vec![
                    vec![InlineButton {
                        text: t!("schedule.geo_update_now").into(),
                        data: "a_wwps_core_latest".into(),
                    }],
                    vec![InlineButton {
                        text: t!("menu.version_tags").into(),
                        data: "a_wwps_core_tags".into(),
                    }],
                    vec![InlineButton {
                        text: t!("menu.wwps_core_restart").into(),
                        data: "a_wwps_core_restart".into(),
                    }],
                    vec![InlineButton {
                        text: t!("menu.wwps_core_status").into(),
                        data: "a_wwps_core_status".into(),
                    }],
                    vec![InlineButton {
                        text: t!("menu.back_settings").into(),
                        data: "m_settings".into(),
                    }],
                ],
            };
```

- [ ] **Step 3: Add the two callbacks**

Place after the `wwps_core_tag:` arm (mirror the `a_wwps_core_latest` pattern):

```rust
        "a_wwps_core_restart" => {
            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("menu.wwps_core_restart").into_owned()),
                )
                .await?;

            let result = match WwpsCoreUpgradeConfig::from_env()
                .and_then(WwpsCoreUpgradeManager::new)
            {
                Ok(manager) => manager.restart_service().await,
                Err(err) => Err(err),
            };

            match result {
                Ok(_) => {
                    event
                        .adapter
                        .edit_message(
                            &event.target,
                            &event.msg_id,
                            MessageContent {
                                text: t!("menu.wwps_core_restart_success").into_owned(),
                                markup: None,
                            },
                        )
                        .await?;
                }
                Err(err) => {
                    event
                        .adapter
                        .edit_message(
                            &event.target,
                            &event.msg_id,
                            MessageContent {
                                text: t!(
                                    "menu.wwps_core_restart_fail",
                                    "0" => err.to_string()
                                )
                                .into_owned(),
                                markup: None,
                            },
                        )
                        .await?;
                }
            }

            Ok(HandlerAction::Done)
        }

        "a_wwps_core_status" => {
            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("menu.wwps_core_status").into_owned()),
                )
                .await?;

            let active = crate::core::system::monitor::check_service_status(xray::DEFAULT_SERVICE)
                .await;
            let status_text = if active {
                t!("menu.wwps_core_status_running")
            } else {
                t!("menu.wwps_core_status_stopped")
            };
            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!("menu.wwps_core_status_text", "0" => status_text).into_owned(),
                        markup: None,
                    },
                )
                .await?;

            Ok(HandlerAction::Done)
        }
```

(`restart_service` is `async`, so the nested `match` form is used instead of `and_then` chains.)

- [ ] **Step 4: Verify**

Run: `cargo nextest run`
Expected: compiles; all tests pass.

- [ ] **Step 5: Commit**

```bash
git add rust/aegis/src/shared/handlers/menu.rs
git commit -m "feat: menu buttons for singbox upgrade and wwps-core restart/status"
```

---

### Task 7: Full quality gate and spec review

**Files:**
- None (verification + wrap-up)

- [ ] **Step 1: Run the complete rust-lint-format gate**

Run from `rust/aegis`:

```bash
cargo fmt && \
cargo clippy --all-targets --all-features -- -D warnings && \
cargo nextest run && \
cargo test --doc
```

Expected: all four succeed; zero Clippy warnings; all tests pass (681 + 8 new: 5 upgrade pure + 3 installer prerelease).

- [ ] **Step 2: Diff review against the spec**

`git diff main..HEAD --stat` must cover exactly the 8 files in the spec's Files Touched table. Verify in the diff:
- No binary backup logic added anywhere (spec decision 2).
- `run_upgrade(None)` and `install()` both resolve a prerelease (spec decision 3).
- Only settings menus touched — `m_singbox_mgmt` / `m_xray_mgmt` handlers unchanged.
- `fetch_recent_tags(5)` used (spec decision 5).
- All new user-facing strings exist in all three i18n files.

- [ ] **Step 3: Commit any formatting residue**

```bash
git add -A
git commit -m "chore: formatting fixes"   # only if Step 1 changed files
```

- [ ] **Step 4: Final state**

Run: `git log --oneline main..HEAD`
Expected: 7 commits (Tasks 1-6 + optional formatting), feature branch `feat/hy2-singbox-mgmt` ready for review then finishing-a-development-branch.
