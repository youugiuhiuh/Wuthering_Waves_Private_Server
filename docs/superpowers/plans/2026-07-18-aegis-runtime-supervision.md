# Aegis Runtime Supervision Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make scheduler replacement transactional and make Discord/Matrix gateway failure terminate Aegis truthfully after bounded retries, while preserving intentional shutdown and Telegram's existing non-retrying behavior.

**Architecture:** `SchedulerManager` owns one atomically swappable active scheduler/state pair. Startup validates the unchanged loaded state, starts a complete candidate, and installs it in memory without rewriting the same file; every mutation is serialized through a candidate-first transaction that validates, registers, starts, atomically persists, swaps, then shuts down the old scheduler. `main/runtime.rs` owns one cancellation token and one joined task set; Discord and Matrix run through a bounded retry helper, while Telegram is only a cancellable sibling when Matrix shares the runtime.

**Tech Stack:** Rust 2024, Tokio `JoinSet`, existing `tokio-util` `CancellationToken`, `tokio-cron-scheduler`, existing `atomic_write_sensitive`, `anyhow`; no new dependency

## File Map

- Modify: `rust/aegis/src/core/system/scheduler/mod.rs` - validated atomic persistence, active scheduler/state ownership, candidate transaction, global replacement, failure-injection tests.
- Modify: `rust/aegis/src/shared/handlers/schedule.rs` - read scheduler snapshots and route GeoData removal through the same transaction as add/remove.
- Modify: `rust/aegis/src/main/discord.rs` - expose a rebuildable Discord client constructor for retry attempts.
- Modify: `rust/aegis/src/main/runtime.rs` - single startup path, gateway retry helper, shared cancellation, joined shutdown, runtime tests.
- Verify only: `rust/aegis/src/main.rs` - its existing `Result<()>` return propagates terminal runtime failure to a nonzero process exit; no edit is required.
- Verify only: `rust/aegis/src/adapters/telegram/adapter.rs` - existing locale-triggered `add_new_task` automatically uses the scheduler transaction; no edit is required.

## Global Constraints

- Startup does not rewrite an unchanged loaded state. It validates and starts the complete candidate before installing the in-memory instance.
- If the state file is absent, startup installs the validated default state in memory without creating a file; the first successful mutation atomically persists the complete state.
- For add/remove mutations, the old persisted file and old running scheduler remain authoritative until a candidate has passed complete-state validation, registration, start, and atomic persistence.
- Persist a mutated complete candidate state before swapping the active runtime pair; shut down the old scheduler only after the swap.
- Add, indexed remove, and remove-by-task-type use the same serialized transaction.
- Corrupt persisted state fails closed and remains byte-for-byte unchanged; it is never replaced by `SchedulerState::default()`.
- Every scheduler transaction error includes `scheduler stage=<load|validation|registration|start|persistence>` and the task index where applicable.
- Discord and Matrix share cancellation and bounded retries. Backoff is exponential and capped; the retry budget resets only after a run lasts the configured stable period.
- Retry exhaustion cancels every sibling, joins shutdown, returns `Err`, and reaches the existing `main() -> Result<()>` boundary as a nonzero exit.
- Intentional shutdown returns `Ok(())`, does not consume retry budget, and is not logged as a gateway crash.
- Gateway logs contain only `platform`, `attempt`, `backoff_ms`, and `terminal_reason`; never include tokens, credentials, Matrix/Discord event content, or raw gateway errors.
- Telegram does not receive Discord/Matrix retry behavior. It only consumes shared cancellation when it is a sibling of Matrix.
- No runtime task is detached: startup work is awaited, gateway/Telegram tasks live in a `JoinSet`, and shutdown drains or aborts and then joins every task.
- No new dependencies, generic supervision framework, command redesign, P2 work, or unrelated refactor.
- From `rust/aegis`, every implementation task follows RED -> GREEN -> REFACTOR and the phase ends with `cargo fmt && cargo clippy -- -D warnings && cargo test`.

---

### Task 1: Fail-Closed Scheduler State And Atomic Persistence

**Files:**
- Modify: `rust/aegis/src/core/system/scheduler/mod.rs:1-53,384-480`

**Interfaces:**
- Produces: `pub fn save_to_file(&self, path: &Path) -> Result<()>`
- Produces: `pub fn load_from_file(path: &Path) -> Result<Self>`
- Produces: `fn validate_state(state: &SchedulerState) -> Result<()>`
- Consumes: `aegis::core::security::secure_fs::atomic_write_sensitive(path, bytes)`

- [ ] **Step 1: Write failing corrupt-state and atomic-write tests**

Add these tests inside the existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn corrupt_state_fails_closed_without_rewriting_file() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("scheduler_state.json");
    let corrupt = b"{not-json";
    fs::write(&path, corrupt).unwrap();
    let error = SchedulerState::load_from_file(&path).unwrap_err();
    assert!(error.to_string().contains("scheduler stage=load"));
    assert_eq!(fs::read(path).unwrap(), corrupt);
}

#[test]
fn complete_state_validation_rejects_disabled_invalid_task() {
    let state = SchedulerState {
        tasks: vec![ScheduledTask {
            task_type: TaskType::GeoUpdate,
            cron_expression: "invalid".to_string(),
            timezone: "UTC".to_string(),
            enabled: false,
        }],
    };
    let error = validate_state(&state).unwrap_err();
    assert!(error.to_string().contains("scheduler stage=validation"));
    assert!(error.to_string().contains("task_index=0"));
}

#[test]
fn save_to_file_atomically_replaces_complete_state() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("scheduler_state.json");
    SchedulerState { tasks: Vec::new() }.save_to_file(&path).unwrap();
    let new = SchedulerState {
        tasks: vec![ScheduledTask::new(TaskType::ReloadCore, "0 4 * * *")],
    };
    new.save_to_file(&path).unwrap();
    assert_eq!(SchedulerState::load_from_file(&path).unwrap().tasks.len(), 1);
    assert!(!dir.path().join("scheduler_state.json.0.tmp").exists());
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

```bash
cd rust/aegis
cargo test --lib core::system::scheduler::tests::corrupt_state_fails_closed_without_rewriting_file -- --exact
cargo test --lib core::system::scheduler::tests::complete_state_validation_rejects_disabled_invalid_task -- --exact
```

Expected: compile failure because `load_from_file` still accepts `&str`, lacks stage context, and `validate_state` does not exist.

- [ ] **Step 3: Replace state persistence and add complete-state validation**

Replace the imports and `save_to_file`/`load_from_file` methods with:

