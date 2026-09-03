//! SDK resource adapters over the organization Eventlog kit.
//!
//! The adapter is generic over Eventlog's one `EventStore` port, so a deployment chooses its
//! existing `SQLite` or `PostgreSQL` implementation without changing generated service code.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use eventlog_core::{
    CommandMeta, EventLogError, EventStore as DurableEventStore, Expected, NewEvent,
    ProjectionSpec, ProjectionStore as DurableProjectionStore, Projector, StreamId, TenantId,
};
use serde::Serialize;
use serde_json::Value;
use service_engine::{
    AppendDisposition, AppendExpectation, AppendReceipt, AppendRequest, AuthorityCheck,
    AuthorityEvaluator, BoxFuture, Clock, ContentPayload, ContentStore, DomainEvent, EventStore,
    IdGenerator, IntentResult, LoadedStream, ProjectionRead, ProjectionRow, ProjectionState,
    ProjectionStore, ProjectionWrite, RequestMetadata, ResourceError, ServiceEngine,
    ServiceResources, ServiceStream, StagedContent, StoredEvent,
};
use service_runtime::VerifiedAuthContext;
use service_runtime::{
    ClaimDisposition, EffectClaim, EffectJournal as EffectJournalPort, EffectOutcome, EffectRecord,
    EffectState, PreparedEffect,
};
use sha2::{Digest as _, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

const PAGE: usize = 1_000;
const MAX_QUERY_ROWS: usize = 10_000;
const MAX_PAGE_ROWS: usize = 1_000;

/// Caller-selected projection page after validation against SDK bounds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageRequest {
    cursor: Option<String>,
    limit: usize,
}

impl PageRequest {
    /// Validates a page request. Cursors are opaque and may only be replayed unchanged.
    pub fn new(cursor: Option<String>, limit: usize) -> Result<Self, PageRequestError> {
        if limit == 0 || limit > MAX_PAGE_ROWS {
            return Err(PageRequestError::Limit);
        }
        if cursor.as_ref().is_some_and(String::is_empty) {
            return Err(PageRequestError::Cursor);
        }
        Ok(Self { cursor, limit })
    }

    /// Opaque cursor supplied by a previous response.
    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    /// Maximum raw projection rows inspected in this page.
    pub const fn limit(&self) -> usize {
        self.limit
    }
}

/// Invalid caller-controlled pagination metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PageRequestError {
    /// Page size must be between one and the SDK hard maximum.
    #[error("page limit must be between 1 and 1000")]
    Limit,
    /// An empty cursor is not a valid continuation token.
    #[error("page cursor must not be empty")]
    Cursor,
}

/// One authorized generated-service projection page.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QueryPage {
    /// Visible rows from this bounded raw projection window.
    pub items: Vec<BTreeMap<String, Value>>,
    /// Exact authorized aggregate version when every visible row belongs to one stream.
    pub through_version: Option<u64>,
    /// Opaque cursor for the next raw window, or `None` at the end.
    pub next_cursor: Option<String>,
    /// True when more raw rows remain, including rows withheld by authorization.
    pub partial: bool,
}

/// One durable aggregate event exposed by the authorized feed.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceEvent {
    /// Aggregate stream version.
    pub version: u64,
    /// Stable Eventlog event identity.
    pub event_id: String,
    /// Semantic event name.
    pub name: String,
    /// RFC 3339 occurrence time.
    pub occurred_at: String,
    /// Authenticated owner subject.
    pub subject: String,
    /// Authenticated immediate actor.
    pub actor: String,
    /// Whether the original body was erased.
    pub redacted: bool,
    /// Event data or its explicit redaction tombstone.
    pub data: Value,
}

/// One resumable page from a single authorized aggregate stream.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EventPage {
    /// Events after the supplied cursor.
    pub events: Vec<ServiceEvent>,
    /// Opaque cursor for reconnect, including when the stream is currently caught up.
    pub cursor: String,
    /// Whether Eventlog reported more events immediately available.
    pub has_more: bool,
}

/// Deployment authorization seam for aggregate event reads.
pub trait EventFeedAuthorizer: Send + Sync {
    /// Decides whether the verified actor may observe this exact aggregate stream.
    fn allows<'a>(
        &'a self,
        context: &'a VerifiedAuthContext,
        category: &'a str,
        key: &'a str,
    ) -> BoxFuture<'a, Result<bool, EventLogError>>;
}

/// Why an aggregate event page was not returned.
#[derive(Debug, thiserror::Error)]
pub enum EventPageError {
    /// The deployment authorizer withheld this stream.
    #[error("aggregate event feed is not authorized")]
    Refused,
    /// The cursor belongs to another Eventlog incarnation or aggregate stream.
    #[error("aggregate event cursor is stale or invalid")]
    Cursor,
    /// Eventlog could not serve the page.
    #[error(transparent)]
    Store(#[from] EventLogError),
}

/// Receiver-verified scope and capability facts evaluated by SDK obligations.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AuthorityFacts {
    /// Opaque principals the current authority may bind as an owner/scope.
    pub principals: BTreeSet<String>,
    /// Verified team identities, normally Identity groups.
    pub teams: BTreeSet<String>,
    /// Verified project identities supplied by an authentication/delegation adapter.
    pub projects: BTreeSet<String>,
    /// Closed extension bindings encoded as `<kind>:<value>`.
    pub extensions: BTreeSet<String>,
    /// Deployment-known service capabilities such as trusted scheduler execution.
    pub capabilities: BTreeSet<String>,
}

/// Initialized Eventlog-backed resource set for one generated service plan.
#[derive(Clone)]
pub struct EventlogService {
    store: Arc<dyn DurableEventStore>,
    engine: Arc<ServiceEngine>,
    projection: ProjectionLayout,
}

/// Eventlog-backed durable journal for prepared external effects.
#[derive(Clone)]
pub struct EventlogEffectJournal {
    store: Arc<dyn DurableEventStore>,
    service: String,
}

impl std::fmt::Debug for EventlogEffectJournal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EventlogEffectJournal")
            .field("service", &self.service)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for EventlogService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EventlogService")
            .field("service", &self.engine.plan().service)
            .finish_non_exhaustive()
    }
}

