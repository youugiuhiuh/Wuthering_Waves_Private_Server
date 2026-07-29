# CDN Port Provider Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove Aliyun/Dnspod providers and restrict TLS xhttp batch to CDN-supported ports.

**Architecture:** `DnsProvider` enum shrinks to Cloudflare + Route53. `cdn_ports()` returns Cloudflare's 6 CDN proxy ports, Route53 returns empty. `batch_create_xhttp_tls_enhanced` uses CDN ports when available, random ports otherwise.

**Tech Stack:** Rust, no new dependencies.

## Global Constraints

- `RUST_VERSION: "1.88.0"`
- No new crates
- All changes must pass `cargo fmt && cargo clippy -- -D warnings && cargo test`
- TDD: write tests first, confirm they fail, implement, confirm they pass
- Each task ends with a commit

---

### Task 1: Add `cdn_ports()` to DnsProvider + remove Aliyun/Dnspod

**Files:**
- Modify: `rust/aegis/src/core/types.rs:7-33`

**Interfaces:**
- Produces: `DnsProvider::cdn_ports() -> &'static [u16]`

- [ ] **Step 1: Write failing tests**

In `rust/aegis/src/core/types.rs`, after the existing `impl DnsProvider` block (after line 33), append a `#[cfg(test)] mod tests` block (merge with existing tests at bottom of file):

In the existing tests block at bottom of types.rs, replace the two Aliyun/Dnspod assertions with the `cdn_ports` tests:

Remove lines 117-123 (Aliyun/Dnspod asserts):
```rust
// DELETE these lines:
        assert_eq!(DnsProvider::Aliyun.acme_flag(), "dns_ali");
        assert_eq!(
            DnsProvider::Aliyun.credential_names(),
            ("Ali_Key", "Ali_Secret")
        );
        assert_eq!(DnsProvider::Dnspod.acme_flag(), "dns_dp");
        assert_eq!(DnsProvider::Dnspod.credential_names(), ("DP_Id", "DP_Key"));
```

Add after the remaining asserts:
```rust
        assert!(DnsProvider::Cloudflare.cdn_ports().len() >= 6);
        assert!(DnsProvider::Route53.cdn_ports().is_empty());
```

- [ ] **Step 2: Run tests, confirm compile failure**

```bash
cargo test -p aegis -- core::types 2>&1 | tail -5
```

Expected: build error — Aliyun/Dnspod still referenced by other code, `cdn_ports` not defined.

- [ ] **Step 3: Remove Aliyun/Dnspod from DnsProvider + add cdn_ports()**

Replace the DnsProvider enum (lines 7-33) with:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsProvider {
    Cloudflare,
    Route53,
}

impl DnsProvider {
    pub const fn acme_flag(self) -> &'static str {
        match self {
            Self::Cloudflare => "dns_cf",
            Self::Route53 => "dns_aws",
        }
    }

    pub const fn credential_names(self) -> (&'static str, &'static str) {
        match self {
            Self::Cloudflare => ("CF_Token", "CF_Zone_ID"),
            Self::Route53 => ("AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"),
        }
    }

    pub const fn cdn_ports(self) -> &'static [u16] {
        match self {
            Self::Cloudflare => &[443, 8443, 2053, 2083, 2087, 2096],
            Self::Route53 => &[],
        }
    }
}
```

- [ ] **Step 4: Verify types.rs tests pass**

```bash
cargo test -p aegis -- core::types 2>&1 | tail -10
```

Expected: PASS (types.rs tests only).

- [ ] **Step 5: Commit**

```bash
git add rust/aegis/src/core/types.rs
git commit -m "refactor(aegis): remove Aliyun/Dnspod DnsProvider, add cdn_ports()"
```

---

### Task 2: Remove Aliyun/Dnspod from acme.rs

**Files:**
- Modify: `rust/aegis/src/core/security/acme.rs:379-400` (configured_provider_from_configs)
- Modify: `rust/aegis/src/core/security/acme.rs:1670-1800` (tests)

**Interfaces:**
- Consumes: `DnsProvider::Cloudflare`, `DnsProvider::Route53`, `DnsProvider::cdn_ports()`

- [ ] **Step 1: Remove Aliyun/Dnspod from configured_provider_from_configs**

Replace lines 389-399:
```rust
    [
        DnsProvider::Aliyun,
        DnsProvider::Dnspod,
        DnsProvider::Route53,
    ]
    .into_iter()
    .find(|provider| {
        let (first, second) = provider.credential_names();
        has_non_empty_assignment(account_config, first)
            && has_non_empty_assignment(account_config, second)
    })
