//! Deterministic aggregate execution pipeline.

use thiserror::Error;

use crate::{
    AppendDisposition, AppendError, AppendReceipt, ContextPolicyViolation, EventEnvelope,
    EventHistory, EventLog, ExpectedVersion, GuardedAppend, HistoryInvariantError, IdempotencyKey,
    ProjectionDelivery, ProjectionOutcome, ProjectionSink, ProjectionTarget, RealmPolicy,
    ServiceId, StreamId, StreamVersion, VerifiedAuthContext, projection,
};

/// A generated aggregate implementation consumed by the runtime.
pub trait Aggregate {
    /// Folded aggregate state.
    type State;
    /// Authenticated application intent.
    type Intent;
    /// Validated semantic command.
    type Command;
    /// Persisted domain event.
    type Event;
    /// Stable domain refusal.
    type Rejection;

    /// Returns the aggregate's realm-context contract.
    fn realm_policy(&self) -> RealmPolicy;

    /// Returns the stable standalone-service partition that owns this aggregate.
    fn service_id(&self) -> &ServiceId;

    /// Creates empty state for a stream before history is folded.
    fn initial_state(&self, stream: &StreamId) -> Self::State;

    /// Reduces one committed event into state. Used for both history folding and new events.
    fn apply(&self, state: &mut Self::State, event: &EventEnvelope<Self::Event>);

    /// Authorizes the intent against verified context and current state.
    fn authorize(
        &self,
        context: &VerifiedAuthContext,
        state: &Self::State,
        intent: &Self::Intent,
    ) -> Result<(), Self::Rejection>;

    /// Validates domain invariants without changing state.
    fn validate(
        &self,
        context: &VerifiedAuthContext,
        state: &Self::State,
        intent: &Self::Intent,
    ) -> Result<(), Self::Rejection>;

    /// Converts a valid intent into a semantic command.
    fn command(
        &self,
        context: &VerifiedAuthContext,
        state: &Self::State,
        intent: Self::Intent,
    ) -> Result<Self::Command, Self::Rejection>;

    /// Decides the events produced by a semantic command.
    fn decide(
        &self,
        context: &VerifiedAuthContext,
        state: &Self::State,
        command: Self::Command,
    ) -> Result<Vec<Self::Event>, Self::Rejection>;
}

/// Input required to execute one authenticated intent.
#[derive(Clone, Debug)]
pub struct ExecutionRequest<I> {
    /// Verified authentication context.
    pub context: VerifiedAuthContext,
    /// Service-, tenant-, and exact-optional-realm-scoped aggregate stream.
    pub stream: StreamId,
    /// Key used for atomic idempotency at append time.
    pub idempotency_key: IdempotencyKey,
    /// Caller-declared optimistic-concurrency requirement.
    pub expected_version: ExpectedVersion,
    /// Typed application intent.
    pub intent: I,
    /// Required projection delivery guarantee.
    pub projection_delivery: ProjectionDelivery,
}

/// State obtained after validating and folding event history.
#[derive(Debug)]
pub struct FoldStage<S> {
    /// Folded state.
    pub state: S,
    /// Version observed while folding; it never replaces the caller's append guard.
    pub version: StreamVersion,
}

/// Successful aggregate execution.
#[derive(Debug)]
pub struct ExecutionResult<S, E> {
    /// State reduced through the committed append result.
    pub state: S,
    /// Guarded append receipt.
    pub append: AppendReceipt<E>,
    /// Projection target and achieved visibility.
    pub projection: ProjectionOutcome,
}

/// Result type for one aggregate execution with concrete event-log and projection ports.
pub type ExecutionOutcome<A, L, P> = Result<
    ExecutionResult<<A as Aggregate>::State, <A as Aggregate>::Event>,
    ExecutionError<
        <A as Aggregate>::Rejection,
        <L as EventLog<<A as Aggregate>::Event>>::Error,
        <P as ProjectionSink<<A as Aggregate>::State, <A as Aggregate>::Event>>::Error,
    >,
>;