impl EventlogService {
    /// Exact generated plan executed by this initialized service.
    #[must_use]
    pub fn plan(&self) -> &service_engine::ServicePlan {
        self.engine.plan()
    }

    /// Returns the restart-safe external-effect journal for this generated service.
    #[must_use]
    pub fn effect_journal(&self) -> EventlogEffectJournal {
        EventlogEffectJournal {
            store: Arc::clone(&self.store),
            service: self.engine.plan().service.clone(),
        }
    }

    /// Register the generated inline projector before the service can accept traffic.
    pub async fn initialize(
        store: Arc<dyn DurableEventStore>,
        engine: ServiceEngine,
    ) -> Result<Self, EventLogError> {
        let engine = Arc::new(engine);
        let projection = ProjectionLayout::for_service(&engine.plan().service);
        let projector = Arc::new(ServiceProjector {
            engine: Arc::clone(&engine),
            layout: projection,
        });
        store.register_inline(projector).await?;
        Ok(Self {
            store,
            engine,
            projection,
        })
    }

    /// Execute one generated intent with a complete SDK-owned resource set.
    pub async fn intent(
        &self,
        context: &VerifiedAuthContext,
        facts: AuthorityFacts,
        metadata: RequestMetadata<'_>,
        operation: &str,
        body: &[u8],
    ) -> Result<IntentResult, service_engine::ExecutionError> {
        let mut events = EventlogEvents::new(Arc::clone(&self.store));
        let mut projections = EventlogProjections::new(
            Arc::clone(&self.store),
            self.projection.rows,
            &self.engine.plan().service,
        );
        let mut content_store = EventlogContent::new(Arc::clone(&self.store));
        let mut authority = VerifiedAuthority::new(facts);
        let mut clock = SystemClock;
        let mut ids = UuidV7;
        let mut resources = ServiceResources {
            events: &mut events,
            projections: &mut projections,
            content: &mut content_store,
            authority: &mut authority,
            clock: &mut clock,
            ids: &mut ids,
        };
        self.engine
            .intent(&mut resources, context, metadata, operation, body)
            .await
    }

    /// Execute one generated projection query with exact hidden tenant/realm partitioning.
    pub async fn query(
        &self,
        context: &VerifiedAuthContext,
        facts: AuthorityFacts,
        operation: &str,
        body: &[u8],
    ) -> Result<Vec<BTreeMap<String, Value>>, service_engine::ExecutionError> {
        let mut events = EventlogEvents::new(Arc::clone(&self.store));
        let mut projections = EventlogProjections::new(
            Arc::clone(&self.store),
            self.projection.rows,
            &self.engine.plan().service,
        );
        let mut content_store = EventlogContent::new(Arc::clone(&self.store));
        let mut authority = VerifiedAuthority::new(facts);
        let mut clock = SystemClock;
        let mut ids = UuidV7;
        let mut resources = ServiceResources {
            events: &mut events,
            projections: &mut projections,
            content: &mut content_store,
            authority: &mut authority,
            clock: &mut clock,
            ids: &mut ids,
        };
        self.engine
            .query(&mut resources, context, operation, body)
            .await
    }

    /// Execute one bounded generated projection query and return an opaque continuation cursor.
    pub async fn query_page(
        &self,
        context: &VerifiedAuthContext,
        facts: AuthorityFacts,
        operation: &str,
        body: &[u8],
        page: PageRequest,
    ) -> Result<QueryPage, service_engine::ExecutionError> {
        let mut events = EventlogEvents::new(Arc::clone(&self.store));
        let mut projections = EventlogProjections::paged(
            Arc::clone(&self.store),
            self.projection.rows,
            &self.engine.plan().service,
            page,
        );
        let mut content_store = EventlogContent::new(Arc::clone(&self.store));
        let mut authority = VerifiedAuthority::new(facts);
        let mut clock = SystemClock;
        let mut ids = UuidV7;
        let mut resources = ServiceResources {
            events: &mut events,
            projections: &mut projections,
            content: &mut content_store,
            authority: &mut authority,
            clock: &mut clock,
            ids: &mut ids,
        };
        let rows = self
            .engine
            .query_rows(&mut resources, context, operation, body)
            .await?;
        let next_cursor = projections.next_cursor.take();
        let through_version = match single_authorized_stream(&rows) {
            Some(service_stream) => {
                let stream = durable_stream(service_stream)
                    .map_err(|_| service_engine::ExecutionError::Resource(ResourceError))?;
                self.store
                    .stream_version(&stream)
                    .await
                    .map_err(|_| service_engine::ExecutionError::Resource(ResourceError))?
            }
            None => None,
        };
        Ok(QueryPage {
            items: rows.into_iter().map(|row| row.value).collect(),
            through_version,
            partial: next_cursor.is_some(),
            next_cursor,
        })
    }

    /// Read a page from one aggregate after a deployment authorizes the exact stream.
    pub async fn events_page(
        &self,
        context: &VerifiedAuthContext,
        authorizer: &dyn EventFeedAuthorizer,
        category: &str,
        key: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<EventPage, EventPageError> {
        let page = PageRequest::new(cursor.map(str::to_owned), limit)
            .map_err(|_| EventPageError::Cursor)?;
        if !authorizer
            .allows(context, category, key)
            .await
            .map_err(EventPageError::Store)?
        {
            return Err(EventPageError::Refused);
        }
        let service_stream = ServiceStream {
            service: self.engine.plan().service.clone(),
            tenant: context.tenant().as_str().to_owned(),
            realm: context.realm().map(|realm| realm.as_str().to_owned()),
            category: category.to_owned(),
            key: key.to_owned(),
        };
        let stream = durable_stream(&service_stream)?;
        let tenant = TenantId::new(context.tenant().as_str())?;
        let identity = feed_identity(&self.store.stream_identity(&tenant).await?, &stream);
        let after = page
            .cursor()
            .map(|cursor| decode_event_cursor(cursor, &identity))
            .transpose()?
            .unwrap_or(0);
        let slice = self.store.read_stream(&stream, after, page.limit()).await?;
        let position = slice.events.last().map_or(after, |event| event.version);
        let events = slice
            .events
            .into_iter()
            .map(|event| {
                let redacted = event.is_redacted();
                Ok(ServiceEvent {
                    version: event.version,
                    event_id: event.event_id,
                    name: event.name,
                    occurred_at: event
                        .occurred_at
                        .format(&Rfc3339)
                        .map_err(|error| EventLogError::Invalid(error.to_string()))?,
                    subject: event.subject,
                    actor: event.actor,
                    redacted,
                    data: event.data,
                })
            })
            .collect::<Result<Vec<_>, EventLogError>>()?;
        Ok(EventPage {
            events,
            cursor: encode_event_cursor(&identity, position),
            has_more: !slice.end_of_stream,
        })
    }
}

impl EffectJournalPort for EventlogEffectJournal {
    type Error = EventLogError;