```rust
use crate::adapters::common::{BotAdapter, TargetId};
use crate::core::security::secure_fs::atomic_write_sensitive;
use anyhow::{Context, Result};
use chrono_tz::Tz;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_cron_scheduler::{Job, JobScheduler};
```

```rust
pub fn save_to_file(&self, path: &Path) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(self)
        .context("scheduler stage=persistence serialize state")?;
    atomic_write_sensitive(path, &bytes)
        .context("scheduler stage=persistence atomic replace")
}

pub fn load_from_file(path: &Path) -> Result<Self> {
    if !path.exists() {
        return Ok(Self::default());
    }
    let content = fs::read(path).context("scheduler stage=load read state")?;
    serde_json::from_slice(&content).context("scheduler stage=load parse state")
}
```

Add after `impl SchedulerState`:

```rust
fn validate_state(state: &SchedulerState) -> Result<()> {
    let validator = SchedulerValidator::new();
    for (index, task) in state.tasks.iter().enumerate() {
        validator.validate_task(task).map_err(|error| {
            anyhow::anyhow!("scheduler stage=validation task_index={index}: {error}")
        })?;
    }
    Ok(())
}
```

- [ ] **Step 4: Run the focused tests and verify GREEN**

Run: `cd rust/aegis && cargo test --lib core::system::scheduler::tests -- --test-threads=1`

Expected: all scheduler tests pass; corrupt input remains unchanged and disabled tasks are validated.

- [ ] **Step 5: Commit Task 1**

```bash
git add rust/aegis/src/core/system/scheduler/mod.rs
git commit -m "fix: fail closed on scheduler state corruption"
```

---

### Task 2: Candidate-First Scheduler Transaction

**Files:**
- Modify: `rust/aegis/src/core/system/scheduler/mod.rs:95-249,296-317`

**Interfaces:**
- Produces: `struct ActiveScheduler { scheduler: Option<JobScheduler>, state: SchedulerState }`
- Produces: `pub async fn state_snapshot(&self) -> SchedulerState`
- Produces: `async fn replace_state_locked(&self, next: SchedulerState) -> Result<()>`
- Produces: `pub async fn remove_tasks_by_type(&self, task_type: TaskType) -> Result<bool>`
- Preserves: `add_new_task`, `remove_task_at`, `get_summary`, `get_manager`, `start_scheduler` caller behavior.

- [ ] **Step 1: Write failing transaction-order and failure-preservation tests**

Add this test-only adapter and manager helper to the scheduler tests module:

```rust
use crate::adapters::common::{
    Attachment, AttachmentError, BotAdapter, MessageContent, MessageId, Platform,
    PlatformCapabilities, TargetId, VerifiedAttachment,
};
use async_trait::async_trait;

struct TestAdapter;

#[async_trait]
impl BotAdapter for TestAdapter {
    fn platform(&self) -> Platform { Platform::Telegram }
    async fn send_message(&self, _: &TargetId, _: MessageContent) -> Result<MessageId> {
        Ok(MessageId("0".to_string()))
    }
    async fn edit_message(&self, _: &TargetId, _: &MessageId, _: MessageContent) -> Result<()> {
        Ok(())
    }
    async fn delete_message(&self, _: &TargetId, _: &MessageId) -> Result<()> { Ok(()) }
    async fn download_attachment(
        &self,
        _attachment: &Attachment,
        _expected_sha256: Option<[u8; 32]>,
    ) -> std::result::Result<VerifiedAttachment, AttachmentError> {
        Err(AttachmentError::Unsupported)
    }
    fn capabilities(&self) -> PlatformCapabilities { PlatformCapabilities::TELEGRAM }
}

async fn test_manager(path: PathBuf) -> Arc<SchedulerManager> {
    SchedulerState::default().save_to_file(&path).unwrap();
    SchedulerManager::new(
        Arc::new(TestAdapter),
        TargetId("0".to_string()),
        path,
    )
    .await
    .unwrap()
}
```

Add these tests:

```rust
#[tokio::test]
async fn candidate_failures_preserve_old_file_and_running_state() {
    for stage in ["registration", "start", "persistence"] {
        let dir = tempdir().unwrap();
        let path = dir.path().join("scheduler_state.json");
        let manager = test_manager(path.clone()).await;
        let old_bytes = fs::read(&path).unwrap();
        let old_state = manager.state_snapshot().await;
        manager.clear_events();
        manager.inject_failure(stage);

        let result = manager
            .add_new_task(ScheduledTask::new(TaskType::ReloadCore, "0 5 * * *"))
            .await;

        assert!(result.is_err(), "stage {stage} must fail");
        assert!(result.unwrap_err().to_string().contains(&format!("stage={stage}")));
        assert_eq!(fs::read(&path).unwrap(), old_bytes);
        let current = manager.state_snapshot().await;
        assert_eq!(current.tasks.len(), old_state.tasks.len());
        assert_eq!(current.get_tasks_summary(), old_state.get_tasks_summary());
        assert!(manager.has_active_scheduler().await);
        assert!(!manager.events().contains(&"swapped"));
        assert!(!manager.events().contains(&"old_shutdown"));
    }
}

#[tokio::test]
async fn successful_replace_persists_then_swaps_then_shuts_down_old() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("scheduler_state.json");
    let manager = test_manager(path.clone()).await;
    manager.clear_events();

    manager
        .add_new_task(ScheduledTask::new(TaskType::ReloadCore, "0 5 * * *"))
        .await
        .unwrap();

    assert_eq!(
        manager.events(),
        vec!["candidate_started", "persisted", "swapped", "old_shutdown"]
    );
    let disk = SchedulerState::load_from_file(&path).unwrap();
    assert_eq!(
        serde_json::to_vec(&disk).unwrap(),
        serde_json::to_vec(&manager.state_snapshot().await).unwrap()
    );
}

#[tokio::test]
async fn new_installs_unchanged_state_without_rewriting_file() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("scheduler_state.json");
    let compact = serde_json::to_vec(&SchedulerState::default()).unwrap();
    fs::write(&path, &compact).unwrap();

    let manager = SchedulerManager::new(
        Arc::new(TestAdapter),
        TargetId("0".to_string()),
        path.clone(),
    )
    .await
    .unwrap();

    assert!(manager.has_active_scheduler().await);
    assert_eq!(fs::read(path).unwrap(), compact);
    assert_eq!(manager.events(), vec!["candidate_started", "swapped"]);
}

#[tokio::test]
async fn new_with_missing_state_does_not_create_file() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("scheduler_state.json");

    let manager = SchedulerManager::new(
        Arc::new(TestAdapter),
        TargetId("0".to_string()),
        path.clone(),
    )
    .await
    .unwrap();

    assert!(manager.has_active_scheduler().await);
    assert!(!path.exists());
    assert_eq!(manager.events(), vec!["candidate_started", "swapped"]);
}

#[tokio::test]
async fn concurrent_mutations_do_not_lose_an_update() {
    let dir = tempdir().unwrap();
    let manager = test_manager(dir.path().join("scheduler_state.json")).await;
    let first = manager.clone();
    let second = manager.clone();

    let (left, right) = tokio::join!(
        first.add_new_task(ScheduledTask::new(TaskType::ReloadCore, "0 5 * * *")),
        second.add_new_task(ScheduledTask::new(TaskType::Reboot, "0 6 * * *")),
    );

    left.unwrap();
    right.unwrap();
    let state = manager.state_snapshot().await;
    assert!(state.tasks.iter().any(|task| task.task_type == TaskType::ReloadCore));
    assert!(state.tasks.iter().any(|task| task.task_type == TaskType::Reboot));
}

#[tokio::test]
#[serial_test::serial]
async fn corrupt_global_replacement_preserves_old_manager() {
    let valid_dir = tempdir().unwrap();
    start_scheduler_at(
        Arc::new(TestAdapter),
        TargetId("0".to_string()),
        valid_dir.path().join("scheduler_state.json"),
    )
    .await
    .unwrap();
    let old = get_manager().await.unwrap();
    let corrupt_dir = tempdir().unwrap();
    let corrupt_path = corrupt_dir.path().join("scheduler_state.json");
    fs::write(&corrupt_path, b"{broken").unwrap();

    let error = start_scheduler_at(
        Arc::new(TestAdapter),
        TargetId("0".to_string()),
        corrupt_path.clone(),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("scheduler stage=load"));
    assert!(Arc::ptr_eq(&old, &get_manager().await.unwrap()));
    assert_eq!(fs::read(corrupt_path).unwrap(), b"{broken");
}
```

