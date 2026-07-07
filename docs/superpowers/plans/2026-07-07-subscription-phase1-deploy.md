# Phase 1: Subscription Server Deployment Pipeline Fixes

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the Rust aegis → Go sub-server deployment pipeline so it reliably deploys, starts, and serves HTTPS subscription responses.

**Architecture:** Aegis (Rust, tonic gRPC server) manages tokens and aggregates proxy configs. Sub-server (Go, chi HTTP) serves `/sub/<token>` with multiple output formats. They communicate over a Unix socket gRPC. Deployment: download → minisign verify → TLS setup → config → systemd → start.

**Tech Stack:** Rust (tonic, rcgen), Go (chi, grpc), Bash (acme.sh, systemctl), GitHub Actions (release workflow)

## Global Constraints

- All Rust code must compile with `cargo build --release` (profile with `opt-level = "z"`, `lto = "thin"`)
- All Rust code must pass `cargo clippy -- -D warnings`
- All Rust code must pass `cargo test`
- Go code must compile with CGO_ENABLED=0
- Go code must pass `go vet ./...`
- gRPC communication: Unix socket at `/var/run/aegis/sub.sock`, permissions 0600
- Sub-server binary: `/usr/local/bin/sub-server`
- Sub-server config: `/etc/wwps/sub-server/config.json`
- Sub-server systemd service: `wwps-sub-server`
- Aegis systemd service name: `wwps-aegis`

---

### Task 1: Fix self-signed certificate (server cert, not CA)

**Files:**

- Modify: `rust/aegis/src/core/subscription/cert.rs:85-114`

**Interfaces:**

- Consumes: `TlsMode::SelfSigned` from `deploy.rs`
- Produces: A valid PEM-encoded X.509 server certificate + key at `paths::sub_server::TLS_CERT` / `paths::sub_server::TLS_KEY`

**Problem:** `cert.rs:99` sets `params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained)` which marks the cert as a CA. Clients reject it as a server TLS certificate. Also, the cert uses SAN `["wwps-sub-server"]` which is not useful for IP connections.

**Fix:** Remove `is_ca`, use `IsCa::ExplicitNoCa`, and add proper server certificate parameters. Use the configured domain/IP for SAN.

- [ ] **Step 1: Read the current file**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/feat/subscription-server
more rust/aegis/src/core/subscription/cert.rs
```

- [ ] **Step 2: Edit `setup_self_signed` to produce a proper server cert**

In `rust/aegis/src/core/subscription/cert.rs`, change `setup_self_signed()`:

```rust
pub fn setup_self_signed() -> Result<TlsResult, String> {
    let cert_dir = paths::sub_server::CERTS_DIR;
    fs::create_dir_all(cert_dir).map_err(|e| format!("create cert dir failed: {e}"))?;

    let cert_path = std::path::Path::new(paths::sub_server::TLS_CERT);
    if cert_path.exists() {
        return Ok(TlsResult::Ready {
            cert_path: paths::sub_server::TLS_CERT.to_string(),
            key_path: paths::sub_server::TLS_KEY.to_string(),
        });
    }

    let mut params = CertificateParams::new(vec!["0.0.0.0".to_string()])
        .map_err(|e| format!("create cert params failed: {e}"))?;
    params.is_ca = IsCa::ExplicitNoCa;
    params.distinguished_name = rcgen::DistinguishedName::new();
    let key_pair = KeyPair::generate().map_err(|e| format!("generate key pair failed: {e}"))?;
    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| format!("self-sign failed: {e}"))?;
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();
    fs::write(paths::sub_server::TLS_CERT, &cert_pem)
        .map_err(|e| format!("write cert failed: {e}"))?;
    fs::write(paths::sub_server::TLS_KEY, &key_pem)
        .map_err(|e| format!("write key failed: {e}"))?;
    Ok(TlsResult::Ready {
        cert_path: paths::sub_server::TLS_CERT.to_string(),
        key_path: paths::sub_server::TLS_KEY.to_string(),
    })
}
```

- [ ] **Step 3: Verify compilation**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/feat/subscription-server/rust/aegis
cargo build --release 2>&1 | tail -5
```

Expected: `Finished release profile`

- [ ] **Step 4: Run tests**

