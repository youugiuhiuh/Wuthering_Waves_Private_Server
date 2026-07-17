# Aegis Remaining Security Roadmap

**Design:** `docs/superpowers/specs/2026-07-17-aegis-remaining-security-design.md`

**Goal:** Repair the Xray version-selection regression, then close the four unresolved Aegis security findings through isolated, reviewable pull requests.

## Delivery Order

| Phase | Trust boundary | Plan | Audit item | Status |
| --- | --- | --- | --- | --- |
| 0 | GitHub API path/query separation | `2026-07-17-aegis-xray-version-selection.md` | AEGIS-013 Xray version selection regression | Ready |
| 1 | Namespaced principals | `2026-07-17-aegis-namespaced-principals.md` | Cross-platform principal collision | Ready |
| 2 | Sensitive-file writes | `2026-07-17-aegis-secure-sensitive-files.md` | Permissive creation and symlink/TOCTOU windows | Ready |
| 3 | Sing-box installer | `2026-07-17-aegis-verified-singbox-install.md` | Unauthenticated privileged installer | Ready |
| 4 | Self-destruct flow | `2026-07-17-aegis-principal-bound-self-destruct.md` | Replay, timeout, authorization, and execution races | Ready |

## Execution Contract

Each phase must:

1. Start from the latest merged `main` in a new isolated worktree.
2. Record the baseline result of the three Rust gates before editing.
3. Follow RED, GREEN, REFACTOR for every task.
4. Receive specification-compliance and code-quality reviews.
5. Block on every Critical or Important review finding.
6. Update only its corresponding audit finding after all gates pass.
7. Land as its own pull request before the next phase begins.

No phase may add a compatibility bypass, generic trust framework, third-party repository support, or unrelated stability/performance work. Phase 0 is the narrowly scoped regression repair required to preserve the already-approved GitHub API security boundary.

## Required Gates Per Phase

Run from `rust/aegis`:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Record command, tool version, exit status, and any pre-existing failure. A missing tool or environment-dependent test is an unsatisfied gate, not a pass.

## Program-Level Gates

After Phase 4 merges:

```bash
cargo audit
```

Also record:

- Malicious archive fixture results.
- Symlink and concurrent replacement fixture results.
- Concurrent identity and self-destruct stress-test results.
- A log review proving tokens, TOTP values, security-file contents, and full hashes are absent.
- Residual risks that remain in the stability and performance sections of the audit.

## Rollback Rules

- Phase 0: query construction failure performs no network request and leaves the updater state unchanged.
- Phase 1: migration failure leaves the legacy encrypted configuration unchanged and stops startup.
- Phase 2: write failure removes only the temporary file and preserves the previous destination.
- Phase 3: any metadata, download, digest, archive, or version failure preserves the old binary and service state.
- Phase 4: any authorization or execution failure enters a terminal state; replay cannot resume or execute it.

## Completion Evidence

For every merged phase, append to the matching finding in
`docs/audits/2026-07-17-aegis-rust-stability-performance-security-audit.md`:

- Merge commit.
- Focused tests and full gate output.
- Security review result.
- Residual risk.

Do not mark stability or performance findings addressed as part of this roadmap.
