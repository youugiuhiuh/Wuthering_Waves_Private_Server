# ACME Final Fix Report

Date: 2026-07-29
Branch: `fix/acme-safe-diagnostics`
Worktree: `/home/fe/dark/Wuthering_Waves_Private_Server/.worktrees/acme-safe-diagnostics`

## Status

All supplied whole-branch review blockers are fixed. No push, deployment, amend, or rebase was performed.

Implementation commit:

- `646dc9220ef33badf905a2f643eec9cd9106041f` (`fix(aegis): close ACME diagnostics blockers`)

## Changed Files

- `rust/aegis/src/core/security/acme.rs`
  - Identifies production ACME commands only when `program == acme::BIN`.
  - Adds a narrow private `run_command_inner` ACME-identification seam for real process tests.
  - Returns the successful ACME status with empty stdout and stderr.
  - Preserves successful non-ACME stdout and stderr unchanged.
  - Converts nonzero ACME-like process output into typed errors without returning captured bytes.
  - Canonicalizes RFC3986 and form representations for exactly two decode passes.
  - Covers raw, fully encoded, form, mixed one-level, mixed two-level, and malformed percent input.
- `rust/aegis/src/shared/handlers/message.rs`
  - Adds exact localized content contracts for Cloudflare permissions and CA/network guidance.
  - Removes the unused error parameter and call-site binding from `localized_acme_install_failure`.
- `rust/aegis/src/resources/i18n/zh.yml`
  - Requires Cloudflare `Zone > DNS > Edit` and `Zone > Zone > Read`.
  - Covers CA order status/rate limiting, waiting/retry, and connectivity.
- `rust/aegis/src/resources/i18n/en.yml`
  - Requires Cloudflare `Zone > DNS > Edit` and `Zone > Zone > Read`.
  - Covers CA order status/rate limiting, waiting/retry, and connectivity.
- `rust/aegis/src/resources/i18n/ja.yml`
  - Requires Cloudflare `Zone > DNS > Edit` and `Zone > Zone > Read`.
  - Covers CA order status/rate limiting, waiting/retry, and connectivity.
- `rust/aegis/src/shared/dispatch.rs`
  - Strengthens the real callback dispatch test to assert the provider state transition and exact sent guidance.
- `rust/aegis/src/shared/handlers/xray.rs`
  - Removes the parser-only callback test superseded by the dispatch-boundary test.
- `.superpowers/sdd/final-fix-report.md`
  - Records implementation, TDD evidence, verification, and security review.

`rust/aegis/Cargo.lock` was restored after Cargo updated the root package version during local commands. Final lockfile diff: none.

## Baseline

Command:

```bash
cd rust/aegis && cargo test
```

Result: PASS before edits. Library: 542 passed, 0 failed, 1 ignored. Binary: 52 passed. All integration and doc tests passed.

## RED Evidence

### Mixed and Two-Level Credential Encoding

Command:

```bash
cargo test core::security::acme::tests::credential_redaction_handles -- --nocapture
```

Result: FAIL as expected. Three existing tests passed; both new tests failed:

- `credential_redaction_handles_mixed_percent_encoding`: got `Authentication`, expected `Unknown`.
- `credential_redaction_handles_two_encoding_levels`: got `Authentication`, expected `Unknown`.

Malformed escape companion command:

```bash
cargo test core::security::acme::tests::malformed_percent_escapes_remain_classifiable -- --nocapture
```

Result: PASS. This established that malformed escapes were preserved and did not panic while the mixed-encoding regressions remained RED.

### Process-Boundary ACME Identification

Command:

```bash
cargo test core::security::acme::tests::acme_like_success_discards_captured_output_at_process_boundary -- --nocapture
```

Result: compile FAIL as expected with `E0061`: `run_command_inner` accepted five arguments, while the wished-for test seam supplied the sixth ACME-identification boolean. The same expected error covered successful ACME-like, nonzero ACME-like, and non-ACME process tests.

### Localized Content Contracts

Command:

```bash
cargo test shared::handlers::message::tests::domain_translation_keys_exist -- --nocapture
```

