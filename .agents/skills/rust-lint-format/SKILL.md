---
name: rust-lint-format
description: Use when completing any Rust code work or before marking Rust tasks as done. Enforces cargo fmt, cargo clippy, cargo nextest, and documentation tests as mandatory quality gates.
---

# Rust Lint & Format Enforcement

Enforces running `cargo fmt`, `cargo clippy -- -D warnings`, `cargo nextest run`, and documentation tests before claiming any Rust work is complete.

## When to Use

- Before marking any Rust task as complete
- Before committing Rust code
- Before creating a PR or merge request for Rust changes
- After editing any `.rs` file or `Cargo.toml`
- When `finishing-a-development-branch` skill runs for Rust projects

## The Rule

**Before claiming any Rust work is done, you MUST run:**

```bash
cargo fmt && \
cargo clippy --all-targets --all-features -- -D warnings && \
cargo nextest run && \
cargo test --doc
```

This command sequence MUST succeed with no errors before you proceed.

> If `cargo-nextest` is unavailable, try to auto-install it:
>
> ```bash
> if ! command -v cargo-nextest &>/dev/null; then
>   cargo install cargo-binstall --locked && \
>   cargo binstall cargo-nextest --secure -y
> fi
> ```
>
> If installation also fails, fall back to:
>
> ```bash
> cargo test
> ```

## Workflow

1. Finish your implementation.
2. Run the quality gate from the project root (where `Cargo.toml` lives).
3. Fix formatting issues.
4. Fix all Clippy warnings and errors.
5. Fix failing tests.
6. Fix failing documentation tests.
7. Re-run until every command succeeds.
8. Only then stage, commit, or declare the task complete.

## Handling Failures

### Formatting Failed

```text
Diff in src/main.rs
```

**Action:** Run `cargo fmt` again and verify the diff is clean.

### Clippy Reports Errors

```text
error: unused import
```

**Action:** Fix the code instead of suppressing the lint unless there is a documented justification.

### Unit or Integration Tests Fail

```text
test result: FAILED
```

**Action:** Fix the failing tests before proceeding.

### Documentation Tests Fail

```text
Doc-tests FAILED
```

**Action:** Update either the documentation examples or the implementation until they pass.

## Enforcement Checklist

- [ ] `cargo fmt`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cargo nextest run`
- [ ] `cargo test --doc`
- [ ] Zero Clippy warnings
- [ ] All tests passing

## Red Flags — STOP

Never skip the quality gate because:

- "It's only a tiny change."
- "CI will catch it."
- "Formatting can wait."
- "The code compiles."
- "I tested it manually."
- "Clippy is too strict."
- "Tests take too long."
- "Doc tests aren't important."

## Quick Reference

| Command | Purpose |
|---------|---------|
| `cargo fmt` | Format code |
| `cargo clippy --all-targets --all-features -- -D warnings` | Strict linting |
| `cargo nextest run` | Run unit & integration tests (recommended) |
| `cargo test --doc` | Run documentation tests |
| `cargo test` | Fallback when `cargo-nextest` is unavailable |

## Common Nextest Commands

| Command | Purpose |
|---------|---------|
| `cargo nextest run` | Run all tests |
| `cargo nextest run -p <package>` | Tests for a specific package only |
| `cargo nextest run <test_name>` | Run tests matching a name/pattern |
| `cargo nextest run --no-fail-fast` (`--nff`) | Run all tests regardless of failures |
| `cargo nextest run --max-fail=N` | Stop after N failures |
| `cargo nextest run --retries=N` | Retry each failing test up to N times |
| `cargo nextest run -j N` (`--test-threads=N`) | Run N tests in parallel |
| `cargo nextest run --no-capture` | Run serially, show stdout/stderr |
| `cargo nextest run --failure-output=immediate` | Print failure output as tests fail |
| `cargo nextest run --all-features` | Test with all Cargo features enabled |
| `cargo nextest run --release` | Build and run in release mode |
| `cargo nextest run --run-ignored=only` | Run only `#[ignore]` tests |

## Project-Specific Notes

- Run from the workspace root.
- Prefer `cargo nextest run` over `cargo test` for daily development.
- Always run `cargo test --doc` because `cargo-nextest` does not execute documentation tests.
- For workspaces, execute the commands from the workspace root.
- If feature-gated code exists, ensure the appropriate feature set is tested (typically `--all-features` where applicable).
