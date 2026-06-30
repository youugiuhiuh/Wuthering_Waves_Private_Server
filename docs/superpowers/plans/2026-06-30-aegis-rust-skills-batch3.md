# Aegis Rust Skills Batch 3: Error Handling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix remaining RUST_SKILLS_REVIEW.md Error Handling violations (err-result-over-panic, err-no-unwrap-prod, err-doc-errors)

**Architecture:** Three independent tasks: (1) convert production `unwrap()` to `expect()` with semantic messages, (2) add `#[source]` annotations to AppError, (3) fill `# Errors` doc gaps on public Result-returning functions.

**Tech Stack:** Rust, thiserror

---

### Task 1: Production `unwrap()` → `expect()` with semantic messages

**Files:**
- Modify: `rust/aegis/src/core/sni/selector.rs:146`
- Modify: `rust/aegis/src/core/system/operations.rs:216`
- Modify: `rust/aegis/src/adapters/telegram/handlers/schedule/handle.rs:676-678`
- Modify: `rust/aegis/src/core/security/firewall_scanner.rs:10,12`
- Modify: `rust/aegis/src/core/xray/port_allocator.rs:142`

- [ ] **Step 1: Fix sni/selector.rs:146**

Change `pop().unwrap()` to `pop().expect("shuffled_indices should have been reset before pop")`:

```rust
let idx = self.shuffled_indices.pop().expect("shuffled_indices 应在 pop 前已重置");
```

- [ ] **Step 2: Fix operations.rs:216**

Change `periodic_config_path().unwrap()` to `periodic_config_path().expect()`:

```rust
let path = distro.periodic_config_path()
    .expect("Debian 应始终包含 periodic_config_path");
```

- [ ] **Step 3: Fix schedule/handle.rs:676-678**

Change `strip_prefix().unwrap()` to `expect()`:

```rust
let idx: usize = data
    .strip_prefix("s_del_confirm:")
    .expect("Callback data 应由路由器预校验为 s_del_confirm: 前缀")
    .parse()
    .unwrap_or(0);
```

- [ ] **Step 4: Fix firewall_scanner.rs:10,12**

Change static Lazy Regex unwraps:

```rust
static PORT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#""(?:port|listen_port)"\s*:\s*(\d+)"#)
        .expect("port regex 应为有效正则表达式"));
static LISTEN_ADDR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#""listen"\s*:\s*"(?:127\.0\.0\.1|localhost)""#)
        .expect("listen addr regex 应为有效正则表达式"));
```

- [ ] **Step 5: Fix port_allocator.rs:142**

Change `Regex::new(...).unwrap()` to `expect()`:

```rust
let re = regex::Regex::new(r#""listen_port"\s*:\s*(\d+)"#)
    .expect("listen_port 正则表达式应为有效");
```

- [ ] **Step 6: Verify**

```bash
cd rust/aegis && cargo fmt --check && cargo clippy -D warnings && cargo test
```

Expected: all clean, 446 passed, 0 failed.

- [ ] **Step 7: Commit**

```bash
git add rust/aegis/src/core/sni/selector.rs rust/aegis/src/core/system/operations.rs rust/aegis/src/adapters/telegram/handlers/schedule/handle.rs rust/aegis/src/core/security/firewall_scanner.rs rust/aegis/src/core/xray/port_allocator.rs
git commit -m "fix(errors): replace production unwrap() with expect() for semantic error messages"
```

---

### Task 2: Add `#[source]` to AppError variants

**Files:**
- Modify: `rust/aegis/src/core/error.rs`

Note: `Io` and `Json` variants already have implicit `#[source]` via `#[from]`. The `Config(String)`, `Service(String)`, and `Network(String)` variants wrap `String` which does not implement `std::error::Error`, so `#[source]` is not applicable unless these are converted to wrap actual error types. This would be a breaking API change outside the current scope.

- [ ] **Step 1: Add explicit `#[source]` to Io variant for clarity**

```rust
#[error("IO 错误: {0}")]
Io(#[source] #[from] std::io::Error),
```

- [ ] **Step 2: Document in code comment that String variants intentionally lack `#[source]`**

```rust
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum AppError {
    /// Configuration error. The inner `String` is a user-facing message.
    /// No source error is available — this wraps static or computed messages.
    #[error("配置错误: {0}")]
    Config(String),

    // ... same for Service, Network, NotInstalled, InvalidParameter, Unknown
}
```

- [ ] **Step 3: Verify**

```bash
cd rust/aegis && cargo fmt --check && cargo clippy -D warnings && cargo test
```

Expected: all clean, 446 passed, 0 failed.

- [ ] **Step 4: Commit**

```bash
git add rust/aegis/src/core/error.rs
git commit -m "docs(errors): add explicit #[source] and variant doc comments to AppError"
```

---

### Task 3: Fill `# Errors` doc gaps on public Result-returning functions

**Files:**
- Modify: `rust/aegis/src/core/system/monitor.rs`
- Possible additional files found during scan

- [ ] **Step 1: Search for public Result-returning functions without `# Errors`**

```bash
grep -rn 'pub.*fn.*Result<' rust/aegis/src/ --include='*.rs' | grep -v '#\[cfg(test)\]' | grep -v 'mod tests' | grep -v 'test' | grep -v '_test'
```

Manually check each candidate for existing `# Errors` section in the doc comment above it.

- [ ] **Step 2: Add `# Errors` to SystemMonitor methods**

Read `rust/aegis/src/core/system/monitor.rs` and add `# Errors` sections to functions like `get_status_report`, `get_cpu_usage`, `get_memory_usage`, `get_network_traffic`, `get_load_avg`.

Example:
```rust
/// Returns a formatted status report.
///
/// # Errors
///
/// Returns an error if reading system files (`/proc`, `/sys`) fails
/// or if subprocess commands (ufw, ss, xray) time out.
pub async fn get_status_report() -> Result<String> {
```

Add similar `# Errors` to each `get_*` method.

- [ ] **Step 3: Add `# Errors` to any other found gaps**

For any additional Result-returning functions found without `# Errors`, add appropriate doc sections.

- [ ] **Step 4: Verify**

```bash
cd rust/aegis && cargo fmt --check && cargo clippy -D warnings && cargo test
```

Expected: all clean, 446 passed, 0 failed.

- [ ] **Step 5: Commit**

```bash
git add [modified files]
git commit -m "docs(errors): add # Errors sections to Result-returning functions"
```

---

### Summary

| Task | Files | ❌ Fixed |
|------|-------|----------|
| 1: unwrap→expect | 5 files | err-result-over-panic, err-no-unwrap-prod |
| 2: #[source] + comments | 1 file | err-source-chain (⚠️ → ✅) |
| 3: #Errors docs | monitor.rs + gaps | err-doc-errors |