- [ ] **Step 2: Run tests and verify RED**

Run: `cd rust/aegis && cargo test --lib core::system::scheduler::tests -- --test-threads=1`

Expected: compile failure for missing `state_snapshot`, failure injection, event trace, and `PathBuf` constructor. After those compile errors are resolved but before Step 5 removes startup persistence, `new_installs_unchanged_state_without_rewriting_file` fails because compact bytes are rewritten and `new_with_missing_state_does_not_create_file` fails because startup creates the file.

- [ ] **Step 3: Replace manager storage and add test-only deterministic hooks**

Replace `SchedulerManager` fields with:

```rust
struct ActiveScheduler {
    scheduler: Option<JobScheduler>,
    state: SchedulerState,
}

pub struct SchedulerManager {
    active: Mutex<ActiveScheduler>,
    transaction: Mutex<()>,
    state_path: PathBuf,
    adapter: Arc<dyn BotAdapter>,
    target: TargetId,
    #[cfg(test)]
    failure: std::sync::Mutex<Option<&'static str>>,
    #[cfg(test)]
    events: std::sync::Mutex<Vec<&'static str>>,
}
```

Add these private/test helpers at the start of `impl SchedulerManager`:

```rust
fn fail_if_injected(&self, _stage: &'static str) -> Result<()> {
    #[cfg(test)]
    {
        let mut failure = self.failure.lock().unwrap();
        if *failure == Some(_stage) {
            *failure = None;
            anyhow::bail!("scheduler stage={_stage}: injected failure");
        }
    }
    Ok(())
}

fn record(&self, _event: &'static str) {
    #[cfg(test)]
    self.events.lock().unwrap().push(_event);
}

#[cfg(test)]
fn inject_failure(&self, stage: &'static str) {
    *self.failure.lock().unwrap() = Some(stage);
}

#[cfg(test)]
fn clear_events(&self) { self.events.lock().unwrap().clear(); }

#[cfg(test)]
fn events(&self) -> Vec<&'static str> { self.events.lock().unwrap().clone() }

#[cfg(test)]
async fn has_active_scheduler(&self) -> bool {
    self.active.lock().await.scheduler.is_some()
}
```

- [ ] **Step 4: Implement complete candidate preparation and persistence**

Add these methods:

```rust
async fn prepare_candidate(&self, state: &SchedulerState) -> Result<JobScheduler> {
    validate_state(state)?;
    let scheduler = JobScheduler::new()
        .await
        .context("scheduler stage=registration create candidate")?;
    self.fail_if_injected("registration")?;
    for (index, task) in state.tasks.iter().enumerate().filter(|(_, task)| task.enabled) {
        let cron = normalize_cron_expression(&task.cron_expression);
        let job = build_job(self.adapter.clone(), self.target.clone(), task, &cron)
            .with_context(|| format!("scheduler stage=registration task_index={index}"))?;
        scheduler.add(job).await
            .with_context(|| format!("scheduler stage=registration task_index={index}"))?;
    }
    self.fail_if_injected("start")?;
    scheduler.start().await.context("scheduler stage=start")?;
    self.record("candidate_started");
    Ok(scheduler)
}

async fn persist_state(&self, state: SchedulerState) -> Result<()> {
    self.fail_if_injected("persistence")?;
    let path = self.state_path.clone();
    tokio::task::spawn_blocking(move || state.save_to_file(&path))
        .await
        .context("scheduler stage=persistence join writer")??;
    self.record("persisted");
    Ok(())
}
```

- [ ] **Step 5: Replace constructor, mutation methods, and global start with the transaction**

Replace `SchedulerManager::new`, `start_all_tasks`, `add_new_task`, `remove_task_at`, and `get_summary` with:

```rust
pub async fn new(
    adapter: Arc<dyn BotAdapter>,
    target: TargetId,
    state_path: PathBuf,
) -> Result<Arc<Self>> {
    let load_path = state_path.clone();
    let state = tokio::task::spawn_blocking(move || SchedulerState::load_from_file(&load_path))
        .await
        .context("scheduler stage=load join reader")??;
    validate_state(&state)?;

    let manager = Arc::new(Self {
        active: Mutex::new(ActiveScheduler {
            scheduler: None,
            state: SchedulerState { tasks: Vec::new() },
        }),
        transaction: Mutex::new(()),
        state_path,
        adapter,
        target,
        #[cfg(test)]
        failure: std::sync::Mutex::new(None),
        #[cfg(test)]
        events: std::sync::Mutex::new(Vec::new()),
    });

    let candidate = manager.prepare_candidate(&state).await?;
    *manager.active.lock().await = ActiveScheduler {
        scheduler: Some(candidate),
        state,
    };
    manager.record("swapped");
    Ok(manager)
}

pub async fn state_snapshot(&self) -> SchedulerState {
    self.active.lock().await.state.clone()
}

async fn replace_state_locked(&self, next: SchedulerState) -> Result<()> {
    let candidate = self.prepare_candidate(&next).await?;
    if let Err(error) = self.persist_state(next.clone()).await {
        let mut candidate = candidate;
        if let Err(shutdown_error) = candidate.shutdown().await {
            log::error!(
                "scheduler stage=candidate_cleanup terminal_reason=shutdown_error: {}",
                shutdown_error
            );
        }
        return Err(error);
    }

    let mut old = {
        let mut active = self.active.lock().await;
        let old = active
            .scheduler
            .replace(candidate)
            .context("scheduler stage=swap missing active scheduler")?;
        active.state = next;
        old
    };
    self.record("swapped");
    if let Err(error) = old.shutdown().await {
        log::error!(
            "scheduler stage=old_shutdown terminal_reason=shutdown_error: {}",
            error
        );
    }
    self.record("old_shutdown");
    Ok(())
}

pub async fn add_new_task(&self, task: ScheduledTask) -> Result<String> {
    if let Err(error) = SchedulerValidator::new().validate_task(&task) {
        return Ok(format!("❌ {error}"));
    }
    let _transaction = self.transaction.lock().await;
    let mut next = self.state_snapshot().await;
    next.add_task(task.clone());
    self.replace_state_locked(next).await?;
    Ok(format!(
        "✅ 新任务已添加: {} ({}, {})",
        task.task_type.get_display_name(), task.cron_expression, task.timezone
    ))
}

pub async fn remove_task_at(&self, index: usize) -> Result<String> {
    let _transaction = self.transaction.lock().await;
    let mut next = self.state_snapshot().await;
    if let Err(error) = next.remove_task(index) {
        return Ok(format!("❌ 删除任务失败: {error}"));
    }
    self.replace_state_locked(next).await?;
    Ok("✅ 任务已删除".to_string())
}

pub async fn remove_tasks_by_type(&self, task_type: TaskType) -> Result<bool> {
    let _transaction = self.transaction.lock().await;
    let mut next = self.state_snapshot().await;
    let old_len = next.tasks.len();
    next.tasks.retain(|task| task.task_type != task_type);
    if next.tasks.len() == old_len {
        return Ok(false);
    }
    self.replace_state_locked(next).await?;
    Ok(true)
}

pub async fn get_summary(&self) -> String {
    self.state_snapshot().await.get_tasks_summary()
}

async fn shutdown(&self) {
    let scheduler = self.active.lock().await.scheduler.take();
    let Some(mut scheduler) = scheduler else { return; };
    let result = scheduler.shutdown().await;
    if let Err(error) = result {
        log::error!(
            "scheduler stage=old_shutdown terminal_reason=shutdown_error: {}",
            error
        );
    }
}
```

Delete `start_all_tasks`; all replacements now go through `replace_state_locked`.

Replace `start_scheduler` with this candidate-first global swap:

```rust
pub async fn start_scheduler(adapter: Arc<dyn BotAdapter>, target: TargetId) -> Result<()> {
    start_scheduler_at(
        adapter,
        target,
        PathBuf::from("/etc/wwps/aegis/scheduler_state.json"),
    )
    .await
}

async fn start_scheduler_at(
    adapter: Arc<dyn BotAdapter>,
    target: TargetId,
    state_path: PathBuf,
) -> Result<()> {
    log::info!("scheduler stage=load terminal_reason=begin");
    let candidate = SchedulerManager::new(adapter, target, state_path).await?;
    let old = SCHEDULER.lock().await.replace(candidate);
    if let Some(old) = old {
        old.shutdown().await;
    }
    log::info!("scheduler stage=complete terminal_reason=active");
    Ok(())
}
```

- [ ] **Step 6: Update the pre-existing invalid-task test to use the helper**

Replace its manual `SchedulerManager` literal and assertions with:

```rust
let manager = test_manager(state_path.clone()).await;
let before = fs::read(&state_path).unwrap();
let result = manager
    .add_new_task(ScheduledTask::new_with_timezone(
        TaskType::GeoUpdate,
        "* *",
        "UTC",
    ))
    .await
    .unwrap();
assert!(result.starts_with("❌"));
assert_eq!(fs::read(&state_path).unwrap(), before);
assert_eq!(manager.state_snapshot().await.tasks.len(), 1);
```

- [ ] **Step 7: Run transaction tests and verify GREEN**

Run: `cd rust/aegis && cargo test --lib core::system::scheduler::tests -- --test-threads=1`

Expected: all scheduler tests pass, including each injected stage, ordering, and concurrent mutation preservation.

- [ ] **Step 8: Commit Task 2**

```bash
git add rust/aegis/src/core/system/scheduler/mod.rs
git commit -m "fix: replace scheduler state transactionally"
```

---

### Task 3: Route Shared Schedule Handlers Through The Transaction

**Files:**
- Modify: `rust/aegis/src/shared/handlers/schedule.rs:57-63,160-174,196-255,285-310,350-389`
- Test: `rust/aegis/src/core/system/scheduler/mod.rs` tests module

**Interfaces:**
- Consumes: `SchedulerManager::state_snapshot() -> SchedulerState`
- Consumes: `SchedulerManager::remove_tasks_by_type(TaskType) -> Result<bool>`
- Removes: every direct `manager.state` lock, `save_to_file`, and `start_all_tasks` call from handlers.

- [ ] **Step 1: Add a failing remove-by-type rollback test**

Add to the scheduler tests module:

```rust
#[tokio::test]
async fn remove_tasks_by_type_rolls_back_on_persistence_failure() {
    let dir = tempdir().unwrap();
    let manager = test_manager(dir.path().join("scheduler_state.json")).await;
    manager.inject_failure("persistence");

    let error = manager
        .remove_tasks_by_type(TaskType::GeoUpdate)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("stage=persistence"));
    assert!(manager
        .state_snapshot()
        .await
        .tasks
        .iter()
        .any(|task| task.task_type == TaskType::GeoUpdate));

    assert!(manager
        .remove_tasks_by_type(TaskType::GeoUpdate)
        .await
        .unwrap());
    assert!(!manager
        .state_snapshot()
        .await
        .tasks
        .iter()
        .any(|task| task.task_type == TaskType::GeoUpdate));
}
```

- [ ] **Step 2: Run the test and verify RED**