```bash
cargo test 2>&1 | tail -10
```

Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/feat/subscription-server
git add rust/aegis/src/core/subscription/cert.rs
git commit -m "fix(sub): generate server certificate (not CA) in setup_self_signed"
```

---

### Task 2: Fix systemd service ordering and add gRPC readiness check

**Files:**

- Modify: `rust/aegis/src/core/subscription/deploy.rs`

**Interfaces:**

- Consumes: `DeployParams` (domain, port, rate_limit, tls_mode), `TokenManager`
- Produces: `DeployResult { sub_url, token }`

**Problem 1:** The systemd unit has `After=network.target` but no dependency on the aegis service that creates the gRPC socket. Sub-server may start before the socket exists.

**Problem 2:** After `systemctl restart`, there's no check that the gRPC Unix socket is actually ready before returning the subscription URL.

**Fix:** Add `After=wwps-aegis.service` + `BindsTo=wwps-aegis.service` to the systemd unit. Add a retry loop after start that polls for socket existence.

- [ ] **Step 1: Read deploy.rs**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/feat/subscription-server
more rust/aegis/src/core/subscription/deploy.rs
```

- [ ] **Step 2: Fix systemd unit template**

Find the `write_systemd_service` function and change the `[Unit]` section:

```rust
pub fn write_systemd_service(port: u16) -> Result<(), String> {
    let unit = format!(
        "[Unit]\n\
         Description=WWPS Subscription Server\n\
         After=network.target wwps-aegis.service\n\
         BindsTo=wwps-aegis.service\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={bin} --listen-addr=:{port} --aegis-grpc=unix:///var/run/aegis/sub.sock --rate-limit=10\n\
         Restart=always\n\
         RestartSec=5\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        bin = paths::sub_server::BIN,
    );
    // ... rest stays the same
}
```

- [ ] **Step 3: Add readiness check in `run_deploy`**

After the systemctl restart call in `run_deploy`, add a socket readiness check:

```rust
    // Start service
    let status = std::process::Command::new("systemctl")
        .args(["restart", paths::sub_server::SERVICE])
        .status()
        .map_err(|e| format!("systemctl restart failed: {e}"))?;
    if !status.success() {
        return Err("systemctl restart failed".to_string());
    }

    // Wait for gRPC socket to be ready (up to 30 seconds)
    let sock_path = paths::sub_server::GRPC_SOCK;
    let mut ready = false;
    for i in 0..30 {
        if std::path::Path::new(sock_path).exists() {
            ready = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    if !ready {
        return Err(format!(
            "sub-server did not create gRPC socket at {} within 30s",
            sock_path
        ));
    }
```

- [ ] **Step 4: Verify compilation**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/feat/subscription-server/rust/aegis
cargo build --release 2>&1 | tail -5
```

- [ ] **Step 5: Run tests**

```bash
cargo test 2>&1 | tail -10
```

- [ ] **Step 6: Commit**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/feat/subscription-server
git add rust/aegis/src/core/subscription/deploy.rs
git commit -m "fix(sub): add aegis service dependency and gRPC readiness check to systemd unit"
```

---

### Task 3: Make minisign verification optional

**Files:**

- Modify: `rust/aegis/src/core/subscription/deploy.rs`

**Interfaces:**

- Consumes: `DeployParams`, binary data, sig data
- Produces: Skip verification if signature file not available

**Problem:** If the GitHub release has no `.minisig` file (because CI secret is not configured), `download_binary` returns an error when trying to download the signature, and deployment fails.

**Fix:** Change `download_binary` to make the signature download optional. If no signature is available, skip `verify_binary`.

- [ ] **Step 1: Read `download_binary` in deploy.rs**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/feat/subscription-server
grep -n "download_binary\|verify_binary\|run_deploy" rust/aegis/src/core/subscription/deploy.rs
```

- [ ] **Step 2: Modify `download_binary` to make signature optional**

Change the function to return `Option<Vec<u8>>` for sig:

```rust
pub async fn download_binary(
    repo_owner: &str,
    repo_name: &str,
) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
    // ... same client setup, fetch release info ...

    let binary_data = client
        .get(&binary_url)
        .header("User-Agent", "wwps-aegis")
        .send()
        .await
        .map_err(|e| format!("download binary failed: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("read binary body failed: {e}"))?;

    // Signature download is optional
    let sig_data = match client
        .get(&sig_url)
        .header("User-Agent", "wwps-aegis")
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            match resp.bytes().await {
                Ok(b) => Some(b.to_vec()),
                Err(_) => None,
            }
        }
        _ => None,
    };

    Ok((binary_data.to_vec(), sig_data))
}
```

- [ ] **Step 3: Update `run_deploy` to conditionally verify**

```rust
pub async fn run_deploy(params: &DeployParams, tm: &TokenManager) -> Result<DeployResult, String> {
    let repo_owner = "youugiuhiuh";
    let repo_name = "Wuthering_Waves_Private_Server";

    let (binary_data, sig_data) = download_binary(repo_owner, repo_name).await?;
    if let Some(sig) = &sig_data {
        verify_binary(&binary_data, sig, "3", "sub-server")?;
    } else {
        log::warn!("No minisign signature available, skipping binary verification");
    }
    deploy_binary(&binary_data)?;
    // ... rest stays the same
}
```

- [ ] **Step 4: Verify compilation**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/feat/subscription-server/rust/aegis
cargo build --release 2>&1 | tail -5
```

