//! End-to-end contract tests for the transport-independent execution pipeline.

use std::collections::HashMap;
use std::convert::Infallible;

use futures::executor::block_on;
use service_runtime::{
    Aggregate, AppendDisposition, AppendError, AppendReceipt, AuthorityId, BoxFuture,
    ContextPolicyViolation, EventEnvelope, EventHistory, EventId, EventLog, ExecutionError,
    ExecutionRequest, ExecutionResult, ExecutorId, ExpectedVersion, GuardedAppend,
    HistoryInvariantError, IdempotencyConflict, IdempotencyKey, ProjectionDelivery, ProjectionSink,
    ProjectionTarget, ProjectionVisibility, RealmId, RealmPolicy, ServiceId, StreamId,
    StreamVersion, TenantId, UserId, VerifiedAuthContext, VerifiedIdentity, VersionConflict,
    execute,
};

#[derive(Clone, Debug, Eq, PartialEq)]
enum CounterEvent {
    Added(u64),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CounterIntent {
    Add(u64),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Add(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Refusal {
    Forbidden,
    Zero,
}

struct Counter {
    service_id: ServiceId,
    realm_policy: RealmPolicy,
    refuse_authorization: bool,
}

impl Aggregate for Counter {
    type State = u64;
    type Intent = CounterIntent;
    type Command = Add;
    type Event = CounterEvent;
    type Rejection = Refusal;

    fn realm_policy(&self) -> RealmPolicy {
        self.realm_policy
    }

    fn service_id(&self) -> &ServiceId {
        &self.service_id
    }

    fn initial_state(&self, _stream: &StreamId) -> Self::State {
        0
    }

    fn apply(&self, state: &mut Self::State, envelope: &EventEnvelope<Self::Event>) {
        let CounterEvent::Added(value) = envelope.event();
        *state += value;
    }

    fn authorize(
        &self,
        _context: &VerifiedAuthContext,
        _state: &Self::State,
        _intent: &Self::Intent,
    ) -> Result<(), Self::Rejection> {
        if self.refuse_authorization {
            Err(Refusal::Forbidden)
        } else {
            Ok(())
        }
    }

    fn validate(
        &self,
        _context: &VerifiedAuthContext,
        _state: &Self::State,
        intent: &Self::Intent,
    ) -> Result<(), Self::Rejection> {
        match intent {
            CounterIntent::Add(0) => Err(Refusal::Zero),
            CounterIntent::Add(_) => Ok(()),
        }
    }

    fn command(
        &self,
        _context: &VerifiedAuthContext,
        _state: &Self::State,
        intent: Self::Intent,
    ) -> Result<Self::Command, Self::Rejection> {
        let CounterIntent::Add(value) = intent;
        Ok(Add(value))
    }

    fn decide(
        &self,
        _context: &VerifiedAuthContext,
        _state: &Self::State,
        command: Self::Command,
    ) -> Result<Vec<Self::Event>, Self::Rejection> {
        Ok(vec![CounterEvent::Added(command.0)])
    }
}

#[derive(Clone)]
struct RememberedAppend<E> {
    events: Vec<E>,
    envelopes: Vec<EventEnvelope<E>>,
}

struct MemoryLog<E> {
    streams: HashMap<StreamId, Vec<EventEnvelope<E>>>,
    idempotency: HashMap<(StreamId, IdempotencyKey), RememberedAppend<E>>,
    load_calls: usize,
    append_calls: usize,
    next_event_id: u64,
}

impl<E> Default for MemoryLog<E> {
    fn default() -> Self {
        Self {
            streams: HashMap::new(),
            idempotency: HashMap::new(),
            load_calls: 0,
            append_calls: 0,
            next_event_id: 0,
        }
    }
}

impl<E> EventLog<E> for MemoryLog<E>
where
    E: Clone + Eq + Send,
{
    type Error = Infallible;

    fn load<'a>(
        &'a mut self,
        stream: &'a StreamId,
    ) -> BoxFuture<'a, Result<EventHistory<E>, Self::Error>> {
        Box::pin(async move {
            self.load_calls += 1;
            Ok(EventHistory::loaded(
                stream.clone(),
                self.streams.get(stream).cloned().unwrap_or_default(),
            ))
        })
    }

    fn append_guarded(
        &mut self,
        request: GuardedAppend<E>,
    ) -> BoxFuture<'_, Result<AppendReceipt<E>, AppendError<Self::Error>>> {
        Box::pin(async move {
            self.append_calls += 1;
            let (stream, expected, key, events) = request.into_parts();
            if let Some(remembered) = self.idempotency.get(&(stream.clone(), key.clone())) {
                if remembered.events == events {
                    return Ok(AppendReceipt::new(
                        AppendDisposition::Replayed,
                        remembered.envelopes.clone(),
                    ));
                }
                return Err(IdempotencyConflict { key }.into());
            }

            let current = self
                .streams
                .get(&stream)
                .and_then(|events| events.last())
                .map_or(StreamVersion::EMPTY, EventEnvelope::version);
            if !expected.matches(current) {
                return Err(VersionConflict {
                    expected,
                    actual: current,
                }
                .into());
            }

            let mut version = current;
            let mut envelopes = Vec::with_capacity(events.len());
            for event in &events {
                version = version
                    .checked_next()
                    .expect("test versions remain bounded");
                self.next_event_id += 1;
                envelopes.push(EventEnvelope::committed(
                    EventId::new(format!("event-{}", self.next_event_id)).unwrap(),
                    stream.clone(),
                    version,
                    key.clone(),
                    event.clone(),
                ));
            }
            self.streams
                .entry(stream.clone())
                .or_default()
                .extend(envelopes.clone());
            self.idempotency.insert(
                (stream, key),
                RememberedAppend {
                    events,
                    envelopes: envelopes.clone(),
                },
            );
            Ok(AppendReceipt::new(AppendDisposition::Committed, envelopes))
        })
    }
}

