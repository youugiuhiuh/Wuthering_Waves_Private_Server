# tgbot main.rs Refactoring Design

**Date**: 2026-05-16
**Status**: Approved

## Problem Statement

`main.rs` is 5184 lines with a single `handle_callback` function spanning ~4140 lines (lines 755-4895). All Telegram inline keyboard callback logic is in one monolithic `match` block. This creates severe maintainability, testability, and readability problems.

Key issues:
- `handle_callback` is ~4140 lines — the largest function by far
- UI (keyboard building) and business logic are tightly coupled
- Callback data is parsed as raw strings with no type safety
- Utility functions (`escape_html`, `format_duration_human`, etc.) are mixed with handler logic
- The `loop { ... continue }` redirect pattern is hard to follow
- No unit tests for callback logic

## Constraints

- **Callback data format must remain backward compatible** — existing users' inline keyboards must continue to work
- **Incremental migration** — each step must compile and pass all existing tests independently
- **Three goals**: maintainability (modular split), type safety (typed callback parsing), test coverage (separable logic)

## Approach: Handler Trait Dispatcher (Method A)

### Architecture

#### Target File Structure

```
src/
  main.rs                  # ~100 lines: init + dispatcher registration
  app/
    mod.rs
    state.rs              # Unchanged
    auth.rs               # Unchanged
    destruct_flow.rs      # Unchanged
    batch_handler.rs      # Unchanged
  handlers/
    mod.rs                # HandlerRegistry + dispatch logic
    context.rs            # CallbackContext, HandlerResult
    menu.rs               # Navigation: m_main, m_ops_center, m_settings, m_danger
    monitor.rs            # System status: m_mon
    user_mgmt.rs          # User management: m_usr, m_xray_mgmt, m_singbox_mgmt, m_pq_*
    xray_batch.rs         # Xray batch creation: u_batch_*, u_xhttp_*, u_kcp_*
    singbox.rs            # Sing-box management: sb_*
    warp.rs               # WARP routing: m_warp, a_warp_*
    security.rs           # Security: m_security, a_fw
    network.rs            # Network optimization: m_net_opt, a_bbr3_*, a_tune
    schedule.rs           # Scheduled tasks: m_sched, s_add_*, s_custom_*, s_del_*
    core_upgrade.rs       # Core upgrades: a_wwps_core_*, a_wwps_box_*, a_upgrade
    config_delete.rs       # Config deletion: m_del_cfg, cfg_filter_*, cfg_del_*
    session.rs            # Session timeout: m_session_timeout, set_timeout:*
    geo.rs                # Geo data: a_geo_menu, a_geo, a_geo_sched_*
    log.rs                # Log audit: m_log, l_xray, l_box
  utils/
    mod.rs                # escape_html, format_duration_human, validate_hash_prefix, validate_idx
```

#### Core Types

```rust
// handlers/context.rs
pub struct CallbackContext {
    pub bot: Bot,
    pub chat_id: ChatId,
    pub msg_id: MessageId,
    pub data: String,
    pub state: Arc<AppState>,
    pub user_id: i64,
}

pub enum HandlerResult {
    Handled,
    NotHandled,
    Redirect(String),
}
```

```rust
// handlers/mod.rs
#[async_trait]
pub trait CallbackHandler: Send + Sync {
    fn patterns(&self) -> &[&str];
    async fn handle(&self, ctx: CallbackContext) -> HandlerResult;
}

pub struct HandlerRegistry {
    handlers: Vec<Box<dyn CallbackHandler>>,
}

impl HandlerRegistry {
    pub async fn dispatch(&self, ctx: CallbackContext) -> ResponseResult<()> {
        // Loop to support Redirect
        let mut current_data = ctx.data.clone();
        loop {
            for handler in &self.handlers {
                if handler.patterns().iter().any(|p| current_data.starts_with(p) || current_data == *p) {
                    let handler_ctx = ctx.clone_for_redirect(&current_data);
                    match handler.handle(handler_ctx).await {
                        HandlerResult::Handled => return Ok(()),
                        HandlerResult::Redirect(new_data) => {
                            current_data = new_data;
                            continue;
                        }
                        HandlerResult::NotHandled => continue,
                    }
                }
            }
            return Ok(());
        }
    }
}
```

