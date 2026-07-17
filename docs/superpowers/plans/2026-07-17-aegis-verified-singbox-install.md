# Aegis Verified Sing-box Install Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the privileged curl/tar Sing-box installer with a bounded, digest-verified, structurally validated GitHub installation pipeline.

**Architecture:** Reuse the fixed GitHub API and asset clients. Download to a private temporary directory, verify SHA256, inspect the tar.gz entry-by-entry, validate the candidate version, then atomically replace the binary before touching service state.

**Tech Stack:** Rust 2024, reqwest, sha2, tempfile, existing GitHub release helpers; `flate2` and `tar` added only because stdlib cannot inspect gzip/tar archives.

## Global Constraints

- Repository is compile-time fixed to `SagerNet/sing-box` and latest GitHub release.
- `GITHUB_TOKEN` is sent only to `api.github.com`; asset requests are unauthenticated.
- Only exact `browser_download_url` and approved HTTPS redirect hosts are accepted.
- Compressed size limit is 128 MiB; expanded regular-file limit is 256 MiB.
- Accept exactly one regular binary at `sing-box-<version>-linux-<arch>/sing-box`.
- No service or installed-binary side effect occurs before all verification passes.
- Any failure preserves the old binary/service and removes private temporary data.

---

### Task 1: Archive Dependencies and Release Metadata

**Files:**
- Modify with Cargo commands: `rust/aegis/Cargo.toml`, `rust/aegis/Cargo.lock`
- Modify: `rust/aegis/src/core/singbox/installer.rs`

**Interfaces:**
- Produces: `SingBoxRelease { tag, version, asset_name, download_url, sha256, size }`
- Produces: `fetch_release(api_client, token, arch) -> Result<SingBoxRelease>`

- [ ] **Step 1: Add required archive dependencies through Cargo**

Run from `rust/aegis`:

```bash
cargo add flate2 --no-default-features --features rust_backend
cargo add tar
```

Expected: lockfile updated without unrelated dependency edits.

- [ ] **Step 2: Add failing tests for fixed repository, exact asset name, required digest, and API-only token**

```rust
#[test]
fn singbox_asset_identity_is_exact() {
    assert_eq!(release_path(), "repos/SagerNet/sing-box/releases/latest");
    assert_eq!(asset_name("1.13.14", "amd64"), "sing-box-1.13.14-linux-amd64.tar.gz");
}
```

- [ ] **Step 3: Verify RED**

Run: `cargo test core::singbox::installer::tests --all-features`
Expected: FAIL because metadata helpers do not exist.

- [ ] **Step 4: Implement fixed metadata selection**

```rust
const OWNER: &str = "SagerNet";
const REPO: &str = "sing-box";
const MAX_ARCHIVE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_EXPANDED_BYTES: u64 = 256 * 1024 * 1024;

fn release_path() -> String { format!("repos/{OWNER}/{REPO}/releases/latest") }
fn asset_name(version: &str, arch: &str) -> String {
    format!("sing-box-{version}-linux-{arch}.tar.gz")
}
```

Use `fetch_github_json`, `find_named_asset`, `parse_digest`, and reject empty `browser_download_url`, absent size, or size above `MAX_ARCHIVE_BYTES`.

- [ ] **Step 5: Run focused tests and commit**

```bash
git add rust/aegis/Cargo.toml rust/aegis/Cargo.lock rust/aegis/src/core/singbox/installer.rs
git commit -m "security: fix Sing-box release trust source"
```

### Task 2: Bounded Private Download

**Files:**
- Modify: `rust/aegis/src/core/singbox/installer.rs`

**Interfaces:**
- Produces: `download_verified_archive(client, release, dir) -> Result<PathBuf>`

- [ ] **Step 1: Add HTTP fixture tests for missing, metadata-mismatched, or oversized Content-Length; streamed underflow/overflow; and hash mismatch**
- [ ] **Step 2: In every failure test, assert old binary bytes and mock service call count remain unchanged**
- [ ] **Step 3: Verify RED with focused installer tests**
- [ ] **Step 4: Implement bounded streaming and digest verification**

