# One-Click Split-Stack Prompt — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add interactive split-stack prompt to one-click deployment, allowing users to choose between v4↑v6↓ / v6↑v4↓ / no-split when both IPv4 and IPv6 are available.

**Architecture:** A `oneshot` channel bridges the spawned deployment task (which blocks waiting for user choice) and the callback handler (which delivers the choice). A `LazyLock<Mutex<HashMap>>` stores pending senders keyed by `TargetId`.

**Tech Stack:** Rust, tokio, i18n (rust-i18n)

## Global Constraints

- Only XHTTP protocol receives user-chosen `ip_version`; Vision and Hysteria2 use own internal IP detection (unchanged)
- Prompt only shown when BOTH IPv4 and IPv6 public IPs resolve successfully
- 30-second timeout cancels deployment (not fallback)
- 2 files modified: `ops.rs` + i18n toml files (zh + en)
- No dependency additions

---

### Task 1: Add i18n keys

**Files:**
- Modify: `rust/aegis/i18n/zh.toml`
- Modify: `rust/aegis/i18n/en.toml`

**Produces:** i18n keys `ops.deploy_ip_split_title`, `ops.deploy_ip_cancelled`

- [ ] **Step 1: Add zh keys**

Insert in `rust/aegis/i18n/zh.toml` near other `ops.deploy_*` keys:

```toml
ops.deploy_ip_split_title = "你的机器同时支持 IPv4 和 IPv6，XHTTP 是否启用上下行分离？"
ops.deploy_ip_cancelled = "部署已取消"
```

- [ ] **Step 2: Add en keys**

Insert in `rust/aegis/i18n/en.toml` near other `ops.deploy_*` keys:

```toml
ops.deploy_ip_split_title = "Your machine supports both IPv4 and IPv6. Enable XHTTP split-stack?"
ops.deploy_ip_cancelled = "Deployment cancelled"
```

- [ ] **Step 3: Commit**

```bash
git add rust/aegis/i18n/zh.toml rust/aegis/i18n/en.toml
git commit -m "feat: add i18n keys for split-stack prompt"
```

---

### Task 2: Add global state and callback route

**Files:**
- Modify: `rust/aegis/src/shared/handlers/ops.rs:1-36`

**Interfaces:**
- Produces: `ONE_CLICK_IP_PENDING` static, callback route `a_one_click_ip:*` in `handle()`

- [ ] **Step 1: Add imports and static**

In `ops.rs`, after the existing imports (around line 17), add:

```rust
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use tokio::sync::oneshot;

static ONE_CLICK_IP_PENDING: LazyLock<Mutex<HashMap<String, oneshot::Sender<IpVersion>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
```

Note: If `Arc`, `Mutex`, or any part of the chain is already imported, don't duplicate. Check existing imports at lines 1-17.

- [ ] **Step 2: Add callback route in handle()**

In `handle()`, insert the new route before the catch-all `_ =>` (around line 34):

```rust
d if d.starts_with("a_one_click_ip:") => handle_one_click_ip_response(event, d).await,
```

- [ ] **Step 3: Commit**

```bash
git add rust/aegis/src/shared/handlers/ops.rs
git commit -m "feat: add ONE_CLICK_IP_PENDING state and callback route"
```

---

### Task 3: Write unit test for resolve_one_click_ip_version (single-IP cases)

**Files:**
- Modify: `rust/aegis/src/shared/handlers/ops.rs` (add test module at bottom)

**Produces:** Test functions verifying `resolve_one_click_ip_version` logic for IPv4-only and IPv6-only

The function's signature:
```rust
async fn resolve_one_click_ip_version(
    adapter: &Arc<dyn BotAdapter>,
    target: &TargetId,
    msg_id: &AegisMsgId,
) -> Result<IpVersion>
```

Tests target ONLY the non-interactive branches (no mock for adapter needed):
- IPv4 ok + IPv6 err → `Ok(IpVersion::IPv4)`
- IPv4 err + IPv6 ok → `Ok(IpVersion::IPv6)`
- Both err → `Ok(IpVersion::IPv4)`

