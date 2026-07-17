# Aegis Xray Version Selection Regression Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore Xray's recent-version selector without allowing query delimiters inside validated GitHub API paths.

**Architecture:** Preserve `build_github_api_request` as the fixed-origin path validator and add a narrow structured-query wrapper that delegates encoding to reqwest. Xray supplies the fixed releases path and `per_page` separately; no URL string concatenation or relaxed path validation is allowed.

**Tech Stack:** Rust 2024, Tokio, reqwest, serde, anyhow, existing unit-test framework

## Global Constraints

- GitHub API origin remains exactly `https://api.github.com`.
- `GITHUB_TOKEN` may be attached only to requests whose origin is exactly `https://api.github.com`.
- API paths containing `?`, `#`, `..`, `//`, or `@` remain rejected.
- Query keys and values must be encoded by `reqwest::RequestBuilder::query`; never concatenate them into a URL or path.
- Xray repository remains exactly `XTLS/Xray-core`.
- Do not change release asset selection, digest verification, download behavior, installation behavior, or UI flow.
- Do not add dependencies, compatibility switches, generic URL policy abstractions, mirrors, or third-party repositories.
- Do not commit unless the user explicitly requests it; commit commands below are execution checkpoints, not authorization to commit.

## File Map

- Modify `rust/aegis/src/core/network/release_api.rs`: add fixed-origin structured-query request and JSON helpers.
- Modify `rust/aegis/src/core/system/core_upgrade.rs`: keep the recent-releases path query-free and pass `per_page` structurally.
- Modify `docs/audits/2026-07-17-aegis-rust-stability-performance-security-audit.md`: mark AEGIS-013 addressed only after all gates pass.

## Root Cause Record

`fetch_recent_tags` currently constructs:

```text
repos/XTLS/Xray-core/releases?per_page=5
```

`build_github_api_request` intentionally rejects `?`, so the failure occurs before the request is sent. The security check is correct; the caller violates its path-only contract.

---

### Task 1: Separate GitHub API Paths From Queries

**Files:**
- Modify: `rust/aegis/src/core/network/release_api.rs:100-146`

**Interfaces:**
- Consumes: `build_github_api_request(client, api_path, token) -> Result<reqwest::RequestBuilder>`
- Produces: `build_github_api_query_request(client, api_path, query, token) -> Result<reqwest::RequestBuilder>`
- Produces: `fetch_github_json_with_query<T>(client, api_path, query, token) -> Result<T>`
- Preserves: `fetch_github_json<T>(client, api_path, token) -> Result<T>`

- [ ] **Step 1: Write failing structured-query boundary tests**

Add inside `core/network/release_api.rs`'s existing test module:

```rust
#[test]
fn github_api_query_is_structured_and_percent_encoded() {
    let client = github_api_client(Duration::from_secs(1)).unwrap();
    let request = build_github_api_query_request(
        &client,
        "repos/XTLS/Xray-core/releases",
        &[("per_page", "5&unexpected=true")],
        Some("secret"),
    )
    .unwrap()
    .build()
    .unwrap();

    assert_eq!(
        request.url().as_str(),
        "https://api.github.com/repos/XTLS/Xray-core/releases?per_page=5%26unexpected%3Dtrue"
    );
    assert_eq!(
        request.headers().get(reqwest::header::AUTHORIZATION).unwrap(),
        "Bearer secret"
    );
}

#[test]
fn github_api_query_does_not_relax_path_validation() {
    let client = github_api_client(Duration::from_secs(1)).unwrap();
    assert!(
        build_github_api_query_request(
            &client,
            "repos/XTLS/Xray-core/releases?per_page=5",
            &[],
            None,
        )
        .is_err()
    );
}
```

- [ ] **Step 2: Run the tests and confirm RED**

Run from `rust/aegis`:

```bash
cargo test core::network::release_api::tests::github_api_query --all-features
```

Expected: compilation fails because `build_github_api_query_request` does not exist.

- [ ] **Step 3: Add the minimal structured-query helpers**

Add after `build_github_api_request`:

```rust
pub fn build_github_api_query_request(
    client: &reqwest::Client,
    api_path: &str,
    query: &[(&str, &str)],
    token: Option<&str>,
) -> Result<reqwest::RequestBuilder> {
    Ok(build_github_api_request(client, api_path, token)?.query(query))
}

async fn send_github_json<T: DeserializeOwned>(
    request: reqwest::RequestBuilder,
) -> Result<T> {
    request
        .send()
        .await
        .context("GitHub API request failed")?
        .error_for_status()
        .context("GitHub API returned error status")?
        .json::<T>()
        .await
        .context("Failed to parse GitHub API response")
}

pub async fn fetch_github_json_with_query<T: DeserializeOwned>(
    client: &reqwest::Client,
    api_path: &str,
    query: &[(&str, &str)],
    token: Option<&str>,
) -> Result<T> {
    send_github_json(build_github_api_query_request(
        client, api_path, query, token,
    )?)
    .await
}
```

Replace the body of `fetch_github_json` with:

```rust
send_github_json(build_github_api_request(client, api_path, token)?).await
```