    fn prepare<'a>(
        &'a mut self,
        context: &'a VerifiedAuthContext,
        effect: PreparedEffect,
    ) -> BoxFuture<'a, Result<EffectRecord, Self::Error>> {
        Box::pin(async move {
            let stream = effect_stream(context, &self.service, &effect.operation_id)?;
            if let Some(record) = load_effect(&self.store, &stream).await? {
                if record.prepared == effect {
                    return Ok(record);
                }
                return Err(EventLogError::IdempotencyMismatch {
                    key: effect.idempotency_key,
                });
            }
            let body = serde_json::to_value(&effect)
                .map_err(|error| EventLogError::Invalid(error.to_string()))?;
            let event = NewEvent::new("service_effect_prepared", 1, body.clone())?;
            let meta = effect_meta(
                context,
                &effect.idempotency_key,
                &body,
                &effect.operation_id,
                OffsetDateTime::now_utc(),
            )?;
            self.store
                .append(&stream, Expected::NoStream, &[event], &meta)
                .await?;
            load_effect(&self.store, &stream)
                .await?
                .ok_or_else(|| EventLogError::Backend("prepared effect was not visible".into()))
        })
    }

    fn claim<'a>(
        &'a mut self,
        context: &'a VerifiedAuthContext,
        operation_id: &'a str,
        claim: EffectClaim,
        now: &'a str,
    ) -> BoxFuture<'a, Result<ClaimDisposition, Self::Error>> {
        Box::pin(async move {
            let stream = effect_stream(context, &self.service, operation_id)?;
            let current = load_effect(&self.store, &stream)
                .await?
                .ok_or(EventLogError::NotFound)?;
            match &current.state {
                EffectState::Completed { .. } => return Ok(ClaimDisposition::Terminal(current)),
                EffectState::Claimed { claim: active } if active.lease_id == claim.lease_id => {
                    return Ok(ClaimDisposition::Acquired(current));
                }
                EffectState::Claimed { claim: active }
                    if !lease_expired(&active.expires_at, now)? =>
                {
                    return Ok(ClaimDisposition::Busy(current));
                }
                EffectState::Prepared | EffectState::Claimed { .. } => {}
            }
            validate_claim(&claim, now)?;
            let body = serde_json::to_value(&claim)
                .map_err(|error| EventLogError::Invalid(error.to_string()))?;
            let event = NewEvent::new("service_effect_claimed", 1, body.clone())?;
            let meta = effect_meta(
                context,
                &format!("effect-claim:{}", claim.lease_id),
                &body,
                operation_id,
                parse_time(now)?,
            )?;
            self.store
                .append(&stream, Expected::Exact(current.revision), &[event], &meta)
                .await?;
            let record = load_effect(&self.store, &stream)
                .await?
                .ok_or_else(|| EventLogError::Backend("claimed effect was not visible".into()))?;
            Ok(ClaimDisposition::Acquired(record))
        })
    }

    fn complete<'a>(
        &'a mut self,
        context: &'a VerifiedAuthContext,
        operation_id: &'a str,
        lease_id: &'a str,
        outcome: EffectOutcome,
    ) -> BoxFuture<'a, Result<EffectRecord, Self::Error>> {
        Box::pin(async move {
            let stream = effect_stream(context, &self.service, operation_id)?;
            let current = load_effect(&self.store, &stream)
                .await?
                .ok_or(EventLogError::NotFound)?;
            match &current.state {
                EffectState::Completed { outcome: existing } if existing == &outcome => {
                    return Ok(current);
                }
                EffectState::Completed { .. } => {
                    return Err(EventLogError::IdempotencyMismatch {
                        key: format!("effect-complete:{lease_id}"),
                    });
                }
                EffectState::Claimed { claim } if claim.lease_id == lease_id => {}
                EffectState::Prepared | EffectState::Claimed { .. } => {
                    return Err(EventLogError::Conflict {
                        expected: current.revision,
                        actual: current.revision,
                    });
                }
            }
            let body = serde_json::to_value(&outcome)
                .map_err(|error| EventLogError::Invalid(error.to_string()))?;
            let event = NewEvent::new("service_effect_completed", 1, body.clone())?;
            let meta = effect_meta(
                context,
                &format!("effect-complete:{lease_id}"),
                &body,
                operation_id,
                OffsetDateTime::now_utc(),
            )?;
            self.store
                .append(&stream, Expected::Exact(current.revision), &[event], &meta)
                .await?;
            load_effect(&self.store, &stream)
                .await?
                .ok_or_else(|| EventLogError::Backend("completed effect was not visible".into()))
        })
    }
}

#[derive(Clone, Copy)]
struct ProjectionLayout {
    projector_name: &'static str,
    state: &'static ProjectionSpec,
    rows: &'static ProjectionSpec,
    specs: &'static [ProjectionSpec],
}

