# Aegis Principal-Bound Self-Destruct Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make self-destruct principal-bound, deadline-limited, replay-resistant, exactly-once, and observable.

**Architecture:** A typed state machine lives in `AppState`, keyed by `Principal + TargetId`. TOTP verification returns the accepted counter so counters can be consumed globally. Dispatch authorizes every message/callback before mutation, and final confirmation atomically enters `Executing` before awaiting a supervised executor.

**Tech Stack:** Rust 2024, Tokio synchronization, existing `totp-rs`, rand, shared event boundary.

## Global Constraints

- Requires Phase 1 namespaced principals.
- Flow creation requires configured security-file hash, administrator principal, and recent session.
- Deadline is exactly five minutes and is never refreshed.
- First three failed submissions record delays of 1, 2, and 4 seconds; the fourth locks without TOTP verification.
- Accepted TOTP counters are globally consumed and the two flow counters must differ.
- Every event rechecks principal, target, admin status, recent session, deadline, and expected state.
- Final nonce is one-time; concurrent confirmations call the executor exactly once.
- Missing security-file configuration disables the flow and prompts configuration.

---

### Task 1: Counter-Aware TOTP Verification

**Files:**
- Modify: `rust/aegis/src/core/totp.rs`
- Modify: `rust/aegis/src/app/state.rs`

**Interfaces:**
- Produces: `TotpManager::verify_counter(token: &str, unix_secs: u64) -> Option<u64>`
- Produces: globally consumed `HashSet<u64>` in `AppState`; consumption and flow advancement occur under the same state lock.

- [ ] **Step 1: Add failing tests for accepted counter, adjacent skew, invalid token, and replayed counter**

```rust
#[test]
fn verify_counter_returns_matching_counter() {
    let now = 1_800_000_000;
    let token = manager.totp.generate(now);
    assert_eq!(manager.verify_counter(&token, now), Some(now / 30));
}
```

- [ ] **Step 2: Verify RED with `cargo test core::totp:: --all-features`**
- [ ] **Step 3: Implement counter-returning verification using the configured 30-second period and ±1 skew**

```rust
pub fn verify_counter(&self, token: &str, unix_secs: u64) -> Option<u64> {
    let current = unix_secs / 30;
    [current.saturating_sub(1), current, current + 1]
        .into_iter()
        .find(|counter| self.totp.generate(counter * 30) == token)
}
```

- [ ] **Step 4: Add an internal `consume_totp_counter_locked` operation; while holding the state lock, prune counters older than the accepted skew window, reject duplicates, then insert the accepted counter**
- [ ] **Step 5: Run focused tests and commit**

```bash
git add rust/aegis/src/core/totp.rs rust/aegis/src/app/state.rs
git commit -m "security: track consumed TOTP counters"
```

### Task 2: Typed Principal-Bound State Machine

**Files:**
- Modify: `rust/aegis/src/adapters/common/trait.rs`
- Modify: `rust/aegis/src/app/state.rs`

**Interfaces:**
- Produces: `DestructKey { principal: Principal, target: TargetId }`
- Produces: `DestructStatus::{AwaitFirstTotp, AwaitFirstConfirm, AwaitSecondTotp, AwaitSecurityFile, AwaitFinalConfirm, Cancelled, Expired, Locked, Executing, Succeeded, Failed}`
- Produces: atomic transition methods returning typed outcomes.

- [ ] **Step 1: Derive `PartialEq`, `Eq`, and `Hash` for `TargetId`; add cross-principal/cross-target tests**
- [ ] **Step 2: Add deadline and attempt-cap tests using explicit `Instant` parameters; verify RED**
- [ ] **Step 3: Replace string-keyed state with the exact model**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DestructKey { pub principal: Principal, pub target: TargetId }

