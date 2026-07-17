# Aegis Rust Stability, Performance, and Security Audit

**Date:** 2026-07-17  
**Scope:** `rust/aegis` non-documentation source code  
**Repository state:** `main` at `4633b2f` (`chore: bump version to 3.4.4`)  
**Audit type:** Read-only static review  
**Remediation branch:** `security/aegis-github-only-update`

## Executive Summary

The primary improvement opportunities are not ordinary Rust syntax or Clippy
issues. They are concentrated at privileged trust boundaries, transactional
state updates, process lifecycle management, and unbounded resource use.

Remediation should proceed in this order:

1. P0: identity separation, signed upgrades/installers, and token-safe URLs.
2. P1: child-process cleanup, atomic state, bounded downloads, and rollback.
3. P2: SNI persistence, bounded TLS probing, caching, and benchmarks.

## Evidence Checked

- CodeGraph source and call-path analysis of runtime dispatch, authorization,
  installers, upgrades, command execution, persistence, and batch generation.
- Focused source inspection of the files cited below.
- `PROTOC=/tmp/protoc/bin/protoc cargo clippy --lib --all-features -- -D warnings`
  completed successfully.
- Clippy reported only a future-incompatibility warning from the dependency
  `proc-macro-error2 v2.0.1`.
- No source files were modified during the initial audit.

## Verification Executed (post-remediation, 2026-07-17)

The following verification commands were run on the remediation branch
(`security/aegis-github-only-update`) after Tasks 1-3 were implemented:

### Source-level regression check

```bash
grep -rn 'AEGIS_RELEASE_MIRRORS\|WWPS_CORE_RELEASE_MIRRORS\|AEGIS_RELEASE_REPOSITOR\|
GITHUB_TOKEN.*asset\|fetch_json_from_mirrors\|find_minisig_asset\|extract_sha256_from_body\|
parse_sha256_manifest\|SHA256_LINE_RE\|DEFAULT_RELEASE_REPOSITORIES\|ReleaseRepo\|
release_api_bases\|select_asset' rust/aegis/src/
```

Result: **No forbidden strings found** — all mirror/fallback env-vars, multi-repo configs,
and fuzzy-asset-selectors have been removed from the modified files.

### Formatting

```bash
PROTOC=/tmp/protoc/bin/protoc cargo fmt --check
```

Result: **Pass** — no formatting diffs.

### Focused regression tests

| Test suite | Result |
|---|---|
| `core::network::release_api::tests` | 9/9 pass |
| `core::system::upgrade::tests` | 8/8 pass |
| `core::system::core_upgrade::tests` | 11/11 pass |
| `core::crypto::minisign::tests` | 4/4 pass |

All 32 focused tests pass.

### Clippy (pre-existing baseline only)

```bash
PROTOC=/tmp/protoc/bin/protoc cargo clippy --all-targets --all-features -- -D warnings
```

Result: **Failures in files outside the 5 modified files — pre-existing baseline.**
- `src/main/config.rs:137,147,160,181,203` — 5 × `undocumented_unsafe_blocks`
- `src/adapters/discord/adapter.rs:95` — `items_after_test_module`
- `src/shared/commands.rs:203,234,248,264,280` — 5 × `len_zero`

These errors exist on the base `main` commit and are unrelated to the remediation
changes. No lint allowances were added.

### Full test suite

```bash
PROTOC=/tmp/protoc/bin/protoc cargo test --all-features
```

| Target | Tests | Result |
|---|---|---|
| lib (unittests) | 519 | **PASS** (1 ignored, pre-existing) |
| main (unittests) | 52 | **PASS** |
| cli integration | 6 | **PASS** |
| integration_security | 6 | **PASS** |
| integration_setup_roundtrip | 1 | **PASS** |
| integration_totp_trim | 3 | **PASS** |
| test_self_destruct | 21 | **PASS** |
| test_self_destruct_e2e | 10 | **PASS** |
| **Total** | **620** | **All pass, 0 failures** |

### Credential/side-effect ordering review

- Asset HTTP requests never carry `Authorization` header (enforced by
  `build_asset_request`). Confirmed via test `asset_request_never_contains_authorization`.
- API requests carry `Authorization` only to `https://api.github.com` (enforced by
  `build_github_api_request`). Confirmed via test `api_request_is_fixed_origin_and_may_contain_authorization`.
- Redirects are validated per-hop against `release-assets.githubusercontent.com`
  and `objects.githubusercontent.com` only (enforced by `github_asset_client`).
- SHA256 verification is mandatory in all three paths (Aegis via Minisign + SHA256,
  Xray via three-way API/.dgst/actual hash, Minisign via decode + trusted comment).

