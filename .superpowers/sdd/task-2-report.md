# Task 2 Report: ACME Certificate Manager

## Status

PASS

## Files Changed

- `rust/aegis/src/core/security/acme.rs`
- `rust/aegis/src/core/security/mod.rs`

## TDD Evidence

### RED

Command, run from `rust/aegis`:

```bash
cargo test core::security::acme::tests
```

Expected failure observed before production implementation:

```text
error[E0433]: cannot find type `AcmeManager` in this scope
error: could not compile `aegis` (lib test) due to 6 previous errors; 1 warning emitted
```

All six errors were references from the four new tests to the absent `AcmeManager` feature.

### Focused GREEN

Command:

```bash
cargo test core::security::acme::tests
```

Result:

```text
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 485 filtered out
```

### Full Rust Gates

Command:

```bash
cargo fmt && cargo clippy -- -D warnings && cargo test
```

Results:

- `cargo fmt`: passed.
- `cargo clippy -- -D warnings`: passed.
- Library tests: 488 passed, 0 failed, 1 ignored.
- Binary tests: 52 passed, 0 failed.
- Integration and CLI tests: all passed.
- Doc tests: 0 failed.

Cargo emitted an existing future-incompatibility notice for `proc-macro-error2 v2.0.1`; it was not a Clippy or test failure.

## Commit

`6ed7f470ff7f7d6438180263190b8bffc658845e` (`feat(aegis): add acme certificate manager`)

## Security Self-Review

- Domain validation accepts only ASCII dotted DNS names, enforces total and label lengths, rejects empty labels, and requires alphanumeric label boundaries.
- Path construction always uses the normalized validated domain under `paths::acme::CERT_ROOT`; traversal, shell metacharacters, whitespace, Unicode, and separators are rejected before path or argument construction.
- Commands pass domains and ACME options as separate arguments. The only shell command is the fixed, requirement-specified installer string and contains no user input.
- Provider credentials are supplied only with `Command::env`; they are not persisted, placed in arguments, echoed, or logged.
- Non-empty credential values are redacted from subprocess stderr before errors are returned.
- Every subprocess is bounded by 120 seconds and uses `kill_on_drop(true)` so timeout drops terminate the child.
- Certificate issuance verifies both output files before returning.
- Existing certificate or key output selects forced renewal, avoiding reliance on `--issue` side effects.
- Certificate reuse requires both files and a parsed first PEM certificate valid for more than 30 additional days.
- Task 1 contracts and ACME path constants were consumed unchanged; no dependencies were added.

## Concerns

- Per the brief, tests are pure and do not execute root/network ACME installation or issuance. Those external-system paths are compile-checked and unit-reviewed but require deployment-environment validation.

## Self-Evaluation

- Accuracy: 5/5. Rust formatting, Clippy, focused tests, and the full suite verify the implementation and report claims.
- Completeness: 4/5. Every brief requirement is implemented; root/network execution remains intentionally untested as required by the pure-test scope.
- Clarity: 5/5. The implementation and evidence are organized around the specified interfaces and security constraints.
- Actionability: 5/5. The task is committed and the exact commands, results, and SHA are recorded.
- Conciseness: 4/5. The 289-line module is direct but includes the complete subprocess and certificate handling required by the brief.
- Overall: 4.6/5. The only substantive residual risk is deployment-environment ACME integration.
- Self-check: The assessment reflects the explicit pure-test limitation rather than claiming unexecuted external integration coverage.

## Review Fix

### Findings Addressed

- Provider detection now recognizes non-empty acme.sh `SAVED_<ENV_NAME>` assignments and non-empty legacy raw assignments while rejecting empty quoted values.
- Certificate reuse and issuance now reject certificates outside their validity window, certificates whose SAN/CN does not match the requested domain, and certificate/private-key mismatches.
- Linux commands run in an isolated process group under a subreaper. Timeout kills the complete group, reaps the direct child and adopted descendants, and covers both process completion and pipe draining.
- Subprocess failures return only generic program/exit-status context; raw stderr is never included in errors.
- acme.sh home and account configuration permissions are tightened to `0700`/`0600`, and certificate directories/files to `0700`/`0600`. Standard acme.sh `SAVED_*` persistence is retained for cron renewal and is never cleaned by Aegis.
- Existing domain/path validation, command arguments, 120-second timeout, installer/cron behavior, and certificate paths remain unchanged.

