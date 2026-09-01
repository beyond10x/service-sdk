//! Opaque references returned by content staging adapters.

use serde::Serialize;
use thiserror::Error;

use crate::{BoxFuture, IdempotencyKey, ServiceId, VerifiedAuthContext};

/// A validated content-object identifier.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ContentId(String);

impl ContentId {
    /// Creates a content identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, ContentMetadataError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(ContentMetadataError::EmptyContentId);
        }
        Ok(Self(value))
    }

    /// Returns the content identifier as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A validated media type.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct MediaType(String);

impl MediaType {
    /// Creates a media type using the required `type/subtype` form.
    pub fn new(value: impl Into<String>) -> Result<Self, ContentMetadataError> {
        let value = value.into();
        let (top_level, subtype) = value.split_once('/').unwrap_or_default();
        if top_level.trim().is_empty()
            || subtype.trim().is_empty()
            || top_level.chars().any(char::is_whitespace)
            || subtype.chars().any(char::is_whitespace)
            || subtype.contains('/')
        {
            return Err(ContentMetadataError::InvalidMediaType(value));
        }
        Ok(Self(value))
    }

    /// Returns the media type as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A validated SHA-256 content digest.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ContentDigest(String);

impl ContentDigest {
    /// Creates a digest in `sha256:<64 lowercase hexadecimal characters>` form.
    pub fn sha256(value: impl Into<String>) -> Result<Self, ContentMetadataError> {
        let value = value.into();
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err(ContentMetadataError::InvalidSha256Digest(value));
        };
        if hex.len() != 64
            || !hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(ContentMetadataError::InvalidSha256Digest(value));
        }
        Ok(Self(value))
    }

    /// Returns the digest as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Metadata validation failed while an adapter was constructing a staged reference.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ContentMetadataError {
    /// Content identifiers cannot be empty.
    #[error("content identifier must not be empty")]
    EmptyContentId,
    /// Media types must use a non-empty `type/subtype` form without whitespace.
    #[error("invalid media type {0}")]
    InvalidMediaType(String),
    /// Digests must use canonical lowercase SHA-256 form.
    #[error("invalid SHA-256 digest {0}")]
    InvalidSha256Digest(String),
}

/// An opaque content reference that may safely be included in a semantic command or event.
///
/// `ContentRef` intentionally implements [`Serialize`] but not `Deserialize`: callers cannot turn
/// untrusted request JSON into a trusted content reference. Its serialized representation contains
/// only the opaque identifier: plaintext and staging metadata cannot leak through this type.
///
/// ```compile_fail
/// use service_runtime::ContentRef;
///
/// let _: ContentRef = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ContentRef {
    id: ContentId,
}

impl ContentRef {
    /// Returns the opaque object identifier.
    pub fn id(&self) -> &ContentId {
        &self.id
    }
}

/// Adapter-minted staged content awaiting append outcome.
///
/// This type does not implement [`Serialize`]. A generated command extracts only
/// [`Self::reference`], and the lifecycle token is consumed exactly once by either acceptance or
/// abandonment after the append result is known.
#[derive(Debug, Eq, PartialEq)]
pub struct StagedContent {
    reference: ContentRef,
    media_type: MediaType,
    digest: ContentDigest,
    size_bytes: u64,
}

impl StagedContent {
    /// Creates the trusted result of durable staging and metadata verification.
    pub fn after_staging(
        id: ContentId,
        media_type: MediaType,
        digest: ContentDigest,
        size_bytes: u64,
    ) -> Self {
        Self {
            reference: ContentRef { id },
            media_type,
            digest,
            size_bytes,
        }
    }

    /// Returns the only value allowed into a semantic command or event.
    pub fn reference(&self) -> &ContentRef {
        &self.reference
    }

    /// Returns the verified media type.
    pub fn media_type(&self) -> &MediaType {
        &self.media_type
    }

    /// Returns the verified content digest.
    pub fn digest(&self) -> &ContentDigest {
        &self.digest
    }

    /// Returns the staged byte count.
    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }
}

/// Untrusted content bytes presented to a staging adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContentPayload<'a> {
    /// Declared media type, to be checked by the adapter.
    pub media_type: &'a str,
    /// Untrusted bytes to stage.
    pub bytes: &'a [u8],
}

