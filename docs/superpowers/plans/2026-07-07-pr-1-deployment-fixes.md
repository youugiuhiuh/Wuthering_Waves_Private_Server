# PR-1: Phase 1 Remaining Deployment Fixes

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix remaining deployment pipeline issues so `aegis` reliably deploys `sub-server` with correct systemd dependency, gRPC readiness check, arch-aware binary, retry logic, and optional minisign verification.

**Architecture:** Two processes (Rust aegis on Unix socket gRPC, Go sub-server as HTTP endpoint). Fixes concentrate in `deploy.rs` (systemd, retry, gRPC readiness, arch-aware) and `public-release.yml` (arm64 build). Minisign soft-fail spans `deploy.rs` and `minisign.rs`.

**Tech Stack:** Rust (tokio, tonic, reqwest), Go (chi, gRPC), systemd, GitHub Actions

## Global Constraints

- All Rust changes must pass `cargo fmt && cargo clippy -- -D warnings && cargo test`
- All Go changes must pass `go fmt ./... && go test ./... && go vet ./...`
- All new code must have tests
- Systemd unit must use `After=wwps-aegis.service` + `BindsTo=wwps-aegis.service`
- gRPC readiness check must poll for socket existence with 30s timeout
- Arch-aware binary: resolve name from GOARCH → `sub-server` (amd64) or `sub-server-arm64` (arm64)
- Retry: 3 attempts with exponential backoff (2s, 4s, 8s)
- Minisign: if no `.minisig` file in release, log warning and skip verification (do not fail deploy)
- Worktree path: `/home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-1-deployment-fixes`

---

### Task 1: Fix systemd unit — Add aegis dependency

**Files:**
- Modify: `rust/aegis/src/core/subscription/deploy.rs`

**Interfaces:**
- Consumes: `write_systemd_service(port, tls_cert, tls_key) -> Result<(), String>` (existing)
- Produces: updated `write_systemd_service()` with `After=wwps-aegis.service` + `BindsTo=wwps-aegis.service`

- [ ] **Step 1: Write the failing test**

Add a test that verifies the generated systemd unit string contains the expected dependency lines:

```rust
#[test]
fn test_systemd_unit_has_aegis_dependency() {
    let port = 8443;
    let cert = "/etc/wwps/sub-server/certs/fullchain.pem";
    let key = "/etc/wwps/sub-server/certs/privkey.pem";
    let result = super::generate_systemd_unit(port, cert, key);
    assert!(result.contains("After=wwps-aegis.service"), "unit should depend on aegis");
    assert!(result.contains("BindsTo=wwps-aegis.service"), "unit should bind to aegis");
}
```

Note: you may need to extract systemd unit generation into a testable function `generate_systemd_unit()` if the current code writes directly.

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-1-deployment-fixes/rust/aegis
cargo test test_systemd_unit_has_aegis_dependency -- --test-threads=1 2>&1
```
Expected: FAIL — systemd unit currently uses `After=network.target` only

- [ ] **Step 3: Implement**

In `deploy.rs`, refactor `write_systemd_service` to extract unit content generation into `generate_systemd_unit()`:

```rust
pub fn generate_systemd_unit(port: u16, tls_cert: &str, tls_key: &str) -> String {
    let tls_flags = if !tls_cert.is_empty() && !tls_key.is_empty() {
        format!(" --tls-cert={} --tls-key={}", tls_cert, tls_key)
    } else {
        String::new()
    };
    format!(
        "[Unit]\n\
         Description=WWPS Subscription Server\n\
         After=network.target\n\
         After=wwps-aegis.service\n\
         BindsTo=wwps-aegis.service\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={bin} --listen-addr=:{port} --aegis-grpc=unix:///var/run/aegis/sub.sock --rate-limit=10{tls_flags}\n\
         Restart=always\n\
         RestartSec=5\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        bin = paths::sub_server::BIN,
    )
}
```

Then update `write_systemd_service` to call `generate_systemd_unit()` and write the result.

- [ ] **Step 4: Run test to verify it passes**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-1-deployment-fixes/rust/aegis
cargo test test_systemd_unit_has_aegis_dependency -- --test-threads=1 2>&1
```
Expected: PASS

