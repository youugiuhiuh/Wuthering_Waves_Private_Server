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

**Fast lane policy:** prefer the Cranelift lane (below) while iterating. If Cranelift is unavailable, fails to build, or fails a test, fall back to the LLVM lane above. When the two disagree, **LLVM is authoritative** — re-run the failing check under LLVM before concluding the code is broken, because Cranelift has its own codegen gaps (see the VAES caveat below). Always run the LLVM lane once before commit/PR; never report a Cranelift result as the gate result.

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
| `RUSTFLAGS="-Zcodegen-backend=cranelift" cargo +nightly nextest run --cargo-profile cranelift-dev` | Preferred fast lane; fall back to LLVM on failure |
| `-C link-arg=-fuse-ld=mold` | Fast linker (Linux); add to `RUSTFLAGS` |
| `cargo test --doc` | Run documentation tests |
| `cargo test` | Fallback when `cargo-nextest` is unavailable |

## Fast Test Profile

Projects may define a `fast-test` Cargo profile tuned for test throughput. Use it whenever it exists, because the default `dev` profile rebuilds the dependency graph with full debuginfo.

```bash
cargo nextest run --cargo-profile fast-test
```

It inherits `dev` with `opt-level = 1`, `debug = 0`, `incremental = true`, `codegen-units = 256`. Keep `opt-level >= 1`. The original reason was `aws-lc-sys` mangling symbols differently at `O0`; `aws-lc-sys` is gone from the graph (see below), and whether `O0` links today has not been re-checked, so the constraint is kept conservatively.

`nextest` 0.9.x only accepts the Cargo profile through the `--cargo-profile` CLI flag. There is no `cargo-profile` key in `.config/nextest.toml`, so there is no config-file or alias shortcut for it.

### Cranelift backend

A `cranelift-dev` profile exists for faster codegen. The backend must be passed through `RUSTFLAGS`, because a profile-level `codegen-backend` in `.cargo/config.toml` makes stable Cargo fail on every profile, not just that one.

```bash
rustup component add rustc-codegen-cranelift --toolchain nightly
# Linux, with mold (see "Fast linkers" below)
RUSTFLAGS="-Zcodegen-backend=cranelift -C link-arg=-fuse-ld=mold" cargo +nightly nextest run --cargo-profile cranelift-dev
# Linux, without a fast linker
RUSTFLAGS="-Zcodegen-backend=cranelift" cargo +nightly nextest run --cargo-profile cranelift-dev
```

**Status: build, test, doc-test and clippy all work.** Re-verified 2026-09-18. The old "`cargo check` only / linking fails" restriction is obsolete: it came from `aws-lc-sys`, which exported `\u{1}`-prefixed symbols that only the LLVM backend understands (<https://github.com/rust-lang/rustc_codegen_cranelift/issues/1520>). `aws-lc-sys` is no longer in the graph — `matrix-sdk` 0.19 goes through `reqwest` 0.13's `rustls-no-provider` and the provider is `ring`.

Evidence for that:

- `Cargo.lock` contains no `aws-lc-sys` / `aws-lc-rs` / `openssl-sys` (626 packages total).
- `nm` over 554 built artifacts under `target/release` found zero `\u{1}` symbols, checked against an `objcopy`-crafted positive control so the detector was not silently blind.
- `cargo +nightly build --profile cranelift-dev --bin aegis` links and exits 0 (1m25s).
- Full suite under cranelift: `929 passed, 1 skipped` — identical to the LLVM `fast-test` verdict. Doc tests and `cargo +nightly clippy` also run clean under the same `RUSTFLAGS`.

Remaining caveats — use Cranelift as a fast pre-check, keep LLVM as the gate that decides:

- Cranelift reports `unsupported x86 llvm intrinsic llvm.x86.aesni.aesenc.256/.512; replacing with trap` for `aes` 0.9.3's `x86_vaes256` / `x86_vaes512` parallel-block backends. Those are reachable only via `BlockCipherEncrypt::encrypt_par_blocks`; aegis's AES-GCM use goes through `encrypt_block`, which delegates to the 128-bit `x86_aes` path (a 64 KiB round-trip passes). New code that starts using par-block AES can turn this into a `SIGILL` under cranelift that LLVM never produces.
- The artifact under test is not the shipped artifact: release is LLVM with `lto = "thin"`, `opt-level = "z"`, `panic = "abort"`.
- C dependencies are irrelevant to backend choice — `ring`'s C/asm objects and the system `libsqlite3` are produced by `cc`/pkg-config and consumed by the linker, which Cranelift does not replace. Do not strip C dependencies to make Cranelift work.

### Fast linkers

Link time is the second half of the loop; swap the default linker for a parallel one. Add the flag to `RUSTFLAGS` alongside the codegen backend:

| OS | Linker | Flag |
|----|--------|------|
| Linux | **mold** | `-C link-arg=-fuse-ld=mold` |
| macOS | **zld**, else lld | `-C link-arg=-fuse-ld=zld` (or `-C linker=/path/to/zld`), else `-C link-arg=-fuse-ld=lld` |
| Windows (MSVC) | **lld** | `-C linker=lld-link` (or `-C linker=rust-lld`) |

```bash
export RUSTFLAGS="-Zcodegen-backend=cranelift -C link-arg=-fuse-ld=mold"
cargo +nightly nextest run --cargo-profile cranelift-dev
```

Measured on this repo (Linux, mold 2.40.4, cranelift-dev, `--bin aegis`): relink 6.21s with `ld.bfd` → **0.36s with mold**, and the resulting binary runs. mold was also verified end-to-end on the LLVM `fast-test` lane (32/32 crypto tests, full rebuild 146s). Full cold build barely moves (1m25s → 1m20s) because codegen dominates — the linker is worth it for the edit/test/relink loop, not for a clean CI build. The macOS/Windows rows are conventional recipes, **not verified on this machine** (no zld/lld installed here).

If the flag is rejected, that means the C compiler driving the link is too old to understand `-fuse-ld`; use `-C linker=<path-to-mold>` instead. Changing the linker invalidates the whole build graph once, so expect one full rebuild after adding it. `.cargo/config.toml` can hold these as `[target.<triple>] rustflags` if you want them permanently — but keep the **codegen backend out of that file** (see above), and do not set both config `rustflags` and a `RUSTFLAGS` env var unless you intend them to combine.

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