## P0 Security Findings

### 1. Cross-platform principal collision

**Severity:** Critical  
**Locations:**

- `rust/aegis/src/main/runtime.rs:154-180`
- `rust/aegis/src/shared/types.rs:39-44`
- `rust/aegis/src/app/state.rs:123-146`
- `rust/aegis/src/app/state.rs:198-201`

Matrix user IDs are reduced to the numeric localpart. Telegram, Discord, and
Matrix authorization and sessions are then keyed by the same unnamespaced
integer. For example, Telegram user `42` and Matrix user
`@42:attacker.example` can represent the same internal principal.

**Remediation:** Introduce a typed principal such as
`Principal::{Telegram(u64), Discord(u64), Matrix(OwnedUserId)}`. Key sessions,
failed attempts, and administrator configuration by the complete principal.
Clear existing sessions during migration.

### 2. Optional signatures for privileged upgrades [AEGIS-002]

**Severity:** Critical  
**Locations:**

- `rust/aegis/src/core/system/upgrade.rs` (full rewrite)
- `rust/aegis/src/core/system/core_upgrade.rs` (full rewrite)

Minisign download failures could previously be converted to an absent signature,
and SHA-256 metadata came from the same release trust domain as the executable.

**Remediation:** Production upgrades must fail closed when signatures are
missing, unavailable, invalid, or signed for the wrong version, architecture,
or asset.

**Status: ADDRESSED** (remediation branch `security/aegis-github-only-update`)

- Aegis self-upgrade (`upgrade.rs`): `ReleaseArtifact.minisig` changed from
  `Option<Vec<u8>>` to `Vec<u8>` — mandatory. The `verify_downloaded_update`
  function always validates SHA256 + Minisign signature + trusted comment, and
  removes the temporary file on any failure. The `validate_trusted_comment`
  helper requires exact tag and asset-name match.
- Xray core upgrade (`core_upgrade.rs`): Uses three-way SHA256 verification
  (API digest + remote `.dgst` file + actual hash) via `verify_xray_hashes`.
  Minisign not applicable (XTLS/Xray-core does not publish Minisign signatures).
- Repository identity is compile-time fixed: `AEGIS_RELEASE_OWNER`/`REPO` and
  `XRAY_RELEASE_OWNER`/`REPO` are `const` — no env-var or mirror override.
- Asset selection uses exact name match via `find_named_asset` — no prefix
  matching, no fallback to first asset, no fuzzy selectors.

### 3. Unauthenticated Sing-box installation [AEGIS-003]

**Severity:** Critical  
**Location:** `rust/aegis/src/core/singbox/installer.rs:33-209`

The root process downloads and extracts a release with `curl` and `tar`, then
installs and starts the binary without signature or digest verification. The
predictable `/tmp/sing-box-install` directory also permits prepopulation,
symlink, and concurrent-install attacks.

**Remediation:** Require a pinned signing key and exact asset metadata. Use a
random `0700` temporary directory, no-follow/create-new files, safe archive
extraction, atomic installation, health checks, and rollback.

**Status: NOT ADDRESSED** — outside the scope of this remediation slice.
Sing-box installer hardening requires a separate effort.

### 4. GitHub token disclosure and SSRF through release URLs [AEGIS-004]

**Severity:** High  
**Locations:**

- `rust/aegis/src/core/network/release_api.rs` (new client factories)
- `rust/aegis/src/core/system/upgrade.rs` (migrated to new clients)
- `rust/aegis/src/core/system/core_upgrade.rs` (migrated to new clients)

The updater previously trusted metadata-provided URLs and could attach
`GITHUB_TOKEN` to an arbitrary destination. There was no strict host, scheme,
port, private-IP, or redirect validation.

**Remediation:** Use separate authenticated API and unauthenticated asset
clients. Send credentials only to exact API hosts, reject unsafe destinations,
and revalidate every redirect.

**Status: ADDRESSED** (remediation branch `security/aegis-github-only-update`)

- `github_api_client(timeout)` — no-redirect client for API calls.
- `github_asset_client(timeout)` — redirect-locked client that validates every
  redirect against `release-assets.githubusercontent.com` or
  `objects.githubusercontent.com`; rejects all other hosts.
- `validate_asset_url(input, stage)` — exact host equality, credential-less
  HTTPS enforcement at both initial-URL and redirect stages.
- `build_asset_request(client, url)` — constructs GET requests without any
  `Authorization` header (confirmed by test).
- `build_github_api_request(client, api_path, token)` — enforces
  `https://api.github.com` origin; rejects paths with unsafe characters
  (`?`, `#`, `..`, `//`, `@`).
