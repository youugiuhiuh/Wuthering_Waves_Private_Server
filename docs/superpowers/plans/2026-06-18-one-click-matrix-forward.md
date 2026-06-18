# One-Click Deploy Matrix Forward Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Forward one-click deploy batch creation links to both Telegram and Matrix via RoutingAdapter.

**Architecture:** Three batch creation calls (XHTTP Reality ×20, Reality Vision ×20, TUIC ×3) currently discard `Ok(result)` via `if let Err` pattern. Change to `match` to capture `Ok(result)` and call `send_singbox_batch_result()`, which uses RoutingAdapter to route protocol links (vless://) to secondary channels.

**Tech Stack:** Rust, tokio, teloxide, RoutingAdapter, batch_handler

---

### Task 1: Add import and adapter capture

**Files:**
- Modify: `rust/aegis/src/adapters/telegram/handlers/ops.rs`

- [ ] **Step 1: Add `send_singbox_batch_result` import**

Insert after line 5 (`use aegis::core::system::maintenance::MaintenanceManager;`):

```rust
use crate::app::batch_handler::send_singbox_batch_result;
```

- [ ] **Step 2: Add adapter capture before tokio::spawn**

Add line after `let msg_id_clone = ctx.msg_id;` (after line 438):

```rust
let adapter = ctx.state.adapter.clone();
```

The `async move` closure on line 440 will automatically capture `adapter`.

---

### Task 2: Forward step 4 (XHTTP Reality) and step 5 (Reality Vision) results

**Files:**
- Modify: `rust/aegis/src/adapters/telegram/handlers/ops.rs:501-519`

- [ ] **Step 1: Change step 4 from `if let Err` to `match` with Ok forwarding**

Replace lines 501-509:

```rust
                if !failed {
                    send_progress(&tx, 4, 7, format!("{} ({})", t!("ops.deploy_step_xhttp"), ip_version.label()));
                    if let Err(e) = aegis::core::xray::config::ConfigManager::batch_create_xhttp_reality_enhanced(
                        20, ip_version,
                    ).await {
                        let _ = tx.send(t!("ops.deploy_fail", "0" => format!("{}: {}", t!("ops.deploy_fail_xhttp"), e)).to_string());
                        failed = true;
                    }
                }
```

Replace with:

```rust
                if !failed {
                    send_progress(&tx, 4, 7, format!("{} ({})", t!("ops.deploy_step_xhttp"), ip_version.label()));
                    match aegis::core::xray::config::ConfigManager::batch_create_xhttp_reality_enhanced(
                        20, ip_version,
                    ).await {
                        Ok(result) => {
                            let _ = send_singbox_batch_result(
                                adapter.clone(), chat_id_clone, "XHTTP Reality", &result,
                            ).await;
                        }
                        Err(e) => {
                            let _ = tx.send(t!("ops.deploy_fail", "0" => format!("{}: {}", t!("ops.deploy_fail_xhttp"), e)).to_string());
                            failed = true;
                        }
                    }
                }
```

- [ ] **Step 2: Change step 5 (Reality Vision) same pattern**

Replace lines 511-519:

```rust
                if !failed {
                    send_progress(&tx, 5, 7, format!("{} ({})", t!("ops.deploy_step_vision"), ip_version.label()));
                    if let Err(e) = aegis::core::xray::config::ConfigManager::batch_create_reality_vision_enhanced(
                        20, ip_version,
                    ).await {
                        let _ = tx.send(t!("ops.deploy_fail", "0" => format!("{}: {}", t!("ops.deploy_fail_vision"), e)).to_string());
                        failed = true;
                    }
                }
```

Replace with:

```rust
                if !failed {
                    send_progress(&tx, 5, 7, format!("{} ({})", t!("ops.deploy_step_vision"), ip_version.label()));
                    match aegis::core::xray::config::ConfigManager::batch_create_reality_vision_enhanced(
                        20, ip_version,
                    ).await {
                        Ok(result) => {
                            let _ = send_singbox_batch_result(
                                adapter.clone(), chat_id_clone, "Reality Vision", &result,
                            ).await;
                        }
                        Err(e) => {
                            let _ = tx.send(t!("ops.deploy_fail", "0" => format!("{}: {}", t!("ops.deploy_fail_vision"), e)).to_string());
                            failed = true;
                        }
                    }
                }
```

---

### Task 3: Forward step 7 (TUIC) results

**Files:**
- Modify: `rust/aegis/src/adapters/telegram/handlers/ops.rs:529-541`

- [ ] **Step 1: Change step 7 from `if let Err` to `match` with Ok forwarding**

Replace lines 529-541:

```rust
                if !failed {
                    send_progress(&tx, 7, 7, format!("{} ({})", t!("ops.deploy_step_tuic"), ip_version.label()));
                    if let Err(e) =
                        aegis::core::singbox::config::SingBoxConfigManager::batch_create_tuic(
                            3,
                            ip_version,
                        )
                        .await
                    {
                        let _ = tx.send(t!("ops.deploy_fail", "0" => format!("{}: {}", t!("ops.deploy_fail_tuic"), e)).to_string());
                        failed = true;
                    }
                }
```

Replace with:

```rust
                if !failed {
                    send_progress(&tx, 7, 7, format!("{} ({})", t!("ops.deploy_step_tuic"), ip_version.label()));
                    match aegis::core::singbox::config::SingBoxConfigManager::batch_create_tuic(
                        3,
                        ip_version,
                    )
                    .await
                    {
                        Ok(result) => {
                            let _ = send_singbox_batch_result(
                                adapter.clone(), chat_id_clone, "TUIC", &result,
                            ).await;
                        }
                        Err(e) => {
                            let _ = tx.send(t!("ops.deploy_fail", "0" => format!("{}: {}", t!("ops.deploy_fail_tuic"), e)).to_string());
                            failed = true;
                        }
                    }
                }
```

---

### Task 4: Verify compilation

**Files:** N/A

- [ ] **Step 1: Run cargo check**

Run: `cargo check -p aegis`
Expected: Compilation succeeds with no errors

- [ ] **Step 2: Run cargo build (optional)**

Run: `cargo build -p aegis`
Expected: Binary builds successfully