Run: `cd rust/aegis && cargo test --lib core::system::scheduler::tests::remove_tasks_by_type_rolls_back_on_persistence_failure -- --exact`

Expected: compile failure because `remove_tasks_by_type` does not exist.

- [ ] **Step 3: Replace every handler read with a snapshot**

Use this exact pattern in `handle_sched`, `handle_del_menu`, `handle_del_select`, and `handle_geo_sched_menu`:

```rust
let state = manager.state_snapshot().await;
```

Remove the corresponding `manager.state.lock().await` and `drop(state)` calls. Keep existing `state.tasks` reads unchanged because the snapshot is owned.

- [ ] **Step 4: Replace `handle_geo_sched_off` with transactional removal**

```rust
async fn handle_geo_sched_off(event: &CallbackEvent) -> HandlerResult {
    let Some(manager) = get_manager().await else {
        event
            .adapter
            .answer_callback(
                &event.target,
                &event.callback_id,
                Some(t!("schedule.scheduler_not_init").into_owned()),
            )
            .await?;
        return Ok(HandlerAction::Done);
    };

    match manager.remove_tasks_by_type(TaskType::GeoUpdate).await {
        Ok(removed) => {
            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(if removed {
                        t!("schedule.geo_stopped").into_owned()
                    } else {
                        t!("schedule.geo_stop_info").into_owned()
                    }),
                )
                .await?;
            Ok(HandlerAction::Redirect("a_geo_sched_menu".to_string()))
        }
        Err(error) => {
            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(format!("❌ {error}")),
                )
                .await?;
            Ok(HandlerAction::Done)
        }
    }
}
```

- [ ] **Step 5: Run handler and scheduler tests and verify GREEN**

Run:

```bash
cd rust/aegis
cargo test --lib core::system::scheduler::tests -- --test-threads=1
cargo check --lib
```

Expected: both commands pass; remove-by-type preserves the active state on persistence failure, then succeeds through the same transaction.

- [ ] **Step 6: Commit Task 3**

```bash
git add rust/aegis/src/shared/handlers/schedule.rs rust/aegis/src/core/system/scheduler/mod.rs
git commit -m "fix: route schedule handlers through transactions"
```

---

### Task 4: Add A Rebuildable Discord Gateway Boundary

**Files:**
- Modify: `rust/aegis/src/main/discord.rs:118-133`

**Interfaces:**
- Produces: `pub async fn build_client(token: &str, admin_channel: ChannelId, adapter: Arc<dyn BotAdapter>, state: Arc<AppState>) -> Result<Client>`
- Preserves: `build_handle(DiscordRawHandle, Arc<AppState>) -> Result<DiscordHandle>` for existing callers/tests.

- [ ] **Step 1: Replace `build_handle` construction with a reusable client constructor**

```rust
pub async fn build_client(
    token: &str,
    admin_channel: ChannelId,
    adapter: Arc<dyn BotAdapter>,
    state: Arc<AppState>,
) -> Result<Client> {
    let intents = GatewayIntents::DIRECT_MESSAGES | GatewayIntents::MESSAGE_CONTENT;
    let handler = DiscordHandler {
        state,
        adapter,
        admin_channel,
    };
    Client::builder(token, intents)
        .event_handler(handler)
        .await
        .context("构建 Discord Client 失败")
}

#[allow(dead_code)]
pub async fn build_handle(raw: DiscordRawHandle, state: Arc<AppState>) -> Result<DiscordHandle> {
    let client = build_client(
        &raw.token,
        raw.admin_channel,
        raw.adapter.clone(),
        state,
    )
    .await?;
    Ok((client, raw.admin_channel, raw.adapter))
}
```

- [ ] **Step 2: Compile the binary target**

Run: `cd rust/aegis && cargo check --bin aegis`

Expected: PASS; Discord client construction is reusable without logging or exposing the token.

- [ ] **Step 3: Commit Task 4**

```bash
git add rust/aegis/src/main/discord.rs
git commit -m "refactor: make Discord clients rebuildable"
```

---

### Task 5: Implement Bounded Gateway Retry Semantics

**Files:**
- Modify: `rust/aegis/src/main/runtime.rs:1-22` and append tests after `run`

**Interfaces:**
- Produces: `RetryPolicy { max_retries, initial_backoff, max_backoff, stable_after }`
- Produces: `async fn supervise_gateway<F, Fut>(platform, cancellation, policy, run_once) -> Result<()>`
- Produces: log lines with exactly `platform`, `attempt`, `backoff_ms`, `terminal_reason`.

- [ ] **Step 1: Write failing retry, stable-period, cancellation, and log-safety tests**

Append to `runtime.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn test_policy() -> RetryPolicy {
        RetryPolicy {
            max_retries: 2,
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(2),
            stable_after: Duration::from_millis(5),
        }
    }

    #[test]
    fn backoff_is_exponential_and_capped() {
        let policy = test_policy();
        assert_eq!(policy.backoff(1), Duration::from_millis(1));
        assert_eq!(policy.backoff(2), Duration::from_millis(2));
        assert_eq!(policy.backoff(3), Duration::from_millis(2));
    }

    #[tokio::test]
    async fn transient_failures_retry_only_within_budget() {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let error = supervise_gateway(
            "matrix",
            CancellationToken::new(),
            test_policy(),
            move || {
                let observed = observed.clone();
                async move {
                    observed.fetch_add(1, Ordering::SeqCst);
                    anyhow::bail!("secret event body")
                }
            },
        )
        .await
        .unwrap_err();

        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert!(error.to_string().contains("retries_exhausted"));
        assert!(!error.to_string().contains("secret event body"));
    }

    #[tokio::test]
    async fn stable_run_resets_retry_budget() {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let error = supervise_gateway(
            "discord",
            CancellationToken::new(),
            test_policy(),
            move || {
                let call = observed.fetch_add(1, Ordering::SeqCst);
                async move {
                    if call == 1 {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    anyhow::bail!("gateway ended")
                }
            },
        )
        .await
        .unwrap_err();

        assert_eq!(calls.load(Ordering::SeqCst), 4);
        assert!(error.to_string().contains("retries_exhausted"));
    }

    #[tokio::test]
    async fn intentional_cancellation_is_success_without_retry() {
        let cancellation = CancellationToken::new();
        let trigger = cancellation.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            trigger.cancel();
        });

        let result = supervise_gateway("matrix", cancellation, test_policy(), move || {
            observed.fetch_add(1, Ordering::SeqCst);
            std::future::pending::<Result<()>>()
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn gateway_log_fields_exclude_raw_errors_and_content() {
        assert_eq!(
            gateway_log_fields("discord", 2, Duration::from_secs(4), "error"),
            "gateway platform=discord attempt=2 backoff_ms=4000 terminal_reason=error"
        );
    }
}
```