impl ProjectionLayout {
    fn for_service(service: &str) -> Self {
        // Keep 96 bits inside Eventlog's 40-byte SQL identifier ceiling. ServiceBundle already
        // rejects exact service identity collisions; this also makes accidental table aliases
        // negligible without embedding caller-controlled service text in SQL identifiers.
        let identity = &hex_digest(service.as_bytes())[..24];
        let projector_name = leak(format!("sdk_{identity}"));
        let state_name = leak(format!("sdk_{identity}_state"));
        let rows_name = leak(format!("sdk_{identity}_rows"));
        let state = Box::leak(Box::new(ProjectionSpec {
            name: state_name,
            indexed: &[],
        }));
        let rows = Box::leak(Box::new(ProjectionSpec {
            name: rows_name,
            indexed: &[],
        }));
        let specs = Box::leak(vec![*state, *rows].into_boxed_slice());
        Self {
            projector_name,
            state,
            rows,
            specs,
        }
    }

    fn specs(self) -> &'static [ProjectionSpec] {
        self.specs
    }
}

fn leak(value: String) -> &'static str {
    Box::leak(value.into_boxed_str())
}

struct ServiceProjector {
    engine: Arc<ServiceEngine>,
    layout: ProjectionLayout,
}

impl Projector for ServiceProjector {
    fn name(&self) -> &'static str {
        self.layout.projector_name
    }

    fn projections(&self) -> &'static [ProjectionSpec] {
        self.layout.specs()
    }

    fn apply<'a>(
        &'a self,
        event: &'a eventlog_core::RecordedEvent,
        store: &'a mut dyn DurableProjectionStore,
    ) -> eventlog_core::BoxFuture<'a, Result<(), EventLogError>> {
        Box::pin(async move {
            if event.stream_type != stream_type(&self.engine.plan().service) {
                return Ok(());
            }
            if event.is_redacted() {
                return Err(EventLogError::Invalid(
                    "a redacted generated-service event cannot drive a projection".to_owned(),
                ));
            }
            let (realm, category, key) = decode_stream_id(&event.stream_id)?;
            let source_stream = ServiceStream {
                service: self.engine.plan().service.clone(),
                tenant: event.tenant.as_str().to_owned(),
                realm: realm.clone(),
                category,
                key,
            };
            let state_key = event.stream_id.clone();
            let previous = store
                .get(self.layout.state, &event.tenant, &state_key)
                .await?
                .map(serde_json::from_value::<ProjectionState>)
                .transpose()
                .map_err(|_| {
                    EventLogError::Invalid("generated projection state is invalid".to_owned())
                })?
                .unwrap_or_default();
            for row in
                self.engine
                    .projection_rows(event.tenant.as_str(), realm.as_deref(), &previous)
            {
                store
                    .delete(
                        self.layout.rows,
                        &event.tenant,
                        &row_key(
                            &self.engine.plan().service,
                            realm.as_deref(),
                            &row.row.view,
                            &event.stream_id,
                            &row.entity_key,
                        ),
                    )
                    .await?;
            }
            let fields = event
                .data
                .as_object()
                .ok_or_else(|| {
                    EventLogError::Invalid("generated event body is not an object".to_owned())
                })?
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect();
            let mut next = previous;
            self.engine
                .apply_projection_event(
                    &mut next,
                    &DomainEvent {
                        name: event.name.clone(),
                        fields,
                    },
                )
                .map_err(|error| EventLogError::Invalid(error.to_string()))?;
            store
                .upsert(
                    self.layout.state,
                    &event.tenant,
                    &state_key,
                    &serde_json::to_value(&next).map_err(|_| {
                        EventLogError::Invalid("generated projection state is invalid".to_owned())
                    })?,
                )
                .await?;
            for mut row in
                self.engine
                    .projection_rows(event.tenant.as_str(), realm.as_deref(), &next)
            {
                row.row.source_stream = Some(source_stream.clone());
                store
                    .upsert(
                        self.layout.rows,
                        &event.tenant,
                        &row_key(
                            &self.engine.plan().service,
                            realm.as_deref(),
                            &row.row.view,
                            &event.stream_id,
                            &row.entity_key,
                        ),
                        &serde_json::to_value(row.row).map_err(|_| {
                            EventLogError::Invalid("generated projection row is invalid".to_owned())
                        })?,
                    )
                    .await?;
            }
            Ok(())
        })
    }
}

struct EventlogEvents {
    store: Arc<dyn DurableEventStore>,
}

impl EventlogEvents {
    fn new(store: Arc<dyn DurableEventStore>) -> Self {
        Self { store }
    }
}

impl EventStore for EventlogEvents {
    fn load<'a>(
        &'a mut self,
        stream: &'a ServiceStream,
    ) -> BoxFuture<'a, Result<LoadedStream, ResourceError>> {
        Box::pin(async move {
            let stream = durable_stream(stream).map_err(|_| ResourceError)?;
            let mut after = 0;
            let mut events = Vec::new();
            loop {
                let slice = self
                    .store
                    .read_stream(&stream, after, PAGE)
                    .await
                    .map_err(|_| ResourceError)?;
                for event in slice.events {
                    let fields = event
                        .data
                        .as_object()
                        .ok_or(ResourceError)?
                        .iter()
                        .map(|(name, value)| (name.clone(), value.clone()))
                        .collect();
                    after = event.version;
                    events.push(StoredEvent {
                        version: event.version,
                        event: DomainEvent {
                            name: event.name,
                            fields,
                        },
                    });
                }
                if slice.end_of_stream {
                    return Ok(LoadedStream {
                        version: after,
                        events,
                    });
                }
                after = slice.next_version.saturating_sub(1);
            }
        })
    }

    fn append(
        &mut self,
        request: AppendRequest,
    ) -> BoxFuture<'_, Result<AppendReceipt, ResourceError>> {
        Box::pin(async move {
            let stream = durable_stream(&request.stream).map_err(|_| ResourceError)?;
            let events = request
                .events
                .iter()
                .map(|event| {
                    NewEvent::new(
                        event.name.clone(),
                        1,
                        Value::Object(event.fields.clone().into_iter().collect()),
                    )
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| ResourceError)?;
            let request_body = serde_json::to_value(&request.events).map_err(|_| ResourceError)?;
            let meta = CommandMeta {
                idempotency_key: request.idempotency_key,
                request_hash: eventlog_core::request_hash(&request_body)
                    .map_err(|_| ResourceError)?,
                subject: request.metadata.subject,
                actor: request.metadata.actor,
                request_id: request.metadata.request_id,
                trace_id: request.metadata.trace_id,
                causation_id: None,
                causation_depth: 0,
                occurred_at: OffsetDateTime::parse(&request.metadata.occurred_at, &Rfc3339)
                    .map_err(|_| ResourceError)?,
                claim: None,
            };
            let expected = match request.expected {
                AppendExpectation::NoStream => Expected::NoStream,
                AppendExpectation::Exact(version) => Expected::Exact(version),
            };
            let result = self
                .store
                .append(&stream, expected, &events, &meta)
                .await
                .map_err(|_| ResourceError)?;
            Ok(AppendReceipt {
                disposition: if result.deduplicated {
                    AppendDisposition::Replayed
                } else {
                    AppendDisposition::Committed
                },
                through_version: result.last_version,
            })
        })
    }
}

