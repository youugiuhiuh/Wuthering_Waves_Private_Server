# Sing-box Management & Xray Service Controls Design

## Scope

Three related enhancements to `rust/aegis` settings management:

1. **Sing-box version management** in `a_wwps_box_menu`, mirroring the existing
   wwps-core (Xray) version management.
2. **wwps-core restart + status buttons** in `a_wwps_core_menu` (currently it
   only has version management).
3. **Sing-box default install channel switches to prerelease**, aligning with
   wwps-core's existing behavior and with the project's actual deployment
   (sing-box 1.14.0-rc.4 runs in production because stable 1.13.20 lacks
   gecko).

## Current State (verified)

| Management menu | Version mgmt | Restart | Status |
|---|---|---|---|
| wwps-core (`a_wwps_core_menu`, menu.rs) | ✅ latest-prerelease upgrade + pick version (5) | ❌ | ❌ |
| Sing-box (`a_wwps_box_menu`, menu.rs) | ❌ | ✅ | ✅ |
| Sing-box user menu (`m_singbox_mgmt`, singbox.rs) | ❌ | ❌ | ❌ |

Key facts:

- `wwps-core` IS the Xray core service: `WWPS_CORE_DEFAULT_SERVICE = xray::DEFAULT_SERVICE` (`"wwps-core"`), binary `/etc/wwps/wwps-core/wwps-core`.
- Sing-box: `singbox::BIN = /etc/wwps/wwps-box/wwps-box`, `CONF_DIR = /etc/wwps/wwps-box/conf`, systemd unit `wwps-box.service` (`ExecStart=... wwps-box run -C .../conf`).
- `WwpsCoreUpgradeManager` (core/system/core_upgrade.rs) is the reference implementation: `fetch_recent_tags(limit)`, `fetch_release(tag: Option<&str>)` — `None` resolves via `fetch_prerelease` (first `prerelease=true` among `releases?per_page=20`), `run_upgrade(tag, adapter, target)` drives progress messages then replace + restart.
- `release_api.rs` provides `fetch_json_from_mirrors`, `fetch_prerelease`, `ReleaseResponse` (with `prerelease: bool`), `ReleaseAsset` — reusable as-is.
- `SingBoxInstaller` (singbox/installer.rs) already has `restart_service()`, `status()`, `is_installed()`, plus private `download_file` / `extract_archive` helpers. Its `fetch_latest_version()` currently uses `releases/latest` (stable).
- `log_audit::service_status(service)` and `monitor::check_service_status(service_name)` exist for generic service status.
- Design decision from the debugging session: sing-box "upgrade to latest" resolves the newest prerelease, matching wwps-core (`fetch_prerelease`). Production already runs 1.14.0-rc.4 because gecko requires >= 1.14.0 and stable is still 1.13.x.

## Decisions (approved)