#[derive(Default)]
struct RecordingProjection {
    projects: usize,
    waits: usize,
    last_target: Option<ProjectionTarget>,
}

impl ProjectionSink<u64, CounterEvent> for RecordingProjection {
    type Error = Infallible;

    fn project<'a>(
        &'a mut self,
        _context: &'a VerifiedAuthContext,
        target: &'a ProjectionTarget,
        _state: &'a u64,
        _events: &'a [EventEnvelope<CounterEvent>],
    ) -> BoxFuture<'a, Result<(), Self::Error>> {
        Box::pin(async move {
            self.projects += 1;
            self.last_target = Some(target.clone());
            Ok(())
        })
    }

    fn wait_until_visible<'a>(
        &'a mut self,
        _target: &'a ProjectionTarget,
    ) -> BoxFuture<'a, Result<(), Self::Error>> {
        Box::pin(async move {
            self.waits += 1;
            Ok(())
        })
    }
}

fn context(tenant: &str, realm: Option<&str>) -> VerifiedAuthContext {
    VerifiedAuthContext::from_verified(VerifiedIdentity::after_verification(
        TenantId::new(tenant).unwrap(),
        AuthorityId::new("authority-a").unwrap(),
        UserId::new("user-a").unwrap(),
        Some(ExecutorId::new("agent-a").unwrap()),
        realm.map(|value| RealmId::new(value).unwrap()),
    ))
}

fn stream() -> StreamId {
    StreamId::new(
        ServiceId::new("counter-service").unwrap(),
        TenantId::new("tenant-a").unwrap(),
        None,
        "counter",
        "one",
    )
    .unwrap()
}

fn request(delivery: ProjectionDelivery) -> ExecutionRequest<CounterIntent> {
    ExecutionRequest {
        context: context("tenant-a", None),
        stream: stream(),
        idempotency_key: IdempotencyKey::new("request-1").unwrap(),
        expected_version: ExpectedVersion::no_stream(),
        intent: CounterIntent::Add(3),
        projection_delivery: delivery,
    }
}

type CounterExecution =
    Result<ExecutionResult<u64, CounterEvent>, ExecutionError<Refusal, Infallible, Infallible>>;