- [ ] **Step 5: Run full test suite**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-1-deployment-fixes/rust/aegis
cargo fmt && cargo clippy -- -D warnings && cargo test 2>&1 | tail -15
```
Expected: fmt ok, clippy clean, all tests pass

- [ ] **Step 6: Commit**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-1-deployment-fixes
git add rust/aegis/src/core/subscription/deploy.rs
git commit -m "fix(deploy): add After+BindsTo dependency on wwps-aegis.service"
```

---

### Task 2: Add gRPC readiness wait loop

**Files:**
- Modify: `rust/aegis/src/core/subscription/deploy.rs`

**Interfaces:**
- Consumes: `path` (gRPC socket path), `max_wait: Duration` from above
- Produces: `wait_for_grpc_socket(path, timeout)` function called in `run_deploy()` after systemctl start

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn test_grpc_readiness_timeout() {
    // Should return error for non-existent socket with short timeout
    let result = super::wait_for_grpc_socket("/nonexistent/sock", std::time::Duration::from_millis(10));
    assert!(result.is_err(), "should timeout on non-existent socket");
    assert!(result.unwrap_err().contains("timed out"), "error should mention timeout");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-1-deployment-fixes/rust/aegis
cargo test test_grpc_readiness_timeout -- --test-threads=1 2>&1
```
Expected: FAIL — `wait_for_grpc_socket` not defined

- [ ] **Step 3: Implement**

Add `wait_for_grpc_socket()` to `deploy.rs`:

```rust
pub async fn wait_for_grpc_socket(socket_path: &str, timeout: std::time::Duration) -> Result<(), String> {
    let start = std::time::Instant::now();
    loop {
        if tokio::fs::metadata(socket_path).await.is_ok() {
            return Ok(());
        }
        if start.elapsed() >= timeout {
            return Err(format!("timed out waiting for gRPC socket: {socket_path}"));
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}
```

In `run_deploy()`, after `write_systemd_service()` and `open_firewall_port()`, add:

```rust
wait_for_grpc_socket(paths::sub_server::GRPC_SOCK, std::time::Duration::from_secs(30)).await?;
```

- [ ] **Step 4: Run tests**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-1-deployment-fixes/rust/aegis
cargo test test_grpc_readiness_timeout -- --test-threads=1 2>&1
```
Expected: PASS

Then run full suite:
```bash
cargo fmt && cargo clippy -- -D warnings && cargo test 2>&1 | tail -15
```

- [ ] **Step 5: Commit**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-1-deployment-fixes
git add rust/aegis/src/core/subscription/deploy.rs
git commit -m "fix(deploy): add gRPC readiness wait loop with 30s timeout"
```

---

### Task 3: Add arch-aware binary download

**Files:**
- Modify: `rust/aegis/src/core/subscription/deploy.rs`

**Interfaces:**
- Consumes: `download_binary(repo_owner, repo_name)` (existing, refactored)
- Produces: `resolve_binary_name() -> &'static str` returning `"sub-server"` or `"sub-server-arm64"`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn test_resolve_binary_name() {
    let name = super::resolve_binary_name();
    // Should be one of the expected names
    assert!(name == "sub-server" || name == "sub-server-arm64",
        "binary name should match known architectures");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-1-deployment-fixes/rust/aegis
cargo test test_resolve_binary_name -- --test-threads=1 2>&1
```
Expected: FAIL

- [ ] **Step 3: Implement**

Add `resolve_binary_name()` to `deploy.rs`:

```rust
/// Resolve the sub-server binary asset name based on target architecture.
pub fn resolve_binary_name() -> &'static str {
    #[cfg(target_arch = "aarch64")]
    { "sub-server-arm64" }
    #[cfg(not(target_arch = "aarch64"))]
    { "sub-server" }
}
```

In `download_binary()`, use `resolve_binary_name()` for the binary URL instead of hardcoded `"sub-server"`. The minisig URL remains `"sub-server.minisig"` (GitHub appends `.minisig` to asset name).

- [ ] **Step 4: Run tests**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-1-deployment-fixes/rust/aegis
cargo test test_resolve_binary_name -- --test-threads=1 2>&1
```
Expected: PASS

- [ ] **Step 5: Commit**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-1-deployment-fixes
git add rust/aegis/src/core/subscription/deploy.rs
git commit -m "fix(deploy): arch-aware binary name resolution (amd64/arm64)"
```

---

### Task 4: Make minisign verification optional

**Files:**
- Modify: `rust/aegis/src/core/subscription/deploy.rs`
- Modify: `rust/aegis/src/core/subscription/minisign.rs`

**Interfaces:**
- Consumes: `download_binary()` returns `(Vec<u8>, Vec<u8>)` — second is sig data
- Produces: if sig data is empty, `run_deploy()` skips `verify_binary()` with warning

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn test_verify_binary_empty_sig_skips() {
    let result = super::verify_binary(&[0u8; 10], &[], "3", "sub-server");
    // Should not error when sig is empty — treat as optional
    assert!(result.is_err(), "empty sig should not pass (no sig to verify)");
}
```

Actually, this needs to be thought through: The current `verify_binary()` calls `minisign::verify_minisign()` which requires non-empty sig. Instead, the logic in `run_deploy()` should check if sig is empty and skip. The test should verify the wrapping logic:

Create a helper in deploy.rs:
```rust
pub fn should_verify_binary(sig_data: &[u8]) -> bool {
    !sig_data.is_empty()
}
```

Test:
```rust
#[test]
fn test_should_verify_binary() {
    assert!(!super::should_verify_binary(&[]), "empty sig = skip");
    assert!(super::should_verify_binary(&[0u8; 64]), "non-empty sig = verify");
}
```

- [ ] **Step 2: Run test**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-1-deployment-fixes/rust/aegis
cargo test test_should_verify_binary -- --test-threads=1 2>&1
```
Expected: FAIL

- [ ] **Step 3: Implement**

Add `should_verify_binary()` to `deploy.rs`:

```rust
pub fn should_verify_binary(sig_data: &[u8]) -> bool {
    !sig_data.is_empty()
}
```

In `run_deploy()`, change:
```rust
let (binary_data, sig_data) = download_binary(repo_owner, repo_name).await?;
verify_binary(&binary_data, &sig_data, "3", "sub-server")?;
```

To:
```rust
let (binary_data, sig_data) = download_binary(repo_owner, repo_name).await?;
if should_verify_binary(&sig_data) {
    verify_binary(&binary_data, &sig_data, "3", resolve_binary_name())?;
} else {
    log::warn!("no minisig signature found, skipping binary verification");
}
```

Note: Also fix the asset name in `verify_binary` call — use `resolve_binary_name()` instead of hardcoded `"sub-server"`.

- [ ] **Step 4: Run tests**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-1-deployment-fixes/rust/aegis
cargo fmt && cargo clippy -- -D warnings && cargo test 2>&1 | tail -15
```
Expected: all pass

- [ ] **Step 5: Commit**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-1-deployment-fixes
git add rust/aegis/src/core/subscription/deploy.rs
git commit -m "fix(deploy): make minisign verification optional, soft-fail when no .minisig"
```

---

### Task 5: Add retry on download failure

**Files:**
- Modify: `rust/aegis/src/core/subscription/deploy.rs`

**Interfaces:**
- Consumes: `download_binary()` (existing, but renamed to `download_binary_inner` or wrapped)
- Produces: `download_binary_with_retry()` with max_attempts=3, backoff 2s/4s/8s

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn test_download_retry_exhausts_attempts() {
    // Simulate: retry should fail after max attempts with appropriate error
    let result = super::download_with_retry(
        || Err::<Vec<u8>, String>("mock failure".to_string()),
        3,
        std::time::Duration::from_millis(1),
    );
    assert!(result.is_err(), "should fail after exhausting retries");
    // The actual error message depends on implementation — verify attempts exhausted
}
```

- [ ] **Step 2: Run test**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-1-deployment-fixes/rust/aegis
cargo test test_download_retry_exhausts_attempts -- --test-threads=1 2>&1
```
Expected: FAIL

- [ ] **Step 3: Implement**

Add retry helper to `deploy.rs`:

```rust
pub async fn download_with_retry<F, Fut, T>(f: F, max_attempts: u32) -> Result<T, String>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let mut last_err = String::new();
    for attempt in 1..=max_attempts {
        match f().await {
            Ok(val) => return Ok(val),
            Err(e) => {
                last_err = e;
                if attempt < max_attempts {
                    let delay = std::time::Duration::from_secs(2u64.pow(attempt));
                    log::warn!("download attempt {}/{} failed, retrying in {}s: {}", attempt, max_attempts, delay.as_secs(), last_err);
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }
    Err(format!("download failed after {max_attempts} attempts: {last_err}"))
}
```

In `run_deploy()`, wrap the download call:
```rust
let (binary_data, sig_data) = download_with_retry(|| download_binary(repo_owner, repo_name), 3).await?;
```

- [ ] **Step 4: Run tests**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-1-deployment-fixes/rust/aegis
cargo fmt && cargo clippy -- -D warnings && cargo test 2>&1 | tail -15
```
Expected: all pass

- [ ] **Step 5: Commit**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-1-deployment-fixes
git add rust/aegis/src/core/subscription/deploy.rs
git commit -m "fix(deploy): add retry with exponential backoff on download failure"
```

---

### Task 6: CI — Build arm64 sub-server + dual signing

**Files:**
- Modify: `.github/workflows/public-release.yml`

**Interfaces:**
- Produces: `sub-server-arm64` binary artifact + `sub-server-arm64.minisig` signature in release

- [ ] **Step 1: Update CI to build sub-server for both amd64 and arm64**

In `public-release.yml`, after the existing "Build Sub-Server Binary" step, add an arm64 build:

```yaml
      - name: Build Sub-Server Binary (arm64)
        run: |
          cd tools/sub-server
          go mod download
          CGO_ENABLED=0 GOTOOLCHAIN=local GOOS=linux GOARCH=arm64 garble -literals -tiny -seed=random build \
            -ldflags="-s -w -X main.version=${{ env.NEW_VERSION }}" \
            -o sub-server-arm64 .
```

- [ ] **Step 2: Sign arm64 binary**

Modify the "Sign Sub-Server Binary" step to also sign the arm64 binary:

```yaml
      - name: Sign Sub-Server Binaries
        env:
          MINISIGN_PRIVATE_KEY: ${{ secrets.MINISIGN_PRIVATE_KEY }}
        run: |
          if [ -n "$MINISIGN_PRIVATE_KEY" ]; then
            KEY_FILE=$(mktemp)
            echo "$MINISIGN_PRIVATE_KEY" > "$KEY_FILE"
            cd tools/sub-server
            
            # Sign amd64
            minisign -S -s "$KEY_FILE" \
              -m sub-server \
              -t "${{ env.NEW_VERSION }}:sub-server" \
              -x sub-server.minisig
            echo "✅ amd64 binary signed"
            
            # Sign arm64
            minisign -S -s "$KEY_FILE" \
              -m sub-server-arm64 \
              -t "${{ env.NEW_VERSION }}:sub-server-arm64" \
              -x sub-server-arm64.minisig
            echo "✅ arm64 binary signed"
            
            rm -f "$KEY_FILE"
          else
            echo "⚠️ MINISIGN_PRIVATE_KEY not set, skipping signatures"
          fi
```

- [ ] **Step 3: Update release artifacts**

Modify the "Prepare Distribution Artifacts" step:

```yaml
      - name: Prepare Distribution Artifacts
        run: |
          mkdir -p dist
          cp rust/aegis/target/release/aegis dist/
          cp go/installer/installer dist/
          cp tools/sub-server/sub-server dist/ 2>/dev/null || true
          cp tools/sub-server/sub-server-arm64 dist/ 2>/dev/null || true
          cp tools/sub-server/sub-server.minisig dist/ 2>/dev/null || true
          cp tools/sub-server/sub-server-arm64.minisig dist/ 2>/dev/null || true
```

And the release `files:` list:

```yaml
          files: |
            dist/aegis
            dist/installer
            dist/sub-server
            dist/sub-server-arm64
            dist/sub-server.minisig
            dist/sub-server-arm64.minisig
```

- [ ] **Step 4: Verify CI syntax is valid YAML**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-1-deployment-fixes
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/public-release.yml')); print('YAML OK')"
```
Expected: YAML OK

- [ ] **Step 5: Commit**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-1-deployment-fixes
git add .github/workflows/public-release.yml
git commit -m "ci(release): add arm64 sub-server build + dual minisign signing"
```

---

## Verification

After all tasks, run:

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-1-deployment-fixes/rust/aegis
cargo fmt && cargo clippy -- -D warnings && cargo test 2>&1 | tail -20
```
