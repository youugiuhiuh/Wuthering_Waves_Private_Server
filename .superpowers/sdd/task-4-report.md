# Task 4 Report: dispatch.rs — File capture + hash + persist

## Status: ✅ Complete

## Commits
- `ba292681` — `feat(aegis): security-file capture in pending state — download, hash, persist`

## Summary

Modified `src/shared/dispatch.rs`:

1. **Imports added**: `sha2::Digest`, `MessageContent`, `TimeoutStatus`
2. **Logic added** in `handle_message` (after TOTP auth check):
   - Calls `state.take_security_file_input_status()` with 180s timeout
   - On `TimeoutStatus::Active` and `msg.file_id.is_some()`:
     - Downloads file via `msg.adapter.download_file()`
     - Checks size ≤ 10 MB
     - SHA-256 hashes content via `sha2::Sha256::digest`
     - Stores hash via `state.set_self_destruct_key_hash()`
     - Persists to encrypted config via `crate::bootstrap::save_self_destruct_key_hash_to_config()`
     - Sends success confirmation with truncated hash prefix
3. **Test added**: `dispatch_security_file_tests::file_captured_when_pending_sets_hash`

## Test results
- `cargo test dispatch_security_file_tests` → **1 passed** (TDD: RED → GREEN)
- `cargo test` → **450+46+1+2+1+1+1+6+1+3+21+10 = 543 passed**, 0 failed, 1 ignored, 0 filtered
- `cargo clippy -- -D warnings` → clean (no warnings)
- `cargo fmt` → clean

## Concerns
- `save_self_destruct_key_hash_to_config` is a synchronous function that reads/writes the encrypted config file — it will fail silently (logged only) if the config file is missing or corrupted, which is acceptable behavior since it's non-critical for the current session
- The file timeout (180s) is hardcoded; could be made configurable in the future