fn run(
    aggregate: &Counter,
    event_log: &mut MemoryLog<CounterEvent>,
    projections: &mut RecordingProjection,
    request: ExecutionRequest<CounterIntent>,
) -> CounterExecution {
    block_on(execute(aggregate, event_log, projections, request))
}

#[test]
fn execution_accepts_async_event_log_and_projection_trait_objects() {
    let aggregate = Counter {
        service_id: ServiceId::new("counter-service").unwrap(),
        realm_policy: RealmPolicy::Optional,
        refuse_authorization: false,
    };
    let mut log = MemoryLog::default();
    let mut projections = RecordingProjection::default();
    let event_log: &mut dyn EventLog<CounterEvent, Error = Infallible> = &mut log;
    let projection_sink: &mut dyn ProjectionSink<u64, CounterEvent, Error = Infallible> =
        &mut projections;

    let result = block_on(execute(
        &aggregate,
        event_log,
        projection_sink,
        request(ProjectionDelivery::Eventual),
    ))
    .unwrap();

    assert_eq!(result.state, 3);
    assert_eq!(result.append.disposition(), AppendDisposition::Committed);
}

#[test]
fn executes_full_pipeline_and_waits_for_read_your_writes() {
    let mut log = MemoryLog::default();
    let mut projections = RecordingProjection::default();
    let result = run(
        &Counter {
            service_id: ServiceId::new("counter-service").unwrap(),
            realm_policy: RealmPolicy::Optional,
            refuse_authorization: false,
        },
        &mut log,
        &mut projections,
        request(ProjectionDelivery::ReadYourWrites),
    )
    .unwrap();

    assert_eq!(result.state, 3);
    assert_eq!(result.append.disposition(), AppendDisposition::Committed);
    assert_eq!(result.append.through_version(), Some(StreamVersion::new(1)));
    assert_eq!(
        result.projection.visibility(),
        ProjectionVisibility::Visible
    );
    assert_eq!(projections.projects, 1);
    assert_eq!(projections.waits, 1);
}

#[test]
fn eventual_projection_does_not_claim_visibility() {
    let mut log = MemoryLog::default();
    let mut projections = RecordingProjection::default();
    let result = run(
        &Counter {
            service_id: ServiceId::new("counter-service").unwrap(),
            realm_policy: RealmPolicy::Optional,
            refuse_authorization: false,
        },
        &mut log,
        &mut projections,
        request(ProjectionDelivery::Eventual),
    )
    .unwrap();

    assert_eq!(
        result.projection.visibility(),
        ProjectionVisibility::Scheduled
    );
    assert_eq!(projections.projects, 1);
    assert_eq!(projections.waits, 0);
}

#[test]
fn rejected_intent_never_calls_append_or_projection() {
    let mut log = MemoryLog::default();
    let mut projections = RecordingProjection::default();
    let error = run(
        &Counter {
            service_id: ServiceId::new("counter-service").unwrap(),
            realm_policy: RealmPolicy::Optional,
            refuse_authorization: true,
        },
        &mut log,
        &mut projections,
        request(ProjectionDelivery::ReadYourWrites),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ExecutionError::Rejected(Refusal::Forbidden)
    ));
    assert_eq!(log.append_calls, 0);
    assert_eq!(log.load_calls, 1);
    assert!(log.streams.is_empty());
    assert_eq!(projections.projects, 0);
}

#[test]
fn validation_refusal_never_calls_append_or_projection() {
    let mut log = MemoryLog::default();
    let mut projections = RecordingProjection::default();
    let mut invalid = request(ProjectionDelivery::ReadYourWrites);
    invalid.intent = CounterIntent::Add(0);
    let error = run(
        &Counter {
            service_id: ServiceId::new("counter-service").unwrap(),
            realm_policy: RealmPolicy::Optional,
            refuse_authorization: false,
        },
        &mut log,
        &mut projections,
        invalid,
    )
    .unwrap_err();

    assert!(matches!(error, ExecutionError::Rejected(Refusal::Zero)));
    assert_eq!(log.load_calls, 1);
    assert_eq!(log.append_calls, 0);
    assert_eq!(projections.projects, 0);
}