- [ ] **Step 2: Run tests and verify RED**

Run: `cd rust/aegis && cargo test --bin aegis main::runtime::tests -- --test-threads=1`

Expected: compile failure because `RetryPolicy`, `supervise_gateway`, and `gateway_log_fields` do not exist.

- [ ] **Step 3: Add imports, policy, safe logging, and supervisor**

Add imports:

```rust
use std::future::Future;
use std::time::Duration;
use tokio::time::Instant;
use tokio::task::JoinSet;
```

Add before `run`:

```rust
#[derive(Clone, Copy)]
struct RetryPolicy {
    max_retries: u32,
    initial_backoff: Duration,
    max_backoff: Duration,
    stable_after: Duration,
}

impl RetryPolicy {
    fn gateway_default() -> Self {
        Self {
            max_retries: 5,
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(30),
            stable_after: Duration::from_secs(300),
        }
    }

    fn backoff(self, failure: u32) -> Duration {
        let exponent = failure.saturating_sub(1).min(31);
        self.initial_backoff
            .saturating_mul(1_u32 << exponent)
            .min(self.max_backoff)
    }
}

fn gateway_log_fields(
    platform: &'static str,
    attempt: u32,
    backoff: Duration,
    terminal_reason: &'static str,
) -> String {
    format!(
        "gateway platform={platform} attempt={attempt} backoff_ms={} terminal_reason={terminal_reason}",
        backoff.as_millis()
    )
}

async fn supervise_gateway<F, Fut>(
    platform: &'static str,
    cancellation: CancellationToken,
    policy: RetryPolicy,
    mut run_once: F,
) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let mut failures = 0_u32;
    loop {
        let started = Instant::now();
        let terminal_reason = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Ok(()),
            result = run_once() => if result.is_ok() { "returned" } else { "error" },
        };

        if started.elapsed() >= policy.stable_after {
            failures = 0;
        }
        failures = failures.saturating_add(1);
        if failures > policy.max_retries {
            log::error!(
                "{}",
                gateway_log_fields(
                    platform,
                    failures,
                    Duration::ZERO,
                    "retries_exhausted"
                )
            );
            anyhow::bail!(
                "gateway platform={platform} retries_exhausted terminal_reason={terminal_reason}"
            );
        }

        let backoff = policy.backoff(failures);
        log::warn!(
            "{}",
            gateway_log_fields(platform, failures, backoff, terminal_reason)
        );
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Ok(()),
            _ = tokio::time::sleep(backoff) => {}
        }
    }
}
```

- [ ] **Step 4: Run tests and verify GREEN**

Run: `cd rust/aegis && cargo test --bin aegis main::runtime::tests -- --test-threads=1`

Expected: all four retry tests pass; three immediate failures exhaust a two-retry budget, a stable second run permits a fourth attempt, and cancellation returns success after one call.

- [ ] **Step 5: Commit Task 5**

```bash
git add rust/aegis/src/main/runtime.rs
git commit -m "feat: add bounded gateway retry policy"
```

---

### Task 6: Join Runtime Tasks And Propagate Terminal Failure

**Files:**
- Modify: `rust/aegis/src/main/runtime.rs:55-394`

**Interfaces:**
- Produces: `type RuntimeExit = (&'static str, Result<()>)`
- Produces: `async fn monitor_runtime<F>(tasks: JoinSet<RuntimeExit>, cancellation: CancellationToken, shutdown: F) -> Result<()>`
- Consumes: `supervise_gateway` for Discord and Matrix only.
- Consumes: Teloxide `Dispatcher::shutdown_token`; Telegram receives cancellation but no retry loop.
- Preserves: existing Telegram/Discord/Matrix event translation and authorization code.

- [ ] **Step 1: Add failing sibling-cancellation and intentional-shutdown tests**

Add inside the existing runtime test module:

```rust
#[tokio::test]
async fn terminal_gateway_failure_cancels_and_joins_sibling() {
    let cancellation = CancellationToken::new();
    let sibling_cancel = cancellation.clone();
    let sibling_stopped = Arc::new(AtomicUsize::new(0));
    let stopped = sibling_stopped.clone();
    let mut tasks: JoinSet<RuntimeExit> = JoinSet::new();
    tasks.spawn(async { ("matrix", Err(anyhow::anyhow!("retries_exhausted"))) });
    tasks.spawn(async move {
        sibling_cancel.cancelled().await;
        stopped.store(1, Ordering::SeqCst);
        ("telegram", Ok(()))
    });

    let result = monitor_runtime(tasks, cancellation, std::future::pending()).await;

    assert!(result.is_err());
    assert_eq!(sibling_stopped.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn intentional_shutdown_cancels_siblings_and_returns_success() {
    let cancellation = CancellationToken::new();
    let sibling_cancel = cancellation.clone();
    let mut tasks: JoinSet<RuntimeExit> = JoinSet::new();
    tasks.spawn(async move {
        sibling_cancel.cancelled().await;
        ("matrix", Ok(()))
    });

    let result = monitor_runtime(tasks, cancellation, std::future::ready(())).await;

    assert!(result.is_ok());
}
```

- [ ] **Step 2: Run tests and verify RED**

Run: `cd rust/aegis && cargo test --bin aegis main::runtime::tests -- --test-threads=1`

Expected: compile failure because `RuntimeExit` and `monitor_runtime` do not exist.

- [ ] **Step 3: Implement joined monitoring and bounded drain**

Add before `run`:

```rust
type RuntimeExit = (&'static str, Result<()>);

async fn drain_runtime_tasks(tasks: &mut JoinSet<RuntimeExit>) {
    if tokio::time::timeout(Duration::from_secs(10), async {
        while tasks.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }
}

async fn monitor_runtime<F>(
    mut tasks: JoinSet<RuntimeExit>,
    cancellation: CancellationToken,
    shutdown: F,
) -> Result<()>
where
    F: Future<Output = ()>,
{
    tokio::pin!(shutdown);
    let terminal = loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => {
                cancellation.cancel();
                break None;
            }
            joined = tasks.join_next() => match joined {
                Some(Ok((platform, Ok(())))) if cancellation.is_cancelled() => {
                    if tasks.is_empty() { break None; }
                }
                Some(Ok((platform, Ok(())))) => {
                    break Some(anyhow::anyhow!(
                        "runtime platform={platform} terminal_reason=unexpected_return"
                    ));
                }
                Some(Ok((platform, Err(_)))) => {
                    break Some(anyhow::anyhow!(
                        "runtime platform={platform} terminal_reason=task_failure"
                    ));
                }
                Some(Err(_)) => {
                    break Some(anyhow::anyhow!(
                        "runtime platform=unknown terminal_reason=join_failure"
                    ));
                }
                None => {
                    break Some(anyhow::anyhow!(
                        "runtime platform=all terminal_reason=no_tasks"
                    ));
                }
            }
        }
    };

    cancellation.cancel();
    drain_runtime_tasks(&mut tasks).await;
    terminal.map_or(Ok(()), Err)
}
```