```rust
let response = build_asset_request(client, &release.download_url)?.send().await?.error_for_status()?;
let declared = response.content_length().context("Sing-box response missing Content-Length")?;
if declared != release.size || declared > MAX_ARCHIVE_BYTES {
    anyhow::bail!("Sing-box archive exceeds declared size limit");
}
let mut downloaded = 0u64;
while let Some(chunk) = response.bytes_stream().next().await {
    let chunk = chunk?;
    downloaded = downloaded.checked_add(chunk.len() as u64).context("download size overflow")?;
    if downloaded > MAX_ARCHIVE_BYTES { anyhow::bail!("Sing-box archive exceeds stream limit"); }
    hasher.update(&chunk);
    file.write_all(&chunk).await?;
}
file.sync_all().await?;
if downloaded != release.size { anyhow::bail!("Sing-box archive size mismatch"); }
if hex::encode(hasher.finalize()) != release.sha256 { anyhow::bail!("Sing-box SHA256 mismatch"); }
```

Create the directory through `tempfile::Builder` under the configured temp root, set it `0700`, and create the archive with mode `0600`.

- [ ] **Step 5: Run focused tests and commit**

```bash
git add rust/aegis/src/core/singbox/installer.rs
git commit -m "security: bound and verify Sing-box downloads"
```

### Task 3: Safe Tar Inspection

**Files:**
- Modify: `rust/aegis/src/core/singbox/installer.rs`
- Test fixtures: generated in module tests; do not commit opaque archives.

**Interfaces:**
- Produces: `extract_candidate(archive: &Path, output: &Path, release: &SingBoxRelease) -> Result<PathBuf>`

- [ ] **Step 1: Generate failing tar.gz tests for absolute path, `..`, symlink, hardlink, device, duplicate binary, unexpected binary, and expanded overflow**
- [ ] **Step 2: Verify RED**
- [ ] **Step 3: Inspect entries and copy only the exact candidate**

```rust
for entry in archive.entries()? {
    let mut entry = entry?;
    let path = entry.path()?.into_owned();
    if path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        anyhow::bail!("unsafe Sing-box archive path");
    }
    let kind = entry.header().entry_type();
    if kind.is_dir() { continue; }
    if !kind.is_file() { anyhow::bail!("unsupported Sing-box archive entry"); }
    expanded = expanded.checked_add(entry.size()).context("expanded size overflow")?;
    if expanded > MAX_EXPANDED_BYTES { anyhow::bail!("expanded archive exceeds limit"); }
    if path == expected_path {
        if candidate.is_some() { anyhow::bail!("duplicate Sing-box candidate"); }
        entry.unpack(&candidate_path)?;
        candidate = Some(candidate_path.clone());
    } else if path.file_name().is_some_and(|n| n == "sing-box") {
        anyhow::bail!("unexpected Sing-box binary path");
    }
}
```

- [ ] **Step 4: Require exactly one candidate, mode it `0755`, and run all malicious fixture tests**
- [ ] **Step 5: Commit**

```bash
git add rust/aegis/src/core/singbox/installer.rs
git commit -m "security: inspect Sing-box archives before extraction"
```

### Task 4: Version Gate and Atomic Installation

**Files:**
- Modify: `rust/aegis/src/core/singbox/installer.rs`
- Modify: service test seams in the same module only.

- [ ] **Step 1: Add tests for version mismatch, execution failure, rename failure, and success ordering**
- [ ] **Step 2: Assert old binary and service remain unchanged for every pre-replacement failure**
- [ ] **Step 3: Stage beside destination, run `candidate version`, parse its first semantic-version token and require exact equality with the normalized release tag, sync, then rename**

```rust
let output = Command::new(&candidate).arg("version").output().await?;
let reported = parse_singbox_version(&output.stdout)?;
if !output.status.success() || reported != release.version {
    anyhow::bail!("Sing-box candidate version mismatch");
}
fs::copy(&candidate, &staged).await?;
fs::set_permissions(&staged, Permissions::from_mode(0o755)).await?;
File::open(&staged).await?.sync_all().await?;
fs::rename(&staged, singbox::BIN).await?;
```

- [ ] **Step 4: Move old-service cleanup and `create_service()` after successful replacement**
- [ ] **Step 5: Run full gates and dependency vulnerability scan**

Run: `cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all-features && cargo audit`
Expected: all available gates PASS; unavailable `cargo audit` is recorded as unsatisfied.

- [ ] **Step 6: Update only the Sing-box installer audit finding and commit**

```bash
git add rust/aegis/Cargo.toml rust/aegis/Cargo.lock rust/aegis/src/core/singbox/installer.rs docs/audits/2026-07-17-aegis-rust-stability-performance-security-audit.md
git commit -m "security: verify Sing-box before privileged install"
```
