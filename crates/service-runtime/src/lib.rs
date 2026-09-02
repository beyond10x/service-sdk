//! Transport-independent execution primitives for generated services.
//!
//! This crate deliberately knows nothing about ESS, AEP, HTTP, or route shapes. Authentication
//! adapters hand it a [`VerifiedAuthContext`], generated aggregate implementations hand it typed
//! intents and events, and persistence adapters implement the event-log and projection ports.

use std::future::Future;
use std::pin::Pin;

mod auth;
mod content;
mod effect;
mod eventlog;
mod execution;
mod projection;

/// Object-safe, allocation-explicit future returned by runtime adapter ports.
///
/// This matches the zero-blocking shape used by the standalone Eventlog adapters without taking a
/// dependency on that implementation or on an async runtime.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub use auth::{
    AgentId, AttemptId, AuthorityId, ContextPolicyViolation, DelegationId, ExecutorId, GrantId,
    InvalidIdentity, RealmId, RealmPolicy, TenantId, UserId, VerifiedAuthContext,
    VerifiedExecution, VerifiedIdentity,
};
pub use content::{
    ContentAbandonReason, ContentAbandonRequest, ContentAcceptRequest, ContentDigest, ContentId,
    ContentLifecycle, ContentMetadataError, ContentPayload, ContentRef, ContentStageRequest,
    MediaType, StagedContent,
};
pub use effect::{
    ClaimDisposition, EFFECT_PLAN_FORMAT, EffectAdapter, EffectClaim, EffectDispatchError,
    EffectJournal, EffectObservation, EffectOutcome, EffectPlan, EffectPlanError, EffectRecord,
    EffectRisk, EffectState, PreparedEffect, resume_effect,
};
pub use eventlog::{
    AppendDisposition, AppendError, AppendReceipt, EventEnvelope, EventHistory, EventId, EventLog,
    ExpectedVersion, GuardedAppend, HistoryInvariantError, IdempotencyConflict, IdempotencyKey,
    InvalidEventId, InvalidIdempotencyKey, InvalidServiceId, InvalidStreamId, STREAM_ID_ENCODING,
    ServiceId, StreamId, StreamVersion, VersionConflict,
};
pub use execution::{
    Aggregate, ExecutionError, ExecutionOutcome, ExecutionRequest, ExecutionResult, FoldStage,
    execute,
};
pub use projection::{
    ProjectionDelivery, ProjectionOutcome, ProjectionSink, ProjectionTarget, ProjectionVisibility,
};
