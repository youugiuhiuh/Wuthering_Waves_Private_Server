# Minisign Binary Verification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace simple SHA256 verification with Minisign (Ed25519) signature verification for binary downloads and self-updates in both go/installer and rust/aegis, plus a GitHub Actions workflow to sign release binaries.

**Architecture:** Add a minisign verification layer alongside existing SHA256 (both must pass). Embed public keys in both codebases as constant arrays (supporting key rotation). Add a new GH workflow that signs release assets with a private key stored as an Environment secret.

**Tech Stack:** Go (`aead.dev/minisign` v0.3.0), Rust (`minisign-verify` v0.2.5), GitHub Actions (no third-party actions, only `gh` CLI)

---

### Task 1: Add Go dependency `aead.dev/minisign`

**Files:**
- Modify: `go/installer/go.mod`

- [ ] **Step 1: Add dependency using `go get`**

Run:
```bash
export PATH=$HOME/.local/share/mise/installs/go/1.26.4/bin:$PATH
cd go/installer && go get aead.dev/minisign@v0.3.0
```

Expected: `go.mod` updated with `aead.dev/minisign v0.3.0`, `go.sum` updated.

- [ ] **Step 2: Verify build**

Run:
```bash
export PATH=$HOME/.local/share/mise/installs/go/1.26.4/bin:$PATH
cd go/installer && go build ./...
```

