---
name: rust-lint-format
description: Use when completing any Rust code work or before marking Rust tasks as done. Enforces cargo clippy and cargo fmt checks as mandatory quality gates.
---

# Rust Lint & Format Enforcement

Enforces running `cargo clippy --fix --allow-dirty && cargo fmt` before claiming any Rust work is complete.

## When to Use

- Before marking any Rust task as complete
- Before committing Rust code
- Before creating a PR or merge request for Rust changes
- After editing any `.rs` file or `Cargo.toml`
- When `finishing-a-development-branch` skill runs for Rust projects

## The Rule

**Before claiming any Rust work is done, you MUST run:**

```bash
cargo clippy --fix --allow-dirty && cargo fmt
```

This command MUST succeed with no errors before you proceed.

## Workflow

1. **Finish your implementation** - all code changes done
2. **Run the enforcement command** - from the Rust project root (where Cargo.toml lives)
3. **Handle clippy errors** - if `--fix` couldn't auto-fix, manually fix them
4. **Re-run until clean** - repeat until both commands pass
5. **Stage and commit** - only then proceed to commit/PR

## Handling Failures

### Clippy Reports Unfixable Errors

```
error: unused imports: `use std::io`
```

**Action:** Manually fix the error in the source code, then re-run the enforcement command.

### Clippy Suggests Restructuring

```
warning: unnecessary struct wrapping
```

**Action:** Apply the suggested refactoring, then re-run.

### Cargo Fmt Changes Files

This is expected. Review the changes, stage them, and verify with a second run:

```bash
cargo clippy --fix --allow-dirty && cargo fmt
```

Expected output: `Checking, formatting, and compiling... OK`

## Red Flags - STOP and Start Over

These mean you MUST run the enforcement command:

- "Let me commit first, lint can be done later"
- "Clippy warnings are not errors, they won't block compilation"
- "The code works, formatting is optional"
- "I'll run clippy in the CI, no need locally"
- "This is a small change, no need for full lint"
- "I tested it manually, no need for clippy"
- "Let me just check if it compiles first"

**Violating the letter of this rule is violating the spirit of this rule.**

## Quick Reference

| Command | Purpose |
|---------|---------|
| `cargo clippy --fix --allow-dirty` | Auto-fix clippy warnings, allow dirty working tree |
| `cargo fmt` | Format code with rustfmt |
| `cargo clippy --fix --allow-dirty && cargo fmt` | **MUST run before claiming done** |

## Rationalization Countermeasures

| Excuse | Reality |
|-------|---------|
| "Small change, lint is overkill" | Small changes still violate lint rules; one warning is too many |
| "I'll do it before the final commit" | The final commit is now. The enforcement is non-negotiable |
| "CI will catch issues" | CI is a safety net, not an excuse to skip local checks |
| "Clippy is too strict" | Clippy catches real bugs; "strict" is a feature |
| "Code compiles, that's enough" | Compiling != correct; clippy catches logic errors |
| "I already manually tested" | Manual testing doesn't replace static analysis |

## Project-Specific Notes

- This project uses this rule in `AGENTS.md` step 7
- Run from the Rust project root: `rust/tgbot/` or `rust/version-sync/`
- If working in a Rust workspace, run from the workspace root
- For multi-crate projects, run in each affected crate