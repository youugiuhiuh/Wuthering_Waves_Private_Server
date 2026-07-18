# Aegis Port Allocation Transactions Design

**Date:** 2026-07-18
**Status:** Approved design

## Goal And Evidence

Make `core/xray/port_allocator.rs` allocation state transactional. Current scan,
load, append, and save steps are separate, direct writes are non-atomic, and
several errors become empty defaults.

## Transaction Boundary

Use an in-process mutex plus a Linux advisory lock on a dedicated lock file.
Acquire both before reading allocation state, scanning occupied ports, choosing
or releasing a port, and atomically persisting the new complete state. Keep the
cross-process lock until persistence and directory synchronization complete.

The allocation JSON remains the source of truth. Reuse the secure same-directory
atomic-write primitive where its directory assumptions fit; otherwise add only
the smallest sibling primitive needed for this non-secret state file.

## Invariants And Recovery

- Two tasks or processes cannot allocate the same port from one state version.
- Corrupt, unreadable, or unpersistable state aborts the operation.
- Failed persistence leaves the previous file authoritative and returns no port.
- Port scanning errors are propagated instead of ignored.
- Lock acquisition is bounded and produces an actionable error.

## Tests And Acceptance

- Concurrent tasks allocate distinct ports.
- Separate test processes using the same state file allocate distinct ports.
- Corrupt JSON fails closed and remains unchanged.
- Injected persistence failure returns no successful allocation.
- Release and lookup operations no longer use `unwrap_or_default()` fallbacks.

## Residual Risk

An external program can bind a selected port after the critical section. The
consumer must still handle bind failure; this phase prevents allocator-state
races rather than reserving operating-system sockets.