struct EventlogProjections {
    store: Arc<dyn DurableEventStore>,
    rows: &'static ProjectionSpec,
    service: String,
    page: Option<PageRequest>,
    next_cursor: Option<String>,
}

impl EventlogProjections {
    fn new(
        store: Arc<dyn DurableEventStore>,
        rows: &'static ProjectionSpec,
        service: &str,
    ) -> Self {
        Self {
            store,
            rows,
            service: service.to_owned(),
            page: None,
            next_cursor: None,
        }
    }

    fn paged(
        store: Arc<dyn DurableEventStore>,
        rows: &'static ProjectionSpec,
        service: &str,
        page: PageRequest,
    ) -> Self {
        Self {
            store,
            rows,
            service: service.to_owned(),
            page: Some(page),
            next_cursor: None,
        }
    }
}

impl ProjectionStore for EventlogProjections {
    fn project(&mut self, _write: ProjectionWrite) -> BoxFuture<'_, Result<(), ResourceError>> {
        // Eventlog invokes the registered projector inside the append transaction. Returning from
        // append therefore already provides read-your-writes visibility.
        Box::pin(std::future::ready(Ok(())))
    }

    fn query(
        &mut self,
        read: ProjectionRead,
    ) -> BoxFuture<'_, Result<Vec<ProjectionRow>, ResourceError>> {
        Box::pin(async move {
            let tenant = TenantId::new(read.tenant.clone()).map_err(|_| ResourceError)?;
            let prefix = row_prefix(&self.service, read.realm.as_deref(), &read.view);
            if let Some(page) = &self.page {
                if page
                    .cursor()
                    .is_some_and(|cursor| !cursor.starts_with(&prefix))
                {
                    return Err(ResourceError);
                }
                let result = self
                    .store
                    .projection_page(
                        self.rows,
                        &tenant,
                        Some(&prefix),
                        page.cursor(),
                        page.limit(),
                    )
                    .await
                    .map_err(|_| ResourceError)?;
                self.next_cursor = result.next_cursor;
                return result
                    .rows
                    .into_iter()
                    .filter_map(|(_, body)| {
                        let row = serde_json::from_value::<ProjectionRow>(body)
                            .map_err(|_| ResourceError);
                        match row {
                            Ok(row)
                                if read.selectors.iter().all(|(name, expected)| {
                                    row.value.get(name) == Some(expected)
                                }) =>
                            {
                                Some(Ok(row))
                            }
                            Ok(_) => None,
                            Err(error) => Some(Err(error)),
                        }
                    })
                    .collect();
            }
            let mut after = prefix.clone();
            let mut rows = Vec::new();
            loop {
                let page = self
                    .store
                    .projection_list(self.rows, &tenant, Some(&after), PAGE)
                    .await
                    .map_err(|_| ResourceError)?;
                if page.is_empty() {
                    break;
                }
                let mut reached_other_prefix = false;
                for (key, body) in page {
                    after.clone_from(&key);
                    if !key.starts_with(&prefix) {
                        reached_other_prefix = true;
                        break;
                    }
                    let row: ProjectionRow =
                        serde_json::from_value(body).map_err(|_| ResourceError)?;
                    if read
                        .selectors
                        .iter()
                        .all(|(name, expected)| row.value.get(name) == Some(expected))
                    {
                        rows.push(row);
                        if rows.len() > MAX_QUERY_ROWS {
                            return Err(ResourceError);
                        }
                    }
                }
                if reached_other_prefix {
                    break;
                }
            }
            Ok(rows)
        })
    }
}

struct EventlogContent {
    store: Arc<dyn DurableEventStore>,
}

impl EventlogContent {
    fn new(store: Arc<dyn DurableEventStore>) -> Self {
        Self { store }
    }
}

impl ContentStore for EventlogContent {
    fn stage<'a>(
        &'a mut self,
        context: &'a VerifiedAuthContext,
        policy: &'a str,
        idempotency_key: &'a str,
        payload: ContentPayload<'a>,
    ) -> BoxFuture<'a, Result<StagedContent, ResourceError>> {
        Box::pin(async move {
            let tenant = TenantId::new(context.tenant().as_str()).map_err(|_| ResourceError)?;
            let mut hash = Sha256::new();
            frame_hash(&mut hash, b"service-content/1");
            frame_hash(&mut hash, context.tenant().as_str().as_bytes());
            frame_hash(
                &mut hash,
                context
                    .realm()
                    .map_or(&[][..], |realm| realm.as_str().as_bytes()),
            );
            frame_hash(&mut hash, policy.as_bytes());
            frame_hash(&mut hash, idempotency_key.as_bytes());
            frame_hash(&mut hash, payload.media_type.as_bytes());
            frame_hash(&mut hash, payload.bytes);
            let digest = format!("sha256:{}", hex::encode(hash.finalize()));
            self.store
                .put_blob(&tenant, &digest, payload.bytes)
                .await
                .map_err(|_| ResourceError)?;
            Ok(StagedContent {
                reference: format!("content:{digest}"),
                token: digest,
            })
        })
    }

    fn accept<'a>(
        &'a mut self,
        _context: &'a VerifiedAuthContext,
        _token: String,
    ) -> BoxFuture<'a, Result<(), ResourceError>> {
        Box::pin(std::future::ready(Ok(())))
    }

    fn abandon<'a>(
        &'a mut self,
        _context: &'a VerifiedAuthContext,
        _token: String,
    ) -> BoxFuture<'a, Result<(), ResourceError>> {
        // The digest is deterministic for idempotent retries. Deleting here could race a retry
        // whose event committed, so abandoned unreferenced blobs are left for bounded GC.
        Box::pin(std::future::ready(Ok(())))
    }
}

