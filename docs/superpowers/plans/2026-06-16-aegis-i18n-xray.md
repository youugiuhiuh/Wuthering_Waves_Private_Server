# xray.rs Internationalization Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace all hardcoded Chinese strings in `rust/aegis/src/adapters/telegram/handlers/xray.rs` with `t!()` macro calls using keys from zh.yml.

**Architecture:** Single-file edit. Add `use rust_i18n::t;` import, replace every UI-facing string literal with `t!("xray.xxx")`, `t!("menu.xxx")`, or `t!("callback.xxx")`.

**Tech Stack:** Rust, rust_i18n, teloxide

**Key mappings reference (zh.yml):**
- Button labels: xray.batch_reality, xray.batch_xhttp, xray.batch_kcp, xray.pq_mgmt, xray.del_all, xray.del_count, xray.del_select, xray.filter_all, xray.filter_reality, xray.filter_xhttp, xray.filter_kcp, xray.confirm_clear_btn, xray.pq_delete, xray.pq_init, xray.del_success_file
- Text messages: xray.mgmt_title, xray.no_config_title, xray.del_title, xray.confirm_del_all, xray.del_count_title, xray.del_select_title, xray.del_confirm_msg, xray.del_success_count, xray.del_success_all, xray.reality_batch_title, xray.reality_qty_title, xray.reality_init, xray.reality_init_start, xray.reality_ready, xray.reality_init_fail, xray.batch_gen_progress, xray.batch_gen_done, xray.pq_status_enabled, xray.pq_status_disabled, xray.pq_mgmt_title, xray.pq_deleted
- Answer query: xray.reality_init_start, callback.internal_error
- Back buttons: menu.back, menu.back_user
- KCP keys: kcp_title, kcp_cat_enc, kcp_cat_obf, kcp_cat_dis, kcp_cat_ext, kcp_select_mask, kcp_current_stack, kcp_add_more, kcp_done_btn, kcp_clear_btn, kcp_back_cat, kcp_realm_note, kcp_min_one, kcp_unknown, kcp_unknown_type, kcp_stack_more, kcp_select_cat_stack, kcp_stack_config, kcp_batch_title
- Batch keys: batch_title, batch_security, batch_step_ip, batch_step_qty, batch_config_file, batch_backup_file, kcp_config_file
- Gen keys: gen_progress, gen_kcp_progress, gen_fail, batch_done, kcp_batch_done
- User keys: user_list_title, user_del_confirm, user_del_not_supported, user_cfg_not_found
- Other: del_mgmt_btn, base_missing, master_fail, master_missing, split_v6_up, split_v4_up, dual_v4, dual_v6, preparing_reality, init_reality, type_all, type_reality, type_xhttp, type_kcp, del_label, file_btn, del_nonexist, del_unknown_filter, del_all_result, del_count_result, pq_del_fail, pq_init_success, pq_init_fail, del_msg_fail

---

### Task 1: Add import + fix show_reality_batch_prompt + show_reality_qty_prompt + trigger_reality_auto_init

**Files:**
- Modify: `rust/aegis/src/adapters/telegram/handlers/xray.rs` (lines 1-163)

**Changes:**
1. Add `use rust_i18n::t;` after line 14
2. In `show_reality_batch_prompt`: replace the title format string with `t!()`, replace "⬅️ 返回" with `t!("menu.back")`
3. In `show_reality_qty_prompt`: replace the format string with `t!()`, replace back button
4. In `trigger_reality_auto_init`: replace the success/error message strings

- [ ] Execute all changes for lines 1-163

### Task 2: Fix m_xray_mgmt, m_pq_mgmt, m_pq_del, m_pq_init handlers

**Files:**
- Modify: `rust/aegis/src/adapters/telegram/handlers/xray.rs` (lines 165-293)

**Changes:**
1. `m_xray_mgmt`: Replace button labels and message text with `t!()`
2. `m_pq_mgmt`: Replace status strings and message text with `t!()`
3. `m_pq_del`: Replace answer_callback_query text with `t!()`
4. `m_pq_init`: Replace answer_callback_query text with `t!()`

