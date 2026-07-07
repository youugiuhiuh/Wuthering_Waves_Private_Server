---
name: rust-lint-format
description: Use when completing any Rust code work or before marking Rust tasks as done. Enforces cargo fmt, cargo clippy, and cargo test as mandatory quality gates.
---

# Rust Lint & Format Enforcement

Enforces running `cargo fmt`, `cargo clippy -- -D warnings`, and `cargo test` before claiming any Rust work is complete.

## When to Use

- Before marking any Rust task as complete
- Before committing Rust code
- Before creating a PR or merge request for Rust changes
- After editing any `.rs` file or `Cargo.toml`
- When `finishing-a-development-branch` skill runs for Rust projects

## The Rule

**Before claiming any Rust work is done, you MUST run:**

```bash
cargo fmt && cargo clippy -- -D warnings && cargo test
```

This command MUST succeed with no errors before you proceed.

## Workflow

1. **Finish your implementation** - all code changes done
2. **Run the enforcement command** - from the Rust project root (where Cargo.toml lives)
3. **Handle clippy errors** - if `--fix` couldn't auto-fix, manually fix them
4. **Re-run until clean** - repeat until all commands pass
5. **Stage and commit** - only then proceed to commit/PR

## Handling Failures

### Clippy Reports Unfixable Errors

```
error: unused imports: `use std::io`
```

**Action:** Remove the unused import, then re-run the enforcement command.

### Clippy Suggests Restructuring

```
warning: unnecessary struct wrapping
```

**Action:** Apply the suggested refactoring, then re-run.

### Tests Fail

```
test result: FAILED. 0 passed, 1 failed
```

**Action:** Fix the failing test before proceeding. Do not skip tests.

## Enforcement Checklist

- [ ] `cargo fmt` passes (no output = success)
- [ ] `cargo clippy -- -D warnings` passes (exit code 0)
- [ ] `cargo test` passes (all tests green)
- [ ] No warnings treated as errors

## Red Flags - STOP and Start Over

These mean you MUST run the enforcement command:

- "Let me commit first, lint can be done later"
- "Clippy warnings are not errors, they won't block compilation"
- "The code works, formatting is optional"
- "I'll run clippy in the CI, no need locally"
- "A small change, no need for full lint"
- "I tested it manually, no need for clippy"
- "Let me just check if it compiles first"
- "Tests are optional for this change"

**Violating the letter of this rule is violating the spirit of this rule.**

## Quick Reference

| Command | Purpose |
|---------|---------|
| `cargo fmt` | Format code with rustfmt |
| `cargo clippy -- -D warnings` | Lint with deny-all-warnings |
| `cargo test` | Run all tests |
| `cargo fmt && cargo clippy -- -D warnings && cargo test` | **MUST run before claiming done** |

## Rationalization Countermeasures

| Excuse | Reality |
|-------|---------|
| "Small change, lint is overkill" | Small changes still violate lint rules; one warning is too many |
| "I'll do it before the final commit" | The final commit is now. The enforcement is non-negotiable |
| "CI will catch issues" | CI is a safety net, not an excuse to skip local checks |
| "Clippy is too strict" | Clippy catches real bugs; "strict" is a feature |
| "Code compiles, that's enough" | Compiling != correct; clippy catches logic errors |
| "I already manually tested" | Manual testing doesn't replace static analysis |
| "Tests take too long" | Skipping tests means shipping broken code |

## Project-Specific Notes

- Run from the Rust project root: `rust/tgbot/` or `rust/version-sync/`
- If working in a Rust workspace, run from the workspace root
- For multi-crate projects, run in each affected crate
- Use `cargo test --all-features` if your project has feature-gated code