#[test]
fn invalid_loaded_history_is_never_authorized_or_appended() {
    let stream = stream();
    let mut log = MemoryLog::default();
    log.streams.insert(
        stream.clone(),
        vec![EventEnvelope::committed(
            EventId::new("bad-version").unwrap(),
            stream,
            StreamVersion::new(2),
            IdempotencyKey::new("historical-request").unwrap(),
            CounterEvent::Added(1),
        )],
    );
    let mut projections = RecordingProjection::default();
    let error = run(
        &Counter {
            service_id: ServiceId::new("counter-service").unwrap(),
            realm_policy: RealmPolicy::Optional,
            refuse_authorization: false,
        },
        &mut log,
        &mut projections,
        request(ProjectionDelivery::Eventual),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ExecutionError::History(HistoryInvariantError::NonContiguousVersion {
            expected,
            actual
        }) if expected == StreamVersion::new(1) && actual == StreamVersion::new(2)
    ));
    assert_eq!(log.append_calls, 0);
    assert_eq!(projections.projects, 0);
}

#[test]
fn context_policy_and_exact_stream_partition_run_before_load_or_append() {
    let mut log = MemoryLog::default();
    let mut projections = RecordingProjection::default();
    let mut missing_realm = request(ProjectionDelivery::Eventual);
    missing_realm.context = context("tenant-a", None);
    let error = run(
        &Counter {
            service_id: ServiceId::new("counter-service").unwrap(),
            realm_policy: RealmPolicy::Required,
            refuse_authorization: false,
        },
        &mut log,
        &mut projections,
        missing_realm,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ExecutionError::Context(ContextPolicyViolation::RealmRequired)
    ));
    assert_eq!(log.append_calls, 0);
    assert_eq!(log.load_calls, 0);

    let mut wrong_tenant = request(ProjectionDelivery::Eventual);
    wrong_tenant.context = context("tenant-b", None);
    let error = run(
        &Counter {
            service_id: ServiceId::new("counter-service").unwrap(),
            realm_policy: RealmPolicy::Optional,
            refuse_authorization: false,
        },
        &mut log,
        &mut projections,
        wrong_tenant,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ExecutionError::Context(ContextPolicyViolation::TenantMismatch { .. })
    ));
    assert_eq!(log.append_calls, 0);
    assert_eq!(log.load_calls, 0);

    let mut wrong_service = request(ProjectionDelivery::Eventual);
    wrong_service.stream = StreamId::new(
        ServiceId::new("other-service").unwrap(),
        TenantId::new("tenant-a").unwrap(),
        None,
        "counter",
        "one",
    )
    .unwrap();
    let error = run(
        &Counter {
            service_id: ServiceId::new("counter-service").unwrap(),
            realm_policy: RealmPolicy::Optional,
            refuse_authorization: false,
        },
        &mut log,
        &mut projections,
        wrong_service,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ExecutionError::Context(ContextPolicyViolation::ServiceMismatch { .. })
    ));
    assert_eq!(log.append_calls, 0);
    assert_eq!(log.load_calls, 0);

    let mut wrong_realm = request(ProjectionDelivery::Eventual);
    wrong_realm.context = context("tenant-a", Some("default"));
    let error = run(
        &Counter {
            service_id: ServiceId::new("counter-service").unwrap(),
            realm_policy: RealmPolicy::Optional,
            refuse_authorization: false,
        },
        &mut log,
        &mut projections,
        wrong_realm,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ExecutionError::Context(ContextPolicyViolation::RealmMismatch {
            stream_realm: None,
            context_realm: Some(ref realm),
        }) if realm.as_str() == "default"
    ));
    assert_eq!(log.append_calls, 0);
    assert_eq!(log.load_calls, 0);
}