- [ ] Execute all changes for lines 165-293

### Task 3: Fix m_del_cfg, cfg_filter, cfg_del_all_confirm/exec handlers

**Files:**
- Modify: `rust/aegis/src/adapters/telegram/handlers/xray.rs` (lines 295-440)

**Changes:**
1. `m_del_cfg`: Replace button labels and title with `t!()`
2. `cfg_filter:`: Replace filter label strings and title with `t!()`
3. `cfg_del_all_confirm:`: Replace filter type labels and title with `t!()`
4. `cfg_del_all_exec:`: Replace error text and result text with `t!()`

- [ ] Execute all changes for lines 295-440

### Task 4: Fix cfg_del_count, cfg_del_exec_count, cfg_del_select handlers

**Files:**
- Modify: `rust/aegis/src/adapters/telegram/handlers/xray.rs` (lines 442-588)

**Changes:**
1. `cfg_del_count`: Replace filter labels, button text, and title with `t!()`
2. `cfg_del_exec_count`: Replace result text with `t!()`
3. `cfg_del_select`: Replace filter labels, button text, and title with `t!()`

- [ ] Execute all changes for lines 442-588

### Task 5: Fix cfg_del_file, cfg_del_confirm, a_inst_base handlers

**Files:**
- Modify: `rust/aegis/src/adapters/telegram/handlers/xray.rs` (lines 590-714)

**Changes:**
1. `cfg_del_file`: Replace button text, title with `t!()`
2. `cfg_del_confirm`: Replace callback text with `t!()`
3. `a_inst_base`: Replace answer_query and edit text with `t!()`

- [ ] Execute all changes for lines 590-714

### Task 6: Fix u_batch_init, u_xhttp_batch_init, u_batch/u_xhttp_exec handlers

**Files:**
- Modify: `rust/aegis/src/adapters/telegram/handlers/xray.rs` (lines 716-949)

**Changes:**
1. `u_batch_init`: Replace strings with `t!()`
2. `u_xhttp_batch_init`: Replace strings with `t!()`
3. `u_batch/u_xhttp_exec`: Replace progress, result, error strings with `t!()`

- [ ] Execute all changes for lines 716-949

### Task 7: Fix KCP init, cat, add handlers (u_kcp_init, u_kcp_cat, u_kcp_add)

**Files:**
- Modify: `rust/aegis/src/adapters/telegram/handlers/xray.rs` (lines 951-1096)

**Changes:**
Replace all hardcoded Chinese button labels and text messages with `t!("xray.kcp_*")` keys.

- [ ] Execute all changes for lines 951-1096

### Task 8: Fix KCP more, mcat, push handlers (u_kcp_more, u_kcp_mcat, u_kcp_push)

**Files:**
- Modify: `rust/aegis/src/adapters/telegram/handlers/xray.rs` (lines 1098-1388)

**Changes:**
Replace all hardcoded Chinese strings with `t!("xray.kcp_*")` keys.

- [ ] Execute all changes for lines 1098-1388

### Task 9: Fix KCP done, ip, ok handlers (u_kcp_done, u_kcp_ip, u_kcp_ok)

**Files:**
- Modify: `rust/aegis/src/adapters/telegram/handlers/xray.rs` (lines 1390-1674)

**Changes:**
Replace all hardcoded Chinese strings with `t!("xray.kcp_*")` keys and batch result keys.

- [ ] Execute all changes for lines 1390-1674

### Task 10: Fix user list/delete handlers + final verify

**Files:**
- Modify: `rust/aegis/src/adapters/telegram/handlers/xray.rs` (lines 1676-1781)

**Changes:**
Replace all hardcoded Chinese strings with `t!("xray.user_*")` keys and `t!("menu.back_user")`.

- [ ] Execute all changes for lines 1676-1781

### Task 11: Run cargo check to verify compilation

- [ ] Run `cargo check --manifest-path rust/aegis/Cargo.toml 2>&1 | head -80`
