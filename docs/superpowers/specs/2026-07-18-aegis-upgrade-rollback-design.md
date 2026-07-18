# Aegis Installer And Upgrade Rollback Design

**Date:** 2026-07-18
**Status:** Approved design

## Goal And Evidence

Prevent privileged Sing-box, core, and Aegis upgrade paths from leaving disk and
runtime state divergent after replacement, restart, or health-check failure.
Existing download, origin, digest, signature, and archive checks remain
unchanged.

## Operation Protocol

Apply per-operation single-flight so duplicate requests for the same component
cannot overlap. After all existing verification succeeds:

1. Stage the candidate on the destination filesystem.
2. Preserve the current binary as a same-directory backup.
3. Atomically publish the candidate.
4. Restart or activate the component deterministically.
5. Run a bounded component-specific health check.
6. Remove the backup only after health succeeds.

For Aegis self-replacement, the protocol must define which supervisor performs
the restart and how the post-exec health result is observed before claiming
success. User-visible success is emitted only after the applicable gate passes.

## Rollback And Errors

Restart or health failure restores the backup atomically and attempts to return
the prior service to a healthy state. The returned error includes both the
original failure and any rollback failure without hiding either. Failed staging
or publication does not disturb the current binary. Temporary and backup files
have deterministic cleanup rules.

## Tests And Acceptance

- Single-flight rejects or joins concurrent operations for one component.
- Replacement failure preserves the old binary.
- Restart failure restores the old binary.
- Health failure restores the old binary.
- Rollback failure is separately observable with the original error.
- Existing release-verification tests remain green.

## Residual Risk

Host-level failure between filesystem synchronization and service control can
still require operator recovery. The retained backup and combined diagnostics
must make that recovery deterministic.