/// Aggregate execution failed at a named boundary.
#[derive(Debug, Error)]
pub enum ExecutionError<R, L, P> {
    /// Verified context violates service context policy.
    #[error(transparent)]
    Context(#[from] ContextPolicyViolation),
    /// Event-log load failed.
    #[error("event-log load failed")]
    Load(L),
    /// Loaded history violated stream invariants.
    #[error(transparent)]
    History(#[from] HistoryInvariantError),
    /// Authorization, validation, command construction, or decision refused the intent.
    #[error("intent was refused")]
    Rejected(R),
    /// A successful command must emit at least one event.
    #[error("accepted command emitted no events")]
    EmptyDecision,
    /// Guarded append failed.
    #[error("guarded append failed: {0}")]
    Append(AppendError<L>),
    /// Projection delivery failed.
    #[error("projection delivery failed")]
    Projection(P),
}

/// Executes one intent through the complete event-sourced mutation pipeline.
///
/// Ordering is fixed: context policy and exact service/tenant/realm partition; load and fold;
/// authorize; validate; construct command; decide; guarded append under the caller's unchanged
/// expected version; reduce; project; and, for read-your-writes, wait for visibility. No rejection
/// before append can emit an event.
pub async fn execute<A, L, P>(
    aggregate: &A,
    event_log: &mut L,
    projections: &mut P,
    request: ExecutionRequest<A::Intent>,
) -> ExecutionOutcome<A, L, P>
where
    A: Aggregate,
    L: EventLog<A::Event> + ?Sized,
    P: ProjectionSink<A::State, A::Event> + ?Sized,
{
    aggregate.realm_policy().enforce(&request.context)?;
    if request.stream.service() != aggregate.service_id() {
        return Err(ContextPolicyViolation::ServiceMismatch {
            stream_service: request.stream.service().clone(),
            runtime_service: aggregate.service_id().clone(),
        }
        .into());
    }
    if request.stream.tenant() != request.context.tenant() {
        return Err(ContextPolicyViolation::TenantMismatch {
            stream_tenant: request.stream.tenant().clone(),
            context_tenant: request.context.tenant().clone(),
        }
        .into());
    }
    if request.stream.realm() != request.context.realm() {
        return Err(ContextPolicyViolation::RealmMismatch {
            stream_realm: request.stream.realm().cloned(),
            context_realm: request.context.realm().cloned(),
        }
        .into());
    }

    let history = event_log
        .load(&request.stream)
        .await
        .map_err(ExecutionError::Load)?;
    let FoldStage {
        mut state,
        version: _,
    } = fold(aggregate, &request.stream, &history)?;

    aggregate
        .authorize(&request.context, &state, &request.intent)
        .map_err(ExecutionError::Rejected)?;
    aggregate
        .validate(&request.context, &state, &request.intent)
        .map_err(ExecutionError::Rejected)?;
    let command = aggregate
        .command(&request.context, &state, request.intent)
        .map_err(ExecutionError::Rejected)?;
    let events = aggregate
        .decide(&request.context, &state, command)
        .map_err(ExecutionError::Rejected)?;
    if events.is_empty() {
        return Err(ExecutionError::EmptyDecision);
    }

    let append = event_log
        .append_guarded(GuardedAppend::new(
            request.stream.clone(),
            request.expected_version,
            request.idempotency_key,
            events,
        ))
        .await
        .map_err(ExecutionError::Append)?;

    // A replay's events were already part of the history loaded above. Projecting them again is
    // intentional and the projection port is idempotent, which closes the append/project crash gap.
    if append.disposition() == AppendDisposition::Committed {
        for event in append.events() {
            aggregate.apply(&mut state, event);
        }
    }

    let through = append
        .through_version()
        .ok_or(ExecutionError::EmptyDecision)?;
    let projection = projection::deliver(
        projections,
        &request.context,
        request.projection_delivery,
        ProjectionTarget::new(request.stream, through),
        &state,
        append.events(),
    )
    .await
    .map_err(ExecutionError::Projection)?;

    Ok(ExecutionResult {
        state,
        append,
        projection,
    })
}

fn fold<A>(
    aggregate: &A,
    stream: &StreamId,
    history: &EventHistory<A::Event>,
) -> Result<FoldStage<A::State>, HistoryInvariantError>
where
    A: Aggregate,
{
    history.validate()?;
    if history.stream() != stream {
        return Err(HistoryInvariantError::WrongStream {
            expected: Box::new(stream.clone()),
            actual: Box::new(history.stream().clone()),
        });
    }
    let version = history.current_version();
    let mut state = aggregate.initial_state(stream);
    for event in history.events() {
        aggregate.apply(&mut state, event);
    }
    Ok(FoldStage { state, version })
}