- [ ] **Step 5: Run tests**

```bash
cargo test 2>&1 | tail -10
```

- [ ] **Step 6: Commit**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/feat/subscription-server
git add rust/aegis/src/core/subscription/deploy.rs
git commit -m "fix(sub): make minisign verification optional on missing signature"
```

---

### Task 4: Add arm64 sub-server build target to release workflow

**Files:**

- Modify: `.github/workflows/public-release.yml`

**Interfaces:**

- Consumes: CI build environment
- Produces: `sub-server` (amd64) + `sub-server-arm64` (arm64) release artifacts

**Problem:** The release workflow only builds `GOARCH=amd64`. arm64/aarch64 VPS users cannot deploy sub-server.

**Fix:** Add a second Go build step with `GOARCH=arm64`, produce `sub-server-arm64`. The deploy.rs can later detect architecture and download accordingly.

- [ ] **Step 1: Read the release workflow**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/feat/subscription-server
more .github/workflows/public-release.yml
```

- [ ] **Step 2: Add arm64 build after the existing sub-server build**

Background: in `.github/workflows/public-release.yml`, rename the existing build step to output `sub-server-amd64`, then add arm64. Actually simpler: keep amd64 as `sub-server` and add a parallel arm64 build step.

Add after the "Build Sub-Server Binary" step:

```yaml
- name: Build Sub-Server Binary (arm64)
  run: |
    cd tools/sub-server
    go mod download
    CGO_ENABLED=0 GOTOOLCHAIN=local GOOS=linux GOARCH=arm64 garble -literals -tiny -seed=random build \
      -ldflags="-s -w -X main.version=${{ env.NEW_VERSION }}" \
      -o sub-server-arm64 .
```

Then in "Prepare Distribution Artifacts", add:

```yaml
cp tools/sub-server/sub-server-arm64 dist/ 2>/dev/null || true
```

In "Create Release" `files:` section, add:

```yaml
dist/sub-server-arm64
```

And in the release body:

```yaml
            - `sub-server-arm64`: HTTP subscription endpoint (ARM64)
```

- [ ] **Step 3: Verify YAML syntax (dry run)**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/feat/subscription-server
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/public-release.yml'))" && echo "YAML OK"
```

- [ ] **Step 4: Commit**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/feat/subscription-server
git add .github/workflows/public-release.yml
git commit -m "ci: add arm64 sub-server build target to release workflow"
```

---

### Task 5: Detect CPU architecture in deploy.rs and download correct binary

**Files:**

- Modify: `rust/aegis/src/core/subscription/deploy.rs`

**Interfaces:**

- Consumes: Runtime CPU architecture detection
- Produces: Correct binary download URL (sub-server or sub-server-arm64)

**Problem:** deploy.rs always downloads `sub-server` (amd64). On arm64 VPS, this won't execute.

**Fix:** Detect architecture at runtime and download the matching binary.

- [ ] **Step 1: Read deploy.rs to find the download URLs**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/feat/subscription-server
grep -n "binary_url\|sig_url\|base_url" rust/aegis/src/core/subscription/deploy.rs
```

- [ ] **Step 2: Add architecture detection and conditional URL**

In `download_binary`, at the beginning, detect arch:

```rust
pub async fn download_binary(
    repo_owner: &str,
    repo_name: &str,
) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
    // Detect CPU architecture
    let arch = std::env::consts::ARCH;
    let binary_name = match arch {
        "aarch64" | "arm64" => "sub-server-arm64",
        _ => "sub-server",
    };

    // ... existing release fetch code ...

    let binary_url = format!("{}/{}", &base_url, binary_name);
    let sig_url = format!("{}/{}.minisig", &base_url, binary_name);

    // ... rest stays the same ...
}
```

- [ ] **Step 3: Verify compilation**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/feat/subscription-server/rust/aegis
cargo build --release 2>&1 | tail -5
```