- `ReleaseAsset::download_url()` no longer falls back to the `url` API field
  (which could be a different origin).

### 5. Sensitive file creation depends on umask

**Severity:** High  
**Locations:**

- `rust/aegis/src/core/security/crypto.rs:22-38`
- `rust/aegis/src/main/matrix.rs:57-80`
- `rust/aegis/src/core/network/warp_api.rs:109-116`
- `rust/aegis/src/core/xray/reality.rs:130-149`

Several keys, tokens, account files, and seeds are created with ordinary writes
or are chmodded only after creation. This creates permission, TOCTOU, and
symlink windows.

**Remediation:** Create directories as `0700` and sensitive files as `0600`
using `create_new`, `O_NOFOLLOW`, same-directory atomic replacement, `fsync`,
and owner/type/permission validation.

### 6. Self-destruct state-machine weaknesses

**Severity:** High  
**Locations:**

- `rust/aegis/src/shared/dispatch.rs:16-35`
- `rust/aegis/src/shared/destruct.rs:89-224`
- `rust/aegis/src/shared/destruct.rs:422-492`
- `rust/aegis/src/app/state.rs:289-303`

Failed TOTP attempts can refresh the interaction window and do not use a
dedicated failure limit. The cancel callback is intercepted before generic
authorization and does not independently verify the caller.

**Remediation:** Use an absolute deadline unaffected by failures, attempt caps,
backoff, consumed-counter replay protection, recent reauthentication, and
principal-bound authorization for every callback.

## P1 Stability Findings

### 1. Timed-out commands may survive

**Severity:** High  
**Location:** `rust/aegis/src/core/cmd_async.rs:9-100`

Timeouts drop the command future without explicitly killing and reaping the
child process or process group. Commands can continue holding package-manager
locks and file descriptors.

**Remediation:** Explicitly spawn with `kill_on_drop(true)`, terminate the
process group on timeout, call `wait()`, concurrently drain both output streams,
and retain only a bounded diagnostic tail.

### 2. Configuration updates are non-atomic and unlocked

**Severity:** High  
**Locations:**

- `rust/aegis/src/bootstrap.rs:343-389`
- `rust/aegis/src/shared/dispatch.rs:167-189`

Several settings use direct read-modify-write operations. Concurrent updates
can overwrite one another, and the security-file path updates memory and
reports success even when persistence fails.

**Remediation:** Centralize configuration writes behind one serialized store.
Persist atomically first, then update memory and report success. Sync both the
file and parent directory.

### 3. Port allocation is a racing read-scan-write transaction

**Severity:** High  
**Location:** `rust/aegis/src/core/xray/port_allocator.rs:32-205`

Concurrent allocations can select overlapping ranges and overwrite state.
Loading errors are treated as an empty allocation set.

**Remediation:** Add an in-process mutex and cross-process file lock, propagate
load/corruption errors, allocate under the lock, and atomically persist state.

### 4. Installers and upgrades lack single-flight rollback

**Severity:** High  
**Locations:**

- `rust/aegis/src/core/singbox/installer.rs:14-232`
- `rust/aegis/src/core/system/core_upgrade.rs:440-499`
- `rust/aegis/src/core/system/core_upgrade.rs:621-668`
- `rust/aegis/src/core/system/upgrade.rs:545-587`

Repeated callbacks can launch concurrent privileged operations. Core upgrade
creates a backup but does not restore it after restart or health-check failure.
Aegis self-upgrade can leave the disk and running process on different versions.

**Remediation:** Introduce a per-operation registry, unique temporary state,
atomic replacement, deterministic restart, health validation, and automatic
rollback with combined error reporting.

### 5. Downloads are bounded after buffering, or not bounded

**Severity:** High  
**Locations:**

- `rust/aegis/src/shared/dispatch.rs:141-150`
- `rust/aegis/src/shared/destruct.rs:225-235`
- `rust/aegis/src/adapters/telegram/adapter.rs:89-93`
- `rust/aegis/src/adapters/discord/adapter.rs:83-86`
- `rust/aegis/src/adapters/matrix/adapter.rs:190-201`

Attachments are returned as a complete `Vec<u8>`. The 10 MiB security-file
limit is checked only after download, while the self-destruct verification path
has no corresponding limit.

**Remediation:** Stream into a bounded hash operation, stop at `MAX + 1`, check
trusted metadata when available, enforce request/idle timeouts, and limit
concurrent downloads.

### 6. Scheduler and gateway state can silently diverge

**Severity:** Medium to High  
**Locations:**