Expected: Build succeeds (no compile errors yet since imports aren't used).

---

### Task 2: Create `go/installer/minisign_verify.go` – verification core + public keys

**Files:**
- Create: `go/installer/minisign_verify.go`
- Test: `go/installer/main_test.go`

- [ ] **Step 1: Create the file with verification logic**

```go
package main

import (
	"encoding/json"
	"fmt"
	"os"
	"strings"

	"aead.dev/minisign"
)

type MinisigInfo struct {
	KeyID           [8]byte
	TrustedComment string
}

var minisignPublicKeys = []string{
	// TODO: replace with actual public key
	"RWRx............placeholder..................publickey..................==",
}

func verifyMinisign(binaryPath, sigPath string, pubKeys []string) (*MinisigInfo, error) {
	binaryData, err := os.ReadFile(binaryPath)
	if err != nil {
		return nil, fmt.Errorf("读取二进制文件失败: %w", err)
	}

	sigFile, err := os.Open(sigPath)
	if err != nil {
		return nil, fmt.Errorf("打开签名文件失败: %w", err)
	}
	defer sigFile.Close()

	sig, err := minisign.SignatureFromReader(sigFile)
	if err != nil {
		return nil, fmt.Errorf("解析签名文件失败: %w", err)
	}

	for _, pubKeyStr := range pubKeys {
		pubKey, err := minisign.PublicKeyFromBase64(pubKeyStr)
		if err != nil {
			continue
		}
		if minisign.Verify(binaryData, sig, pubKey) {
			return &MinisigInfo{
				KeyID:           sig.KeyID,
				TrustedComment: sig.TrustedComment,
			}, nil
		}
	}

	return nil, fmt.Errorf("Minisign 验证失败: 无匹配公钥")
}

func parseTrustedComment(comment string) (version string, assetName string, err error) {
	parts := strings.SplitN(comment, ":", 2)
	if len(parts) != 2 {
		return "", "", fmt.Errorf("无效的可信注释格式: %s", comment)
	}
	return parts[0], parts[1], nil
}

func downloadMinisig(client *http.Client, url, dest string) error {
	return downloadFile(client, url, dest)
}

func findMinisigAsset(release *latestRelease, binaryName string) *releaseAsset {
	sigName := binaryName + ".minisig"
	for i := range release.Assets {
		if release.Assets[i].Name == sigName {
			return &release.Assets[i]
		}
	}
	return nil
}

type minisignPublicKeysFile struct {
	Keys []string `json:"keys"`
}

func init() {
	// Allow key overrides via a JSON config embedded or file
}
```

- [ ] **Step 2: Run test to verify it compiles**

Run:
```bash
export PATH=$HOME/.local/share/mise/installs/go/1.26.4/bin:$PATH
cd go/installer && go build ./...
```

Expected: Build succeeds.

---

### Task 3: Update Go i18n – zh.json, en.json, ja.json

**Files:**
- Modify: `go/installer/i18n/zh.json`
- Modify: `go/installer/i18n/en.json`
- Modify: `go/installer/i18n/ja.json`

- [ ] **Step 1: Add minisign keys to zh.json**

Add after the sha256 block (line 27):
```json

  "minisign.verify_start": "正在验证 Minisign 签名...",
  "minisign.verify_ok": "✅ Minisign 签名验证通过",
  "minisign.verify_failed": "❌ Minisign 签名验证失败: %s",
  "minisign.download_start": "正在下载 Minisign 签名文件...",
  "minisign.pubkey_not_found": "Minisign 公钥未配置",
  "minisign.trusted_comment": "可信注释: %s",
  "minisign.expected_version": "期望版本: %s",
  "minisign.version_mismatch": "Minisign 版本不匹配: 期望 %s, 实际 %s",
  "minisign.asset_mismatch": "Minisign 文件名不匹配: 期望 %s, 实际 %s",
```

- [ ] **Step 2: Add minisign keys to en.json**

Add after sha256 block:
```json

  "minisign.verify_start": "Verifying Minisign signature...",
  "minisign.verify_ok": "✅ Minisign signature verified",
  "minisign.verify_failed": "❌ Minisign verification failed: %s",
  "minisign.download_start": "Downloading Minisign signature file...",
  "minisign.pubkey_not_found": "Minisign public key not configured",
  "minisign.trusted_comment": "Trusted comment: %s",
  "minisign.expected_version": "Expected version: %s",
  "minisign.version_mismatch": "Minisign version mismatch: expected %s, got %s",
  "minisign.asset_mismatch": "Minisign asset mismatch: expected %s, got %s",
```

- [ ] **Step 3: Add minisign keys to ja.json**

Add after sha256 block:
```json

  "minisign.verify_start": "Minisign 署名を検証中...",
  "minisign.verify_ok": "✅ Minisign 署名が確認されました",
  "minisign.verify_failed": "❌ Minisign 検証失敗: %s",
  "minisign.download_start": "Minisign 署名ファイルをダウンロード中...",
  "minisign.pubkey_not_found": "Minisign 公開鍵が設定されていません",
  "minisign.trusted_comment": "信頼できるコメント: %s",
  "minisign.expected_version": "期待されるバージョン: %s",
  "minisign.version_mismatch": "Minisign バージョン不一致: 期待 %s, 実際 %s",
  "minisign.asset_mismatch": "Minisign アセット不一致: 期待 %s, 実際 %s",
```

- [ ] **Step 4: Verify tests pass**

Run:
```bash
export PATH=$HOME/.local/share/mise/installs/go/1.26.4/bin:$PATH
cd go/installer && go test ./...
```

Expected: All tests pass (i18n tests check map keys exist).

---

### Task 4: Integrate minisign verification into `go/installer/main.go`

**Files:**
- Modify: `go/installer/main.go` (lines 555-608, `downloadAndDeployAegis`)

- [ ] **Step 1: Modify `downloadAndDeployAegis()` to add minisign download + verification**

Replace the section from SHA256 verification onward (lines 600-608) with the new dual-verification flow:

```go
	// --- Minisign verification ---
	printYellow(i18n.T("minisign.download_start"))
	assetMinisig := findMinisigAsset(release, binaryName)
	var minisigPassed bool
	if assetMinisig != nil {
		sigURL := assetDownloadURL(assetMinisig, fallbackDownload+".minisig")
		if sigURL != "" {
			sigPath := filepath.Join(tmpDir, binaryName+".minisig")
			if err := downloadMinisig(newHTTPClient(30*time.Second), sigURL, sigPath); err != nil {
				printRed(i18n.T("minisign.verify_failed", err.Error()))
				return ""
			}
			printYellow(i18n.T("minisign.verify_start"))
			info, err := verifyMinisign(binaryPath, sigPath, minisignPublicKeys)
			if err != nil {
				printRed(i18n.T("minisign.verify_failed", err.Error()))
				return ""
			}
			// Validate trusted comment: version:assetName
			expectedVersion := strings.TrimPrefix(ver, "v")
			gotVersion, gotAsset, err := parseTrustedComment(info.TrustedComment)
			if err != nil {
				printRed(i18n.T("minisign.verify_failed", err.Error()))
				return ""
			}
			if !strings.HasPrefix(gotVersion, expectedVersion) {
				printRed(i18n.T("minisign.version_mismatch", expectedVersion, gotVersion))
				return ""
			}
			if gotAsset != binaryName {
				printRed(i18n.T("minisign.asset_mismatch", binaryName, gotAsset))
				return ""
			}
			printGreen(i18n.T("minisign.verify_ok"))
			printYellow(i18n.T("minisign.trusted_comment", info.TrustedComment))
			minisigPassed = true
		}
	}
	if !minisigPassed {
		printRed(i18n.T("minisign.pubkey_not_found"))
		return ""
	}

	// --- SHA256 verification (existing, keep after minisign) ---
	expectedHash, err := findExpectedSHA256(release, binaryName)
	if err != nil {
		printRed(i18n.T("sha256.fetch_failed", err.Error()))
		return ""
	}
	if err := verifySHA256(binaryPath, expectedHash); err != nil {
		printRed(i18n.T("sha256.verify_failed", err.Error()))
		return ""
	}
```

Also add `"strings"` to the imports at the top of the file (it may already be there - line 18 has `"strings"`).

- [ ] **Step 2: Verify it compiles**

Run:
```bash
export PATH=$HOME/.local/share/mise/installs/go/1.26.4/bin:$PATH
cd go/installer && go build ./...
```

Expected: Build succeeds.

---

### Task 5: Write and run Go tests for minisign verification

**Files:**
- Modify: `go/installer/main_test.go`

- [ ] **Step 1: Add minisign-related test**

```go
func TestParseTrustedComment(t *testing.T) {
	tests := []struct {
		input    string
		wantVer  string
		wantName string
		wantErr  bool
	}{
		{"v3.1.8:aegis", "v3.1.8", "aegis", false},
		{"v2.0.0:installer", "v2.0.0", "installer", false},
		{"nocolon", "", "", true},
		{"too:many:colons", "too", "many:colons", false},
		{"", "", "", true},
	}
	for _, tc := range tests {
		ver, name, err := parseTrustedComment(tc.input)
		if tc.wantErr && err == nil {
			t.Errorf("parseTrustedComment(%q) expected error", tc.input)
		}
		if !tc.wantErr {
			if err != nil {
				t.Errorf("parseTrustedComment(%q) unexpected error: %v", tc.input, err)
			}
			if ver != tc.wantVer {
				t.Errorf("parseTrustedComment(%q) ver = %q, want %q", tc.input, ver, tc.wantVer)
			}
			if name != tc.wantName {
				t.Errorf("parseTrustedComment(%q) name = %q, want %q", tc.input, name, tc.wantName)
			}
		}
	}
}

func TestFindMinisigAsset(t *testing.T) {
	release := &latestRelease{
		Assets: []releaseAsset{
			{Name: "aegis", BrowserDownloadURL: "https://example.com/aegis"},
			{Name: "aegis.minisig", BrowserDownloadURL: "https://example.com/aegis.minisig"},
			{Name: "installer", BrowserDownloadURL: "https://example.com/installer"},
			{Name: "installer.minisig", BrowserDownloadURL: "https://example.com/installer.minisig"},
		},
	}
	asset := findMinisigAsset(release, "aegis")
	if asset == nil {
		t.Fatal("findMinisigAsset(aegis) returned nil")
	}
	if asset.Name != "aegis.minisig" {
		t.Errorf("findMinisigAsset(aegis).Name = %q, want %q", asset.Name, "aegis.minisig")
	}
	if asset.BrowserDownloadURL != "https://example.com/aegis.minisig" {
		t.Errorf("findMinisigAsset(aegis).BrowserDownloadURL = %q, want %q", asset.BrowserDownloadURL, "https://example.com/aegis.minisig")
	}
	asset2 := findMinisigAsset(release, "nonexistent")
	if asset2 != nil {
		t.Errorf("findMinisigAsset(nonexistent) = %v, want nil", asset2)
	}
}
```

- [ ] **Step 2: Run tests**

Run:
```bash
export PATH=$HOME/.local/share/mise/installs/go/1.26.4/bin:$PATH
cd go/installer && go test -v -run "TestParseTrustedComment|TestFindMinisigAsset" ./...
```

Expected: All tests PASS.

- [ ] **Step 3: Run full test suite**

Run:
```bash
export PATH=$HOME/.local/share/mise/installs/go/1.26.4/bin:$PATH
cd go/installer && go test ./...
```

Expected: All tests pass.

---

### Task 6: Add Rust dependency `minisign-verify`

**Files:**
- Modify: `rust/aegis/Cargo.toml`

- [ ] **Step 1: Add dependency using `cargo add`**

Run:
```bash
export PATH=$HOME/.cargo/bin:$PATH
cd rust/aegis && cargo add minisign-verify@0.2.5
```

Expected: `Cargo.toml` updated with `minisign-verify = "0.2.5"`, `Cargo.lock` updated.

- [ ] **Step 2: Verify it builds**

Run:
```bash
export PATH=$HOME/.cargo/bin:$PATH
cd rust/aegis && cargo check
```

Expected: Build succeeds.

---

### Task 7: Create `rust/aegis/src/core/crypto/minisign.rs`

**Files:**
- Create: `rust/aegis/src/core/crypto/minisign.rs`
- Create: `rust/aegis/src/core/crypto/mod.rs` (if not exists)
- Modify: if `core/crypto/mod.rs` exists, add `pub mod minisign;`

- [ ] **Step 1: Create `core/crypto/` directory and `mod.rs`**

Check if `core/crypto/` exists. If not:
```bash
mkdir -p rust/aegis/src/core/crypto
```

- [ ] **Step 2: Create `core/crypto/mod.rs`**

```rust
pub mod minisign;
```

- [ ] **Step 3: Create `core/crypto/minisign.rs`**

```rust
use anyhow::{Context, Result, anyhow};

pub const MINISIGN_PUBLIC_KEYS: &[&str] = &[
    // TODO: replace with actual public key
    "RWRx............placeholder..................publickey..................==",
];

pub struct MinisigInfo {
    pub key_id: [u8; 8],
    pub trusted_comment: String,
}

/// Verify a binary against a .minisig file.
/// Tries each public key in order; returns Ok on first match.
pub fn verify_minisign(data: &[u8], sig_bytes: &[u8], pub_keys: &[&str]) -> Result<MinisigInfo> {
    let sig_str = std::str::from_utf8(sig_bytes).context("签名文件不是有效的 UTF-8")?;
    let sig = minisign_verify::Signature::from_string(&sig_str)
        .map_err(|e| anyhow!("解析签名失败: {}", e))?;

    for pub_key_str in pub_keys {
        let pub_key = match minisign_verify::PublicKey::from_base64(pub_key_str) {
            Ok(k) => k,
            Err(_) => continue,
        };
        let key_id = pub_key.key_id();
        if pub_key.verify(data, &sig, true).is_ok() {
            return Ok(MinisigInfo {
                key_id,
                trusted_comment: sig.trusted_comment().to_string(),
            });
        }
    }

    Err(anyhow!("Minisign 验证失败: 无匹配公钥"))
}

/// Parse trusted comment in format `{version}:{asset_name}`
pub fn parse_trusted_comment(comment: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = comment.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(anyhow!("无效的可信注释格式: {}", comment));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_trusted_comment_valid() {
        let (ver, name) = parse_trusted_comment("v3.1.8:aegis").unwrap();
        assert_eq!(ver, "v3.1.8");
        assert_eq!(name, "aegis");
    }

    #[test]
    fn test_parse_trusted_comment_no_colon() {
        assert!(parse_trusted_comment("no-colon").is_err());
    }

    #[test]
    fn test_parse_trusted_comment_empty() {
        assert!(parse_trusted_comment("").is_err());
    }

    #[test]
    fn test_parse_trusted_comment_multi_colon() {
        let (ver, name) = parse_trusted_comment("v1.0.0:file:extra").unwrap();
        assert_eq!(ver, "v1.0.0");
        assert_eq!(name, "file:extra");
    }
}
```

- [ ] **Step 4: Check if `core/crypto/mod.rs` already exists and edit correctly**

Run:
```bash
ls rust/aegis/src/core/crypto/mod.rs 2>/dev/null && echo "EXISTS" || echo "NOT_EXISTS"
```

If exists, read and add `pub mod minisign;` line. If not, create it with the content above.

- [ ] **Step 5: Verify it builds**

Run:
```bash
export PATH=$HOME/.cargo/bin:$PATH
cd rust/aegis && cargo check
```

Expected: Build succeeds. Warnings about unused functions are acceptable at this stage.

---

### Task 8: Update `rust/aegis/src/core/network/release_api.rs` – add minisig asset finder

**Files:**
- Modify: `rust/aegis/src/core/network/release_api.rs`

- [ ] **Step 1: Add `find_minisig_asset` function and test**

Add after `extract_sha256_from_body` (line 121):
```rust
pub fn find_minisig_asset<'a>(assets: &'a [ReleaseAsset], binary_name: &str) -> Option<&'a ReleaseAsset> {
    let sig_name = format!("{}.minisig", binary_name);
    assets.iter().find(|a| a.name == sig_name)
}

#[cfg(test)]
mod minisig_tests {
    use super::*;

    #[test]
    fn test_find_minisig_asset_found() {
        let assets = vec![
            ReleaseAsset {
                name: "aegis".to_string(),
                browser_download_url: "https://example.com/aegis".to_string(),
                url: String::new(),
                size: None,
                digest: None,
            },
            ReleaseAsset {
                name: "aegis.minisig".to_string(),
                browser_download_url: "https://example.com/aegis.minisig".to_string(),
                url: String::new(),
                size: None,
                digest: None,
            },
        ];
        let result = find_minisig_asset(&assets, "aegis");
        assert!(result.is_some());
        assert_eq!(result.unwrap().name, "aegis.minisig");
    }

    #[test]
    fn test_find_minisig_asset_not_found() {
        let assets = vec![
            ReleaseAsset {
                name: "aegis".to_string(),
                browser_download_url: "https://example.com/aegis".to_string(),
                url: String::new(),
                size: None,
                digest: None,
            },
        ];
        assert!(find_minisig_asset(&assets, "aegis").is_none());
    }
}
```

- [ ] **Step 2: Verify it builds and tests pass**

Run:
```bash
export PATH=$HOME/.cargo/bin:$PATH
cd rust/aegis && cargo test -p aegis --lib core::network::release_api 2>&1
```

Expected: Tests pass.

---

### Task 9: Integrate minisign into `rust/aegis/src/core/system/upgrade.rs`

**Files:**
- Modify: `rust/aegis/src/core/system/upgrade.rs`
- Modify: `rust/aegis/src/core/system/core_upgrade.rs`

#### Part A: `upgrade.rs` changes

- [ ] **Step 1: Add imports to `upgrade.rs`**

After the existing imports (line 20), add:
```rust
use crate::core::crypto::minisign::{self, MinisigInfo, MINISIGN_PUBLIC_KEYS};
```

- [ ] **Step 2: Add `download_minisig` method to `UpgradeManager`**

Add after `download_sha256_manifest` method (line 351):
```rust
    async fn download_minisig(
        &self,
        assets: &[ReleaseAsset],
        target_asset: &str,
    ) -> Result<Option<Vec<u8>>> {
        let sig_asset = find_minisig_asset(assets, target_asset);
        let Some(sig_asset) = sig_asset else {
            return Ok(None);
        };
        let sig_url = sig_asset.download_url();
        if sig_url.is_empty() {
            return Ok(None);
        }
        let bytes = self
            .build_request(sig_url)
            .send()
            .await
            .context("下载 Minisign 签名文件失败")?
            .error_for_status()
            .context("Minisign 签名文件返回错误状态")?
            .bytes()
            .await
            .context("读取 Minisign 签名文件失败")?;
        Ok(Some(bytes.to_vec()))
    }
```

- [ ] **Step 3: Add `verify_minisign` method to `UpgradeManager`**

Add after `download_minisig`:
```rust
    async fn verify_downloaded_minisign(
        &self,
        data: &[u8],
        sig_bytes: &[u8],
        artifact: &ReleaseArtifact,
    ) -> Result<()> {
        let info = minisign::verify_minisign(data, sig_bytes, MINISIGN_PUBLIC_KEYS)?;

        let (got_version, got_asset) = minisign::parse_trusted_comment(&info.trusted_comment)?;
        let expected_version = artifact.tag_name.trim_start_matches('v');
        if !got_version.contains(expected_version) {
            anyhow::bail!(
                "Minisign 版本不匹配: 期望包含 {}, 实际 {}",
                expected_version,
                got_version
            );
        }
        if got_asset != artifact.asset_name {
            anyhow::bail!(
                "Minisign 文件名不匹配: 期望 {}, 实际 {}",
                artifact.asset_name,
                got_asset
            );
        }
        Ok(())
    }
```

- [ ] **Step 4: Modify `fetch_latest_release_from_repo` to also find minisig asset**

After the SHA256 block (after line 299, before returning `ReleaseArtifact`), add:
```rust
        let minisig = self
            .download_minisig(&release.assets, &asset.name)
            .await
            .ok()
            .flatten();
```

Add `minisig` to `ReleaseArtifact`:
```rust
        Ok(ReleaseArtifact {
            repository: repository.display_name(),
            tag_name: release.tag_name,
            asset_name: asset.name.clone(),
            download_url,
            sha256,
            size: asset.size,
            minisig,
        })
```

- [ ] **Step 5: Add `minisig` field to `ReleaseArtifact` struct (line 117-125)**

```rust
pub struct ReleaseArtifact {
    pub repository: String,
    pub tag_name: String,
    pub asset_name: String,
    pub download_url: String,
    pub sha256: String,
    pub size: Option<u64>,
    pub minisig: Option<Vec<u8>>,
}
```

- [ ] **Step 6: Modify `download_with_progress` to verify minisig after SHA256**

After the SHA256 check (line 432), before returning `Ok(update_path)` (line 445), add:
```rust
        // Minisign verification
        if let Some(sig_bytes) = &artifact.minisig {
            if let Err(e) = self
                .verify_downloaded_minisign(
                    &std::fs::read(&update_path).context("读取下载文件用于 Minisign 验证失败")?,
                    sig_bytes,
                    artifact,
                )
                .await
            {
                fs::remove_file(&update_path).await.ok();
                anyhow::bail!("Minisign 验证失败: {}", e);
            }
        }
```

- [ ] **Step 7: Verify it builds and tests pass**

Run:
```bash
export PATH=$HOME/.cargo/bin:$PATH
cd rust/aegis && cargo check 2>&1
```

---

### Task 10: Integrate minisign into `rust/aegis/src/core/system/core_upgrade.rs`

**Files:**
- Modify: `rust/aegis/src/core/system/core_upgrade.rs`

- [ ] **Step 1: Add imports**

After existing imports, add:
```rust
use crate::core::crypto::minisign::{self, MINISIGN_PUBLIC_KEYS};
use crate::core::network::release_api::find_minisig_asset;
```

- [ ] **Step 2: Add minisig download in `fetch_release` method**

After the SHA256 block (after line 270, before `Ok(WwpsCoreReleaseInfo {` at line 271), add:
```rust
        let minisig = asset
            .download_url()
            .let_sig = |url| -> Option<String> {
            let sig_asset = find_minisig_asset(&release.assets, &asset.name)?;
            Some(sig_asset.download_url().to_string())
        };
        let minisig_url = find_minisig_asset(&release.assets, &asset.name)
            .map(|a| a.download_url().to_string());
```

Wait, actually `WwpsCoreReleaseInfo` doesn't have a `minisig` field. I need to check if I should add one or handle minisig separately.

Actually, looking at the existing code, `WwpsCoreReleaseInfo` has:
```rust
pub struct WwpsCoreReleaseInfo {
    pub tag_name: String,
    pub download_url: String,
    pub sha256: String,
    pub size: u64,
}
```

I need to add a `minisig` field here too.

Actually, let me think about this differently. The minisig for wwps-core would be stored as a release asset too (e.g., `wwps-core-amd64.zip.minisig`). Let me add the minisig URL/bytes to `WwpsCoreReleaseInfo`.

- [ ] **Step 3: Add `minisig_url` field to `WwpsCoreReleaseInfo` struct**

Find the struct definition and add:
```rust
    pub minisig_url: Option<String>,
```

- [ ] **Step 4: Store minisig URL in `fetch_release`**

After the SHA256 resolution (line 270), before the `Ok(WwpsCoreReleaseInfo {` line (271):
```rust
        let minisig_url = find_minisig_asset(&release.assets, &asset.name)
            .map(|a| a.download_url().to_string());
```

And in the struct:
```rust
        Ok(WwpsCoreReleaseInfo {
            tag_name: release.tag_name,
            download_url,
            sha256,
            size: asset.size.unwrap_or(0),
            minisig_url,
        })
```

- [ ] **Step 5: Modify `download_release` to verify minisig**

After the SHA256 check (line 363), before returning:
```rust
        // Minisign verification
        if let Some(sig_url) = &release.minisig_url {
            let sig_bytes = self
                .build_request(sig_url)
                .send()
                .await
                .context("下载 Minisign 签名文件失败")?
                .error_for_status()
                .context("Minisign 签名文件下载失败")?
                .bytes()
                .await
                .context("读取 Minisign 签名文件失败")?;

            let download_data = std::fs::read(&temp_file)?;
            if let Err(e) = minisign::verify_minisign(&download_data, &sig_bytes, MINISIGN_PUBLIC_KEYS)
                .map(|info| {
                    // Validate trusted comment
                    let (got_version, got_asset) = minisign::parse_trusted_comment(&info.trusted_comment)
                        .expect("可信注释格式无效");
                    let expected_version = release.tag_name.trim_start_matches('v');
                    assert!(
                        got_version.contains(expected_version),
                        "Minisign 版本不匹配: 期望包含 {}, 实际 {}",
                        expected_version,
                        got_version
                    );
                    assert_eq!(
                        got_asset, asset.name,
                        "Minisign 文件名不匹配: 期望 {}, 实际 {}",
                        asset.name, got_asset
                    );
                })
            {
                fs::remove_file(&temp_file).await.ok();
                anyhow::bail!("Minisign 验证失败: {}", e);
            }
        }
```

Wait, this approach is a bit clunky. Let me simplify - just call the helper function.

- [ ] **Step 6: Verify it builds**

Run:
```bash
export PATH=$HOME/.cargo/bin:$PATH
cd rust/aegis && cargo check 2>&1
```

---

### Task 11: Update Rust i18n – zh.yml, en.yml, ja.yml

**Files:**
- Modify: `rust/aegis/src/resources/i18n/zh.yml`
- Modify: `rust/aegis/src/resources/i18n/en.yml`
- Modify: `rust/aegis/src/resources/i18n/ja.yml`

- [ ] **Step 1: Add minisign keys to `upgrade:` section of zh.yml**

Add after the `bot_sha256_mismatch` line (542):
```yaml
  bot_minisign_downloading: "⏳ 正在下载 Minisign 签名..."
  bot_minisign_verifying: "🔐 正在验证 Minisign 签名..."
  bot_minisign_ok: "✅ Minisign 签名验证通过"
  bot_minisign_fail: "❌ Minisign 签名验证失败: %{0}"
```

- [ ] **Step 2: Add minisign keys to `upgrade:` section of en.yml**

Add after the `bot_sha256_mismatch` line (516):
```yaml
  bot_minisign_downloading: "⏳ Downloading Minisign signature..."
  bot_minisign_verifying: "🔐 Verifying Minisign signature..."
  bot_minisign_ok: "✅ Minisign signature verified"
  bot_minisign_fail: "❌ Minisign signature verification failed: %{0}"
```

- [ ] **Step 3: Add minisign keys to `upgrade:` section of ja.yml (read first)**

Read `ja.yml` to find the right section, then add equivalent keys.

- [ ] **Step 4: Verify tests pass**

Run:
```bash
export PATH=$HOME/.cargo/bin:$PATH
cd rust/aegis && cargo test
```

Expected: All tests pass.

---

### Task 12: Add i18n messages to `download_with_progress` for minisign progress

**Files:**
- Modify: `rust/aegis/src/core/system/upgrade.rs`

Add progress messages before/after minisign verification:

```rust
        // Minisign verification
        if let Some(sig_bytes) = &artifact.minisig {
            let _ = adapter
                .edit_message(
                    target,
                    progress_msg_id,
                    MessageContent {
                        text: t!("upgrade.bot_minisign_verifying").to_string(),
                        markup: None,
                    },
                )
                .await;

            if let Err(e) = self
                .verify_downloaded_minisign(
                    &std::fs::read(&update_path).context("读取下载文件用于 Minisign 验证失败")?,
                    sig_bytes,
                    artifact,
                )
                .await
            {
                fs::remove_file(&update_path).await.ok();
                anyhow::bail!("Minisign 验证失败: {}", e);
            }

            let _ = adapter
                .edit_message(
                    target,
                    progress_msg_id,
                    MessageContent {
                        text: t!("upgrade.bot_minisign_ok").to_string(),
                        markup: None,
                    },
                )
                .await;
        }
```

---

### Task 13: Create `.github/workflows/sign-release.yml`

**Files:**
- Create: `.github/workflows/sign-release.yml`

- [ ] **Step 1: Create the signing workflow file**

```yaml
name: Minisign Sign Release

on:
  release:
    types: [published]

permissions:
  contents: write

jobs:
  sign:
    name: Sign Release Assets with Minisign
    environment: production
    runs-on: ubuntu-latest
    steps:
      - name: Install Minisign
        run: |
          curl -fsSL "https://github.com/jedisct1/minisign/releases/download/0.12/minisign-0.12-linux.tar.gz" \
            | tar xz
          sudo install minisign-linux/x86_64/minisign /usr/local/bin/minisign
          minisign -v

      - name: Prepare Secret Key
        env:
          MINISIGN_SECRET_KEY: ${{ secrets.MINISIGN_SECRET_KEY }}
        run: |
          mkdir -p ~/.minisign
          echo "$MINISIGN_SECRET_KEY" > ~/.minisign/minisign.key
          chmod 600 ~/.minisign/minisign.key

      - name: Sign and Upload Release Assets
        env:
          GH_TOKEN: ${{ github.token }}
        working-directory: ${{ runner.temp }}
        run: |
          TAG="${{ github.event.release.tag_name }}"
          ASSETS=$(gh release view "$TAG" --json assets -q '.assets[].name')

          if [ -z "$ASSETS" ]; then
            echo "No assets found for release $TAG"
            exit 0
          fi

          echo "Signing release $TAG..."
          for asset in $ASSETS; do
            # Skip already-signed files
            case "$asset" in
              *.minisig) echo "Skipping $asset (already a signature)"; continue ;;
            esac

            echo "::group::Signing $asset"
            gh release download "$TAG" --pattern "$asset" --clobber
            minisign -S -m "$asset" -t "${TAG}:${asset}" -W
            gh release upload "$TAG" "${asset}.minisig" --clobber
            sha256sum "$asset"
            echo "::endgroup::"
          done

          echo "✅ Signing complete for $TAG"
```

- [ ] **Step 2: Validate workflow syntax**

GitHub Actions syntax can be checked online. We can verify basic YAML validity:
```bash
# Install yq or use python to validate
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/sign-release.yml'))" && echo "YAML OK"
```

---

### Task 14: Final lint + format pass (Go)

**Files:**
- All Go files in `go/installer/`

- [ ] **Step 1: Run `gofmt`**

Run:
```bash
export PATH=$HOME/.local/share/mise/installs/go/1.26.4/bin:$PATH
cd go/installer && gofmt -l -w .
```

Expected: No errors. Files are reformatted.

- [ ] **Step 2: Run `go vet`**

Run:
```bash
export PATH=$HOME/.local/share/mise/installs/go/1.26.4/bin:$PATH
cd go/installer && go vet ./...
```

Expected: No vet warnings.

- [ ] **Step 3: Run all tests**

```bash
export PATH=$HOME/.local/share/mise/installs/go/1.26.4/bin:$PATH
cd go/installer && go test ./...
```

Expected: All tests pass.

---

### Task 15: Final lint + format pass (Rust)

**Files:**
- All Rust files in `rust/aegis/src/`

- [ ] **Step 1: Run `cargo fmt`**

Run:
```bash
export PATH=$HOME/.cargo/bin:$PATH
cd rust/aegis && cargo fmt
```

Expected: No errors.

- [ ] **Step 2: Run `cargo clippy`**

Run:
```bash
export PATH=$HOME/.cargo/bin:$PATH
cd rust/aegis && cargo clippy --all-targets 2>&1
```

Expected: No new warnings.

- [ ] **Step 3: Run all tests**

```bash
export PATH=$HOME/.cargo/bin:$PATH
cd rust/aegis && cargo test
```

Expected: All tests pass.

---

## Key Design Decisions

| Decision | Choice |
|----------|--------|
| Public key location | Hardcoded in `minisignPublicKeys` (Go) and `MINISIGN_PUBLIC_KEYS` (Rust) as constant arrays |
| Verification order | Minisign first (fails fast), SHA256 second (defense-in-depth) |
| Trusted comment | `{version}:{asset_name}` format, validated at both client sides |
| Key rotation | Multiple keys in array; first match succeeds — new key added, old key removed after transition |
| Workflow safety | Zero third-party actions, `contents: write` only, `production` environment with required reviewers |
| Signature file name | `{binary}.minisig` alongside the binary in release assets |
