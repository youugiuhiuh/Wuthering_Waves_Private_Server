# Uniform Random Port Allocation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Unify non-hopping HY2 and TUIC port allocation to use random ports with dual collision checks (`is_port_in_locked_range` + `is_port_available`), matching the existing KCP pattern.

**Architecture:** 3 files modified. Task order matters: first change the callers (hy2_batch + tuic_batch), then delete the now-unused `allocate_port()` function.

**Tech Stack:** Rust (tokio, rand), sing-box (wwps-box)

---

### Task 1: hy2_batch.rs — random port + dual checks

**Files:**
- Modify: `rust/aegis/src/core/singbox/hy2_batch.rs:46-51`
- Test: existing `cargo test` (no new test — port allocation is integration-level)

- [ ] **Step 1: Replace allocate_port() with random + dual checks**

Change lines 46-51 from:
```rust
let (main_port, hop_range) = if enable_hopping {
    PortAllocator::allocate_hysteria2().await?
} else {
    let port = PortAllocator::allocate_port().await?;
    (port, (port, port))
};
```

To:
```rust
let (main_port, hop_range) = if enable_hopping {
    PortAllocator::allocate_hysteria2().await?
} else {
    let port = loop {
        let p = rand::rngs::StdRng::from_entropy().gen_range(10000..60000);
        if crate::core::xray::port_allocator::PortAllocator::is_port_in_locked_range(p).await {
            continue;
        }
        if crate::core::system::maintenance::MaintenanceManager::is_port_available(p).await {
            break p;
        }
    };
    (port, (port, port))
};
```

Add these imports at the top of hy2_batch.rs (after line 1 `use crate::...`):
```rust
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
```

- [ ] **Step 2: Verify compiles**

```bash
cargo check
```
Expected: No errors

- [ ] **Step 3: Run tests**

```bash
cargo test
```
Expected: All pass

- [ ] **Step 4: Commit**

```bash
git add rust/aegis/src/core/singbox/hy2_batch.rs
git commit -m "fix: non-hopping HY2 uses random port with dual collision checks"
```

---

### Task 2: tuic_batch.rs — add is_port_in_locked_range check

**Files:**
- Modify: `rust/aegis/src/core/singbox/tuic_batch.rs:47-52`

- [ ] **Step 1: Add is_port_in_locked_range to TUIC's random port loop**

Change lines 47-52 from:
```rust
loop {
    let p = StdRng::from_entropy().gen_range(10000..60000);
    if MaintenanceManager::is_port_available(p).await {
        break p;
    }
};
```

To:
```rust
loop {
    let p = StdRng::from_entropy().gen_range(10000..60000);
    if crate::core::xray::port_allocator::PortAllocator::is_port_in_locked_range(p).await {
        continue;
    }
    if MaintenanceManager::is_port_available(p).await {
        break p;
    }
};
```

- [ ] **Step 2: Verify compiles**

```bash
cargo check
```
Expected: No errors

- [ ] **Step 3: Run tests**

```bash
cargo test
```
Expected: All pass

- [ ] **Step 4: Commit**

```bash
git add rust/aegis/src/core/singbox/tuic_batch.rs
git commit -m "fix: TUIC port allocation checks is_port_in_locked_range"
```

---

### Task 3: port_allocator.rs — delete allocate_port()

**Files:**
- Modify: `rust/aegis/src/core/xray/port_allocator.rs:169-184`

- [ ] **Step 1: Remove allocate_port() function**

Delete lines 169-184 (the entire `allocate_port` function from `pub async fn allocate_port` through `}` after `return Ok(port)`).

The function spans from:
```rust
    pub async fn allocate_port() -> Result<u16> {
```
to:
```rust
    }
```

And all code in between (scan_all_occupied_ports loop, save_port_alloc, etc.).

- [ ] **Step 2: Verify compiles**

```bash
cargo check
```
Expected: No errors. Confirm no "unused function" warnings — `allocate_port` should be gone entirely.

- [ ] **Step 3: Run tests**

```bash
cargo test
```
Expected: All pass

- [ ] **Step 4: Commit**

```bash
git add rust/aegis/src/core/xray/port_allocator.rs
git commit -m "refactor: remove unused allocate_port() function"
```

---

### Task 4: Final verification

**Files:** None to modify.

- [ ] **Step 1: Format and clippy**

```bash
cargo fmt
cargo clippy -- -D warnings
```
Expected: Clean

- [ ] **Step 2: Full test suite**

```bash
cargo test 2>&1 | tail -10
```
Expected: "test result: ok. N passed; 0 failed"

- [ ] **Step 3: Verify no allocate_port references remain**

```bash
rg "allocate_port" rust/aegis/src/
```
Expected: No matches

- [ ] **Step 4: Commit any remaining changes**

```bash
git add -A
git commit -m "chore: fmt + clippy fixes"
```