Since `resolve_one_click_ip_version` calls `SystemMonitor::get_public_ip()` and `SystemMonitor::get_public_ipv6()` which require network, we test the match logic by extracting a pure helper `match_ip_version(v4: bool, v6: bool) -> Option<IpVersion>` that covers the non-interactive branches.

- [ ] **Step 1: Add test**

At the end of `ops.rs`, after the last `}` of the main module (before any existing `#[cfg(test)]` module), add or extend:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn match_ip_version(v4_ok: bool, v6_ok: bool) -> Option<IpVersion> {
        match (v4_ok, v6_ok) {
            (true, true) => None, // interactive — handled in Task 4
            (true, false) => Some(IpVersion::IPv4),
            (false, true) => Some(IpVersion::IPv6),
            (false, false) => Some(IpVersion::IPv4),
        }
    }

    #[test]
    fn test_match_ip_version_v4_only() {
        assert_eq!(match_ip_version(true, false), Some(IpVersion::IPv4));
    }

    #[test]
    fn test_match_ip_version_v6_only() {
        assert_eq!(match_ip_version(false, true), Some(IpVersion::IPv6));
    }

    #[test]
    fn test_match_ip_version_neither() {
        assert_eq!(match_ip_version(false, false), Some(IpVersion::IPv4));
    }

    #[test]
    fn test_match_ip_version_both_triggers_interactive() {
        assert_eq!(match_ip_version(true, true), None);
    }
}
```

- [ ] **Step 2: Run test to verify it fails (function not yet defined)**

```bash
cargo test ops::tests::test_match_ip_version_v4_only 2>&1
```
Expected: compilation error or test failure (function not in production code yet)

- [ ] **Step 3: Commit**

```bash
git add rust/aegis/src/shared/handlers/ops.rs
git commit -m "test: add match_ip_version tests for single-IP branches"
```

---

### Task 4: Implement resolve_one_click_ip_version and handle_one_click_ip_response

**Files:**
- Modify: `rust/aegis/src/shared/handlers/ops.rs` (add two functions before `spawn_progress_updater`, around line 38)

**Interfaces:**
- Consumes: `ONE_CLICK_IP_PENDING` (Task 2), `SystemMonitor::get_public_ip`, `SystemMonitor::get_public_ipv6`
- Produces: `resolve_one_click_ip_version()`, `handle_one_click_ip_response()`

- [ ] **Step 1: Extract pure match logic into production code**

Replace the `#[cfg(test)]` version of `match_ip_version` with a module-level helper (NOT behind `#[cfg(test)]`):

```rust
/// Returns None when both IPs available (needs interactive prompt);
/// Some(version) for all other cases.
fn match_ip_version(non_interactive: (bool, bool)) -> Option<IpVersion> {
    let (v4_ok, v6_ok) = non_interactive;
    match (v4_ok, v6_ok) {
        (true, true) => None,
        (true, false) => Some(IpVersion::IPv4),
        (false, true) => Some(IpVersion::IPv6),
        _ => Some(IpVersion::IPv4),
    }
}
```

- [ ] **Step 2: Implement resolve_one_click_ip_version**

Insert after `match_ip_version`:

```rust
async fn resolve_one_click_ip_version(
    adapter: &Arc<dyn BotAdapter>,
    target: &TargetId,
    msg_id: &AegisMsgId,
) -> anyhow::Result<IpVersion> {
    let (v4, v6) = tokio::join!(
        crate::core::system::SystemMonitor::get_public_ip(),
        crate::core::system::SystemMonitor::get_public_ipv6(),
    );

    let v4_ok = v4.is_ok();
    let v6_ok = v6.is_ok();

    if let Some(version) = match_ip_version((v4_ok, v6_ok)) {
        return Ok(version);
    }

    let (tx, mut rx) = oneshot::channel::<IpVersion>();

    let markup = Markup {
        buttons: vec![vec![
            InlineButton {
                text: "v4 ↑ v6 ↓ (XHTTP)".into(),
                data: "a_one_click_ip:split4".into(),
            },
            InlineButton {
                text: "v6 ↑ v4 ↓ (XHTTP)".into(),
                data: "a_one_click_ip:split6".into(),
            },
            InlineButton {
                text: "no split (IPv4 only)".into(),
                data: "a_one_click_ip:v4".into(),
            },
        ]],
    };

    adapter
        .send_message(
            target,
            MessageContent {
                text: t!("ops.deploy_ip_split_title").into_owned(),
                markup: Some(markup),
            },
        )
        .await?;

    ONE_CLICK_IP_PENDING
        .lock()
        .map_err(|_| anyhow::anyhow!("lock poisoned"))?
        .insert(target.to_string(), tx);

    match tokio::time::timeout(tokio::time::Duration::from_secs(30), &mut rx).await {
        Ok(Ok(version)) => Ok(version),
        _ => {
            // Clean up on timeout or channel error
            if let Ok(mut map) = ONE_CLICK_IP_PENDING.lock() {
                map.remove(&target.to_string());
            }
            Err(anyhow::anyhow!("user did not respond"))
        }
    }
}
```