/// Idempotent request presented to a content staging adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContentStageRequest<'a> {
    /// Stable service partition that will own the reference.
    pub service: &'a ServiceId,
    /// Verified tenant and exact optional realm partition.
    pub context: &'a VerifiedAuthContext,
    /// Mutation key that identifies retries of the same staging operation.
    pub idempotency_key: &'a IdempotencyKey,
    /// Untrusted bytes and declared metadata to verify and stage.
    pub payload: ContentPayload<'a>,
}

/// Request to accept staged content after its referencing append succeeds or replays.
#[derive(Debug, Eq, PartialEq)]
pub struct ContentAcceptRequest<'a> {
    /// Stable service partition that owns the reference.
    pub service: &'a ServiceId,
    /// Verified tenant and exact optional realm partition.
    pub context: &'a VerifiedAuthContext,
    /// Mutation key that made staging and append idempotent.
    pub idempotency_key: &'a IdempotencyKey,
    /// Staged lifecycle token consumed by acceptance.
    pub staged: StagedContent,
}

/// Why staged content is being abandoned without a referencing append.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentAbandonReason {
    /// Command construction, validation, or decision refused the intent after staging.
    IntentRefused,
    /// The guarded append did not commit or replay the referencing event.
    AppendFailed,
}

/// Request to abandon staged content when no event reference committed.
#[derive(Debug, Eq, PartialEq)]
pub struct ContentAbandonRequest<'a> {
    /// Stable service partition that would have owned the reference.
    pub service: &'a ServiceId,
    /// Verified tenant and exact optional realm partition.
    pub context: &'a VerifiedAuthContext,
    /// Mutation key that made staging idempotent.
    pub idempotency_key: &'a IdempotencyKey,
    /// Staged lifecycle token consumed by abandonment.
    pub staged: StagedContent,
    /// Pipeline boundary that prevented an append.
    pub reason: ContentAbandonReason,
}

/// Content custody port used by the runtime-IR stage/accept/abandon pipeline.
///
/// Generated execution stages content after guards and before command construction. It accepts
/// the token only after append, reduction, and required projection delivery have succeeded;
/// failures between staging and append consume the token through [`Self::abandon`].
pub trait ContentLifecycle: Send {
    /// Adapter-specific lifecycle failure.
    type Error;

