# Aegis Bounded Attachment Downloads Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace every `BotAdapter::download_file -> Vec<u8>` path with one bounded attachment operation that returns only a validated byte count and streaming SHA-256 digest.

**Architecture:** Add a common attachment boundary that validates optional declared length, acquires one process-wide semaphore permit with immediate rejection, applies independent total and idle-progress timeouts, counts with checked arithmetic, hashes while consuming, and stops streaming adapters at exactly `MAX + 1`. Telegram and Discord feed network chunks into this boundary; Matrix uses the same policy and typed result but, because matrix-sdk 0.18 only returns `Vec<u8>`, rejects immediately after the SDK call and records that unavoidable pre-validation allocation as residual risk.

**Tech Stack:** Rust 2024, existing `tokio`, `futures-util`, `sha2`, `hex`, `thiserror`, `reqwest`, `teloxide`, `matrix-sdk`, and `wiremock`; no new crate.

## Files Touched

- Create: `rust/aegis/src/adapters/common/attachment.rs` - policy, typed request/result/error, shared semaphore, stream and Matrix-buffer consumers, focused boundary tests.
- Modify: `rust/aegis/src/adapters/common/mod.rs` - export attachment types.
- Modify: `rust/aegis/src/adapters/common/trait.rs` - replace `download_file` with `download_attachment`.
- Modify: `rust/aegis/src/adapters/common/routing.rs` - forward the bounded operation unchanged.
- Modify: `rust/aegis/src/adapters/telegram/adapter.rs` - stream Telegram chunks.
- Modify: `rust/aegis/src/adapters/discord/adapter.rs` - stream Discord response chunks.
- Modify: `rust/aegis/src/adapters/matrix/adapter.rs` - narrowly bound the SDK-provided complete buffer.
- Modify: `rust/aegis/src/shared/types.rs` - replace split file fields with one `Attachment`.
- Modify: `rust/aegis/src/main/runtime.rs` - capture Telegram and Matrix declared sizes when present.
- Modify: `rust/aegis/src/main/discord.rs` - capture Discord attachment size.
- Modify: `rust/aegis/src/shared/dispatch.rs` - persist only the validated digest.
- Modify: `rust/aegis/src/shared/destruct.rs` - verify only the validated digest.
- Modify: `rust/aegis/src/shared/commands.rs` - remove obsolete test-only `download_file` overrides.
- Modify: `rust/aegis/src/app/state.rs` - remove obsolete test-only `download_file` override.
- Modify: `rust/aegis/src/core/system/scheduler/mod.rs` - remove obsolete test-only `download_file` override.
- Modify: `rust/aegis/src/shared/boundary.rs` - update `MessageEvent` test construction.

## Global Constraints

- `MAX_ATTACHMENT_BYTES` is exactly `10 * 1024 * 1024` bytes for Telegram, Discord, and Matrix.
- `TOTAL_TIMEOUT` is 30 seconds; `IDLE_TIMEOUT` is 5 seconds for streaming adapters.
- `MAX_CONCURRENT_ATTACHMENTS` is 4 process-wide; saturation is rejected immediately with `AttachmentError::Busy`, never queued.
- Optional declared size is a preflight hint and an end-of-stream consistency check; `None` still receives full streamed enforcement, and a forged value cannot weaken the byte limit.
- Byte counting and `MAX + 1` calculation use `checked_add`; arithmetic failure returns `AttachmentError::Arithmetic`.
- Telegram and Discord stop polling their body at exactly `MAX_ATTACHMENT_BYTES + 1` observed bytes.
- SHA-256 is updated per accepted chunk and finalized only after size validation; no caller receives body bytes.
- Expected-digest absence or malformed hex in the self-destruct verification path fails before download; mismatch returns a stage-only error and removes/retains no attachment body.
- Errors may name only the stage and bounded byte counts. They must not include a URL/file identifier, body content, transport error text that can echo a URL, or a full digest.
- No temporary attachment file is created: the bounded digest sink is the cleanup strategy. On every failure its hasher/chunks are dropped and no residue exists.
- Matrix SDK 0.18 has no streaming media API and internally materializes `Vec<u8>` (and may materialize a second decrypted buffer). Matrix must apply declared-size preflight, a 30-second total timeout, and immediate post-return length rejection before hashing; it cannot honestly provide `MAX + 1` cancellation or idle-progress timeout.
- Changes stay inside bounded attachment downloads. No command redesign, generic download framework, compatibility bypass, or unrelated refactor.
- Every task follows RED -> GREEN -> REFACTOR and ends with the stated commit during execution. This planning session does not run those commits.

## Execution Index