Change the guarded success arm to avoid an unused binding under Clippy:

```rust
Some(Ok((_platform, Ok(())))) if cancellation.is_cancelled() => {
    if tasks.is_empty() { break None; }
}
```

- [ ] **Step 4: Replace detached scheduler/notification startup with one awaited helper**

Add before `run`:

```rust
async fn initialize_runtime(adapter: Arc<dyn aegis::adapters::common::BotAdapter>, target: TargetId) -> Result<()> {
    aegis::core::system::scheduler::start_scheduler(adapter.clone(), target.clone()).await?;
    let (upgrade, reboot, online) = tokio::join!(
        crate::notify_upgrade_success(&*adapter, &target),
        crate::notify_bbr3_reboot_result(&*adapter, &target),
        crate::notify_online(&*adapter, &target),
    );
    for (stage, result) in [
        ("upgrade_notification", upgrade),
        ("reboot_notification", reboot),
        ("online_notification", online),
    ] {
        if result.is_err() {
            log::warn!("runtime platform=notification attempt=1 backoff_ms=0 terminal_reason={stage}");
        }
    }
    Ok(())
}
```

Rename the now-redundant parameter in `run` from `enable_matrix: bool` to `_enable_matrix: bool`; preserve its position so `main.rs` requires no edit.

After language initialization and before gateway setup, add:

```rust
let (scheduler_adapter, scheduler_target) = if let Some(raw) = discord_raw.as_ref() {
    (raw.adapter.clone(), TargetId(raw.admin_channel.to_string()))
} else {
    (state.adapter.clone(), TargetId(admin_id.to_string()))
};
initialize_runtime(scheduler_adapter, scheduler_target).await?;

let cancellation = CancellationToken::new();
let mut tasks: JoinSet<RuntimeExit> = JoinSet::new();
```

Delete all three detached scheduler/notification `tokio::spawn` blocks at current lines 98-120, 320-342, and 356-378.

- [ ] **Step 5: Replace Discord gateway spawning with supervised rebuilding**

Replace the Discord block's `build_handle`, Discord-only token, and both detached `client.start()` branches with:

```rust
if let Some(raw) = discord_raw {
    let super::discord::DiscordRawHandle {
        token,
        admin_channel,
        adapter,
        ..
    } = raw;
    let discord_state = state.clone();
    let discord_cancel = cancellation.clone();
    tasks.spawn(async move {
        let result = supervise_gateway(
            "discord",
            discord_cancel,
            RetryPolicy::gateway_default(),
            move || {
                let token = token.clone();
                let adapter = adapter.clone();
                let state = discord_state.clone();
                async move {
                    let mut client = super::discord::build_client(
                        &token,
                        admin_channel,
                        adapter,
                        state,
                    )
                    .await?;
                    client.start().await.context("discord gateway")
                }
            },
        )
        .await;
        ("discord", result)
    });
}
```

- [ ] **Step 6: Keep Matrix event registration and supervise only its sync loop**

Keep the complete existing `client.add_event_handler(...)` block unchanged. Replace only the detached Matrix sync spawn with:

```rust
let sync_client = client.clone();
let matrix_cancel = cancellation.clone();
tasks.spawn(async move {
    let result = supervise_gateway(
        "matrix",
        matrix_cancel,
        RetryPolicy::gateway_default(),
        move || {
            let client = sync_client.clone();
            async move {
                client
                    .sync(matrix_sdk::config::SyncSettings::default())
                    .await
                    .context("matrix gateway")
            }
        },
    )
    .await;
    ("matrix", result)
});
```

- [ ] **Step 7: Make Telegram a cancellable, non-retrying sibling**

Replace the final Telegram dispatcher expression with:

```rust
let mut dispatcher = Dispatcher::builder(bot.clone(), handler)
    .dependencies(dptree::deps![state.clone()])
    .build();
let shutdown_token = dispatcher.shutdown_token();
let telegram_cancel = cancellation.clone();
tasks.spawn(async move {
    let mut dispatch = Box::pin(dispatcher.dispatch());
    tokio::select! {
        _ = &mut dispatch => ("telegram", Ok(())),
        _ = telegram_cancel.cancelled() => {
            if let Ok(shutdown) = shutdown_token.shutdown() {
                let _ = tokio::join!(&mut dispatch, shutdown);
            } else {
                dispatch.await;
            }
            ("telegram", Ok(()))
        }
    }
});
```

Do not call `.enable_ctrlc_handler()`: `monitor_runtime` is now the sole signal owner. Do not wrap Telegram in `supervise_gateway`.

- [ ] **Step 8: Replace Matrix-only and Discord-only keepalive branches with one monitor call**

Delete the Matrix-only cancellation/keepalive block and the Discord-only cancellation/keepalive block. End `run` with:

```rust
monitor_runtime(tasks, cancellation, async {
    if tokio::signal::ctrl_c().await.is_ok() {
        log::info!(
            "runtime platform=all attempt=0 backoff_ms=0 terminal_reason=intentional_shutdown"
        );
    }
})
.await
```

This return value is already propagated by `src/main.rs:109-118`; no `main.rs` change is needed for nonzero terminal failure.

- [ ] **Step 9: Run runtime tests and verify GREEN**

Run: `cd rust/aegis && cargo test --bin aegis main::runtime::tests -- --test-threads=1`

Expected: all runtime tests pass; terminal failure cancels and joins its sibling, intentional shutdown returns success, and retry tests remain green.

- [ ] **Step 10: Compile all runtime platform paths**

Run:

```bash
cd rust/aegis
cargo check --bin aegis
cargo check --bin aegis --all-features
```

Expected: both commands pass. There is no detached `client.start`, `client.sync`, scheduler startup, or notification task in `main/runtime.rs`.

- [ ] **Step 11: Commit Task 6**

```bash
git add rust/aegis/src/main/runtime.rs rust/aegis/src/main/discord.rs
git commit -m "fix: supervise gateway runtime tasks"
```

---