pub struct DestructState {
    pub status: DestructStatus,
    pub deadline: Instant,
    pub failed_attempts: u8,
    pub accepted_counters: Vec<u64>,
    pub final_nonce: Option<[u8; 32]>,
}
```

- [ ] **Step 4: Implement `begin_destruct` with `deadline = now + Duration::from_secs(300)` and reject missing key hash**
- [ ] **Step 5: Implement failure transition so attempts 1-3 return delays and submission 4 sets `Locked` before verification**

```rust
match state.failed_attempts {
    0..=2 => { state.failed_attempts += 1; Failure::Delay(Duration::from_secs(1 << (state.failed_attempts - 1))) }
    _ => { state.status = DestructStatus::Locked; Failure::Locked }
}
```

- [ ] **Step 6: Test that no transition changes `deadline`, then commit**

```bash
git add rust/aegis/src/adapters/common/trait.rs rust/aegis/src/app/state.rs
git commit -m "security: model self-destruct as principal-bound state"
```

### Task 3: Authorize Every Message and Callback

**Files:**
- Modify: `rust/aegis/src/shared/destruct.rs`
- Modify: `rust/aegis/src/shared/dispatch.rs`
- Modify: `rust/aegis/src/shared/state_ops.rs`

**Interfaces:**
- Consumes: `DestructKey` and typed transitions.
- Produces: authorization guard shared by message, cancel, and confirm handling.

- [ ] **Step 1: Add tests for missing key, stale session, non-admin, wrong principal, wrong target, expired state, cancel-before-auth, and replayed counter**
- [ ] **Step 2: Assert every rejected event leaves state and executor call count unchanged; verify RED**
- [ ] **Step 3: Add a single guard used before every destructive transition**

```rust
async fn authorize_flow(state: &AppState, key: &DestructKey, now: Instant) -> Result<()> {
    if !state.has_self_destruct_key().await { anyhow::bail!("self-destruct security file is not configured"); }
    if !state.is_admin_user(&key.principal) || !state.is_authorized(&key.principal).await {
        anyhow::bail!("self-destruct authorization required");
    }
    state.ensure_destruct_active(key, now).await
}
```

- [ ] **Step 4: Move destruct callback interception after generic identity extraction but before mutation only through this guard**
- [ ] **Step 5: In one `AppState` critical section, verify expected state, reject a globally consumed or same-flow counter, insert the counter, and advance the flow**
- [ ] **Step 6: Generate a random 32-byte nonce only when entering final-confirm state; embed an encoded nonce in callback data**
- [ ] **Step 7: Run `cargo test shared::destruct:: shared::dispatch:: --all-features` and commit**

```bash
git add rust/aegis/src/shared/destruct.rs rust/aegis/src/shared/dispatch.rs rust/aegis/src/shared/state_ops.rs
git commit -m "security: authorize every self-destruct transition"
```

### Task 4: Exactly-Once Supervised Execution

**Files:**
- Modify: `rust/aegis/src/core/security/self_destruct.rs`
- Modify: `rust/aegis/src/app/state.rs`
- Modify: `rust/aegis/src/shared/destruct.rs`
- Modify: `rust/aegis/src/main.rs` tests

**Interfaces:**
- Replaces: `trigger(Arc<dyn SelfDestructExecutor>)` fire-and-forget API.
- Produces: `execute_supervised(executor: Arc<dyn SelfDestructExecutor>) -> Result<()>`.

- [ ] **Step 1: Add a two-concurrent-confirmation test with an atomic executor call counter; expected count is one**
- [ ] **Step 2: Add executor failure test; expected terminal state is `Failed`, no replay permitted**
- [ ] **Step 3: Verify RED**
- [ ] **Step 4: Atomically compare nonce and transition `AwaitFinalConfirm -> Executing` while holding one state lock**

```rust
pub async fn claim_execution(&self, key: &DestructKey, nonce: &[u8; 32], now: Instant) -> bool {
    let mut flows = self.pending_destructs.lock().await;
    let Some(flow) = flows.get_mut(key) else { return false; };
    if flow.deadline <= now || flow.status != DestructStatus::AwaitFinalConfirm || flow.final_nonce.as_ref() != Some(nonce) { return false; }
    flow.final_nonce = None;
    flow.status = DestructStatus::Executing;
    true
}
```

- [ ] **Step 5: Await the executor and write `Succeeded` or `Failed`; remove sleep/spawn/eprintln fire-and-forget behavior**

```rust
pub async fn execute_supervised(executor: Arc<dyn SelfDestructExecutor>) -> Result<()> {
    executor.execute().await
}
```

- [ ] **Step 6: Route execution errors through the shared event boundary using generic user text and event ID; inspect logs for secrets**
- [ ] **Step 7: Run full gates and a repeated concurrency stress test**

Run: `cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all-features`
Expected: PASS.

- [ ] **Step 8: Update only the self-destruct audit finding and commit**

```bash
git add rust/aegis/src/core/totp.rs rust/aegis/src/core/security/self_destruct.rs rust/aegis/src/app/state.rs rust/aegis/src/shared rust/aegis/src/main.rs docs/audits/2026-07-17-aegis-rust-stability-performance-security-audit.md
git commit -m "security: make self-destruct exactly once"
```