#[test]
fn caller_expected_version_is_never_replaced_with_loaded_current() {
    let mut log = MemoryLog::default();
    let mut projections = RecordingProjection::default();
    let aggregate = Counter {
        service_id: ServiceId::new("counter-service").unwrap(),
        realm_policy: RealmPolicy::Optional,
        refuse_authorization: false,
    };

    run(
        &aggregate,
        &mut log,
        &mut projections,
        request(ProjectionDelivery::Eventual),
    )
    .unwrap();

    let mut no_stream = request(ProjectionDelivery::Eventual);
    no_stream.idempotency_key = IdempotencyKey::new("request-no-stream-conflict").unwrap();
    no_stream.expected_version = ExpectedVersion::no_stream();
    let error = run(&aggregate, &mut log, &mut projections, no_stream).unwrap_err();
    assert!(matches!(
        error,
        ExecutionError::Append(AppendError::Version(VersionConflict {
            expected,
            actual,
        })) if expected == ExpectedVersion::no_stream() && actual == StreamVersion::new(1)
    ));

    let mut second = request(ProjectionDelivery::Eventual);
    second.idempotency_key = IdempotencyKey::new("request-2").unwrap();
    second.expected_version = ExpectedVersion::exact(StreamVersion::new(1));
    run(&aggregate, &mut log, &mut projections, second).unwrap();

    let mut stale = request(ProjectionDelivery::Eventual);
    stale.idempotency_key = IdempotencyKey::new("request-stale").unwrap();
    stale.expected_version = ExpectedVersion::exact(StreamVersion::new(1));
    let error = run(&aggregate, &mut log, &mut projections, stale).unwrap_err();
    assert!(matches!(
        error,
        ExecutionError::Append(AppendError::Version(VersionConflict {
            expected,
            actual,
        })) if expected == ExpectedVersion::exact(StreamVersion::new(1))
            && actual == StreamVersion::new(2)
    ));
    assert_eq!(log.streams[&stream()].len(), 2);
}

#[test]
fn retry_replays_the_original_append_without_reducing_twice() {
    let mut log = MemoryLog::default();
    let mut projections = RecordingProjection::default();
    let aggregate = Counter {
        service_id: ServiceId::new("counter-service").unwrap(),
        realm_policy: RealmPolicy::Optional,
        refuse_authorization: false,
    };

    let first = run(
        &aggregate,
        &mut log,
        &mut projections,
        request(ProjectionDelivery::ReadYourWrites),
    )
    .unwrap();
    let replay = run(
        &aggregate,
        &mut log,
        &mut projections,
        request(ProjectionDelivery::ReadYourWrites),
    )
    .unwrap();

    assert_eq!(first.state, 3);
    assert_eq!(replay.state, 3);
    assert_eq!(replay.append.disposition(), AppendDisposition::Replayed);
    assert_eq!(log.streams[&stream()].len(), 1);
    // Projection submission is idempotent and is retried to close an append/project crash gap.
    assert_eq!(projections.projects, 2);
    assert_eq!(projections.waits, 2);
}

#[test]
fn memory_log_enforces_conflict_and_idempotency_atomically() {
    let mut log = MemoryLog::default();
    let stream = stream();
    let key = IdempotencyKey::new("same-request").unwrap();
    let append = GuardedAppend::new(
        stream.clone(),
        ExpectedVersion::no_stream(),
        key.clone(),
        vec![CounterEvent::Added(2)],
    );
    let first = block_on(log.append_guarded(append.clone())).unwrap();
    let replay = block_on(log.append_guarded(append)).unwrap();

    assert_eq!(first.disposition(), AppendDisposition::Committed);
    assert_eq!(replay.disposition(), AppendDisposition::Replayed);
    assert_eq!(first.events(), replay.events());
    assert_eq!(log.streams[&stream].len(), 1);

    let reused = block_on(log.append_guarded(GuardedAppend::new(
        stream.clone(),
        ExpectedVersion::no_stream(),
        key,
        vec![CounterEvent::Added(99)],
    )))
    .unwrap_err();
    assert!(matches!(reused, AppendError::Idempotency(_)));

    let conflict = block_on(log.append_guarded(GuardedAppend::new(
        stream,
        ExpectedVersion::no_stream(),
        IdempotencyKey::new("another-request").unwrap(),
        vec![CounterEvent::Added(1)],
    )))
    .unwrap_err();
    assert!(matches!(
        conflict,
        AppendError::Version(VersionConflict {
            expected,
            actual
        }) if expected == ExpectedVersion::no_stream() && actual == StreamVersion::new(1)
    ));
}