struct VerifiedAuthority {
    facts: AuthorityFacts,
}

impl VerifiedAuthority {
    fn new(facts: AuthorityFacts) -> Self {
        Self { facts }
    }

    fn scopes_allowed(&self, scopes: &Value) -> bool {
        let Some(scopes) = scopes.as_object() else {
            return false;
        };
        scopes.iter().all(|(axis, value)| match axis.as_str() {
            "principal" | "team" | "project" | "extension" if value.is_null() => true,
            "principal" => value
                .as_str()
                .is_some_and(|value| self.facts.principals.contains(value)),
            "team" => value
                .as_str()
                .is_some_and(|value| self.facts.teams.contains(value)),
            "project" => value
                .as_str()
                .is_some_and(|value| self.facts.projects.contains(value)),
            "extension" => value.as_object().is_some_and(|extension| {
                extension
                    .get("kind")
                    .and_then(Value::as_str)
                    .zip(extension.get("value").and_then(Value::as_str))
                    .is_some_and(|(kind, value)| {
                        self.facts.extensions.contains(&format!("{kind}:{value}"))
                    })
            }),
            _ => false,
        })
    }
}

impl AuthorityEvaluator for VerifiedAuthority {
    fn allows<'a>(
        &'a mut self,
        context: &'a VerifiedAuthContext,
        check: AuthorityCheck,
    ) -> BoxFuture<'a, Result<bool, ResourceError>> {
        Box::pin(std::future::ready(Ok(match check {
            AuthorityCheck::OwnerAndScopes { owner, scopes } => {
                owner.as_str() == Some(context.authority().as_str()) && self.scopes_allowed(&scopes)
            }
            AuthorityCheck::RequestedScopes { scopes } => self.scopes_allowed(&scopes),
            AuthorityCheck::OwnerTransfer { new_owner } => new_owner
                .as_str()
                .is_some_and(|owner| self.facts.principals.contains(owner)),
            AuthorityCheck::Capability { capability } => {
                self.facts.capabilities.contains(&capability)
            }
        })))
    }
}

struct SystemClock;

impl Clock for SystemClock {
    fn now(&mut self) -> Result<String, ResourceError> {
        OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map_err(|_| ResourceError)
    }
}

struct UuidV7;

impl IdGenerator for UuidV7 {
    fn uuid_v7(&mut self) -> Result<String, ResourceError> {
        Ok(Uuid::now_v7().to_string())
    }
}

fn effect_stream(
    context: &VerifiedAuthContext,
    service: &str,
    operation_id: &str,
) -> Result<StreamId, EventLogError> {
    StreamId::new(
        TenantId::new(context.tenant().as_str())?,
        format!("generated-service-effect:{service}"),
        encode_stream_id(
            context.realm().map(service_runtime::RealmId::as_str),
            "external-effect",
            operation_id,
        ),
    )
}

async fn load_effect(
    store: &Arc<dyn DurableEventStore>,
    stream: &StreamId,
) -> Result<Option<EffectRecord>, EventLogError> {
    let mut after = 0;
    let mut prepared = None;
    let mut state = None;
    loop {
        let slice = store.read_stream(stream, after, PAGE).await?;
        if slice.events.is_empty() && after == 0 {
            return Ok(None);
        }
        for event in slice.events {
            if event.version != after + 1 || event.is_redacted() {
                return Err(EventLogError::Invalid(
                    "external effect journal is incomplete".to_owned(),
                ));
            }
            after = event.version;
            match event.name.as_str() {
                "service_effect_prepared" if prepared.is_none() && state.is_none() => {
                    prepared = Some(serde_json::from_value(event.data).map_err(|error| {
                        EventLogError::Invalid(format!(
                            "external effect prepared event is invalid: {error}"
                        ))
                    })?);
                    state = Some(EffectState::Prepared);
                }
                "service_effect_claimed" if prepared.is_some() => {
                    let claim = serde_json::from_value(event.data).map_err(|error| {
                        EventLogError::Invalid(format!(
                            "external effect claim event is invalid: {error}"
                        ))
                    })?;
                    state = Some(EffectState::Claimed { claim });
                }
                "service_effect_completed"
                    if matches!(state, Some(EffectState::Claimed { .. })) =>
                {
                    let outcome = serde_json::from_value(event.data).map_err(|error| {
                        EventLogError::Invalid(format!(
                            "external effect completion event is invalid: {error}"
                        ))
                    })?;
                    state = Some(EffectState::Completed { outcome });
                }
                _ => {
                    return Err(EventLogError::Invalid(
                        "external effect journal transition is invalid".to_owned(),
                    ));
                }
            }
        }
        if slice.end_of_stream {
            break;
        }
        after = slice.next_version.saturating_sub(1);
    }
    Ok(Some(EffectRecord {
        prepared: prepared.ok_or_else(|| {
            EventLogError::Invalid("external effect journal has no prepared event".to_owned())
        })?,
        state: state.ok_or_else(|| {
            EventLogError::Invalid("external effect journal has no state".to_owned())
        })?,
        revision: after,
    }))
}

fn effect_meta(
    context: &VerifiedAuthContext,
    idempotency_key: &str,
    body: &Value,
    operation_id: &str,
    occurred_at: OffsetDateTime,
) -> Result<CommandMeta, EventLogError> {
    Ok(CommandMeta {
        idempotency_key: idempotency_key.to_owned(),
        request_hash: eventlog_core::request_hash(body)?,
        subject: context.authority().as_str().to_owned(),
        actor: context.executor().map_or_else(
            || context.authority().as_str().to_owned(),
            |executor| executor.as_str().to_owned(),
        ),
        request_id: operation_id.to_owned(),
        trace_id: operation_id.to_owned(),
        causation_id: None,
        causation_depth: 0,
        occurred_at,
        claim: None,
    })
}

