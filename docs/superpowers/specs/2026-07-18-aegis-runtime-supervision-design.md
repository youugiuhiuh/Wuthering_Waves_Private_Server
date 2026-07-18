# Aegis Scheduler And Gateway Supervision Design

**Date:** 2026-07-18
**Status:** Approved design

## Goal And Evidence

Make scheduler replacement transactional and prevent Discord or Matrix gateway
tasks from terminating while the main process continues to report healthy.
Relevant boundaries are `core/system/scheduler/mod.rs`,
`shared/handlers/schedule.rs`, and `main/runtime.rs`.

## Scheduler Transaction

Build a complete candidate scheduler without disturbing the active instance.
Validate every persisted task, register all jobs, and prove the candidate can
start. Persist the complete new scheduler state atomically before swapping the
global runtime reference. Only after the swap may the old scheduler shut down.

Add and remove operations use the same candidate-first transaction. Load,
validation, registration, start, or persistence failure leaves the old file and
old running scheduler authoritative. Invalid persisted state is returned as an
error, never replaced with an empty schedule.

## Gateway Supervision

Run Discord and Matrix gateways under one explicit supervisor with shared
cancellation. Unexpected termination triggers bounded retries with capped
backoff. A successful run resets the retry budget only after a documented stable
period. Exhausting retries cancels sibling runtime tasks and causes `main` to
exit nonzero. Intentional shutdown is distinguishable from failure.

## Errors And Observability

Scheduler transaction failures name the failed stage. Gateway logs include the
platform, attempt, backoff, and terminal reason without credentials or event
content. No detached task may fail silently.

## Tests And Acceptance

- Load, registration, start, and persistence failures preserve the old scheduler.
- Successful replacement persists before swapping and shuts down the old
  scheduler only after the candidate is active.
- Corrupt scheduler state fails closed.
- Gateway transient failures restart with bounded backoff.
- Exhausted retries cancel siblings and produce a nonzero process result.
- Intentional cancellation does not consume retry budget or report a crash.

## Residual Risk

External supervisors can restart a terminally failed process indefinitely. That
deployment policy is outside Aegis, but Aegis must provide a truthful nonzero
exit signal.