- [ ] **Step 3: Implement handle_one_click_ip_response**

```rust
async fn handle_one_click_ip_response(
    event: &CallbackEvent,
    data: &str,
) -> HandlerResult {
    let ip_version = match data.strip_prefix("a_one_click_ip:") {
        Some("split4") => IpVersion::SplitStackV4Primary,
        Some("split6") => IpVersion::SplitStackV6Primary,
        Some("v4") => IpVersion::IPv4,
        _ => return Ok(HandlerAction::Done),
    };

    event
        .adapter
        .answer_callback(
            &event.target,
            &event.callback_id,
            Some(format!("Selected: {}", ip_version.label())),
        )
        .await?;

    if let Ok(mut map) = ONE_CLICK_IP_PENDING.lock() {
        if let Some(tx) = map.remove(&event.target.to_string()) {
            let _ = tx.send(ip_version);
        }
    }

    Ok(HandlerAction::Done)
}
```

- [ ] **Step 4: Run tests to verify build and existing tests pass**

```bash
cargo build 2>&1 | tail -3
cargo test 2>&1 | tail -5
```

- [ ] **Step 5: Commit**

```bash
git add rust/aegis/src/shared/handlers/ops.rs
git commit -m "feat: implement resolve_one_click_ip_version and callback handler"
```

---

### Task 5: Integrate into handle_one_click

**Files:**
- Modify: `rust/aegis/src/shared/handlers/ops.rs` (replace lines ~588-608 in handle_one_click)

**Interfaces:**
- Consumes: `resolve_one_click_ip_version` (Task 4), `t!("ops.deploy_ip_cancelled")` (Task 1)

- [ ] **Step 1: Replace IP version detection block**

Find the block starting at:
```rust
let ip_version = {
    let (v4, v6) = tokio::join!(
        SystemMonitor::get_public_ip(),
        SystemMonitor::get_public_ipv6(),
    );
    match (&v4, &v6) {
        (Ok(_), Ok(_)) => IpVersion::SplitStackV4Primary,
        (Ok(_), Err(_)) => IpVersion::IPv4,
        (Err(_), Ok(_)) => IpVersion::IPv6,
        _ => IpVersion::IPv4,
    }
};
```

Replace with:
```rust
let ip_version = if !failed {
    match resolve_one_click_ip_version(&adapter, &target, &msg_id).await {
        Ok(v) => v,
        Err(_) => {
            let _ = tx.send(t!("ops.deploy_ip_cancelled").to_string());
            return;
        }
    }
} else {
    IpVersion::IPv4
};
```

Note: `if !failed` guard ensures we don't prompt if earlier steps already failed.

- [ ] **Step 2: Verify all tests pass**

```bash
cargo test 2>&1 | tail -5
```

- [ ] **Step 3: Commit**

```bash
git add rust/aegis/src/shared/handlers/ops.rs
git commit -m "feat: integrate split-stack prompt into handle_one_click"
```

---

### Task 6: Run full test suite and finalize

- [ ] **Step 1: Full test suite**

```bash
cargo test 2>&1
```

Expected: 582+ tests pass, 0 failures

- [ ] **Step 2: Verify Go build still passes**

```bash
go build ./... 2>&1
```

- [ ] **Step 3: Final commit if any cleanup needed**

```bash
git status
```
