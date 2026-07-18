# Aegis P1 Stability Roadmap Design

**Date:** 2026-07-18
**Status:** Approved design, awaiting implementation plans
**Scope:** The six unresolved P1 stability boundaries in the Aegis Rust audit

## Goal

Remove lifecycle, transaction, rollback, resource-bound, and supervision failure
modes without mixing in P2 performance work or weakening the security invariants
landed in PRs #179-#184.

## Delivery Order

| Phase | Boundary | Design |
| --- | --- | --- |
| 1 | Child-process lifecycle | `2026-07-18-aegis-command-lifecycle-design.md` |
| 2 | Configuration transactions | `2026-07-18-aegis-config-transactions-design.md` |
| 3 | Port allocation transactions | `2026-07-18-aegis-port-allocation-transactions-design.md` |
| 4 | Installer and upgrade rollback | `2026-07-18-aegis-upgrade-rollback-design.md` |
| 5 | Bounded attachment downloads | `2026-07-18-aegis-bounded-downloads-design.md` |
| 6 | Scheduler and gateway supervision | `2026-07-18-aegis-runtime-supervision-design.md` |

Each phase starts from the latest merged `main`, uses a fresh worktree, receives
its own implementation plan and pull request, and does not depend on an unmerged
phase.

## Global Invariants

- The old valid state, binary, or runtime remains authoritative until its
  replacement is fully usable.
- Failures are returned and logged; corruption is never converted to an empty
  default.
- Existing identity, file-system, release-verification, and self-destruct
  security guarantees remain intact.
- Changes stay inside the named boundary. No generic framework, compatibility
  bypass, unrelated refactor, or speculative dependency is introduced.
- Linux-specific lifecycle and locking guarantees receive a minimal integration
  test in addition to unit tests.

## Execution Contract

Every phase follows RED, GREEN, REFACTOR and must pass, from `rust/aegis`:

```bash
cargo fmt
cargo clippy -- -D warnings
cargo test
```

Specification-compliance and code-quality reviews are mandatory. Critical and
Important findings block completion. Each phase records failure points,
rollback outcome, and residual risk in its pull request.

## Non-Goals

- P2 throughput, caching, TLS-probe, and benchmarking findings.
- Redesigning bot commands or user-facing workflows.
- Combining all stability work into one branch or pull request.

## Program Acceptance

All six phases merge independently, their focused failure-injection tests pass,
the full Rust gates remain green, and no phase silently loses state, leaves a
timed-out process alive, publishes a partial replacement, or keeps the process
healthy after a terminal gateway failure.