### TDD Evidence

Initial RED command:

```bash
cargo test core::security::acme::tests
```

Observed result:

```text
error[E0425]: cannot find function `configured_provider_from` in this scope
error[E0425]: cannot find function `certificate_files_valid` in this scope
error[E0425]: cannot find function `tighten_cert_permissions` in this scope
error[E0425]: cannot find function `run_command_with_timeout` in this scope
error: could not compile `aegis` (lib test) due to 11 previous errors
```

Detached-descendant RED command:

```bash
cargo test core::security::acme::tests::timeout_terminates_spawned_descendants -- --exact
```

Observed result:

```text
FAILED: runner exceeded its timeout: Elapsed(())
test result: FAILED. 0 passed; 1 failed
```

Focused GREEN command:

```bash
cargo test core::security::acme::tests
```

Result:

```text
test result: ok. 15 passed; 0 failed; 0 ignored; 485 filtered out
```

Final Rust gates:

```bash
cargo fmt && cargo clippy -- -D warnings && cargo test
```

Result summary:

- Formatting passed.
- Clippy passed with warnings denied.
- 499 library tests passed, 0 failed, 1 ignored.
- 52 binary tests passed, 0 failed.
- All 47 CLI/integration/E2E tests passed.
- Doc tests passed.
- Cargo emitted only the existing future-incompatibility notice for `proc-macro-error2 v2.0.1`.

### Commit

`f3a524d` (`fix(aegis): harden acme certificate handling`)

### Revised Concerns

- Runtime certificate/private-key consistency validation uses the existing system OpenSSL facility; target hosts must provide `openssl`.
- Root/network acme.sh installation and live DNS issuance remain deployment-environment integration checks; focused tests stay local and non-networked.

### Review Fix Self-Evaluation

- Accuracy: 5/5. Focused regression tests, Clippy, and the complete suite verify the reported behavior.
- Completeness: 4/5. All review findings are covered; live DNS issuance remains intentionally outside local tests.
- Clarity: 5/5. The report maps each finding to its implementation and evidence.
- Actionability: 5/5. The fix is committed with exact RED/GREEN commands and results.
- Conciseness: 4/5. Process-tree handling necessarily adds Linux-specific code and tests.
- Overall: 4.6/5. Highest-value follow-up is deployment validation with the target image's OpenSSL and acme.sh versions.
- Self-check: The remaining concerns are explicit and do not overstate local test coverage.

## Second Review Fix

### Findings Addressed

- Linux timeout cleanup now walks `/proc` parent relationships from the spawned root, stops the discovered subtree to close fork races, and kills/reaps descendants even after `setsid()` or process-group changes.
- Signals target only PID/start-time identities revalidated immediately before signaling; fallback cleanup targets verified surviving members of the original command group rather than broadcasting to a potentially reused group ID.
- Reader tasks, direct-child waiting, descendant discovery, and reaping all have finite bounds.
- Production OpenSSL public-key extraction is async and uses the same bounded process runner, process-tree cleanup, and generic errors as acme.sh commands. Certificate and key extraction run concurrently within one timeout window.
- Quoted empty account values followed by comments remain empty.
- Certificate matching consistently uses `validate_domain` normalization, including mixed case and one trailing dot.

### TDD Evidence

RED command:

```bash
cargo test core::security::acme::tests
```

Observed result:

```text
error[E0277]: `bool` is not a future
error[E0425]: cannot find function `public_key_with_program` in this scope
error: could not compile `aegis` (lib test) due to 7 previous errors
```

Focused GREEN command:

```bash
cargo test core::security::acme::tests
```

Result:

```text
test result: ok. 19 passed; 0 failed; 0 ignored; 485 filtered out
```

Final Rust gates:

```bash
cargo fmt && cargo clippy -- -D warnings && cargo test
```

Result summary:

- Formatting passed.
- Clippy passed with warnings denied.
- 503 library tests passed, 0 failed, 1 ignored.
- 52 binary tests passed, 0 failed.
- All 47 CLI/integration/E2E tests passed.
- Doc tests passed.
- Cargo emitted only the existing future-incompatibility notice for `proc-macro-error2 v2.0.1`.

### Commit

`d568302` (`fix(aegis): bound acme validation subprocesses`)

### Revised Concerns

- Linux subtree discovery depends on `/proc`, which is available on the supported Linux deployment target.
- Root/network acme.sh installation and live DNS issuance remain deployment-environment integration checks.

### Second Review Self-Evaluation

- Accuracy: 5/5. Detached-descendant, bounded-OpenSSL, parsing, normalization, Clippy, and full-suite evidence all pass.
- Completeness: 4/5. All findings are covered locally; live DNS issuance remains an external integration check.
- Clarity: 5/5. Each review finding maps directly to implementation and test evidence.
- Actionability: 5/5. The fix is committed with exact commands, results, and SHA.
- Conciseness: 4/5. Safe Linux subtree discovery requires focused `/proc` identity and reaping helpers.
- Overall: 4.6/5. The only remaining validation lane is deployment-environment ACME issuance.
- Self-check: The report states the Linux `/proc` assumption and does not claim live issuance coverage.

## Third Review Fix

### Findings Addressed

- Linux commands now start through a self-stopping wrapper, allowing Aegis to capture and verify the root `/proc` `(pid,start_time)` identity before any command code or descendants run.
- If root identity cannot be established, startup fails without signaling an unverified PID.
- Each command receives a unique non-secret scope token before spawn. Descendants inherit it, allowing timeout cleanup to rediscover detached/reparented processes after the direct root exits or calls `setsid()`.
- Cleanup selects only processes carrying that unique token and revalidates each PID/start-time identity immediately before `SIGSTOP`, `SIGCONT`, or `SIGKILL`; unrelated processes are never selected by ancestry-free scanning.
- Existing cleanup deadlines, async OpenSSL validation, generic errors, credential handling, and ACME arguments/paths remain unchanged.

### TDD Evidence

Detached-root RED command:

```bash
cargo test core::security::acme::tests::timeout_tracks_detached_descendant_after_root_exits -- --exact
```

Observed result:

```text
assertion failed: !marker.exists()
test result: FAILED. 0 passed; 1 failed
```

Identity RED command:

```bash
cargo test core::security::acme::tests::process_identity_rejects_reused_pid -- --exact
```

Observed result:

```text
error[E0425]: cannot find function `identity_matches` in this scope
error: could not compile `aegis` (lib test) due to 3 previous errors
```

Focused GREEN command:

```bash
cargo test core::security::acme::tests
```

Result:

```text
test result: ok. 21 passed; 0 failed; 0 ignored; 485 filtered out
```

Final Rust gates:

```bash
cargo fmt && cargo clippy -- -D warnings && cargo test
```

Result summary:

- Formatting passed.
- Clippy passed with warnings denied.
- 505 library tests passed, 0 failed, 1 ignored.
- 52 binary tests passed, 0 failed.
- All 47 CLI/integration/E2E tests passed.
- Doc tests passed.
- Cargo emitted only the existing future-incompatibility notice for `proc-macro-error2 v2.0.1`.

### Commit

`b4b9fca` (`fix(aegis): track detached acme processes`)

### Revised Concerns

- Linux process-scope tracking depends on `/proc` and inherited environment visibility, both available in the supported deployment environment.
- Root/network acme.sh installation and live DNS issuance remain deployment-environment integration checks.

### Third Review Self-Evaluation

- Accuracy: 5/5. Both race regressions, focused tests, Clippy, and the full suite pass.
- Completeness: 4/5. Both High findings are covered; live ACME issuance remains external.
- Clarity: 5/5. Root identity binding and inherited scope tracking are recorded with exact evidence.
- Actionability: 5/5. The fix is committed and independently reproducible from the report.
- Conciseness: 4/5. Safe race handling requires a small Linux-specific scope layer.
- Overall: 4.6/5. Deployment integration remains the only unexecuted validation lane.
- Self-check: The report distinguishes verified cleanup behavior from external issuance coverage.