This keeps one response/error parser and leaves the original path-only API unchanged.

- [ ] **Step 4: Run focused release API tests and confirm GREEN**

```bash
cargo test core::network::release_api::tests --all-features
```

Expected: all release API tests pass, including the two new regression tests.

- [ ] **Step 5: Review the credential boundary**

Build both a path-only request and a structured-query request in tests and verify:

```rust
assert_eq!(request.url().origin().ascii_serialization(), "https://api.github.com");
assert_eq!(request.url().query(), Some("per_page=5%26unexpected%3Dtrue"));
```

Expected: adding a query cannot alter scheme, host, port, path, or Authorization destination.

- [ ] **Step 6: Commit checkpoint**

```bash
git add rust/aegis/src/core/network/release_api.rs
git commit -m "fix: separate GitHub API paths from queries"
```

Do not run this commit command without explicit user authorization.

---

### Task 2: Restore Xray Recent-Version Listing

**Files:**
- Modify: `rust/aegis/src/core/system/core_upgrade.rs:24-36,180-196,606-738`

**Interfaces:**
- Consumes: `fetch_github_json_with_query<T>(client, api_path, query, token) -> Result<T>` from Task 1
- Produces: `xray_releases_path() -> String`
- Preserves: `fetch_recent_tags(limit) -> Result<Vec<String>>`, including `limit == 0` returning an empty vector without a request

- [ ] **Step 1: Write the failing Xray path-contract test**

Add beside `xray_release_identity_is_fixed`:

```rust
#[test]
fn xray_recent_releases_path_contains_no_query() {
    let path = xray_releases_path();
    assert_eq!(path, "repos/XTLS/Xray-core/releases");
    assert!(!path.contains('?'));
}
```

- [ ] **Step 2: Run the test and confirm RED**

```bash
cargo test core::system::core_upgrade::tests::xray_recent_releases_path_contains_no_query --all-features
```

Expected: compilation fails because `xray_releases_path` does not exist.

- [ ] **Step 3: Implement the query-free path**

Add next to `xray_release_path`:

```rust
fn xray_releases_path() -> String {
    format!("repos/{XRAY_RELEASE_OWNER}/{XRAY_RELEASE_REPO}/releases")
}
```

Import `fetch_github_json_with_query`, then replace `fetch_recent_tags`'s path and fetch logic with:

```rust
let path = xray_releases_path();
let per_page = limit.to_string();
let query = [("per_page", per_page.as_str())];
let releases: Vec<ReleaseResponse> = fetch_github_json_with_query(
    &self.api_client,
    &path,
    &query,
    self.github_token.as_deref(),
)
.await?;
```

Keep the existing `limit == 0` early return and `.take(limit)` result bound.

- [ ] **Step 4: Run focused Xray and release API tests**

```bash
cargo test core::system::core_upgrade::tests --all-features
cargo test core::network::release_api::tests --all-features
```

Expected: both suites pass; paths containing `?` remain rejected and the version-list path is query-free.

- [ ] **Step 5: Run the real read-only version-list request**

Run an ignored/manual test or a temporary local harness that calls `fetch_recent_tags(5)` without invoking `run_upgrade`.

Expected:

```text
5 or fewer non-empty Xray release tags are returned; no download, extraction, replacement, or service restart occurs.
```

Delete any temporary harness before continuing. If network access is unavailable, record this step as an unsatisfied gate rather than a pass.

- [ ] **Step 6: Commit checkpoint**

```bash
git add rust/aegis/src/core/system/core_upgrade.rs
git commit -m "fix: restore Xray version selection"
```

Do not run this commit command without explicit user authorization.

---

### Task 3: Regression Gate and Audit Closure

**Files:**
- Modify: `docs/audits/2026-07-17-aegis-rust-stability-performance-security-audit.md`

**Interfaces:**
- Consumes: Tasks 1-2 behavior and test evidence
- Produces: verified closure evidence for `AEGIS-013`

- [ ] **Step 1: Run all Rust quality gates**

From `rust/aegis`:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Expected: all commands exit 0. Any missing tool or environment-dependent failure remains an unsatisfied gate.

- [ ] **Step 2: Review the final diff for forbidden regressions**

Confirm the diff does not:

```text
- remove '?' from the unsafe path character list
- concatenate '?per_page=' into a path or URL
- attach GITHUB_TOKEN outside api.github.com
- change Xray asset, digest, download, extraction, replacement, or restart code
```

- [ ] **Step 3: Update AEGIS-013 only after verification**

Record the merge commit or branch commit, focused test names, full gate results, and read-only live request result. Change `Status: NOT ADDRESSED` to `Status: ADDRESSED` only when all required evidence is present.

- [ ] **Step 4: Request specification and code-quality reviews**

Critical and Important findings block completion. The reviewer must explicitly verify that structured query encoding restored functionality without weakening path validation or token isolation.

- [ ] **Step 5: Commit checkpoint**

```bash
git add docs/audits/2026-07-17-aegis-rust-stability-performance-security-audit.md
git commit -m "docs: close Xray version selection regression"
```

Do not run this commit command without explicit user authorization.
