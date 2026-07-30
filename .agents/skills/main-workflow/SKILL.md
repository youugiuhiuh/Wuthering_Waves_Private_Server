---
name: main-workflow
description: AI-native SDLC orchestration layer. Automatically routes tasks to strict/normal/rapid modes and coordinates Superpowers skills.
---

# Main Workflow Orchestration

Deterministic AI-native SDLC orchestration layer that coordinates Superpowers skills automatically.

## Mode Selection

Automatically determine workflow mode from task characteristics:

| Mode | Trigger Conditions |
|------|-------------------|
| **strict** | new features, refactors, architecture changes, database/schema changes, security logic, changes affecting >3 files |
| **normal** | standard bugfixes, medium-complexity tasks, small feature additions |
| **rapid** | docs, comments, typo fixes, formatting, trivial single-file edits |

### Mode Selection Decision Tree

```
Is this a documentation/comment/typo fix?
  │
  └─ Yes → RAPID mode

Is this a trivial single-file edit (not a feature/bugfix)?
  │
  └─ Yes → RAPID mode

Does this involve architecture changes, database/schema, security, or >3 files?
  │
  └─ Yes → STRICT mode

Is this a standard bugfix or medium-complexity task?
  │
  └─ Yes → NORMAL mode

Default to NORMAL if uncertain.
```

---

## Strict Mode Workflow

**Never skip stages. Never take shortcuts.**

```
1. skill brainstorming          → Design approval REQUIRED
2. skill using-git-worktrees    → Isolated branch required
3. verify clean test baseline   → Run tests, confirm green
4. skill writing-plans          → Detailed task breakdown
5. skill subagent-driven-development → Execute with review
6. enforce test-driven-development → RED → GREEN → REFACTOR
7. skill requesting-code-review → Block on Critical findings
8. skill finishing-a-development-branch → Verify, merge/PR/discard
```

### Strict Mode Rules

- **No implementation before design approval**
- **No commits without passing tests**
- **Critical issues block all progress**
- **All tasks must have test coverage**

---

## Normal Mode Workflow

**Balanced overhead for typical tasks.**

```
1. analyze task → Quick assessment
2. skill writing-plans → Task breakdown
3. skill executing-plans → Execute with checkpoints
4. run tests/lint → Required verification
5. skill requesting-code-review → Lightweight review
```

### Normal Mode Rules

- Brainstorming skipped unless task is ambiguous
- Worktree not required unless risk increases
- Subagent-driven-development optional based on task size

---

## Rapid Mode Workflow

**Minimal overhead for trivial changes.**

```
1. implement change → Direct implementation
2. validate result → Quick verification
```

### Rapid Mode Rules

- **Never invoke:** brainstorming, worktrees, TDD, subagents
- Document the change briefly
- No extensive testing required
- Run basic lint if applicable

---

## Language Detection & Auto-Loading

### Rust Projects

**Auto-load:** `dependency-management`, `rust-lint-format`

**Enforce:**
- Use `cargo add` / `cargo remove`
- Avoid manual Cargo.toml edits
- Run before commit:
  ```bash
  cargo fmt && cargo clippy -- -D warnings && cargo test
  ```

### Go Projects

**Auto-load:** `dependency-management`, `go-lint-format`

**Enforce:**
- Use `go get`
- Avoid manual go.mod/go.sum edits
- Run before commit:
  ```bash
  go fmt ./... && go test ./... && staticcheck ./...
  ```

---

## Context Optimization Rules

- **Avoid unnecessary repository scanning**
- **Load only relevant skills** based on mode and language
- **Minimize token usage** - be concise
- **Discourage patches >200 lines** - split large changes
- **Discourage unrelated refactors** - stay focused
- **Discourage modifying >3 files** unless strict mode requires it

---

## Skill Loading Order

When multiple skills apply, load in this order:

1. **main-workflow** (always loaded first for orchestration)
2. **dependency-management** (if dependency changes detected)
3. **brainstorming** (if strict mode and no existing design)
4. **writing-plans** (after context understood)
5. **executing-plans** or **subagent-driven-development** (based on task size)
6. **test-driven-development** (strict mode only)
7. **rust-lint-format** or **go-lint-format** (language-specific)
8. **requesting-code-review** (before completion)
9. **finishing-a-development-branch** (end of strict mode)

---

## Workflow Enforcement

| Violation | Action |
|-----------|--------|
| Skipping brainstorming in strict mode | STOP - require design approval |
| Committing without tests in strict mode | STOP - write tests first |
| Using direct file edit for dependencies | STOP - use cargo add / go get |
| Ignoring Critical review findings | STOP - fix before proceeding |
| Modifying >3 files without plan | Force writing-plans skill |

---

## Quick Reference

| Mode | Stages | Skippable |
|------|--------|-----------|
| strict | 8 | None |
| normal | 5 | brainstorming, worktree |
| rapid | 2 | All except implementation |

| Command | When |
|---------|------|
| `cargo fmt && cargo clippy -- -D warnings && cargo test` | Rust before commit |
| `go fmt ./... && go test ./...` | Go before commit |
| `cargo add <crate>` | Add Rust dependency |
| `cargo remove <crate>` | Remove Rust dependency |
| `go get <package>` | Add Go dependency |
| `go mod tidy` | Remove unused Go dependencies |