fn validate_claim(claim: &EffectClaim, now: &str) -> Result<(), EventLogError> {
    if claim.lease_id.trim().is_empty() || claim.worker.trim().is_empty() {
        return Err(EventLogError::Invalid(
            "external effect claim identity is empty".to_owned(),
        ));
    }
    if parse_time(&claim.expires_at)? <= parse_time(now)? {
        return Err(EventLogError::Invalid(
            "external effect claim must expire in the future".to_owned(),
        ));
    }
    Ok(())
}

fn lease_expired(expires_at: &str, now: &str) -> Result<bool, EventLogError> {
    Ok(parse_time(expires_at)? <= parse_time(now)?)
}

fn parse_time(value: &str) -> Result<OffsetDateTime, EventLogError> {
    OffsetDateTime::parse(value, &Rfc3339)
        .map_err(|_| EventLogError::Invalid("external effect time is invalid".to_owned()))
}

fn durable_stream(stream: &ServiceStream) -> Result<StreamId, EventLogError> {
    StreamId::new(
        TenantId::new(stream.tenant.clone())?,
        stream_type(&stream.service),
        encode_stream_id(stream.realm.as_deref(), &stream.category, &stream.key),
    )
}

fn single_authorized_stream(
    rows: &[service_engine::AuthorizedProjectionRow],
) -> Option<&ServiceStream> {
    let first = rows.first()?.source_stream.as_ref()?;
    rows.iter()
        .all(|row| row.source_stream.as_ref() == Some(first))
        .then_some(first)
}

fn stream_type(service: &str) -> String {
    format!("generated-service:{service}")
}

fn feed_identity(store_identity: &str, stream: &StreamId) -> String {
    let mut hash = Sha256::new();
    frame_hash(&mut hash, b"service-event-feed/1");
    frame_hash(&mut hash, store_identity.as_bytes());
    frame_hash(&mut hash, stream.tenant().as_str().as_bytes());
    frame_hash(&mut hash, stream.stream_type().as_bytes());
    frame_hash(&mut hash, stream.stream_id().as_bytes());
    hex::encode(hash.finalize())
}

fn encode_event_cursor(identity: &str, position: u64) -> String {
    format!("service-event-cursor/1:{identity}:{position}")
}

fn decode_event_cursor(cursor: &str, expected_identity: &str) -> Result<u64, EventPageError> {
    let encoded = cursor
        .strip_prefix("service-event-cursor/1:")
        .ok_or(EventPageError::Cursor)?;
    let (identity, position) = encoded.split_once(':').ok_or(EventPageError::Cursor)?;
    if identity != expected_identity {
        return Err(EventPageError::Cursor);
    }
    position.parse().map_err(|_| EventPageError::Cursor)
}

fn encode_stream_id(realm: Option<&str>, category: &str, key: &str) -> String {
    let realm = realm.map_or_else(
        || "0:".to_owned(),
        |realm| format!("1:{}:{realm}", realm.len()),
    );
    format!("{realm}|{}:{category}|{}:{key}", category.len(), key.len())
}

fn decode_stream_id(value: &str) -> Result<(Option<String>, String, String), EventLogError> {
    let (realm, rest) = value
        .split_once('|')
        .ok_or_else(|| EventLogError::Invalid("generated stream identity is invalid".to_owned()))?;
    let realm = if realm == "0:" {
        None
    } else {
        let encoded = realm.strip_prefix("1:").ok_or_else(|| {
            EventLogError::Invalid("generated realm identity is invalid".to_owned())
        })?;
        let (length, realm) = encoded.split_once(':').ok_or_else(|| {
            EventLogError::Invalid("generated realm identity is invalid".to_owned())
        })?;
        let length = length.parse::<usize>().map_err(|_| {
            EventLogError::Invalid("generated realm identity is invalid".to_owned())
        })?;
        if realm.len() != length || realm.is_empty() {
            return Err(EventLogError::Invalid(
                "generated realm identity is invalid".to_owned(),
            ));
        }
        Some(realm.to_owned())
    };
    let (category, key) = rest
        .split_once('|')
        .ok_or_else(|| EventLogError::Invalid("generated stream identity is invalid".to_owned()))?;
    Ok((realm, decode_framed(category)?, decode_framed(key)?))
}

fn decode_framed(value: &str) -> Result<String, EventLogError> {
    let (length, value) = value
        .split_once(':')
        .ok_or_else(|| EventLogError::Invalid("generated stream identity is invalid".to_owned()))?;
    let length = length
        .parse::<usize>()
        .map_err(|_| EventLogError::Invalid("generated stream identity is invalid".to_owned()))?;
    if value.len() != length || value.is_empty() {
        return Err(EventLogError::Invalid(
            "generated stream identity is invalid".to_owned(),
        ));
    }
    Ok(value.to_owned())
}

fn row_prefix(service: &str, realm: Option<&str>, view: &str) -> String {
    format!(
        "{}/{}/{}/",
        hex_digest(service.as_bytes()),
        hex_digest(realm.unwrap_or("").as_bytes()),
        hex_digest(view.as_bytes())
    )
}

fn row_key(service: &str, realm: Option<&str>, view: &str, stream: &str, entity: &str) -> String {
    format!(
        "{}{}-{}",
        row_prefix(service, realm, view),
        hex_digest(stream.as_bytes()),
        hex_digest(entity.as_bytes())
    )
}

fn hex_digest(value: &[u8]) -> String {
    hex::encode(Sha256::digest(value))
}

fn frame_hash(hash: &mut Sha256, value: &[u8]) {
    hash.update((value.len() as u64).to_be_bytes());
    hash.update(value);
}

#[cfg(test)]
mod tests {
    use super::*;
    use eventlog_sqlite::SqliteEventStore;
    use service_runtime::{
        AuthorityId, EFFECT_PLAN_FORMAT, EffectPlan, EffectRisk, ExecutorId,
        TenantId as ServiceTenantId, UserId, VerifiedIdentity,
    };

