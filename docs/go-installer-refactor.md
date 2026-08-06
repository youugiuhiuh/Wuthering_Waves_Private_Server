# Go Installer Refactoring Plan

## Goal

Reduce risk in `go/installer/main.go` without changing installer behavior or
combining the work with operational bug fixes.

## Why

The installer is a single 1,400+ line file. The highest-risk functions mix
unrelated responsibilities:

- `firstTimeSetup` collects credentials, creates TOTP data, builds payloads,
  and chooses a service platform.
- `downloadAndDeployAegis` fetches releases, verifies signatures and hashes,
  stops the service, and replaces the binary.
- `buildSetupPayload` manually serializes setup JSON.

Existing recovery logic is intentionally kept in the installer and should not
be changed as part of this refactor.

## Scope

1. Replace manual JSON construction with typed setup payload structs and
   `encoding/json`.
2. Split interactive setup into platform-specific credential collection and a
   small orchestration function.
3. Split deployment into download/verify, replace, and service lifecycle
   steps.
4. Add unit tests for payload serialization, platform selection, and service
   unit rendering.

## Non-goals

- Change supported platforms, prompts, setup input formats, or systemd flags.
- Change release repositories, signature verification, or encrypted config
  format.
- Add dependencies or introduce a new installer framework.
- Combine this work with remote host repair or Aegis runtime changes.

## Delivery Order

1. Add characterization tests around current payloads and rendered systemd
   units.
2. Replace manual JSON serialization while preserving byte-for-byte expected
   payload structure where required.
3. Extract setup and deployment helpers in small behavior-preserving commits.
4. Run `go fmt ./...`, `go test ./...`, and `staticcheck ./...` after each
   slice.

## Acceptance Criteria

- Interactive and non-interactive setup continue producing valid Aegis setup
  payloads.
- Existing platform-to-flag mapping remains Telegram: none, Matrix:
  `--matrix`, Discord: `--discord`, Telegram + Matrix: `--all`.
- Installer tests cover each platform and the missing-service recovery choice.
- No new Go module dependencies are introduced.
