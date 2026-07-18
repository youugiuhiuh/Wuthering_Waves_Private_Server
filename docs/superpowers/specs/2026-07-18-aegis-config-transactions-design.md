# Aegis Configuration Transactions Design

**Date:** 2026-07-18
**Status:** Approved design

## Goal And Evidence

Serialize encrypted configuration mutations currently spread across
`bootstrap.rs` and state-publishing call sites. Independent read-modify-write
sequences can overwrite concurrent fields or publish memory before persistence.

## Boundary

Introduce one focused configuration mutation function guarded by an in-process
mutex. Under the lock it reads and decrypts the latest file, applies one typed
mutation, validates and encrypts the complete value, then uses the existing
secure atomic writer. Only successful persistence may publish corresponding
in-memory state or user-visible success.

This boundary covers language, self-destruct key hash, Matrix recovery key, and
other existing mutations of the same encrypted configuration. It does not
redesign setup or define a generic database transaction layer.

## Invariants And Recovery

- Every mutation starts from the latest committed disk value.
- A write, sync, rename, or directory-sync failure leaves old disk and memory
  state authoritative.
- Parse, decrypt, and validation failures abort; no default configuration is
  substituted.
- Locks are not held across unrelated network or bot operations.
- Security material is absent from errors and logs.

## Tests And Acceptance

- Concurrent mutations to different fields preserve both updates.
- Injected temporary-write and rename failures preserve old disk bytes.
- The same failures leave observable in-memory state unchanged.
- Corrupt encrypted input fails closed.
- Existing setup and successful mutation behavior remains compatible.

## Residual Risk

This phase serializes writers in one Aegis process. Cross-process configuration
editing is not supported and must not be implied by the API.
