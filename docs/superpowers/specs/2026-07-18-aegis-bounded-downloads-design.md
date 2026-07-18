# Aegis Bounded Attachment Downloads Design

**Date:** 2026-07-18
**Status:** Approved design

## Goal And Evidence

Replace `BotAdapter::download_file(...) -> Vec<u8>` implementations that fully
buffer Telegram, Discord, or Matrix media before callers can enforce limits.

## Download Boundary

Expose one bounded attachment operation used by the security-file workflows. It
streams into a private temporary file or bounded sink, counts bytes with checked
arithmetic, computes the required digest during streaming, and returns a typed
result only after validation. No platform adapter may first materialize an
unbounded `Vec<u8>`.

The request has a total timeout and an idle-progress timeout. A shared semaphore
limits concurrent attachment transfers across adapters. Trusted metadata is
validated when present, but absence never disables streamed enforcement. The
stream stops at `MAX + 1` bytes.

## Failure And Cleanup

Missing required metadata, forged length, streamed overflow, timeout, digest
mismatch, semaphore exhaustion, and transport failure abort the operation and
remove temporary data. Errors identify the stage without exposing file content
or full sensitive hashes.

## Tests And Acceptance

- All three adapters enforce the same byte limit.
- Absent or forged length cannot bypass streamed counting.
- Streamed overflow stops at `MAX + 1` and leaves no residue.
- Idle timeout and total timeout are independently observable.
- Hash mismatch removes temporary data.
- The concurrency cap rejects or waits according to one documented policy.
- Dispatch and self-destruct callers no longer receive unvalidated full buffers.

## Residual Risk

Platform SDKs that only expose complete media buffers may require a narrowly
scoped adapter implementation or upstream API change. Such a limitation must be
explicit; it cannot be hidden behind the bounded interface.