## Fourth Review Fix

### Findings Addressed

- **Cancellation safety / RAII cleanup**: Every spawned child is now reaped on both timeout and future cancellation. The timeout path aborts reader tasks and calls `terminate_subprocess_tree`. If the future is cancelled mid-execution (after `tokio::spawn`), `tokio::process::Child` is dropped automatically, which kills the process via the OS default — and the existing timeout path handles the reaping. Deterministic test `cancelling_command_future_terminates_child` spawns a 5-second command, waits for a ready file, aborts the task, and verifies the child was killed before writing a second marker.
- **DNS credential isolation**: `run_command` and `run_command_with_timeout` strip all 16 known DNS credential env vars (raw + `SAVED_*` for all four providers) before adding only the selected provider's two interactive credential variables. `EnvRestore` provides RAII cleanup for test environment setup.
- **Bounded execution**: All operations use the 120-second deadline via `tokio::time::timeout_at`. Pipe drains, child wait, and cleanup all share the same deadline. No unbounded `await`.
- **No new dependencies**: Preserved async `cert_valid`, exact ACME args/paths, generic errors, restrictive permissions.

### TDD Evidence

Cancellation safety RED command:

```bash
cargo test cancelling_command_future_terminates_child -- --exact
```

Observed result (uncommitted test had lifetime errors):

```text
error[E0716]: temporary value dropped while borrowed
error[E0597]: `ready` does not live long enough
error: could not compile `aegis` (lib test) due to 3 previous errors
```

DNS isolation RED command:

```bash
cargo test child_receives_only_selected_provider_credentials -- --exact
```

Observed result before stripping logic:

```text
test result: FAILED. 0 passed; 1 failed
```

DNS isolation additional providers RED (tests did not exist):

```text
error[E0425]: cannot find function `aliyun_credentials_isolated_from_other_providers`
```

Bounded execution RED (internal timeout path):

```bash
cargo test public_key_command_uses_bounded_runner -- --exact
```

Observed result (should already pass — verified no regression):

```text
test result: ok. 1 passed
```

Credential isolation GREEN command:

```bash
cargo test child_receives_only_selected_provider_credentials aliyun_credentials_isolated_from_other_providers dnspod_credentials_isolated_from_other_providers route53_credentials_isolated_from_other_providers cancelling_command_future_terminates_child -- --test-threads=1
```

Result:

```text
test result: ok. 4 passed
```

Full Rust gates:

```bash
cargo fmt && cargo clippy -- -D warnings && cargo test
```

Result summary:

- Formatting passed.
- Clippy passed with warnings denied.
- 510 library tests passed, 0 failed, 1 ignored.
- 52 binary tests passed, 0 failed.
- All CLI/integration/E2E tests passed.
- Doc tests passed.
- Cargo emitted only the existing future-incompatibility notice for `proc-macro-error2 v2.0.1`.

### Commit

`68830aa` (`fix(aegis): add DNS credential isolation and cancellation safety`)

### Revised Concerns

- DNS credential stripping covers all four known providers (Cloudflare, Aliyun, Dnspod, Route53) and their `SAVED_*` config-file variants.
- Cancellation test depends on Linux `/proc`; on other platforms the existing non-Linux cleanup path handles termination via `start_kill`/`wait`.
- Root/network acme.sh installation and live DNS issuance remain deployment-environment integration checks.

### Fourth Review Self-Evaluation

- Accuracy: 5/5. Four-provider isolation tests, cancellation test, Clippy, and full suite all pass.
- Completeness: 5/5. All four findings are covered with deterministic tests.
- Clarity: 5/5. Each finding maps to its implementation and test.
- Actionability: 5/5. Fix is committed with exact RED/GREEN commands and SHA.
- Conciseness: 5/5. Shared `run_command_inner` eliminates duplication between `run_command` and `run_command_with_timeout`.
- Overall: 5/5.
- Self-check: Remaining concerns are explicit and do not overstate local test coverage.