Execute by task number, using these document anchors: [Task 1](#task-1-common-bounded-digest-boundary), [Task 2](#task-2-introduce-the-bounded-trait-contract), [Task 3](#task-3-stream-telegram-and-discord-through-the-shared-boundary), [Task 4](#task-4-apply-the-narrow-matrix-sdk-buffer-limit), [Task 5](#task-5-preserve-declared-size-at-every-platform-event-boundary), [Task 6](#task-6-consume-only-validated-digests-in-dispatch-and-self-destruct), then [Task 7](#task-7-full-security-and-rust-gates).

---

### Task 1: Common Bounded Digest Boundary

**Files:**
- Create: `rust/aegis/src/adapters/common/attachment.rs`
- Modify: `rust/aegis/src/adapters/common/mod.rs:1-4`

**Interfaces:**
- Produces: `Attachment { file_id: String, file_name: Option<String>, declared_size: Option<u64> }`.
- Produces: `VerifiedAttachment::size() -> u64`, `sha256_hex() -> String`, `hash_prefix() -> String`, and `redacted_hash() -> String`.
- Produces: `parse_sha256_hex(&str) -> Result<[u8; 32], AttachmentError>`.
- Produces: `consume_stream` for Telegram/Discord and `consume_matrix_buffer` for the Matrix SDK limitation.
- Produces: one static four-permit semaphore with immediate rejection.

- [ ] **Step 1: Write focused failing tests at the bottom of the new module**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn attachment(declared_size: Option<u64>) -> Attachment {
        Attachment {
            file_id: "opaque-id".into(),
            file_name: Some("security.bin".into()),
            declared_size,
        }
    }

    fn policy(max_bytes: u64) -> AttachmentPolicy {
        AttachmentPolicy {
            max_bytes,
            total_timeout: Duration::from_millis(100),
            idle_timeout: Duration::from_millis(20),
        }
    }

    #[tokio::test]
    async fn missing_size_still_hashes_and_enforces_streamed_bytes() {
        let result = consume_stream_with(
            &attachment(None),
            None,
            policy(4),
            &Semaphore::new(1),
            || async { Ok(stream::iter([Ok::<_, AttachmentError>(b"safe".to_vec())])) },
        )
        .await
        .unwrap();
        assert_eq!(result.size(), 4);
        assert_eq!(result.sha256_hex(), hex::encode(Sha256::digest(b"safe")));
    }

    #[tokio::test]
    async fn forged_small_size_cannot_bypass_max_plus_one_stop() {
        let polls = Arc::new(AtomicUsize::new(0));
        let seen = polls.clone();
        let chunks = stream::iter([b"1234".to_vec(), b"5678".to_vec(), b"9".to_vec()]).map(
            move |chunk| {
                seen.fetch_add(1, Ordering::SeqCst);
                Ok::<_, AttachmentError>(chunk)
            },
        );
        let error = consume_stream_with(
            &attachment(Some(1)),
            None,
            policy(4),
            &Semaphore::new(1),
            || async { Ok(chunks) },
        )
        .await
        .unwrap_err();
        assert_eq!(error, AttachmentError::TooLarge { observed: 5, max: 4 });
        assert_eq!(polls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn declared_size_mismatch_is_rejected_after_stream() {
        let error = consume_stream_with(
            &attachment(Some(4)),
            None,
            policy(8),
            &Semaphore::new(1),
            || async { Ok(stream::iter([Ok::<_, AttachmentError>(b"abc".to_vec())])) },
        )
        .await
        .unwrap_err();
        assert_eq!(error, AttachmentError::MetadataMismatch);
    }

    #[tokio::test]
    async fn digest_mismatch_does_not_expose_hashes() {
        let error = consume_stream_with(
            &attachment(None),
            Some([7; 32]),
            policy(8),
            &Semaphore::new(1),
            || async { Ok(stream::iter([Ok::<_, AttachmentError>(b"abc".to_vec())])) },
        )
        .await
        .unwrap_err();
        assert_eq!(error, AttachmentError::DigestMismatch);
        assert_eq!(error.to_string(), "attachment digest mismatch");
    }

    #[tokio::test]
    async fn transport_failure_drops_the_bounded_sink_without_residue() {
        let error = consume_stream_with(
            &attachment(None),
            None,
            policy(8),
            &Semaphore::new(1),
            || async {
                Ok(stream::iter([Err::<Vec<u8>, _>(AttachmentError::Transport)]))
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error, AttachmentError::Transport);
        assert_eq!(error.to_string(), "attachment transport failed");
    }

    #[tokio::test]
    async fn checked_add_rejects_unrepresentable_max_plus_one() {
        let error = consume_stream_with(
            &attachment(None),
            None,
            policy(u64::MAX),
            &Semaphore::new(1),
            || async { Ok(stream::empty::<Result<Vec<u8>, AttachmentError>>()) },
        )
        .await
        .unwrap_err();
        assert_eq!(error, AttachmentError::Arithmetic);
    }

    #[tokio::test]
    async fn idle_and_total_timeouts_are_distinct() {
        let idle = consume_stream_with(
            &attachment(None),
            None,
            AttachmentPolicy {
                max_bytes: 8,
                total_timeout: Duration::from_millis(80),
                idle_timeout: Duration::from_millis(10),
            },
            &Semaphore::new(1),
            || async {
                Ok(stream::once(async {
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    Ok::<_, AttachmentError>(b"a".to_vec())
                }))
            },
        )
        .await
        .unwrap_err();
        assert_eq!(idle, AttachmentError::IdleTimeout);

        let total = consume_stream_with(
            &attachment(None),
            None,
            AttachmentPolicy {
                max_bytes: 8,
                total_timeout: Duration::from_millis(25),
                idle_timeout: Duration::from_millis(20),
            },
            &Semaphore::new(1),
            || async {
                Ok(stream::unfold(0u8, |n| async move {
                    tokio::time::sleep(Duration::from_millis(15)).await;
                    Some((Ok::<_, AttachmentError>(vec![n]), n.wrapping_add(1)))
                }))
            },
        )
        .await
        .unwrap_err();
        assert_eq!(total, AttachmentError::TotalTimeout);
    }

    #[tokio::test]
    async fn saturated_shared_policy_rejects_without_waiting() {
        let semaphore = Semaphore::new(1);
        let _held = semaphore.acquire().await.unwrap();
        let started = std::time::Instant::now();
        let error = consume_stream_with(
            &attachment(None),
            None,
            policy(8),
            &semaphore,
            || async { Ok(stream::empty::<Result<Vec<u8>, AttachmentError>>()) },
        )
        .await
        .unwrap_err();
        assert_eq!(error, AttachmentError::Busy);
        assert!(started.elapsed() < Duration::from_millis(10));
    }

    #[test]
    fn digest_parser_rejects_missing_malformed_and_never_echoes_input() {
        assert_eq!(parse_sha256_hex(""), Err(AttachmentError::MissingDigest));
        let error = parse_sha256_hex(&"z".repeat(64)).unwrap_err();
        assert_eq!(error, AttachmentError::InvalidDigest);
        assert!(!error.to_string().contains('z'));
    }

    #[tokio::test]
    async fn matrix_buffer_rejects_immediately_after_sdk_return() {
        let error = consume_matrix_buffer_with(
            &attachment(Some(1)),
            None,
            policy(4),
            &Semaphore::new(1),
            async { Ok(vec![0; 9]) },
        )
        .await
        .unwrap_err();
        assert_eq!(error, AttachmentError::TooLarge { observed: 9, max: 4 });
    }
}
```

- [ ] **Step 2: Run the focused test target and verify RED**

Run from `rust/aegis`:

```bash
cargo test --lib adapters::common::attachment::tests -- --test-threads=1
```

Expected: FAIL because `adapters::common::attachment` and every listed type/function are undefined.

- [ ] **Step 3: Implement `attachment.rs` with the complete production boundary**

```rust
use std::future::Future;
use std::sync::LazyLock;
use std::time::Duration;

use futures_util::{Stream, StreamExt};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::Semaphore;
use tokio::time::{Instant, timeout, timeout_at};

pub const MAX_ATTACHMENT_BYTES: u64 = 10 * 1024 * 1024;
pub const TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_CONCURRENT_ATTACHMENTS: usize = 4;

static ATTACHMENT_TRANSFERS: LazyLock<Semaphore> =
    LazyLock::new(|| Semaphore::new(MAX_CONCURRENT_ATTACHMENTS));

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub file_id: String,
    pub file_name: Option<String>,
    pub declared_size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedAttachment {
    size: u64,
    sha256: [u8; 32],
}

impl VerifiedAttachment {
    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn sha256_hex(&self) -> String {
        hex::encode(self.sha256)
    }

    pub fn hash_prefix(&self) -> String {
        hex::encode(&self.sha256[..4])
    }

    pub fn redacted_hash(&self) -> String {
        format!("{}..{}", hex::encode(&self.sha256[..4]), hex::encode(&self.sha256[30..]))
    }

    #[cfg(test)]
    pub(crate) fn from_test_bytes(bytes: &[u8]) -> Self {
        Self {
            size: bytes.len() as u64,
            sha256: Sha256::digest(bytes).into(),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AttachmentError {
    #[error("attachment transfer is busy")]
    Busy,
    #[error("attachment metadata exceeds the limit")]
    MetadataTooLarge,
    #[error("attachment metadata does not match transferred bytes")]
    MetadataMismatch,
    #[error("attachment size arithmetic failed")]
    Arithmetic,
    #[error("attachment exceeded {max} bytes after observing {observed} bytes")]
    TooLarge { observed: u64, max: u64 },
    #[error("attachment transfer timed out")]
    TotalTimeout,
    #[error("attachment transfer made no progress")]
    IdleTimeout,
    #[error("attachment transport failed")]
    Transport,
    #[error("required attachment digest is missing")]
    MissingDigest,
    #[error("required attachment digest is invalid")]
    InvalidDigest,
    #[error("attachment digest mismatch")]
    DigestMismatch,
    #[error("platform does not support attachment download")]
    Unsupported,
}

#[derive(Clone, Copy)]
struct AttachmentPolicy {
    max_bytes: u64,
    total_timeout: Duration,
    idle_timeout: Duration,
}

const PRODUCTION_POLICY: AttachmentPolicy = AttachmentPolicy {
    max_bytes: MAX_ATTACHMENT_BYTES,
    total_timeout: TOTAL_TIMEOUT,
    idle_timeout: IDLE_TIMEOUT,
};

pub fn parse_sha256_hex(value: &str) -> Result<[u8; 32], AttachmentError> {
    if value.is_empty() {
        return Err(AttachmentError::MissingDigest);
    }
    let bytes = hex::decode(value).map_err(|_| AttachmentError::InvalidDigest)?;
    bytes.try_into().map_err(|_| AttachmentError::InvalidDigest)
}

fn validate_declared_size(
    attachment: &Attachment,
    max_bytes: u64,
) -> Result<(), AttachmentError> {
    if attachment.declared_size.is_some_and(|size| size > max_bytes) {
        return Err(AttachmentError::MetadataTooLarge);
    }
    Ok(())
}

fn finish_digest(
    attachment: &Attachment,
    expected_sha256: Option<[u8; 32]>,
    size: u64,
    hasher: Sha256,
) -> Result<VerifiedAttachment, AttachmentError> {
    if attachment.declared_size.is_some_and(|declared| declared != size) {
        return Err(AttachmentError::MetadataMismatch);
    }
    let sha256: [u8; 32] = hasher.finalize().into();
    if expected_sha256.is_some_and(|expected| expected != sha256) {
        return Err(AttachmentError::DigestMismatch);
    }
    Ok(VerifiedAttachment { size, sha256 })
}

pub(crate) async fn consume_stream<S, B, F, Fut>(
    attachment: &Attachment,
    expected_sha256: Option<[u8; 32]>,
    open: F,
) -> Result<VerifiedAttachment, AttachmentError>
where
    S: Stream<Item = Result<B, AttachmentError>> + Send,
    B: AsRef<[u8]>,
    F: FnOnce() -> Fut + Send,
    Fut: Future<Output = Result<S, AttachmentError>> + Send,
{
    consume_stream_with(
        attachment,
        expected_sha256,
        PRODUCTION_POLICY,
        &ATTACHMENT_TRANSFERS,
        open,
    )
    .await
}

async fn consume_stream_with<S, B, F, Fut>(
    attachment: &Attachment,
    expected_sha256: Option<[u8; 32]>,
    policy: AttachmentPolicy,
    semaphore: &Semaphore,
    open: F,
) -> Result<VerifiedAttachment, AttachmentError>
where
    S: Stream<Item = Result<B, AttachmentError>> + Send,
    B: AsRef<[u8]>,
    F: FnOnce() -> Fut + Send,
    Fut: Future<Output = Result<S, AttachmentError>> + Send,
{
    validate_declared_size(attachment, policy.max_bytes)?;
    let _permit = semaphore.try_acquire().map_err(|_| AttachmentError::Busy)?;
    let stop_at = policy
        .max_bytes
        .checked_add(1)
        .ok_or(AttachmentError::Arithmetic)?;

    let transfer = async {
        let stream = timeout(policy.idle_timeout, open())
            .await
            .map_err(|_| AttachmentError::IdleTimeout)??;
        let mut stream = Box::pin(stream);
        let mut size = 0u64;
        let mut hasher = Sha256::new();

        let mut idle_deadline = Instant::now() + policy.idle_timeout;
        loop {
            if Instant::now() >= idle_deadline {
                return Err(AttachmentError::IdleTimeout);
            }
            let next = timeout_at(idle_deadline, stream.as_mut().next())
                .await
                .map_err(|_| AttachmentError::IdleTimeout)?;
            let Some(chunk) = next else { break };
            let chunk = chunk?;
            let bytes = chunk.as_ref();
            if bytes.is_empty() {
                continue;
            }
            idle_deadline = Instant::now() + policy.idle_timeout;
            let chunk_len = u64::try_from(bytes.len()).map_err(|_| AttachmentError::Arithmetic)?;
            let next_size = size
                .checked_add(chunk_len)
                .ok_or(AttachmentError::Arithmetic)?;
            if next_size >= stop_at {
                let accepted = usize::try_from(stop_at - size)
                    .map_err(|_| AttachmentError::Arithmetic)?;
                hasher.update(&bytes[..accepted]);
                return Err(AttachmentError::TooLarge {
                    observed: stop_at,
                    max: policy.max_bytes,
                });
            }
            hasher.update(bytes);
            size = next_size;
        }

        finish_digest(attachment, expected_sha256, size, hasher)
    };

    timeout(policy.total_timeout, transfer)
        .await
        .map_err(|_| AttachmentError::TotalTimeout)?
}

pub(crate) async fn consume_matrix_buffer<Fut, B>(
    attachment: &Attachment,
    expected_sha256: Option<[u8; 32]>,
    fetch: Fut,
) -> Result<VerifiedAttachment, AttachmentError>
where
    Fut: Future<Output = Result<B, AttachmentError>> + Send,
    B: AsRef<[u8]> + Send,
{
    consume_matrix_buffer_with(
        attachment,
        expected_sha256,
        PRODUCTION_POLICY,
        &ATTACHMENT_TRANSFERS,
        fetch,
    )
    .await
}

async fn consume_matrix_buffer_with<Fut, B>(
    attachment: &Attachment,
    expected_sha256: Option<[u8; 32]>,
    policy: AttachmentPolicy,
    semaphore: &Semaphore,
    fetch: Fut,
) -> Result<VerifiedAttachment, AttachmentError>
where
    Fut: Future<Output = Result<B, AttachmentError>> + Send,
    B: AsRef<[u8]> + Send,
{
    validate_declared_size(attachment, policy.max_bytes)?;
    let _permit = semaphore.try_acquire().map_err(|_| AttachmentError::Busy)?;
    let bytes = timeout(policy.total_timeout, fetch)
        .await
        .map_err(|_| AttachmentError::TotalTimeout)??;
    let bytes = bytes.as_ref();
    let size = u64::try_from(bytes.len()).map_err(|_| AttachmentError::Arithmetic)?;
    if size > policy.max_bytes {
        return Err(AttachmentError::TooLarge {
            observed: size,
            max: policy.max_bytes,
        });
    }
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    finish_digest(attachment, expected_sha256, size, hasher)
}
```

- [ ] **Step 4: Export the module from `common/mod.rs`**

```rust
mod attachment;
pub use attachment::*;
pub mod r#trait;
pub use r#trait::*;
pub mod routing;
pub use routing::RoutingAdapter;
```

- [ ] **Step 5: Run focused tests and verify GREEN**

Run: `cargo test --lib adapters::common::attachment::tests -- --test-threads=1`

Expected: 10 tests PASS; timeout tests report distinct `IdleTimeout` and `TotalTimeout`, overflow reports `observed: 5`, transport failure exposes no source context, and no temporary attachment file is created.

- [ ] **Step 6: Commit**

```bash
git add src/adapters/common/attachment.rs src/adapters/common/mod.rs
git commit -m "security: add bounded attachment digest boundary"
```

---

### Task 2: Introduce the Bounded Trait Contract

**Files:**
- Modify: `rust/aegis/src/adapters/common/trait.rs:1-5,143-145`
- Modify: `rust/aegis/src/adapters/common/routing.rs:1-6,74-76`

**Interfaces:**
- Consumes: `Attachment`, `AttachmentError`, and `VerifiedAttachment` from Task 1.
- Produces: `BotAdapter::download_attachment(&Attachment, Option<[u8; 32]>) -> Result<VerifiedAttachment, AttachmentError>`.
- Temporarily preserves: `download_file` only so Tasks 3-4 can migrate each production adapter while every commit compiles; Task 6 removes it and every test override after all callers use the bounded method.

- [ ] **Step 1: Add a compile-time failing object-safe signature test in `trait.rs`**

```rust
#[cfg(test)]
mod attachment_contract_tests {
    use super::*;
    use crate::adapters::common::{Attachment, AttachmentError, VerifiedAttachment};

    async fn invoke_bounded(
        adapter: &dyn BotAdapter,
        attachment: &Attachment,
    ) -> std::result::Result<VerifiedAttachment, AttachmentError> {
        adapter.download_attachment(attachment, None).await
    }

#[test]
fn bounded_attachment_signature_is_object_safe() {
    let _ = invoke_bounded;
}
}
```

- [ ] **Step 2: Run the contract test and verify RED**

Run: `cargo test --lib bounded_attachment_signature_is_object_safe`

Expected: FAIL because `BotAdapter` has no `download_attachment` method.

- [ ] **Step 3: Add the bounded trait method in `trait.rs`**

Add this import:

```rust
use super::{Attachment, AttachmentError, VerifiedAttachment};
```

Add the bounded method immediately before the existing `download_file`; retain that old method through Task 5 only:

```rust
async fn download_attachment(
    &self,
    _attachment: &Attachment,
    _expected_sha256: Option<[u8; 32]>,
) -> std::result::Result<VerifiedAttachment, AttachmentError> {
    Err(AttachmentError::Unsupported)
}
```

Replace the closing comment with:

```rust
// MockBotAdapter is auto-generated by mockall above. Default methods do not
// receive expect_* methods; attachment-focused tests use concrete adapters.
```

- [ ] **Step 4: Forward the exact typed operation in `routing.rs`**

Add `Attachment`, `AttachmentError`, and `VerifiedAttachment` to its common imports, then add this method before the temporarily retained old forwarding method:

```rust
async fn download_attachment(
    &self,
    attachment: &Attachment,
    expected_sha256: Option<[u8; 32]>,
) -> std::result::Result<VerifiedAttachment, AttachmentError> {
    self.primary
        .download_attachment(attachment, expected_sha256)
        .await
}
```

- [ ] **Step 5: Run the contract and routing tests and verify GREEN**

Run: `cargo test --lib adapters::common:: -- --test-threads=1`

Expected: PASS. The bounded method is object-safe and routing forwards it; the temporary old method keeps untouched production adapters and tests compiling until Task 6.

- [ ] **Step 6: Commit**

```bash
git add src/adapters/common/trait.rs src/adapters/common/routing.rs
git commit -m "refactor: add bounded attachment contract"
```

---


### Task 3: Stream Telegram and Discord Through the Shared Boundary

**Files:**
- Modify: `rust/aegis/src/adapters/telegram/adapter.rs:1-12,89-94,134-145`
- Modify: `rust/aegis/src/adapters/discord/adapter.rs:1-8,83-87,94-105`

**Interfaces:**
- Consumes: Task 1's `consume_stream`, constants, typed request/result/error.
- Implements: Task 2's `BotAdapter::download_attachment` for Telegram and Discord.
- Guarantees: both transports stop body polling when the common consumer observes byte `10_485_761`.

- [ ] **Step 1: Add failing Telegram and Discord adapter tests**

Append to `telegram/adapter.rs`:

```rust
#[cfg(test)]
mod attachment_tests {
    use super::*;
    use crate::adapters::common::{Attachment, AttachmentError, MAX_ATTACHMENT_BYTES};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn telegram_rejects_stream_at_common_limit_without_declared_size() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/botTOKEN/getFile"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": {
                    "file_id": "opaque",
                    "file_unique_id": "unique",
                    "file_size": MAX_ATTACHMENT_BYTES,
                    "file_path": "security.bin"
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/file/botTOKEN/security.bin"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(vec![0; (MAX_ATTACHMENT_BYTES + 1) as usize]),
            )
            .mount(&server)
            .await;

        let bot = Bot::new("TOKEN").set_api_url(server.uri().parse().unwrap());
        let adapter = TelegramAdapter::new(bot);
        let error = adapter
            .download_attachment(
                &Attachment {
                    file_id: "opaque".into(),
                    file_name: Some("security.bin".into()),
                    declared_size: None,
                },
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(
            error,
            AttachmentError::TooLarge {
                observed: MAX_ATTACHMENT_BYTES + 1,
                max: MAX_ATTACHMENT_BYTES,
            }
        );
    }
}
```

Replace the existing Discord test module with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::common::{Attachment, AttachmentError, MAX_ATTACHMENT_BYTES};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn discord_capabilities_matches_expected() {
        let caps = PlatformCapabilities::DISCORD;
        assert!(caps.can_edit_message);
        assert!(caps.can_delete_message);
        assert!(!caps.has_file_transfer);
    }

    #[tokio::test]
    async fn discord_rejects_stream_at_common_limit_with_forged_size() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/security.bin"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(vec![0; (MAX_ATTACHMENT_BYTES + 1) as usize]),
            )
            .mount(&server)
            .await;
        let adapter = DiscordAdapter::new(Arc::new(Http::new("TOKEN")));
        let error = adapter
            .download_attachment(
                &Attachment {
                    file_id: format!("{}/security.bin", server.uri()),
                    file_name: Some("security.bin".into()),
                    declared_size: Some(1),
                },
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(
            error,
            AttachmentError::TooLarge {
                observed: MAX_ATTACHMENT_BYTES + 1,
                max: MAX_ATTACHMENT_BYTES,
            }
        );
    }
}
```

- [ ] **Step 2: Run both tests and verify RED**

Run:

```bash
cargo test --lib adapters::telegram::adapter::attachment_tests::telegram_rejects_stream_at_common_limit_without_declared_size
cargo test --lib adapters::discord::adapter::tests::discord_rejects_stream_at_common_limit_with_forged_size
```

Expected: FAIL because neither production adapter implements `download_attachment`; the default returns `Unsupported`.

- [ ] **Step 3: Implement Telegram streaming**

Change Telegram's common import and add `StreamExt`:

```rust
use crate::adapters::common::{
    Attachment, AttachmentError, BotAdapter, Markup, MessageContent, MessageId, Platform,
    PlatformCapabilities, TargetId, VerifiedAttachment, consume_stream,
};
use futures_util::StreamExt;
```

Replace `download_file` with:

```rust
async fn download_attachment(
    &self,
    attachment: &Attachment,
    expected_sha256: Option<[u8; 32]>,
) -> std::result::Result<VerifiedAttachment, AttachmentError> {
    consume_stream(attachment, expected_sha256, || async {
        let file = self
            .bot
            .get_file(&attachment.file_id)
            .await
            .map_err(|_| AttachmentError::Transport)?;
        if file.size != u32::MAX && u64::from(file.size) > MAX_ATTACHMENT_BYTES {
            return Err(AttachmentError::MetadataTooLarge);
        }
        if let (Some(declared), size) = (attachment.declared_size, file.size)
            && size != u32::MAX
            && declared != u64::from(size)
        {
            return Err(AttachmentError::MetadataMismatch);
        }
        Ok(self
            .bot
            .download_file_stream(&file.path)
            .map(|chunk| chunk.map_err(|_| AttachmentError::Transport)))
    })
    .await
}
```

- [ ] **Step 4: Implement Discord streaming without enabling another reqwest feature**

Change Discord's common import and add `futures_util::stream`:

```rust
use crate::adapters::common::{
    Attachment, AttachmentError, BotAdapter, Markup, MessageContent, MessageId, Platform,
    PlatformCapabilities, TargetId, VerifiedAttachment, consume_stream,
};
use futures_util::stream;
```

Replace `download_file` with the existing `Response::chunk()` API, which does not require reqwest's optional `stream` feature:

```rust
async fn download_attachment(
    &self,
    attachment: &Attachment,
    expected_sha256: Option<[u8; 32]>,
) -> std::result::Result<VerifiedAttachment, AttachmentError> {
    consume_stream(attachment, expected_sha256, || async {
        let response = reqwest::get(&attachment.file_id)
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| AttachmentError::Transport)?;
        Ok(stream::try_unfold(response, |mut response| async move {
            response
                .chunk()
                .await
                .map(|chunk| chunk.map(|bytes| (bytes, response)))
                .map_err(|_| AttachmentError::Transport)
        }))
    })
    .await
}
```

- [ ] **Step 5: Run adapter and shared-boundary tests and verify GREEN**

Run:

```bash
cargo test --lib adapters::telegram::adapter::attachment_tests -- --test-threads=1
cargo test --lib adapters::discord::adapter::tests -- --test-threads=1
cargo test --lib adapters::common::attachment::tests -- --test-threads=1
```

Expected: all tests PASS. Both over-limit adapter tests return `observed == 10_485_761`; no error contains the Telegram identifier, Discord URL, response body, or digest.

- [ ] **Step 6: Commit**

```bash
git add src/adapters/telegram/adapter.rs src/adapters/discord/adapter.rs
git commit -m "security: stream bounded Telegram and Discord attachments"
```

---



### Task 4: Apply the Narrow Matrix SDK Buffer Limit

**Files:**
- Modify: `rust/aegis/src/adapters/matrix/adapter.rs:1-10,50-121,190-202`

**Interfaces:**
- Consumes: Task 1's `consume_matrix_buffer` and Task 2's bounded trait method.
- Produces: private `download_matrix_with`, which places semaphore acquisition, declared-size preflight, and total timeout around the matrix-sdk future.
- Explicit limitation: matrix-sdk 0.18 `Media::get_media_content` returns `Result<Vec<u8>>`; no chunk stream or progress callback exists.

- [ ] **Step 1: Add a failing Matrix adapter regression test**

Append inside `matrix_adapter_tests`:

```rust
#[tokio::test]
async fn matrix_sdk_buffer_uses_the_common_ten_mib_limit() {
    use crate::adapters::common::{Attachment, AttachmentError, MAX_ATTACHMENT_BYTES};

    let attachment = Attachment {
        file_id: "mxc://example.org/security".into(),
        file_name: Some("security.bin".into()),
        declared_size: None,
    };
    let error = super::download_matrix_with(&attachment, None, async {
        Ok(vec![0; (MAX_ATTACHMENT_BYTES + 1) as usize])
    })
    .await
    .unwrap_err();
    assert_eq!(
        error,
        AttachmentError::TooLarge {
            observed: MAX_ATTACHMENT_BYTES + 1,
            max: MAX_ATTACHMENT_BYTES,
        }
    );
}
```

- [ ] **Step 2: Run the test and verify RED**

Run: `cargo test --lib adapters::matrix::adapter::matrix_adapter_tests::matrix_sdk_buffer_uses_the_common_ten_mib_limit`

Expected: FAIL because `download_matrix_with` is undefined and Matrix still returns the SDK's `Vec<u8>`.

- [ ] **Step 3: Add the narrow future seam and bounded implementation**

Change the common import and add `Future`:

```rust
use crate::adapters::common::{
    Attachment, AttachmentError, BotAdapter, Markup, MessageContent, MessageId, Platform,
    PlatformCapabilities, TargetId, VerifiedAttachment, consume_matrix_buffer,
};
use std::future::Future;
```

Add before the `BotAdapter for MatrixAdapter` implementation:

```rust
async fn download_matrix_with<Fut>(
    attachment: &Attachment,
    expected_sha256: Option<[u8; 32]>,
    fetch: Fut,
) -> std::result::Result<VerifiedAttachment, AttachmentError>
where
    Fut: Future<Output = std::result::Result<Vec<u8>, AttachmentError>> + Send,
{
    consume_matrix_buffer(attachment, expected_sha256, fetch).await
}
```

Replace `download_file` with:

```rust
async fn download_attachment(
    &self,
    attachment: &Attachment,
    expected_sha256: Option<[u8; 32]>,
) -> std::result::Result<VerifiedAttachment, AttachmentError> {
    use matrix_sdk::media::MediaRequestParameters;
    use matrix_sdk::ruma::OwnedMxcUri;
    use matrix_sdk::ruma::events::room::MediaSource;

    let client = self.room.client();
    let request = MediaRequestParameters {
        source: MediaSource::Plain(OwnedMxcUri::from(attachment.file_id.clone())),
        format: matrix_sdk::media::MediaFormat::File,
    };
    download_matrix_with(attachment, expected_sha256, async move {
        client
            .media()
            .get_media_content(&request, false)
            .await
            .map_err(|_| AttachmentError::Transport)
    })
    .await
}
```

- [ ] **Step 4: Run Matrix and common tests and verify GREEN**

Run:

```bash
cargo test --lib adapters::matrix::adapter::matrix_adapter_tests -- --test-threads=1
cargo test --lib adapters::common::attachment::tests::matrix_buffer_rejects_immediately_after_sdk_return
```

Expected: PASS. The Matrix adapter returns the same `10_485_760` byte policy and typed error as the other adapters, but the test does not claim network cancellation at `MAX + 1` or idle-progress observation.

- [ ] **Step 5: Commit**

```bash
git add src/adapters/matrix/adapter.rs
git commit -m "security: bound Matrix SDK attachment buffers"
```

---


### Task 5: Preserve Declared Size at Every Platform Event Boundary

**Files:**
- Modify: `rust/aegis/src/main/runtime.rs:160-213,253-278`
- Modify: `rust/aegis/src/main/discord.rs:1-12,142-165`
- Modify: `rust/aegis/src/shared/types.rs:73-81,137-151`
- Modify: `rust/aegis/src/shared/dispatch.rs` - update test event literals only.
- Modify: `rust/aegis/src/shared/destruct.rs` - update test event literals only.
- Modify: `rust/aegis/src/shared/boundary.rs` - update test event literal only.

**Interfaces:**
- Consumes: `Attachment` from Task 1 and introduces `MessageEvent::attachment`.
- Produces: `None` when Matrix size metadata is absent or Telegram uses its `u32::MAX` fallback; absence never blocks later streamed enforcement.
- Produces: Discord's mandatory `u32` size as `Some(u64)`.
- Transitional invariant: old split fields remain only through this commit so unchanged security callers compile; every producer sets them to `None`, and Task 6 deletes them with the old API.

- [ ] **Step 1: Add a failing atomic attachment metadata test in `shared/types.rs`**

```rust
#[test]
fn message_event_carries_atomic_attachment_metadata() {
    let event = MessageEvent {
        adapter: std::sync::Arc::new(crate::adapters::common::MockBotAdapter::new()),
        target: TargetId("123".into()),
        principal: Principal::telegram(42),
        text: None,
        file_id: None,
        file_name: None,
        attachment: Some(Attachment {
            file_id: "opaque".into(),
            file_name: Some("security.bin".into()),
            declared_size: Some(4),
        }),
        reply_to_text: None,
    };
    assert_eq!(event.attachment.unwrap().declared_size, Some(4));
}
```

Run: `cargo test --lib shared::types::event_tests::message_event_carries_atomic_attachment_metadata`

Expected: FAIL because current `MessageEvent` has no `attachment` field.

- [ ] **Step 2: Add the compile-safe transitional event field and update all existing literals**

Change the common import in `shared/types.rs` to include `Attachment`, then use this transitional shape for one commit:

```rust
pub struct MessageEvent {
    pub adapter: Arc<dyn BotAdapter>,
    pub target: TargetId,
    pub principal: Principal,
    pub text: Option<String>,
    pub file_id: Option<String>,
    pub file_name: Option<String>,
    pub attachment: Option<Attachment>,
    pub reply_to_text: Option<String>,
}
```

In every existing `MessageEvent` literal in `shared/types.rs`, `shared/dispatch.rs`, `shared/destruct.rs`, and `shared/boundary.rs`, insert this exact field after `file_name`:

```rust
attachment: None,
```

- [ ] **Step 3: Replace Matrix extraction in `main/runtime.rs`**

Add `Attachment` to the existing `aegis::adapters::common` import. Replace `extract_media_info` and the following match with:

```rust
fn matrix_attachment(
    source: &MediaSource,
    filename: &str,
    declared_size: Option<u64>,
) -> Attachment {
    let file_id = match source {
        MediaSource::Plain(url) => url.to_string(),
        MediaSource::Encrypted(info) => info.url.to_string(),
    };
    Attachment {
        file_id,
        file_name: Some(filename.to_string()),
        declared_size,
    }
}

let attachment = match &event.content.msgtype {
    MessageType::Audio(c) => Some(matrix_attachment(
        &c.source,
        c.filename(),
        c.info.as_ref().and_then(|info| info.size.map(u64::from)),
    )),
    MessageType::File(c) => Some(matrix_attachment(
        &c.source,
        c.filename(),
        c.info.as_ref().and_then(|info| info.size.map(u64::from)),
    )),
    MessageType::Image(c) => Some(matrix_attachment(
        &c.source,
        c.filename(),
        c.info.as_ref().and_then(|info| info.size.map(u64::from)),
    )),
    MessageType::Video(c) => Some(matrix_attachment(
        &c.source,
        c.filename(),
        c.info.as_ref().and_then(|info| info.size.map(u64::from)),
    )),
    _ => None,
};
```

Construct the Matrix message event with:

```rust
BotEvent::Message(MessageEvent {
    adapter: adapter.clone(),
    target: target.clone(),
    principal,
    text: Some(text),
    file_id: None,
    file_name: None,
    attachment,
    reply_to_text: None,
})
```

- [ ] **Step 4: Replace Telegram extraction in `main/runtime.rs`**

Add this module-level normalizer immediately before `run`:

```rust
fn telegram_declared_size(size: u32) -> Option<u64> {
    (size != u32::MAX).then_some(u64::from(size))
}
```

Inside `handle_message`, construct the attachment before the event:

```rust
let attachment = msg
    .document()
    .map(|document| Attachment {
        file_id: document.file.id.clone(),
        file_name: document.file_name.clone(),
        declared_size: telegram_declared_size(document.file.size),
    })
    .or_else(|| {
        msg.photo().and_then(|photos| {
            photos.last().map(|photo| Attachment {
                file_id: photo.file.id.clone(),
                file_name: Some(rust_i18n::t!("destruct.image_label").to_string()),
                declared_size: telegram_declared_size(photo.file.size),
            })
        })
    });
```

Replace the old file fields in the Telegram event with:

```rust
file_id: None,
file_name: None,
attachment,
```

Add this test module after `run`:

```rust
#[cfg(test)]
mod attachment_metadata_tests {
    use super::telegram_declared_size;

#[test]
fn telegram_missing_size_fallback_stays_untrusted() {
    assert_eq!(telegram_declared_size(u32::MAX), None);
    assert_eq!(telegram_declared_size(42), Some(42));
}
}
```

- [ ] **Step 5: Replace Discord extraction in `main/discord.rs`**

Add `Attachment` to the existing common import. Replace the tuple extraction and event construction with:

```rust
let attachment = msg.attachments.first().map(|attachment| Attachment {
    file_id: attachment.url.to_string(),
    file_name: Some(attachment.filename.clone()),
    declared_size: Some(u64::from(attachment.size)),
});
let event = BotEvent::Message(MessageEvent {
    adapter: self.adapter.clone(),
    target: aegis::adapters::common::TargetId(self.admin_channel.to_string()),
    principal: Principal::discord(msg.author.id.get()),
    text,
    file_id: None,
    file_name: None,
    attachment,
    reply_to_text: None,
});
```

- [ ] **Step 6: Run producer and constructor checks and verify GREEN**

Run:

```bash
cargo test --all-features telegram_missing_size_fallback_stays_untrusted
cargo test --lib shared::types::event_tests
cargo check --all-features
```

Expected: both test targets PASS and `cargo check` exits 0. Matrix's `info: None` and Telegram's fallback produce `declared_size: None`; Discord preserves its declared size.

- [ ] **Step 7: Commit**

```bash
git add src/main/runtime.rs src/main/discord.rs src/shared/types.rs src/shared/dispatch.rs src/shared/destruct.rs src/shared/boundary.rs
git commit -m "refactor: carry attachment size metadata"
```

---


### Task 6: Consume Only Validated Digests in Dispatch and Self-Destruct

**Files:**
- Modify: `rust/aegis/src/shared/dispatch.rs:1-14,106-191,197-320`
- Modify: `rust/aegis/src/shared/destruct.rs:1-22,35-79,122-129,202-285,601-724`
- Modify: `rust/aegis/src/shared/types.rs:73-81,137-151`
- Modify: `rust/aegis/src/adapters/common/trait.rs:143-154`
- Modify: `rust/aegis/src/adapters/common/routing.rs:74-86`
- Modify: `rust/aegis/src/shared/commands.rs:123-158,289-315`
- Modify: `rust/aegis/src/app/state.rs:559-590`
- Modify: `rust/aegis/src/core/system/scheduler/mod.rs:423-457`
- Modify: `rust/aegis/src/shared/boundary.rs:186-194`
- Modify: `rust/aegis/src/main/runtime.rs` and `rust/aegis/src/main/discord.rs` - remove transitional split fields.

**Interfaces:**
- Consumes: `MessageEvent::attachment`, `download_attachment`, `VerifiedAttachment`, `parse_sha256_hex`, and `AttachmentError`.
- Produces: persisted security-file hashes only from a successful bounded result.
- Produces: self-destruct progress only after the bounded operation matches the configured digest.
- Removes: every security caller's access to `Vec<u8>` and every second-pass SHA-256 over body bytes.
- Removes: the compile-safe Task 5 transition (`download_file`, `MessageEvent.file_id`, and `MessageEvent.file_name`) in the same GREEN commit.

- [ ] **Step 1: Rewrite the dispatch test adapter and strengthen the existing failing test**

In `dispatch_security_file_tests`, import `Attachment`, `AttachmentError`, and `VerifiedAttachment`. Replace `TestAdapter::download_file` with:

```rust
async fn download_attachment(
    &self,
    attachment: &Attachment,
    expected_sha256: Option<[u8; 32]>,
) -> std::result::Result<VerifiedAttachment, AttachmentError> {
    assert!(expected_sha256.is_none());
    Ok(VerifiedAttachment::from_test_bytes(attachment.file_id.as_bytes()))
}
```

Replace the attachment fields in `file_captured_when_pending_sets_hash` with:

```rust
file_id: None,
file_name: None,
attachment: Some(Attachment {
    file_id: "test-file".into(),
    file_name: Some("test.txt".into()),
    declared_size: None,
}),
```

Strengthen its final assertion to:

```rust
assert_eq!(
    state.self_destruct_key_hash().await,
    Some(hex::encode(sha2::Sha256::digest(b"test-file")))
);
```

- [ ] **Step 2: Run the dispatch test and verify RED**

Run: `cargo test --lib shared::dispatch::dispatch_security_file_tests::file_captured_when_pending_sets_hash -- --test-threads=1`

Expected: FAIL because production still calls the removed `download_file` method and hashes body bytes itself.

- [ ] **Step 3: Replace dispatch's full-buffer branch**

Remove `use sha2::Digest;`. Add `AttachmentError` to the common import. In `handle_message`, change the presence flag to:

```rust
msg.attachment.is_some(),
```

Replace the complete security-file branch with:

```rust
let file_timeout = Duration::from_secs(180);
if let Some(ref attachment) = msg.attachment
    && state
        .take_security_file_input_status(&msg.target.0, file_timeout)
        .await
        == TimeoutStatus::Active
{
    let verified = match msg.adapter.download_attachment(attachment, None).await {
        Ok(verified) => verified,
        Err(AttachmentError::MetadataTooLarge)
        | Err(AttachmentError::TooLarge { .. }) => {
            msg.adapter
                .send_message(
                    &msg.target,
                    MessageContent {
                        text: rust_i18n::t!(
                            "bot_commands.file_too_big",
                            "0" => crate::adapters::common::MAX_ATTACHMENT_BYTES + 1,
                            "1" => crate::adapters::common::MAX_ATTACHMENT_BYTES
                        )
                        .into(),
                        markup: None,
                    },
                )
                .await?;
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    let hash = verified.sha256_hex();
    if let Err(error) = crate::bootstrap::save_self_destruct_key_hash_to_config(Some(hash.clone()))
    {
        log::error!("保存安全文件雜湊失敗: {}", error);
    } else {
        state.set_self_destruct_key_hash(Some(hash)).await;
    }
    let file_display = attachment
        .file_name
        .as_ref()
        .map(|name| format!("{} | {}", name, verified.hash_prefix()))
        .unwrap_or_else(|| verified.hash_prefix());
    msg.adapter
        .send_message(
            &msg.target,
            MessageContent {
                text: rust_i18n::t!("bot_commands.security_file_set", "0" => file_display).into(),
                markup: None,
            },
        )
        .await?;
    return Ok(());
}
```

- [ ] **Step 4: Rewrite self-destruct's pure digest input and test**

Remove `use sha2::Digest;`. Change the final argument of `process_destruct_message` from `file_content: Option<&[u8]>` to:

```rust
file_sha256: Option<&str>,
```

Replace only its `AwaitSecurityFile` arm with:

```rust
DestructStatus::AwaitSecurityFile => {
    if let Some(hash) = file_sha256 {
        match self_destruct_key_hash {
            Some(correct) if hash == correct => DestructMessageAction::FileVerified {
                hash_short: if hash.len() == 64 {
                    format!("{}..{}", &hash[..8], &hash[60..])
                } else {
                    return DestructMessageAction::FileMismatch;
                },
            },
            Some(_) => DestructMessageAction::FileMismatch,
            None => DestructMessageAction::NoSecurityKey,
        }
    } else {
        DestructMessageAction::AwaitingFile
    }
}
```

Update `security_file_match_returns_file_verified` so its final argument is the already-computed digest:

```rust
Some(&hash),
```

- [ ] **Step 5: Run the pure self-destruct test and verify GREEN for the refactor seam**

Run: `cargo test --lib shared::destruct::tests::security_file_match_returns_file_verified`

Expected: PASS without passing file bytes into `process_destruct_message`.

- [ ] **Step 6: Add a failing self-destruct mismatch integration test**

Add `Attachment`, `AttachmentError`, and `VerifiedAttachment` to the test imports. Implement this method on the existing test `MockAdapter`:

```rust
async fn download_attachment(
    &self,
    _attachment: &Attachment,
    expected_sha256: Option<[u8; 32]>,
) -> std::result::Result<VerifiedAttachment, AttachmentError> {
    assert_eq!(expected_sha256, Some([0; 32]));
    Err(AttachmentError::DigestMismatch)
}
```

Change `make_test_state`'s configured hash from `"test-hash"` to `"00".repeat(32)`, then add:

```rust
#[tokio::test]
async fn mismatched_bounded_digest_does_not_advance_self_destruct() {
    let secret = TotpManager::generate_new_secret();
    let state = make_test_state(&secret).await;
    state.begin_destruct(&test_key(), Instant::now()).await.unwrap();
    assert!(
        state
            .advance_destruct_step(
                &test_key(),
                DestructStatus::AwaitFirstTotp,
                DestructStatus::AwaitFirstConfirm,
            )
            .await
    );
    assert!(
        state
            .advance_destruct_step(
                &test_key(),
                DestructStatus::AwaitFirstConfirm,
                DestructStatus::AwaitSecondTotp,
            )
            .await
    );
    assert!(
        state
            .advance_destruct_step(
                &test_key(),
                DestructStatus::AwaitSecondTotp,
                DestructStatus::AwaitSecurityFile,
            )
            .await
    );
    let message = MessageEvent {
        adapter: Arc::new(MockAdapter),
        target: TargetId("42".into()),
        principal: Principal::telegram(42),
        text: None,
        file_id: None,
        file_name: None,
        attachment: Some(Attachment {
            file_id: "opaque".into(),
            file_name: Some("security.bin".into()),
            declared_size: None,
        }),
        reply_to_text: None,
    };

    assert_eq!(intercept_message(&message, &state).await.unwrap(), FlowOutcome::Handled);
    assert_eq!(
        state.destruct_snapshot(&test_key()).await.unwrap().status,
        DestructStatus::AwaitSecurityFile
    );
}
```

- [ ] **Step 7: Run the mismatch test and verify RED**

Run: `cargo test --lib shared::destruct::tests::mismatched_bounded_digest_does_not_advance_self_destruct`

Expected: FAIL because `intercept_message` still calls the removed full-buffer API.

- [ ] **Step 8: Replace the self-destruct attachment branch**

Add these common imports:

```rust
use aegis::adapters::common::{
    AttachmentError, DestructKey, InlineButton, Markup, MessageContent, parse_sha256_hex,
};
```

Replace the complete `(AwaitSecurityFile, AwaitingFile)` branch with:

```rust
(DestructStatus::AwaitSecurityFile, DestructMessageAction::AwaitingFile) => {
    if let Some(attachment) = msg.attachment.as_ref() {
        let configured = state.self_destruct_key_hash().await;
        let action = match configured.as_deref() {
            None => DestructMessageAction::NoSecurityKey,
            Some(hash) => {
                let expected = parse_sha256_hex(hash)?;
                match adapter
                    .download_attachment(attachment, Some(expected))
                    .await
                {
                    Ok(verified) => {
                        let actual = verified.sha256_hex();
                        process_destruct_message(
                            None,
                            DestructStatus::AwaitSecurityFile,
                            state,
                            Some(hash),
                            Some(&actual),
                        )
                        .await
                    }
                    Err(AttachmentError::DigestMismatch) => DestructMessageAction::FileMismatch,
                    Err(error) => return Err(error.into()),
                }
            }
        };

        match action {
            DestructMessageAction::FileVerified { ref hash_short } => {
                let file_display = attachment
                    .file_name
                    .as_ref()
                    .map(|name| format!("{} | {}", name, hash_short))
                    .unwrap_or_else(|| hash_short.clone());
                if state
                    .advance_destruct_step(
                        &key,
                        DestructStatus::AwaitSecurityFile,
                        DestructStatus::AwaitFinalConfirm,
                    )
                    .await
                {
                    let keyboard = Markup {
                        buttons: vec![
                            vec![btn(t!("destruct.final_btn").as_ref(), "a_destroy_final")],
                            vec![btn(t!("destruct.cancelled").as_ref(), "a_destroy_cancel")],
                        ],
                    };
                    adapter
                        .send_message(
                            target,
                            MessageContent {
                                text: t!("destruct.file_verify_ok", "0" => file_display).into(),
                                markup: Some(keyboard),
                            },
                        )
                        .await?;
                }
            }
            DestructMessageAction::FileMismatch => {
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: t!("destruct.file_verify_fail").into(),
                            markup: None,
                        },
                    )
                    .await?;
            }
            DestructMessageAction::NoSecurityKey => {
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: t!("destruct.no_security_file").into(),
                            markup: None,
                        },
                    )
                    .await?;
            }
            _ => {}
        }
    } else {
        adapter
            .send_message(
                target,
                MessageContent {
                    text: t!("destruct.file_send_prompt").into(),
                    markup: None,
                },
            )
            .await?;
    }
}
```

- [ ] **Step 9: Run both caller suites and verify GREEN**

Run:

```bash
cargo test --lib shared::dispatch::dispatch_security_file_tests -- --test-threads=1
cargo test --lib shared::destruct::tests -- --test-threads=1
```

Expected: all tests PASS. Setting a security file persists the digest returned by the bounded boundary; mismatch leaves self-destruct at `AwaitSecurityFile`.

- [ ] **Step 10: Delete the transitional API and split event fields**

Replace `MessageEvent` with its final shape:

```rust
pub struct MessageEvent {
    pub adapter: Arc<dyn BotAdapter>,
    pub target: TargetId,
    pub principal: Principal,
    pub text: Option<String>,
    pub attachment: Option<Attachment>,
    pub reply_to_text: Option<String>,
}
```

Delete the old default method from `BotAdapter` and the old forwarding method from `RoutingAdapter`:

```rust
async fn download_file(&self, _file_id: &str) -> Result<Vec<u8>> {
    anyhow::bail!("platform does not support file download")
}
```

Delete every remaining test override with that signature from both adapters in `shared/commands.rs`, the general adapter in `shared/dispatch.rs`, the adapter in `shared/destruct.rs`, `app/state.rs`, and `core/system/scheduler/mod.rs`. Delete every transitional pair from event literals in `shared/types.rs`, `shared/dispatch.rs`, `shared/destruct.rs`, `shared/boundary.rs`, `main/runtime.rs`, and `main/discord.rs`:

```rust
file_id: None,
file_name: None,
```

The resulting literals retain only their existing `attachment` field.

- [ ] **Step 11: Prove callers cannot receive an unvalidated body**

Run:

```bash
cargo check --all-features
grep -R -nE --include='*.rs' "download_file\(|Result<Vec<u8>>|Sha256::digest\(&content\)|file_content" src/adapters src/shared/dispatch.rs src/shared/destruct.rs
```

Expected: `cargo check` exits 0. The only `Result<Vec<u8>>` match is the explicitly narrow matrix-sdk future seam in `adapters/matrix/adapter.rs`; there are no `download_file` trait/caller matches, no caller-side body hash, and no `file_content` parameter.

- [ ] **Step 12: Commit**

```bash
git add src/adapters/common/trait.rs src/adapters/common/routing.rs src/shared/dispatch.rs src/shared/destruct.rs src/shared/types.rs src/shared/commands.rs src/shared/boundary.rs src/main/runtime.rs src/main/discord.rs src/app/state.rs src/core/system/scheduler/mod.rs
git commit -m "security: consume only verified attachment digests"
```

---

### Task 7: Full Security and Rust Gates

**Files:**
- Verify only: all files listed in this plan.

**Interfaces:**
- Consumes: completed Tasks 1-6.
- Produces: evidence that the final branch satisfies the approved bounded-download design without hiding Matrix's SDK limitation.

- [ ] **Step 1: Run the complete focused attachment matrix**

```bash
cargo test --lib adapters::common::attachment::tests -- --test-threads=1
cargo test --lib adapters::telegram::adapter::attachment_tests -- --test-threads=1
cargo test --lib adapters::discord::adapter::tests -- --test-threads=1
cargo test --lib adapters::matrix::adapter::matrix_adapter_tests -- --test-threads=1
cargo test --lib shared::dispatch::dispatch_security_file_tests -- --test-threads=1
cargo test --lib shared::destruct::tests -- --test-threads=1
```

Expected: every command exits 0. Evidence covers absent and forged size, exact `MAX + 1` stream stop, checked arithmetic, digest mismatch, total timeout, idle timeout, immediate semaphore rejection, all three adapters' common limit, and both security callers.

- [ ] **Step 2: Run source-level invariant scans**

```bash
if grep -R -n --include='*.rs' "download_file(" src/adapters src/shared; then exit 1; fi
test "$(grep -R -n --include='*.rs' "Result<Vec<u8>>" src/adapters src/shared/dispatch.rs src/shared/destruct.rs | wc -l)" -eq 1
grep -nE "ATTACHMENT_TRANSFERS|try_acquire|checked_add|MAX_ATTACHMENT_BYTES" src/adapters/common/attachment.rs
grep -R -nE --include='*.rs' "file_id:|file_name:" src/shared src/main
```

Expected:
- First command: no matches.
- Second command: one Matrix future-bound match only; no trait, routing, dispatch, or destruct match.
- Third command: matches the single shared semaphore, immediate rejection, checked arithmetic, and common limit.
- Fourth command: matches fields inside `Attachment` construction only; no split `MessageEvent` fields.

- [ ] **Step 3: Run mandatory Rust quality gates from `rust/aegis`**

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Expected: all commands exit 0, clippy emits no warnings, and the full test suite passes.

- [ ] **Step 4: Request both required reviews before branch completion**

Use `superpowers:requesting-code-review` twice against this plan and the approved design:

```text
Review 1 - specification compliance: verify every approved requirement has executable evidence, especially MAX+1, timeout distinction, semaphore policy, cleanup, metadata forgery, three adapters, caller result types, and explicit Matrix residual risk.
Review 2 - code quality: verify cancellation behavior, checked arithmetic, no sensitive error context, no accidental full buffering outside Matrix, and no flaky timeout/concurrency tests.
```

Expected: no Critical or Important findings. Any such finding blocks completion and is fixed with a new RED -> GREEN cycle before rerunning Steps 1-3.

- [ ] **Step 5: Record residual risk in the pull request, not as a false guarantee**

Use this exact residual-risk text:

```text
Residual risk: matrix-sdk 0.18 exposes Media::get_media_content as Result<Vec<u8>> and internally buffers the complete response; encrypted media may additionally allocate a decrypted Vec. Aegis preflights optional event size, applies the shared semaphore and 30-second total timeout around the SDK future, and rejects an oversized returned Vec before hashing. Unlike Telegram and Discord, Matrix cannot stop the network/body allocation at MAX+1 and cannot implement a true idle-progress timeout until the SDK exposes a streaming/progress API. This limitation is explicit and is not represented as streaming enforcement.
```

- [ ] **Step 6: Final commit only if review fixes changed code**

If review fixes changed code after Task 6, stage only those exact files and commit:

```bash
git add src/adapters src/shared src/main src/app/state.rs src/core/system/scheduler/mod.rs
git commit -m "fix: close bounded attachment review findings"
```

If review produced no code changes, do not create an empty commit.

## Plan Self-Review

- **Specification coverage:** Tasks 1, 3, and 4 cover the bounded sink, checked counting, streaming digest, exact stream stop, timeouts, shared rejection policy, three adapters, and Matrix limitation. Task 5 covers present/absent platform metadata. Task 6 covers digest validation and removal of caller buffers. Task 7 covers full gates and residual risk.
- **Failure cleanup:** The selected bounded sink never creates a file. Every error drops in-flight chunks, hasher state, response/stream future, Matrix buffer, and semaphore permit; therefore no temporary residue exists to unlink.
- **Sensitive errors:** `AttachmentError` variants are static except bounded counts. Adapter transport errors are deliberately collapsed and never include URLs, identifiers, content, or digest values.
- **Placeholder scan:** Every code-changing step contains exact code or an exact deletion block; every command includes an expected result; no implementation placeholder remains.
- **Type consistency:** All tasks use `Attachment`, `AttachmentError`, `VerifiedAttachment`, `Option<[u8; 32]>`, and `MessageEvent::attachment` with the signatures established in Tasks 1-2. Routing and all three adapters implement the same trait method.
- **Execution order:** Execute numerically: Task 1, Task 2, Task 3, Task 4, Task 5, Task 6, Task 7.