    /// Stages and verifies plaintext before semantic command construction.
    ///
    /// Repeating a key with identical bytes and metadata must return the original reference;
    /// repeating it with different input must fail rather than overwrite content.
    fn stage<'a>(
        &'a mut self,
        request: ContentStageRequest<'a>,
    ) -> BoxFuture<'a, Result<StagedContent, Self::Error>>;

    /// Accepts the staged object after append/replay, reduction, and projection delivery succeed.
    ///
    /// Acceptance must be idempotent by service, authority partition, and mutation key.
    fn accept<'a>(
        &'a mut self,
        request: ContentAcceptRequest<'a>,
    ) -> BoxFuture<'a, Result<(), Self::Error>>;

    /// Abandons staged content after a pre-append refusal or failed append.
    ///
    /// Abandonment must be idempotent. Adapters may delete immediately or mark the object for
    /// orphan collection, but must never expose it as accepted content.
    fn abandon<'a>(
        &'a mut self,
        request: ContentAbandonRequest<'a>,
    ) -> BoxFuture<'a, Result<(), Self::Error>>;
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use futures::executor::block_on;

    use super::*;
    use crate::{AuthorityId, ExecutorId, RealmId, TenantId, UserId, VerifiedIdentity};

    #[derive(Default)]
    struct RecordingLifecycle {
        stages: usize,
        accepts: usize,
        abandons: usize,
        plaintext_seen_only_while_staging: Vec<u8>,
    }

    impl ContentLifecycle for RecordingLifecycle {
        type Error = Infallible;

        fn stage<'a>(
            &'a mut self,
            request: ContentStageRequest<'a>,
        ) -> BoxFuture<'a, Result<StagedContent, Self::Error>> {
            Box::pin(async move {
                self.stages += 1;
                self.plaintext_seen_only_while_staging = request.payload.bytes.to_vec();
                Ok(StagedContent::after_staging(
                    ContentId::new(format!("objects/{}", request.idempotency_key)).unwrap(),
                    MediaType::new(request.payload.media_type).unwrap(),
                    ContentDigest::sha256(format!("sha256:{}", "a".repeat(64))).unwrap(),
                    request.payload.bytes.len() as u64,
                ))
            })
        }

        fn accept<'a>(
            &'a mut self,
            request: ContentAcceptRequest<'a>,
        ) -> BoxFuture<'a, Result<(), Self::Error>> {
            Box::pin(async move {
                self.accepts += 1;
                assert_eq!(
                    request.staged.reference().id().as_str(),
                    "objects/accept-key"
                );
                Ok(())
            })
        }

        fn abandon<'a>(
            &'a mut self,
            request: ContentAbandonRequest<'a>,
        ) -> BoxFuture<'a, Result<(), Self::Error>> {
            Box::pin(async move {
                self.abandons += 1;
                assert_eq!(request.reason, ContentAbandonReason::IntentRefused);
                assert_eq!(
                    request.staged.reference().id().as_str(),
                    "objects/abandon-key"
                );
                Ok(())
            })
        }
    }

    fn context() -> VerifiedAuthContext {
        VerifiedAuthContext::from_verified(VerifiedIdentity::after_verification(
            TenantId::new("tenant-a").unwrap(),
            AuthorityId::new("authority-a").unwrap(),
            UserId::new("user-a").unwrap(),
            Some(ExecutorId::new("agent-a").unwrap()),
            Some(RealmId::new("default").unwrap()),
        ))
    }

    #[test]
    fn validates_canonical_content_metadata() {
        assert!(ContentId::new("  ").is_err());
        assert!(MediaType::new("text").is_err());
        assert!(MediaType::new("text/ plain").is_err());
        assert!(MediaType::new("text/plain/extra").is_err());
        assert!(ContentDigest::sha256(format!("sha256:{}", "A".repeat(64))).is_err());

        let staged = StagedContent::after_staging(
            ContentId::new("objects/42").unwrap(),
            MediaType::new("text/plain").unwrap(),
            ContentDigest::sha256(format!("sha256:{}", "a".repeat(64))).unwrap(),
            12,
        );
        let json = serde_json::to_value(staged.reference()).unwrap();
        assert_eq!(json["id"], "objects/42");
        assert_eq!(json.as_object().unwrap().len(), 1);
        assert_eq!(staged.media_type().as_str(), "text/plain");
        assert_eq!(staged.size_bytes(), 12);
    }

    #[test]
    fn lifecycle_accepts_or_abandons_tokens_without_carrying_plaintext_forward() {
        let service = ServiceId::new("todo").unwrap();
        let context = context();
        let accept_key = IdempotencyKey::new("accept-key").unwrap();
        let abandon_key = IdempotencyKey::new("abandon-key").unwrap();
        let mut lifecycle = RecordingLifecycle::default();

        {
            let port: &mut dyn ContentLifecycle<Error = Infallible> = &mut lifecycle;
            let accepted = block_on(port.stage(ContentStageRequest {
                service: &service,
                context: &context,
                idempotency_key: &accept_key,
                payload: ContentPayload {
                    media_type: "text/plain",
                    bytes: b"secret plaintext",
                },
            }))
            .unwrap();
            let event_value = serde_json::to_string(accepted.reference()).unwrap();
            assert_eq!(event_value, r#"{"id":"objects/accept-key"}"#);
            assert!(!event_value.contains("secret plaintext"));
            block_on(port.accept(ContentAcceptRequest {
                service: &service,
                context: &context,
                idempotency_key: &accept_key,
                staged: accepted,
            }))
            .unwrap();

            let abandoned = block_on(port.stage(ContentStageRequest {
                service: &service,
                context: &context,
                idempotency_key: &abandon_key,
                payload: ContentPayload {
                    media_type: "text/plain",
                    bytes: b"never committed",
                },
            }))
            .unwrap();
            block_on(port.abandon(ContentAbandonRequest {
                service: &service,
                context: &context,
                idempotency_key: &abandon_key,
                staged: abandoned,
                reason: ContentAbandonReason::IntentRefused,
            }))
            .unwrap();
        }

        assert_eq!(lifecycle.stages, 2);
        assert_eq!(lifecycle.accepts, 1);
        assert_eq!(lifecycle.abandons, 1);
        assert_eq!(
            lifecycle.plaintext_seen_only_while_staging,
            b"never committed"
        );
    }
}