### Task 7: Acceptance, Source Guards, And Full Rust Gates

**Files:**
- Modify: `rust/aegis/src/core/system/scheduler/mod.rs` tests only if acceptance gaps are found.
- Modify: `rust/aegis/src/main/runtime.rs` tests only if acceptance gaps are found.
- Verify: all four modified Rust files.

**Interfaces:**
- Verifies scheduler commit order, all pre-commit rollback stages, fail-closed load, bounded retry/capped backoff, stable reset, sibling cancellation, intentional shutdown, safe logs, and no detached gateway failures.

- [ ] **Step 1: Run the complete focused scheduler acceptance suite**

Run: `cd rust/aegis && cargo test --lib core::system::scheduler::tests -- --test-threads=1`

Expected: PASS, including:

```text
corrupt_state_fails_closed_without_rewriting_file
complete_state_validation_rejects_disabled_invalid_task
save_to_file_atomically_replaces_complete_state
candidate_failures_preserve_old_file_and_running_state
successful_replace_persists_then_swaps_then_shuts_down_old
new_installs_unchanged_state_without_rewriting_file
new_with_missing_state_does_not_create_file
concurrent_mutations_do_not_lose_an_update
corrupt_global_replacement_preserves_old_manager
remove_tasks_by_type_rolls_back_on_persistence_failure
add_new_task_rejects_invalid_task_without_persisting_state
```

- [ ] **Step 2: Run the complete focused runtime acceptance suite**

Run: `cd rust/aegis && cargo test --bin aegis main::runtime::tests -- --test-threads=1`

Expected: PASS, including:

```text
backoff_is_exponential_and_capped
transient_failures_retry_only_within_budget
stable_run_resets_retry_budget
intentional_cancellation_is_success_without_retry
gateway_log_fields_exclude_raw_errors_and_content
terminal_gateway_failure_cancels_and_joins_sibling
intentional_shutdown_cancels_siblings_and_returns_success
```

- [ ] **Step 3: Run exact source guards for forbidden bypasses and detached gateways**

Run:

```bash
cd rust/aegis
if grep -En 'unwrap_or_else\(\|_\| SchedulerState::default|manager\.state\.lock|save_to_file\(&manager\.state_path\)|start_all_tasks' src/core/system/scheduler/mod.rs src/shared/handlers/schedule.rs; then exit 1; fi
if grep -En 'enable_ctrlc_handler|let _ = client\.start|let _ = client\.sync' src/main/runtime.rs; then exit 1; fi
grep -En 'supervise_gateway|shutdown_token\(\)|monitor_runtime\(' src/main/runtime.rs
if grep -En 'supervise_gateway\("telegram"' src/main/runtime.rs; then exit 1; fi
```

Expected: the first two guards print nothing and exit 0. The final command finds Discord supervision, Matrix supervision, Telegram shutdown-token wiring, and the single runtime monitor. It must not find `supervise_gateway("telegram"`.

- [ ] **Step 4: Verify the process-result boundary without editing it**

Run:

```bash
cd rust/aegis
grep -En '#\[tokio::main\]|async fn main\(\) -> Result<\(\)>|main::runtime::run\(' src/main.rs
```

Expected: all three existing lines are found, proving `monitor_runtime(...).await` errors propagate through `run` and cause Rust's `main() -> Result<()>` termination path to return nonzero.

- [ ] **Step 5: Run mandatory formatting, lint, and full tests**

Run from `rust/aegis`:

```bash
cargo fmt
cargo clippy -- -D warnings
cargo test
```

Expected: all commands exit 0; all tests pass and Clippy emits no warning. If the repository's documented pre-existing environment requires `PROTOC`, rerun the same commands with the repository-provided `PROTOC` path, without skipping any target or test.

- [ ] **Step 6: Review the final diff for boundary and credential safety**

Run:

```bash
git diff --check
git diff -- rust/aegis/src/core/system/scheduler/mod.rs rust/aegis/src/shared/handlers/schedule.rs rust/aegis/src/main/discord.rs rust/aegis/src/main/runtime.rs
```

Expected: `git diff --check` is silent. The diff changes only the four named Rust files, contains no token/event logging, no Telegram retry loop, no ignored scheduler stage result, and no unrelated refactor.

- [ ] **Step 7: Request both mandatory reviews**

Use `superpowers:requesting-code-review` twice against this plan:

```text
Review 1: specification compliance. Block on any missing scheduler stage, incorrect transaction order, retry/reset/cancellation mismatch, Telegram overreach, unsafe logging, or detached failure.
Review 2: code quality. Block on races, an active-state lock held across scheduler/network awaits (the transaction mutex intentionally spans the transaction), unjoined tasks, an unbounded runtime-task drain, type/signature mismatch, flaky timing, or Clippy failure.
```

Expected: no Critical or Important findings remain. Fix findings with focused RED -> GREEN tests and rerun Steps 1-6.

- [ ] **Step 8: Commit final acceptance adjustments**

```bash
git add rust/aegis/src/core/system/scheduler/mod.rs rust/aegis/src/shared/handlers/schedule.rs rust/aegis/src/main/discord.rs rust/aegis/src/main/runtime.rs
git commit -m "test: cover scheduler and gateway supervision"
```

Do not create an empty commit if review required no source adjustment.

---

## Implementation Completion Criteria

- Candidate scheduler construction never takes or shuts down the old scheduler.
- `SchedulerManager::new` validates and starts the unchanged loaded state, installs it in memory, and leaves the existing state file byte-for-byte untouched.
- Missing state installs the validated default in memory without creating a file; the first successful mutation persists the full state.
- Every persisted task, including disabled tasks, is validated before registration.
- For mutations, every enabled task is registered and the candidate is started before persistence.
- For mutations, atomic persistence completes before the active scheduler/state pair swaps.
- Old scheduler shutdown occurs only after swap; post-commit shutdown errors are logged explicitly and never roll the file/runtime pair back to an already-shutting-down instance.
- Add, indexed remove, and GeoData remove are serialized through one transaction lock.
- Load, validation, registration, start, or persistence failure preserves old bytes and old active state.
- Corrupt state returns `scheduler stage=load` and remains unchanged.
- Discord/Matrix retries are bounded and capped; only a run lasting 300 seconds resets the budget.
- Exhaustion cancels and joins siblings and returns an error to `main`.
- Intentional signal/cancellation returns success without retry/crash accounting.
- Logs expose only approved fields and static terminal reasons.
- Telegram is cancellable when sharing Matrix runtime but is never gateway-retried.
- Startup, gateways, Telegram, signal handling, and shutdown have no detached silent failure path.
