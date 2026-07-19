#![allow(dead_code)]

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
        format!(
            "{}..{}",
            hex::encode(&self.sha256[..4]),
            hex::encode(&self.sha256[30..])
        )
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

fn validate_declared_size(attachment: &Attachment, max_bytes: u64) -> Result<(), AttachmentError> {
    if attachment
        .declared_size
        .is_some_and(|size| size > max_bytes)
    {
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
    if attachment
        .declared_size
        .is_some_and(|declared| declared != size)
    {
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
                let accepted =
                    usize::try_from(stop_at - size).map_err(|_| AttachmentError::Arithmetic)?;
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
        let chunks =
            stream::iter([b"1234".to_vec(), b"5678".to_vec(), b"9".to_vec()]).map(move |chunk| {
                seen.fetch_add(1, Ordering::SeqCst);
                Ok::<_, AttachmentError>(chunk)
            });
        let error = consume_stream_with(
            &attachment(Some(1)),
            None,
            policy(4),
            &Semaphore::new(1),
            || async { Ok(chunks) },
        )
        .await
        .unwrap_err();
        assert_eq!(
            error,
            AttachmentError::TooLarge {
                observed: 5,
                max: 4
            }
        );
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
                Ok(stream::iter([Err::<Vec<u8>, _>(
                    AttachmentError::Transport,
                )]))
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
        let error = consume_stream_with(&attachment(None), None, policy(8), &semaphore, || async {
            Ok(stream::empty::<Result<Vec<u8>, AttachmentError>>())
        })
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
        assert_eq!(
            error,
            AttachmentError::TooLarge {
                observed: 9,
                max: 4
            }
        );
    }
}