```
with:
```rust
    (
        has_non_empty_assignment(account_config, "AWS_ACCESS_KEY_ID")
            && has_non_empty_assignment(account_config, "AWS_SECRET_ACCESS_KEY")
    ).then_some(DnsProvider::Route53)
```

- [ ] **Step 2: Update acme tests**

Remove the three Aliyun-specific tests (lines 1760-1790):
- `other_providers_remain_discoverable_from_account_config` — used `DnsProvider::Aliyun`, replace with a Route53 test
- `detects_non_empty_legacy_provider_credentials` — used `Ali_Key`/`Ali_Secret`, replace

Replace those test functions with:

```rust
    #[test]
    fn route53_discoverable_from_account_config() {
        assert_eq!(
            configured_provider_from_configs(
                "",
                "SAVED_AWS_ACCESS_KEY_ID='key'\nSAVED_AWS_SECRET_ACCESS_KEY='secret'\n"
            ),
            Some(DnsProvider::Route53)
        );
    }

    #[test]
    fn route53_not_discovered_when_missing_secret() {
        let config = "SAVED_AWS_ACCESS_KEY_ID='key'\n";
        assert_eq!(configured_provider_from_configs("", config), None);
    }
```

- [ ] **Step 3: Run acme tests**

```bash
cargo test -p aegis -- core::security::acme 2>&1 | tail -10
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add rust/aegis/src/core/security/acme.rs
git commit -m "refactor(aegis): remove Aliyun/Dnspod provider discovery from acme"
```

---

### Task 3: Remove Aliyun/Dnspod from message.rs

**Files:**
- Modify: `rust/aegis/src/shared/handlers/message.rs:504-576` (buttons, guidance, parsing)
- Modify: `rust/aegis/src/shared/handlers/message.rs:769-1055` (tests)

- [ ] **Step 1: Update provider_buttons (remove Aliyun/Dnspod)**

Replace lines 504-527:

```rust
fn provider_buttons() -> Vec<Vec<InlineButton>> {
    vec![
        vec![
            InlineButton {
                text: t!("domain.prov_cf").to_string(),
                data: "xhttp_domain_provider:cloudflare".to_string(),
            },
        ],
        vec![
            InlineButton {
                text: t!("domain.prov_aws").to_string(),
                data: "xhttp_domain_provider:route53".to_string(),
            },
        ],
    ]
}
```

- [ ] **Step 2: Update provider_credential_guidance (remove Aliyun/Dnspod)**

Replace lines 529-536:

```rust
pub(crate) fn provider_credential_guidance(provider: DnsProvider) -> String {
    let prompt = match provider {
        DnsProvider::Cloudflare => t!("domain.cred_prompt_cloudflare"),
        DnsProvider::Route53 => t!("domain.cred_prompt_route53"),
    };
    format!("{prompt}\n\n{}", t!("domain.cred_security_warning"))
}
```

- [ ] **Step 3: Update parse_provider_selection (remove Aliyun/Dnspod)**

Replace lines 568-576:

```rust
fn parse_provider_selection(text: &str) -> Option<DnsProvider> {
    match text.to_lowercase().as_str() {
        "cloudflare" | "cf" | "dns_cf" => Some(DnsProvider::Cloudflare),
        "route53" | "aws" | "dns_aws" => Some(DnsProvider::Route53),
        _ => None,
    }
}
```

- [ ] **Step 4: Update test provider_guidance_runtime (remove Aliyun/Dnspod)**

Replace the for loop at lines 769-775:

```rust
        for provider in [DnsProvider::Cloudflare, DnsProvider::Route53] {
```

- [ ] **Step 5: Update i18n coverage test (remove Aliyun/Dnspod required keys)**

Replace lines 856-867 (the `required` array):

```rust
        let required = [
            "cred_prompt_cloudflare",
            "cred_prompt_route53",
            "cred_security_warning",
            "acme_auth_error",
            "acme_scope_error",
            "acme_dns_error",
            "acme_network_error",
            "acme_unknown_error",
        ];
```

Replace lines 877-906 (the `providers` array, remove Aliyun/Dnspod entries):

```rust
        let providers = [
            (
                "cred_prompt_cloudflare",
                "API_TOKEN,ZONE_ID",
                "https://dash.cloudflare.com/profile/api-tokens",
                &["Zone > DNS > Edit", "Zone > Zone > Read", "Zone ID"][..],
            ),
            (
                "cred_prompt_route53",
                "ACCESS_KEY_ID,SECRET_ACCESS_KEY",
                "https://console.aws.amazon.com/iam/home#/users",
                &[
                    "route53:ListHostedZones",
                    "route53:ListResourceRecordSets",
                    "route53:ChangeResourceRecordSets",
                ][..],
            ),
        ];
```

- [ ] **Step 6: Update provider_fallback_presents_routable_buttons test (remove Aliyun/Dnspod)**

Replace lines 1052-1055:

```rust
                "xhttp_domain_provider:cloudflare".to_string(),
                "xhttp_domain_provider:route53".to_string(),
```

(Just these two data values, no Aliyun/Dnspod)

- [ ] **Step 7: Run message tests**

```bash
cargo test -p aegis -- shared::handlers::message 2>&1 | tail -10
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add rust/aegis/src/shared/handlers/message.rs
git commit -m "refactor(aegis): remove Aliyun/Dnspod from message provider UI"
```

---

### Task 4: Remove Aliyun/Dnspod from xray.rs callback parser

**Files:**
- Modify: `rust/aegis/src/shared/handlers/xray.rs:256-264`

- [ ] **Step 1: Update parse_provider_callback**

Replace lines 256-264:

```rust
fn parse_provider_callback(data: &str) -> Option<crate::core::types::DnsProvider> {
    let provider_str = data.strip_prefix("xhttp_domain_provider:")?;
    match provider_str {
        "cloudflare" | "cf" => Some(crate::core::types::DnsProvider::Cloudflare),
        "route53" | "aws" => Some(crate::core::types::DnsProvider::Route53),
        _ => None,
    }
}
```

- [ ] **Step 2: Run xray handler tests**

```bash
cargo test -p aegis -- shared::handlers::xray 2>&1 | tail -5
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add rust/aegis/src/shared/handlers/xray.rs
git commit -m "refactor(aegis): remove Aliyun/Dnspod callback parsing"
```

---

### Task 5: CDN port selection in batch_create_xhttp_tls_enhanced

**Files:**
- Modify: `rust/aegis/src/core/xray/xhttp.rs:97-146`

**Interfaces:**
- Consumes: `DnsProvider::cdn_ports()`
- Produces: `ConfigManager::batch_create_xhttp_tls_enhanced` uses CDN ports

- [ ] **Step 1: Determine provider from configured domain config**

The `batch_create_xhttp_tls_enhanced` needs the DnsProvider to get `cdn_ports()`. Add `use crate::core::types::DnsProvider;` to xhttp.rs. The function already takes a `domain` which can be used to discover the provider via `AcmeManager::configured_provider_for_domain`.

Replace lines 97-146:

```rust
    pub async fn batch_create_xhttp_tls_enhanced(
        domain: &str,
        certs: &CertPaths,
        ip_version: IpVersion,
    ) -> Result<BatchCreationResult> {
        let _ = AcmeManager::validate_domain(domain)?;

        let cdn_ports: Vec<u16> = match AcmeManager::configured_provider_for_domain(domain)? {
            Some(provider) => provider.cdn_ports().to_vec(),
            None => vec![],
        };

        let node_count: usize = if cdn_ports.is_empty() { 20 } else { cdn_ports.len() };

        let mut rng = StdRng::from_entropy();
        let mut links = Vec::new();
        let mut batch_configs = Vec::new();

        for i in 0..node_count {
            let port: i32 = match cdn_ports.get(i).copied() {
                Some(cdn_port) if crate::core::system::maintenance::MaintenanceManager::is_port_available(cdn_port).await => {
                    cdn_port as i32
                }
                _ => loop {
                    let p = rng.gen_range(10000..60000);
                    if crate::core::xray::port_allocator::PortAllocator::is_port_in_locked_range(p)
                        .await
                    {
                        continue;
                    }
                    if crate::core::system::maintenance::MaintenanceManager::is_port_available(p)
                        .await
                    {
                        break p as i32;
                    }
                },
            };

            let uuid = ConfigManager::generate_wwps_uuid().await?;
            let path = ConfigManager::generate_random_path();

            let (config, link) = ConfigManager::build_tls_xhttp_node(
                i, port, &uuid, domain, certs, ip_version, &path,
            );

            batch_configs.push(config);
            links.push(link);

            let _ =
                crate::core::system::maintenance::MaintenanceManager::allow_port(port as u16).await;
        }

        ConfigManager::create_standalone_config(batch_configs, links, Proto::XHTTP).await
    }
```

- [ ] **Step 2: Update tests in xhttp.rs to include DnsProvider import**

```rust
use crate::core::types::{BatchCreationResult, DnsProvider, IpVersion};
```

- [ ] **Step 3: Add test for CDN port behavior**

Replace the tests module (lines 148-173) with:

```rust
#[cfg(test)]
mod tests {
    use crate::core::security::acme::CertPaths;
    use crate::core::types::{DnsProvider, IpVersion};
    use crate::core::xray::config::ConfigManager;

    #[test]
    fn build_tls_node_returns_matching_config_and_link() {
        let certs = CertPaths {
            fullchain: "full.pem".into(),
            privkey: "key.pem".into(),
        };
        let (config, link) = ConfigManager::build_tls_xhttp_node(
            0,
            2053,
            "uuid",
            "example.com",
            &certs,
            IpVersion::IPv4,
            "/xhttp_test",
        );
        assert_eq!(config["port"], 2053);
        assert!(link.contains("security=tls"));
        assert!(link.contains("host=example%2Ecom"));
    }

    #[test]
    fn cloudflare_returns_six_cdn_ports() {
        assert_eq!(DnsProvider::Cloudflare.cdn_ports(), &[443, 8443, 2053, 2083, 2087, 2096]);
    }

    #[test]
    fn route53_returns_no_cdn_ports() {
        assert!(DnsProvider::Route53.cdn_ports().is_empty());
    }
}
```

- [ ] **Step 4: Run xhttp tests**

```bash
cargo test -p aegis -- core::xray::xhttp 2>&1 | tail -10
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add rust/aegis/src/core/xray/xhttp.rs
git commit -m "feat(aegis): CDN port selection for TLS xhttp batch creation"
```

---

### Task 6: Clean up i18n locale files

**Files:**
- Modify: `rust/aegis/src/resources/i18n/zh.yml`
- Modify: `rust/aegis/src/resources/i18n/en.yml`
- Modify: `rust/aegis/src/resources/i18n/ja.yml`

- [ ] **Step 1: Remove Aliyun/Dnspod locale entries**

In each file, remove lines for `prov_ali`, `prov_dp`, `cred_prompt_aliyun`, `cred_prompt_dnspod`.

zh.yml: Remove lines 515-516 and 520-521.
en.yml: Remove lines 490-491 and 495-496.
ja.yml: Remove lines 490-491 and 495-496.

- [ ] **Step 2: Run full test suite to verify nothing breaks**

```bash
cargo test -p aegis 2>&1 | tail -5
```

Expected: all tests PASS.

- [ ] **Step 3: Commit**

```bash
git add rust/aegis/src/resources/i18n/
git commit -m "chore(aegis): remove Aliyun/Dnspod i18n entries"
```

---

### Task 7: Lint and final verification

- [ ] **Step 1: Run full lint gate**

```bash
cargo fmt && cargo clippy -- -D warnings 2>&1 | tail -5
```

Expected: clean, zero warnings.

- [ ] **Step 2: Run full test suite**

```bash
cargo test -p aegis 2>&1 | tail -5
```

Expected: all tests PASS.

- [ ] **Step 3: Commit any remaining changes**

```bash
git status --short
git diff --check
```

---

### Task 8: Code review

- [ ] **Step 1: Get BASE and HEAD SHAs**

```bash
BASE_SHA=$(git merge-base main HEAD) && HEAD_SHA=$(git rev-parse HEAD) && echo "$BASE_SHA..$HEAD_SHA"
```

- [ ] **Step 2: Dispatch code reviewer**

Use `requesting-code-review` skill. DESCRIPTION: "Removed Aliyun/Dnspod DnsProviders, added cdn_ports(), restricted TLS xhttp batch to CDN-supported ports."

- [ ] **Step 3: Fix any Critical/Important issues**

- [ ] **Step 4: Push branch**

```bash
git push origin feat/cdn-port-provider-cleanup
```

---

### Task 9: Deploy

- [ ] **Step 1: Build release**

```bash
cargo build --release 2>&1 | tail -3
```

- [ ] **Step 2: SCP + restart service on 23.165.248.200**

```bash
scp target/release/aegis root@23.165.248.200:/etc/wwps/aegis/aegis
ssh root@23.165.248.200 'systemctl restart wwps-aegis && sleep 2 && systemctl status wwps-aegis --no-pager | head -10'
```

- [ ] **Step 3: Verify Xray cert errors**

```bash
ssh root@23.165.248.200 'journalctl -u wwps-core --no-pager -n 5 | grep "invalid X509"'
```

Expected: no output (zero cert errors).