### Callback Data Type Safety

Introduced in Phase 5 (after all modules are extracted). Each handler module defines its own typed callback data parser:

```rust
// handlers/callback_data.rs (Phase 5)
pub enum CallbackAction {
    MainMenu,
    OpsCenter,
    BatchInit { proto: Proto },
    BatchIpInit { proto: Proto, ip_code: String },
    // ...
    Unknown(String),
}
```

Each handler owns its parse logic, introduced one module at a time during Phase 5.

### Migration Phases

#### Phase 1: Infrastructure (no behavior change)

1. Move `format_duration_human`, `escape_html`, `validate_hash_prefix`, `validate_idx` to `src/utils/mod.rs`
2. Create `handlers/context.rs` with `CallbackContext` and `HandlerResult`
3. Create `handlers/mod.rs` with `CallbackHandler` trait and `HandlerRegistry`
4. Refactor `handle_callback` to use `HandlerRegistry` with a single catch-all handler containing the entire existing match block
5. Verify: `cargo test` passes, behavior identical

#### Phase 2: Extract independent modules (1 PR each)

6. `handlers/singbox.rs` — all `sb_*` callbacks (~400 lines)
7. `handlers/warp.rs` — `m_warp`/`a_warp_*` (~250 lines)
8. `handlers/monitor.rs` — `m_mon` (~30 lines)
9. `handlers/log.rs` — `m_log`/`l_xray`/`l_box` (~60 lines)
10. `handlers/session.rs` — `m_session_timeout`/`set_timeout:*` (~40 lines)
11. `handlers/geo.rs` — `a_geo_*` (~80 lines)

#### Phase 3: Extract medium-dependency modules

12. `handlers/schedule.rs` — `m_sched`/`s_add_*`/`s_custom_*`/`s_del_*` (~300 lines)
13. `handlers/network.rs` — `m_net_opt`/`a_bbr3_*`/`a_tune` (~200 lines)
14. `handlers/security.rs` — `m_security`/`a_fw` (~100 lines)
15. `handlers/core_upgrade.rs` — `a_wwps_core_*`/`a_wwps_box_*`/`a_upgrade` (~150 lines)

#### Phase 4: Extract complex modules

16. `handlers/config_delete.rs` — `m_del_cfg`/`cfg_filter:*`/`cfg_del_*` (~250 lines)
17. `handlers/xray_batch.rs` — `u_batch_*`/`u_xhttp_*`/`u_kcp_*`/`u_batch_init` (~500 lines)
18. `handlers/user_mgmt.rs` — `m_usr`/`m_xray_mgmt`/`m_singbox_mgmt`/`m_pq_*`/`u_l:*`/`u_d:*` (~200 lines)

#### Phase 5: Finalization

19. `handlers/menu.rs` — navigation callbacks (`m_main`/`m_ops_center`/`m_settings`/`m_danger`)
20. Introduce `CallbackAction` enum for type-safe parsing
21. Remove catch-all handler — `handle_callback` becomes a thin `registry.dispatch()` call
22. Move `handle_command` and `handle_message` to `handlers/commands.rs` and `handlers/messages.rs` (optional)

### Redirect Pattern Handling

Current: `q = CallbackQuery { data: Some("m_main".to_string()), ..new_q }; continue;`
New: `HandlerResult::Redirect("m_main".into())`

The `HandlerRegistry::dispatch` loop handles redirects by restarting dispatch. Fully backward compatible.

### Testing Strategy

| Level | Content | When |
|-------|---------|------|
| Unit: utils | `format_duration_human`, `escape_html`, `validate_*` | Phase 1 |
| Unit: CallbackAction parse | Round-trip `parse(to_data()) == original` | Phase 5 |
| Unit: handler logic | Per-module business logic (mocked bot) | Each extraction |
| Integration: dispatch | `HandlerRegistry` dispatches correctly | Phase 1 |
| Existing tests | All existing tests passing | Every step |

### Dependencies

- `async-trait` crate for `CallbackHandler` trait
- No other new dependencies required

### Risk Mitigation

- Phase 1 creates infrastructure only — zero behavior change
- Each module extraction is reversible — move arms back to catch-all
- Callback data format never changes — users see no difference
- `Redirect` replaces `continue` loop — one-to-one mapping, no semantic change
