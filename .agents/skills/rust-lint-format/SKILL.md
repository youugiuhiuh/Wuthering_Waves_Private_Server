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

> If `cargo-nextest` is unavailable, fall back to:
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

## Project-Specific Notes

- Run from the workspace root.
- Prefer `cargo nextest run` over `cargo test` for daily development.
- Always run `cargo test --doc` because `cargo-nextest` does not execute documentation tests.
- For workspaces, execute the commands from the workspace root.
- If feature-gated code exists, ensure the appropriate feature set is tested (typically `--all-features` where applicable).