Result: FAIL as expected at the first unmet assertion: `domain.acme_network_error missing 订单状态`. The new exact Cloudflare `Zone > Zone > Read` assertion was also unmet before the locale updates.

### Callback Coverage

No production behavior was missing. The existing dispatch integration test already reached `handle_domain_provider`; therefore this minor review item was handled as a coverage refactor rather than manufacturing a failing behavior. The parser-only test was replaced by assertions on the actual state transition and `send_message` result.

## GREEN Evidence

### Redaction

Command:

```bash
cargo test core::security::acme::tests::credential_redaction_handles -- --nocapture
```

Result: PASS, 5 passed, 0 failed.

Command:

```bash
cargo test core::security::acme::tests -- --nocapture
```

Result: PASS, 41 passed, 0 failed. This includes raw/form/mixed/fully encoded/malformed coverage, ACME/non-ACME process boundaries, credential isolation, timeout cleanup, certificate checks, and process lifecycle tests.

### Process Boundary

Command:

```bash
cargo test process_boundary -- --nocapture
```

Result: PASS, 2 passed, 0 failed. Successful ACME-like output was empty and nonzero ACME-like output became `AcmeCommandError` with `ACME-DNS` only.

Command:

```bash
cargo test core::security::acme::tests::non_acme_success_preserves_captured_output -- --nocapture
```

Result: PASS, 1 passed, 0 failed. Ordinary stdout and stderr were preserved byte-for-byte.

### Messages and Providers

Command:

```bash
cargo test shared::handlers::message::tests -- --nocapture
```

Result: PASS, 9 passed, 0 failed.

Command:

```bash
cargo test domain_provider -- --nocapture
```

Result: PASS, 2 passed, 0 failed. Both the real provider dispatch/send path and stale callback rejection passed.

### Callback Send Path

Command:

```bash
cargo test shared::dispatch::tests::xhttp_domain_provider_routes_to_xray_handler -- --nocapture
```

Result: PASS, 1 passed, 0 failed. The test asserts `AwaitCredentials(Cloudflare)` and the exact localized guidance sent by the adapter.

## Full Verification

Exact mandatory command:

```bash
cargo fmt && cargo clippy -- -D warnings && cargo test
```

Result: PASS, exit status 0.

- `cargo fmt`: PASS.
- `cargo clippy -- -D warnings`: PASS with no Clippy warnings.
- Library tests: 548 passed, 0 failed, 1 ignored (`requires /etc/ssh/sshd_config`).
- Binary tests: 52 passed, 0 failed.
- Integration tests: all passed.
- Doc tests: passed.
- Cargo emitted only the existing future-incompatibility notice for `proc-macro-error2 v2.0.1`.
- `git diff --check`: PASS.
- Final `Cargo.lock` diff: none.

## Security Self-Review

- Raw ACME stdout/stderr is consumed only for in-process classification and is never logged, formatted into errors, returned on failure, or returned on success.
- Successful ACME `Output` retains only the original success status; both byte vectors are replaced with empty vectors.
- Non-ACME success and failure behavior remains unchanged, verified at the real process boundary.
- Production ACME identification remains the exact `acme::BIN` path comparison. Tests inject only a private boolean into `run_command_inner`; no path spoofing, global mutation, or execution of `/root/.acme.sh/acme.sh` occurs.
- Canonicalization is bounded by `CREDENTIAL_ENCODING_DEPTH = 2`; there is no recursive or combinatorial representation generation.
- Raw and canonical credential views are redacted from both terminal RFC3986 and form views before signature matching.
- Malformed percent escapes remain unchanged and do not panic.
- Process spawn, stop/continue, wait, reader join, timeout cleanup, cleanup disarm, and descendant cleanup ordering is unchanged.
- No dependencies were added.
- Localized install failure still emits only `ACME-UNKNOWN`; the removed parameter eliminates accidental future use of raw error detail.

## Concerns

- Canonical decoding is intentionally capped at two levels. Inputs encoded three or more times are not supported by design; increase the explicit bound only with a concrete provider case and matching tests.
- The existing Cargo future-incompatibility notice for `proc-macro-error2 v2.0.1` is unrelated to this change and does not fail Clippy or tests.
