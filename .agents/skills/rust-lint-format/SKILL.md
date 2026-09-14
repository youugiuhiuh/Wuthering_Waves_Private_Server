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
cargo nextest run --cargo-profile fast-test && \
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
- [ ] `cargo nextest run --cargo-profile fast-test`
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
| `cargo nextest run --cargo-profile fast-test` | Run unit & integration tests (recommended) |
| `cargo test --doc` | Run documentation tests |
| `cargo test` | Fallback when `cargo-nextest` is unavailable |

## Fast Test Profile

Projects may define a `fast-test` Cargo profile tuned for test throughput. Use it whenever it exists, because the default `dev` profile rebuilds the dependency graph with full debuginfo.

```bash
cargo nextest run --cargo-profile fast-test
```

It inherits `dev` with `opt-level = 1`, `debug = 0`, `incremental = true`, `codegen-units = 256`. `opt-level` must stay `>= 1`: `aws-lc-sys` (rustls' C dependency) mangles symbols differently at `O0` and linking fails.

`nextest` 0.9.x only accepts the Cargo profile through the `--cargo-profile` CLI flag. There is no `cargo-profile` key in `.config/nextest.toml`, so there is no config-file or alias shortcut for it.

### Cranelift backend

A `cranelift-dev` profile exists for faster codegen. The backend must be passed through `RUSTFLAGS`, because a profile-level `codegen-backend` in `.cargo/config.toml` makes stable Cargo fail on every profile, not just that one.

```bash
rustup component add rustc-codegen-cranelift --toolchain nightly
RUSTFLAGS="-Zcodegen-backend=cranelift" cargo +nightly check --profile cranelift-dev
```

Do not use it for builds or tests: linking fails with `undefined symbol: aws_lc_0_*_EVP_PKEY_*`. `aws-lc-sys` exports symbols with a `\u{1}` prefix that only the LLVM backend understands, and it cannot be removed from the graph because `matrix-sdk` and `serenity` force `reqwest/__rustls-aws-lc-rs` on. Cargo cannot subtract features. Until these close, Cranelift is `cargo check` only:

- <https://github.com/rust-lang/rustc_codegen_cranelift/issues/1520>
- <https://github.com/rust-lang/rust-bindgen/issues/2935>

## Common Nextest Commands

| Command | Purpose |
|---------|---------|
| `cargo nextest run --cargo-profile fast-test` | Run all tests (speed-first profile) |
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
- Prefer `cargo nextest run --cargo-profile fast-test` over `cargo test` for daily development.
- Always run `cargo test --doc` because `cargo-nextest` does not execute documentation tests.
- For workspaces, execute the commands from the workspace root.
- If feature-gated code exists, ensure the appropriate feature set is tested (typically `--all-features` where applicable).
