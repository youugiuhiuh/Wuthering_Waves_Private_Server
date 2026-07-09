## Task 1: state.rs — Add pending_security_file state

**Files:**
- Modify: `src/app/state.rs`

**Interfaces:**
- Consumes: existing `AppState` with `Mutex<HashMap<String, Instant>>` pattern (see `pending_warp_inputs`)
- Produces: `AppState::start_security_file_input(chat_id: String, now: Instant)`, `AppState::take_security_file_input_status(chat_id: &str, timeout: Duration) -> TimeoutStatus`

- [ ] **Step 1.1: Write failing test**

```rust
#[cfg(test)]
mod security_file_tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn start_sets_pending() {
        let state = make_test_state();
        state.start_security_file_input("42".into(), Instant::now()).await;
        assert_eq!(
            state.take_security_file_input_status("42", Duration::from_secs(60)).await,
            TimeoutStatus::Active
        );
    }

    #[tokio::test]
    async fn take_after_timeout_returns_expired() {
        let state = make_test_state();
        let past = Instant::now() - Duration::from_secs(120);
        state.start_security_file_input("42".into(), past).await;
        assert_eq!(
            state.take_security_file_input_status("42", Duration::from_secs(60)).await,
            TimeoutStatus::Expired
        );
    }

    #[tokio::test]
    async fn take_when_not_started_returns_not_tracked() {
        let state = make_test_state();
        assert_eq!(
            state.take_security_file_input_status("99", Duration::from_secs(60)).await,
            TimeoutStatus::NotTracked
        );
    }
}
```

- [ ] **Step 1.2: Run test to verify it fails**

Run: `cargo test security_file_tests -- --nocapture 2>&1 | tail -10`
Expected: FAIL — functions not defined on AppState

- [ ] **Step 1.3: Add state + methods to AppState**

Add field after existing `pending_schedule_inputs`:
```rust
pending_security_file: Mutex<HashMap<String, Instant>>,
```

Initialize in the constructor:
```rust
pending_security_file: Mutex::new(HashMap::new()),
```

Add methods:
```rust
pub async fn start_security_file_input(&self, chat_id: String, now: Instant) {
    self.pending_security_file.lock().await.insert(chat_id, now);
}

pub async fn take_security_file_input_status(
    &self,
    chat_id: &str,
    timeout: Duration,
) -> TimeoutStatus {
    let mut map = self.pending_security_file.lock().await;
    match map.remove(chat_id) {
        Some(started) if started.elapsed() < timeout => TimeoutStatus::Active,
        Some(_) => TimeoutStatus::Expired,
        None => TimeoutStatus::NotTracked,
    }
}
```

- [ ] **Step 1.4: Run test to verify it passes**

Run: `cargo test security_file_tests -- --nocapture 2>&1 | tail -10`
Expected: PASS — 3 tests

- [ ] **Step 1.5: Run full suite + lint**

Run: `cargo fmt && cargo clippy -- -D warnings && cargo test 2>&1 | grep "^test result:"`
Expected: All pass, 0 failures

- [ ] **Step 1.6: Commit**

```bash
git add src/app/state.rs
git commit -m "feat(aegis): add pending_security_file state for security-file upload flow"
```

---