- [ ] **Step 4: Run tests**

```bash
cargo test 2>&1 | tail -10
```

- [ ] **Step 5: Commit**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/feat/subscription-server
git add rust/aegis/src/core/subscription/deploy.rs
git commit -m "fix(sub): detect CPU arch and download correct sub-server binary"
```

---

### Task 6: Improve acme.sh handling (optional install + fallback)

**Files:**

- Modify: `rust/aegis/src/core/subscription/cert.rs`

**Interfaces:**

- Consumes: `TlsMode::DomainAcme` or `TlsMode::IpAcme` from `DeployParams`
- Produces: TLS cert/key file, or fallback to self-signed with warning

**Problem:** acme.sh may not be installed on the VPS. The current code runs it unconditionally and fails if absent.

**Fix:** Check for acme.sh before running. If absent, warn and fall back to self-signed. Also fix the IP certificate command.

- [ ] **Step 1: Read cert.rs**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/feat/subscription-server
more rust/aegis/src/core/subscription/cert.rs
```

- [ ] **Step 2: Fix `setup_acme_domain` and `setup_acme_ip`**

Edit both functions to check for acme.sh first, and add the IP-specific flags:

```rust
fn check_acme_sh() -> bool {
    std::process::Command::new("acme.sh")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn setup_acme_domain(domain: &str) -> Result<TlsResult, String> {
    if !check_acme_sh() {
        return Err("acme.sh not installed, cannot issue domain certificate".to_string());
    }
    // ... rest stays the same ...
}

pub fn setup_acme_ip(ip: &str) -> Result<TlsResult, String> {
    if !check_acme_sh() {
        return Err("acme.sh not installed, cannot issue IP certificate".to_string());
    }
    // Add --server letsencrypt flag for IP cert
    let output = std::process::Command::new("acme.sh")
        .args(["--issue", "--standalone", "-d", ip, "--keylength", "ec-256", "--server", "letsencrypt"])
        .output()
        .map_err(|e| format!("acme.sh execution failed: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "acme.sh IP issue failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    // ... rest (cert copy) stays the same ...
}
```

- [ ] **Step 3: Update `run_deploy` to handle acme.sh failures gracefully**

In `deploy.rs`, the `run_deploy` function calls the cert setup. If acme.sh is not installed and DomainAcme/IpAcme are selected, the deployment should warn and optionally fall back to self-signed:

```rust
   let tls_result = match params.tls_mode {
        TlsMode::DomainAcme => match cert::setup_acme_domain(&params.domain) {
            Ok(r) => r,
            Err(e) => {
                log::warn!("acme.sh domain cert failed ({}), falling back to self-signed", e);
                cert::setup_self_signed()?
            }
        },
        TlsMode::IpAcme => match cert::setup_acme_ip(&params.domain) {
            Ok(r) => r,
            Err(e) => {
                log::warn!("acme.sh IP cert failed ({}), falling back to self-signed", e);
                cert::setup_self_signed()?
            }
        },
        TlsMode::SelfSigned => cert::setup_self_signed()?,
        TlsMode::ReverseProxy => TlsResult::SkippedReverseProxy,
    };
```

- [ ] **Step 4: Verify compilation**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/feat/subscription-server/rust/aegis
cargo build --release 2>&1 | tail -5
```

- [ ] **Step 5: Run tests**

```bash
cargo test 2>&1 | tail -10
```

- [ ] **Step 6: Commit**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/feat/subscription-server
git add rust/aegis/src/core/subscription/cert.rs rust/aegis/src/core/subscription/deploy.rs
git commit -m "fix(sub): add acme.sh check with self-signed fallback, fix IP cert flags"
```

---

## Verification

After all tasks complete:

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/feat/subscription-server
cargo build --release && cargo test && cargo clippy -- -D warnings
```

Expected output:

```
Finished release profile
test result: ok. N passed; 0 failed
```

Then run go vet on the sub-server:

```bash
cd tools/sub-server
go vet ./...
```