- `rust/aegis/src/core/system/scheduler/mod.rs:37-242`
- `rust/aegis/src/shared/handlers/schedule.rs:350-363`
- `rust/aegis/src/main/runtime.rs:126-147`
- `rust/aegis/src/main/runtime.rs:228-235`

Scheduler load, save, registration, start, and shutdown errors are sometimes
ignored. Discord and Matrix gateway tasks can terminate while the process
continues to appear healthy.

**Remediation:** Validate and persist complete scheduler state before swapping
runtime instances. Supervise gateway tasks with `JoinSet`/`select!`, bounded
reconnect backoff, cancellation, and non-zero exit after terminal failure.

## P2 Performance Findings

### 1. SNI persistence is O(K * N)

**Impact:** Severe  
**Locations:**

- `rust/aegis/src/core/sni/selector.rs:136-175`
- `rust/aegis/src/core/sni/state.rs:136-147`

Every selected SNI clones, serializes, encrypts, and synchronously writes the
entire domain and index state. The audited US dataset contains about 381,921
domains and is approximately 6.3 MiB before runtime expansion.

**Remediation:** Keep immutable domains in `Arc<[String]>`, persist only compact
seed/cursor/index state, save once per batch or debounce writes, and move heavy
crypto/I/O off Tokio workers.

### 2. Batch TLS probes are serial

**Impact:** Severe  
**Locations:**

- `rust/aegis/src/core/xray/xhttp.rs:33-88`
- `rust/aegis/src/core/xray/reality.rs:177-231`
- `rust/aegis/src/core/security/tls_probe.rs:39-81`

Each batch item waits for the previous TLS probe. With combined timeouts of
about 15 seconds, a 50-item batch can approach 12.5 minutes.

**Remediation:** Probe with bounded concurrency of 4-8, preserve output order,
add positive and negative TTL caching, and use single-flight per `(sni, port)`.

### 3. Batch port selection repeats scans and process launches

**Impact:** High  
**Locations:**

- `rust/aegis/src/core/xray/config.rs:279-331`
- `rust/aegis/src/core/xray/port_allocator.rs:32-167`

Candidate selection repeatedly parses allocation state and launches `netstat`.

**Remediation:** Snapshot allocation state and listening ports once per batch,
allocate from an in-memory `HashSet`, detect the firewall backend once, and
commit state once.

### 4. TLS probe cache grows without bounds

**Impact:** High  
**Location:** `rust/aegis/src/core/security/tls_probe.rs:21,110-117`

The global `DashMap` has no capacity, TTL, or eviction, and concurrent misses
can duplicate the same handshake.

**Remediation:** Use a bounded TTL cache keyed by `(sni, port)`, with different
positive and negative TTLs and shared in-flight probes.

### 5. Expensive operations lack backpressure

**Impact:** High  
**Location:** `rust/aegis/src/shared/handlers/ops.rs:38-224`

Detached tasks and an unbounded progress channel permit duplicate upgrades,
package operations, and stale progress queues.

**Remediation:** Use per-operation single-flight, bounded work queues,
`CancellationToken`, supervised tasks, and `watch` or capacity-one channels for
latest-value progress.

## Recommended Verification Work

1. Test identical numeric IDs across all platforms and identical Matrix
   localparts on different homeservers.
2. Reject missing, unavailable, invalid, wrong-version, and wrong-architecture
   signatures.
3. Run malicious tar/zip and `/tmp` symlink tests only in an isolated VM or
   container.
4. Verify timed-out command PIDs and descendants are killed and reaped.
5. Stress 100 concurrent port allocations and concurrent configuration writes.
6. Benchmark SNI selection with batch sizes 1, 10, and 50.
7. Benchmark TLS probing at concurrency 1, 4, 8, and 16.
8. Measure upload and upgrade RSS using 10, 100, and 500 MiB streams.
9. Run `cargo audit` or OSV checks in controlled CI.
10. Inspect the deployed systemd unit for `UMask`, `PrivateTmp`,
    `ProtectSystem`, `NoNewPrivileges`, capabilities, and service user.

## Explicit Limitations

- No full test suite was run during the read-only audit phase. The post-remediation
  verification ran the full suite (620 tests, all pass) — see "Verification Executed"
  above. This does not replace targeted pre-merge CI for the remediation branch.
- No online dependency vulnerability database was queried.
- No production systemd unit is checked into this repository.
- Exploitability of Matrix homeserver behavior, archive handling, redirect
  policy, and local symlink races still requires controlled dynamic testing.
- Performance impact estimates are based on code paths and dataset size; they
  require benchmarks before selecting exact concurrency and cache parameters.