    #[test]
    fn stream_encoding_preserves_absent_and_literal_default_realms() {
        let absent = encode_stream_id(None, "todo-list", "list-1");
        let default = encode_stream_id(Some("default"), "todo-list", "list-1");
        assert_ne!(absent, default);
        assert_eq!(decode_stream_id(&absent).unwrap().0, None);
        assert_eq!(
            decode_stream_id(&default).unwrap().0.as_deref(),
            Some("default")
        );
    }

    #[test]
    fn conjunctive_scope_facts_fail_closed() {
        let authority = VerifiedAuthority::new(AuthorityFacts {
            principals: BTreeSet::from(["person:alice".to_owned()]),
            teams: BTreeSet::from(["engineering".to_owned()]),
            ..AuthorityFacts::default()
        });
        assert!(authority.scopes_allowed(&serde_json::json!({
            "principal": "person:alice",
            "team": "engineering",
            "project": null,
            "extension": null
        })));
        assert!(!authority.scopes_allowed(&serde_json::json!({
            "project": "unverified-project"
        })));
    }

    #[test]
    fn page_requests_refuse_empty_cursors_and_unbounded_limits() {
        assert_eq!(PageRequest::new(None, 0), Err(PageRequestError::Limit));
        assert_eq!(
            PageRequest::new(Some(String::new()), 10),
            Err(PageRequestError::Cursor)
        );
        assert_eq!(
            PageRequest::new(None, MAX_PAGE_ROWS + 1),
            Err(PageRequestError::Limit)
        );
        assert_eq!(PageRequest::new(None, 10).unwrap().limit(), 10);
    }

    #[test]
    fn query_revision_is_available_only_for_one_authorized_aggregate() {
        let stream = ServiceStream {
            service: "agentide".to_owned(),
            tenant: "tenant-a".to_owned(),
            realm: None,
            category: "agentide-session".to_owned(),
            key: "session-a".to_owned(),
        };
        let row = |source_stream| service_engine::AuthorizedProjectionRow {
            value: BTreeMap::new(),
            source_stream,
        };

        assert_eq!(
            single_authorized_stream(&[row(Some(stream.clone())), row(Some(stream.clone()))]),
            Some(&stream)
        );
        assert!(single_authorized_stream(&[row(None)]).is_none());

        let mut other = stream.clone();
        other.key = "session-b".to_owned();
        assert!(
            single_authorized_stream(&[row(Some(stream)), row(Some(other))]).is_none(),
            "a mixed-aggregate page must not publish either stream version"
        );
    }

    #[test]
    fn event_cursors_are_bound_to_the_store_and_aggregate_identity() {
        let cursor = encode_event_cursor(&"a".repeat(64), 42);
        assert_eq!(decode_event_cursor(&cursor, &"a".repeat(64)).unwrap(), 42);
        assert!(matches!(
            decode_event_cursor(&cursor, &"b".repeat(64)),
            Err(EventPageError::Cursor)
        ));
        assert!(matches!(
            decode_event_cursor("42", &"a".repeat(64)),
            Err(EventPageError::Cursor)
        ));
    }

    #[tokio::test]
    async fn effect_journal_prepares_claims_and_completes_in_eventlog() {
        let store: Arc<dyn DurableEventStore> = Arc::new(
            SqliteEventStore::in_memory("service_effect_test")
                .await
                .unwrap(),
        );
        let context = VerifiedAuthContext::from_verified(VerifiedIdentity::after_verification(
            ServiceTenantId::new("tenant-a").unwrap(),
            AuthorityId::new("person-a").unwrap(),
            UserId::new("person-a").unwrap(),
            Some(ExecutorId::new("agent-a").unwrap()),
            None,
        ));
        let effect = EffectPlan {
            format: EFFECT_PLAN_FORMAT.to_owned(),
            service: "agentide".to_owned(),
            operation: "code_edit".to_owned(),
            input_digest: "a".repeat(64),
            input_reference: Some("content:sha256:body".to_owned()),
            binding_digest: "b".repeat(64),
            aggregate_version: 4,
            resource_revision: "manifest:workspace".to_owned(),
            authority_reference: "authority:one-shot".to_owned(),
            grant_reference: Some("grant:agentide".to_owned()),
            grant_revision: Some(3),
            risk: EffectRisk::Medium,
            consequences: BTreeSet::from(["write_file".to_owned()]),
        }
        .prepare("request-1")
        .unwrap();
        let mut journal = EventlogEffectJournal {
            store,
            service: "agentide".to_owned(),
        };

        let prepared = journal.prepare(&context, effect.clone()).await.unwrap();
        assert_eq!(prepared.revision, 1);
        assert_eq!(prepared.state, EffectState::Prepared);
        assert_eq!(
            journal.prepare(&context, effect.clone()).await.unwrap(),
            prepared
        );

        let claim = EffectClaim {
            lease_id: "lease-1".to_owned(),
            worker: "worker-1".to_owned(),
            expires_at: "2030-01-01T00:01:00Z".to_owned(),
        };
        let claimed = journal
            .claim(
                &context,
                &effect.operation_id,
                claim.clone(),
                "2030-01-01T00:00:00Z",
            )
            .await
            .unwrap();
        assert!(matches!(claimed, ClaimDisposition::Acquired(_)));

        let outcome = EffectOutcome::Succeeded {
            result_reference: "evidence:workspace-operation".to_owned(),
            result_digest: "sha256:result".to_owned(),
        };
        let completed = journal
            .complete(
                &context,
                &effect.operation_id,
                &claim.lease_id,
                outcome.clone(),
            )
            .await
            .unwrap();
        assert_eq!(
            completed.state,
            EffectState::Completed {
                outcome: outcome.clone()
            }
        );
        assert!(matches!(
            journal
                .claim(
                    &context,
                    &effect.operation_id,
                    EffectClaim {
                        lease_id: "lease-2".to_owned(),
                        worker: "worker-2".to_owned(),
                        expires_at: "2030-01-01T00:02:00Z".to_owned(),
                    },
                    "2030-01-01T00:01:00Z",
                )
                .await
                .unwrap(),
            ClaimDisposition::Terminal(_)
        ));
    }
}