1. **Symmetry**: Sing-box menu gains version management; wwps-core menu gains restart + status. Scope is the settings menus only — `m_singbox_mgmt` user menu is unchanged.
2. **No backup** of the sing-box binary before replacement (explicit user choice; wwps-core's backup step is not mirrored).
3. **Prerelease is the default channel** for sing-box install (`install()`) and for the new "upgrade to latest" — consistent with wwps-core.
4. Reference implementation is `WwpsCoreUpgradeManager`; reuse `release_api.rs` helpers rather than re-implementing HTTP/mirror logic.
5. Version list shows 5 tags (same as wwps-core).

## Design

### 1. `SingBoxUpgradeManager` — new file `rust/aegis/src/core/singbox/upgrade.rs`

Mirror of `WwpsCoreUpgradeManager`, adapted to sing-box's fixed paths and
tar.gz assets:

- `fetch_recent_tags(limit: usize) -> Result<Vec<String>>` — GitHub
  `repos/SagerNet/sing-box/releases?per_page={limit}` via
  `fetch_json_from_mirrors`, mapped to tag names (owner/repo hardcoded
  `SagerNet/sing-box`, `WWPS_CORE_RELEASE_API_BASE`-style base).
- `fetch_release(tag: Option<&str>) -> Result<SingBoxReleaseInfo>` —
  `releases/tags/{tag}` when given; otherwise `releases?per_page=20` +
  `fetch_prerelease`. `SingBoxReleaseInfo { tag_name, download_url, size }`;
  sha256 best-effort from release body via `extract_sha256_from_body`
  (missing hash does not block — sing-box releases do not carry minisig).
- Download URL follows the existing installer pattern:
  `https://github.com/SagerNet/sing-box/releases/download/v{ver}/sing-box-{ver}-linux-{arch}.tar.gz`
  with `detect_arch_for` reuse.
- `run_upgrade(tag: Option<String>, adapter: &dyn BotAdapter, target: &TargetId) -> Result<()>`
  — progress messages (checking → fetching → downloading with size info →
  extracting → replacing → restarting → success), reusing `download_file` /
  `extract_archive` from `SingBoxInstaller` (promote to `pub(crate)`),
  then `mv` the extracted binary over `singbox::BIN`, `chmod 0o755`, and
  `SingBoxInstaller::restart_service()`.
- `current_version() -> Option<String>` — run `wwps-box version`, parse the
  `sing-box version X.Y.Z` first line; used by the version-pick screen title
  and status display.
- Register in `core/singbox/mod.rs` (`pub mod upgrade; pub use
  upgrade::SingBoxUpgradeManager;`).

Replace order safety (no backup, decision 2): download + extract to a temp
dir first, verify the binary exists, then `mv` over the live binary, then
restart. All failure points precede the replacement.

### 2. Sing-box menu gains version management — `menu.rs` + `singbox.rs`

`a_wwps_box_menu` buttons become (existing restart/status preserved):

- `⬆️ 升级到最新（预发行）` → `sb_upgrade_latest`
- `🏷️ 选择版本` → `sb_upgrade_tags`
- (existing) `🔄 重启 Sing-box` → `a_wwps_box_restart`
- (existing) `📊 Sing-box 状态` → `a_wwps_box_status`

New callbacks in `singbox.rs` handler (matching the `sb_*` convention):

- `sb_upgrade_latest` — answer callback, spawn
  `SingBoxUpgradeManager::run_upgrade(None, ...)`, mirror the
  `a_wwps_core_latest` pattern (progress + failure message).
- `sb_upgrade_tags` — answer callback, `fetch_recent_tags(5)`, render one
  button per tag (`sb_tag:{tag}`) plus Back to `a_wwps_box_menu`; empty →
  `menu.no_version_found`; error → `menu.upgrade_fail`.
- `sb_tag:{tag}` — empty tag → `menu.version_tag_empty`; spawn
  `run_upgrade(Some(tag), ...)`.

### 3. wwps-core menu gains restart + status — `menu.rs`

`a_wwps_core_menu` buttons become (existing version mgmt preserved):

- (existing) `⬆️ 升级到最新` → `a_wwps_core_latest`
- (existing) `🏷️ 选择版本` → `a_wwps_core_tags`
- `🔄 重启 wwps-core` → `a_wwps_core_restart`
- `📊 wwps-core 状态` → `a_wwps_core_status`

New callbacks in `menu.rs`:

- `a_wwps_core_restart` — `WwpsCoreUpgradeManager::from_env()` +
  `restart_service()`, success/failure message.
- `a_wwps_core_status` — `log_audit::service_status("wwps-core")` (or
  `monitor::check_service_status`), render active/inactive.

### 4. Sing-box default install = prerelease — `installer.rs`

`fetch_latest_version()`: replace `releases/latest` with
`repos/SagerNet/sing-box/releases?per_page=20` and return the first
`prerelease == true` tag (parse with the existing curl + `serde_json`
pattern). This changes both `sb_install` (first install) and the
`ops.rs` deploy path that calls `SingBoxInstaller::install()`.

### 5. i18n — `zh.yml` / `en.yml` / `ja.yml`

New keys (reuse existing where possible: `version_tags`,
`no_version_found`, `version_tag_empty`):

- `menu.singbox_upgrade_latest` — button label
- `menu.singbox_upgrade_tags` — button label (or reuse `version_tags`)
- `menu.singbox_upgrade_checking` / `_fetching` / `_downloading` /
  `_extracting` / `_replacing` / `_restarting` / `_success` / `_fail` —
  progress text mirroring the `upgrade.core_*` structure with sing-box copy
- `menu.wwps_core_restart` / `menu.wwps_core_status` — button labels
- `menu.wwps_core_restart_success` / `_fail`, `menu.wwps_core_status_fail`
- Status text format shared for sing-box and wwps-core
  (run/reload + version line).

### 6. Tests

- `upgrade.rs` unit tests (pure functions): version-line parsing from
  `sing-box version 1.14.0-rc.4` output; download URL construction for
  known arch + version; tag-list mapping over a fixture `Vec<ReleaseResponse>`
  (already `Deserialize`-tested in release_api.rs).
- Existing suite regression (`cargo nextest run` full).
- No handler-level unit tests (no baseline exists for handlers).

### 7. Error handling / edge cases

- Download or extract failure: report, service untouched (replacement only
  after both succeed).
- No prerelease found: error surfaced to the caller (install fails with a
  clear message, upgrade reports `upgrade_fail`).
- Empty version list: `menu.no_version_found`; empty tag: `menu.version_tag_empty`.
- `current_version()` unparseable: display "未知版本", upgrade proceeds.

## Files Touched

| File | Change |
|---|---|
| `rust/aegis/src/core/singbox/upgrade.rs` | Create: `SingBoxUpgradeManager` |
| `rust/aegis/src/core/singbox/installer.rs` | `fetch_latest_version` → prerelease; promote `download_file`/`extract_archive` to `pub(crate)` |
| `rust/aegis/src/core/singbox/mod.rs` | Register `upgrade` module + re-export |
| `rust/aegis/src/shared/handlers/singbox.rs` | `sb_upgrade_latest` / `sb_upgrade_tags` / `sb_tag:{tag}` callbacks |
| `rust/aegis/src/shared/handlers/menu.rs` | Both menus' buttons + `a_wwps_core_restart` / `a_wwps_core_status` callbacks |
| `rust/aegis/src/resources/i18n/zh.yml` | new keys |
| `rust/aegis/src/resources/i18n/en.yml` | new keys |
| `rust/aegis/src/resources/i18n/ja.yml` | new keys |

## Verification

From `rust/aegis` (independent crate, no Cargo workspace), before commit:

```bash
cargo fmt && \
cargo clippy --all-targets --all-features -- -D warnings && \
cargo nextest run && \
cargo test --doc
```

All four must pass with zero Clippy warnings.

## Non-Goals

- Changing `m_singbox_mgmt` (user menu) — settings menus only.
- Binary backup / rollback for sing-box upgrades (decision 2).
- Mandatory sha256 verification of sing-box downloads (best-effort only).
- Changing wwps-core's existing upgrade behavior or adding a backup toggle.
- `m_xray_mgmt` (user-facing Xray config menu) — untouched.
