# Command Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) for syntax tracking.

**Goal:** Ensure `run_cmd_output` and `run_cmd_stream` kill the process group (Linux), drain pipes, and reap the child before returning from a timeout.

**Architecture:** Replace `cmd.output()` with spawn + pipe reads inside both functions. On Linux, setpgid before exec; on timeout, SIGTERM → short grace → SIGKILL. Keep all four public signatures unchanged. Bounded diagnostic tail (64 KiB) for timeout errors.

**Tech Stack:** Rust, tokio, libc "0.2" (already a dep).

## Global Constraints

- All four public fn signatures stay the same.
- Normal completion returns FULL output (same as today).
- Timeout returns bounded diagnostic tail (last 64 KiB of each stream).
- No new dependencies.
- Non-Linux: fallback to `child.start_kill()` without process-group control.

---

### Task 1: Process-group helpers + bounded ring buffer

**Files:**
- Modify: `src/core/cmd_async.rs`

**Produces:**
- `#[cfg(target_os = "linux")] fn set_process_group(pid: u32) -> io::Result<()>` — wraps `libc::setpgid(pid as i32, 0)`.
- `#[cfg(target_os = "linux")] fn kill_process_group(pid: u32) -> io::Result<()>` — sends SIGTERM, waits 2s, sends SIGKILL if needed.
- `const MAX_DIAG_BYTES: usize = 65536`
- `fn bounded_tail(buf: &[u8], limit: usize) -> String` — returns `String::from_utf8_lossy` of last `limit` bytes.

- [ ] **Step 1: Add failing test for `set_process_group`**
- [ ] **Step 2: Run test — RED**
- [ ] **Step 3: Implement helpers** — ~30 lines total
- [ ] **Step 4: Run test — GREEN**
- [ ] **Step 5: Commit** `git add src/core/cmd_async.rs && git commit -m "feat: process-group helpers for cmd_async"`

---

### Task 2: Rewrite `run_cmd_output` with lifecycle management

**Files:**
- Modify: `src/core/cmd_async.rs`

**Produces:** Replaces `timeout(cmd.output())` with:
1. `Command::new(program).args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()`
2. `set_process_group(child.id())`
3. Two `tokio::spawn` tasks, each `read_to_end` into `Arc<Mutex<Vec<u8>>>`
4. `timeout(timeout_duration, child.wait())`
5. **Normal path:** `read_out.await; read_err.await; Ok((status, stdout, stderr))`
6. **Timeout path:** `kill_process_group(child_pid)` → `child.wait().await` → drain reads → return error with `bounded_tail` from both buffers

When timeout returns: the child has been killed, waited, and all pipe data consumed. Return `Err` containing bounded tail.

- [ ] **Step 1: Add failing test — timeout kills descendant**
- [ ] **Step 2: Run — RED**
- [ ] **Step 3: Rewrite `run_cmd_output`** per spec above
- [ ] **Step 4: Run existing + new tests — GREEN**
- [ ] **Step 5: Commit** `git commit -m "feat: manage command lifecycle in run_cmd_output"`

---

### Task 3: Rewrite `run_cmd_stream` with lifecycle management

**Files:**
- Modify: `src/core/cmd_async.rs`

**Produces:** Replace `timeout(execution)` with same pattern:
1. `set_process_group(child.id())`
2. Wrap the read+wait loop in `timeout()`
3. On timeout: `kill_process_group(child_pid)` → `child.wait().await` → return timeout error

Also fix `run_cmd_output` → `run_cmd_status` → `run_cmd_checked` chain (they delegate to `run_cmd_output`, which already handles lifecycle — no changes needed beyond `run_cmd_output`).

- [ ] **Step 1: Update `run_cmd_stream` timeout path**
- [ ] **Step 2: Add test — `run_cmd_stream` timeout kills descendant**
- [ ] **Step 3: Run all tests — GREEN**
- [ ] **Step 4: Commit** `git commit -m "feat: manage command lifecycle in run_cmd_stream"`

---

### Task 4: Bounded diagnostic tail tests

**Files:**
- Modify: `src/core/cmd_async.rs` (tests section)

**Produces:** Test that a noisy command's output is bounded in the timeout error and that existing helpers (`run_cmd_status`, `run_cmd_checked`) still pass all pre-existing tests.

- [ ] **Step 1: Add test `timeout_bounded_diag_tail`** — spawn a command that produces >64 KiB output, verify timeout error string does not exceed limit
- [ ] **Step 2: Run — GREEN**
- [ ] **Step 3: Full test suite**

```bash
cargo test --lib cmd_async::tests -- --skip test_deploy_candidate_rejects_version_mismatch
cargo fmt
cargo clippy -- -D warnings
```

- [ ] **Step 4: Commit** `git commit -m "test: bounded diagnostic tail on timeout"`
