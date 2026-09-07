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
    IdGenerator, IntentClaimRequest, IntentResult, LoadedStream, ProjectionRead, ProjectionRow,
    ProjectionState, ProjectionStore, ProjectionWrite, RecordedIntent, RequestMetadata,
    ResourceError, ServiceEngine, ServiceResources, ServiceStream, StagedContent, StoredEvent,
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
    /// Declares the unchanged generated projection names for an exact host roster.
    pub fn projection_declaration(service: &str) -> (&'static str, &'static [ProjectionSpec]) {
        let layout = ProjectionLayout::for_service(service);
        (layout.projector_name, layout.specs())
    }

    /// Constructs the SDK projector for explicit owner-operated maintenance.
    ///
    /// Construction does not register the projector or mutate a store. The owner must fence
    /// every writer before rebuilding through a fresh operational provider with the exact
    /// approved plan. A provider's `is_inline` check covers only that local store instance;
    /// it does not establish a deployment-wide writer fence.
    #[must_use]
    pub fn projector(engine: ServiceEngine) -> Arc<dyn Projector> {
        Arc::new(ServiceProjector::new(Arc::new(engine)))
    }

    /// Performs a real bounded backend read without minting a tenant or a feed identity.
    pub async fn readiness(&self) -> Result<(), EventLogError> {
        let probe = StreamId::new(
            TenantId::new("sdk-host-probe")?,
            "sdk-host-probe",
            "readiness",
        )?;
        self.store.stream_version(&probe).await.map(|_| ())
    }
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
        let projector = Arc::new(ServiceProjector::new(Arc::clone(&engine)));
        let projection = projector.layout;
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
        let mut events = EventlogEvents::new(Arc::clone(&self.store), self.projection);
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
        let mut events = EventlogEvents::new(Arc::clone(&self.store), self.projection);
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
        let mut events = EventlogEvents::new(Arc::clone(&self.store), self.projection);
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

impl ServiceProjector {
    fn new(engine: Arc<ServiceEngine>) -> Self {
        let layout = ProjectionLayout::for_service(&engine.plan().service);
        Self { engine, layout }
    }
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
    layout: ProjectionLayout,
}

impl EventlogEvents {
    fn new(store: Arc<dyn DurableEventStore>, layout: ProjectionLayout) -> Self {
        Self { store, layout }
    }
}

impl EventStore for EventlogEvents {
    fn recorded_intent<'a>(
        &'a mut self,
        claim: &'a IntentClaimRequest,
    ) -> BoxFuture<'a, Result<Option<RecordedIntent>, ResourceError>> {
        Box::pin(async move {
            let tenant = TenantId::new(&claim.tenant).map_err(|_| ResourceError)?;
            let durable_claim = intent_claim(claim)?;
            let Some(recorded) = self
                .store
                .recorded_claim(&tenant, &durable_claim)
                .await
                .map_err(|_| ResourceError)?
            else {
                return Ok(None);
            };
            let count = recorded
                .last_version
                .checked_sub(recorded.first_version)
                .and_then(|distance| distance.checked_add(1))
                .filter(|count| *count <= eventlog_core::MAX_EVENTS_PER_APPEND as u64)
                .ok_or(ResourceError)?;
            if recorded.first_version == 0 {
                return Err(ResourceError);
            }
            let mut events = Vec::with_capacity(usize::try_from(count).map_err(|_| ResourceError)?);
            let mut after = recorded.first_version - 1;
            while after < recorded.last_version {
                let remaining =
                    usize::try_from(recorded.last_version - after).map_err(|_| ResourceError)?;
                let expected_count = remaining.min(eventlog_core::MAX_READ_LIMIT);
                let page = self
                    .store
                    .read_stream(&recorded.stream, after, expected_count)
                    .await
                    .map_err(|_| ResourceError)?;
                if page.events.len() != expected_count {
                    return Err(ResourceError);
                }
                for event in page.events {
                    if event.version != after + 1 {
                        return Err(ResourceError);
                    }
                    after = event.version;
                    events.push(event);
                }
            }
            Ok(Some(recorded_intent(
                claim,
                &events,
                recorded.first_version,
                recorded.last_version,
            )?))
        })
    }

    fn authorization_state<'a>(
        &'a mut self,
        stream: &'a ServiceStream,
    ) -> BoxFuture<'a, Result<ProjectionState, ResourceError>> {
        Box::pin(async move {
            let durable = durable_stream(stream).map_err(|_| ResourceError)?;
            let state = self
                .store
                .projection_get(self.layout.state, durable.tenant(), durable.stream_id())
                .await
                .map_err(|_| ResourceError)?
                .ok_or(ResourceError)?;
            serde_json::from_value(state).map_err(|_| ResourceError)
        })
    }

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
                claim: Some(intent_claim(&request.claim)?),
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
            let recorded = recorded_intent(
                &request.claim,
                &result.events,
                result.first_version,
                result.last_version,
            )?;
            Ok(AppendReceipt {
                disposition: if result.deduplicated {
                    AppendDisposition::Replayed
                } else {
                    AppendDisposition::Committed
                },
                through_version: result.last_version,
                stream: recorded.stream,
                events: recorded.events,
            })
        })
    }
}

fn intent_claim(claim: &IntentClaimRequest) -> Result<eventlog_core::Claim, ResourceError> {
    let scope = eventlog_core::request_hash(&serde_json::json!({
        "protocol": "service-intent-claim/1",
        "service": claim.service,
        "realm": claim.realm,
        "category": claim.category,
        "selector": match &claim.stream_key {
            Some(key) => serde_json::json!({"kind": "command_field", "key": key}),
            None => serde_json::json!({"kind": "generated_uuid_v7"}),
        },
    }))
    .map_err(|_| ResourceError)?;
    eventlog_core::Claim::new(
        format!("service-intent-claim/1:{scope}"),
        &claim.key,
        &claim.digest,
    )
    .map_err(|_| ResourceError)
}

fn recorded_intent(
    claim: &IntentClaimRequest,
    events: &[eventlog_core::RecordedEvent],
    first: u64,
    last: u64,
) -> Result<RecordedIntent, ResourceError> {
    let first_event = events.first().ok_or(ResourceError)?;
    let (realm, category, key) =
        decode_stream_id(&first_event.stream_id).map_err(|_| ResourceError)?;
    let expected_count = last
        .checked_sub(first)
        .and_then(|distance| distance.checked_add(1))
        .ok_or(ResourceError)?;
    if first == 0
        || events.len() > eventlog_core::MAX_EVENTS_PER_APPEND
        || expected_count != events.len() as u64
        || first_event.tenant.as_str() != claim.tenant
        || first_event.stream_type != stream_type(&claim.service)
        || realm != claim.realm
        || category != claim.category
        || claim
            .stream_key
            .as_ref()
            .is_some_and(|expected| expected != &key)
    {
        return Err(ResourceError);
    }
    let mut domain_events = Vec::with_capacity(events.len());
    for (offset, event) in events.iter().enumerate() {
        if event.is_redacted()
            || event.schema_version != 1
            || event.version != first + offset as u64
            || event.tenant != first_event.tenant
            || event.stream_type != first_event.stream_type
            || event.stream_id != first_event.stream_id
        {
            return Err(ResourceError);
        }
        let fields = event
            .data
            .as_object()
            .ok_or(ResourceError)?
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        domain_events.push(DomainEvent {
            name: event.name.clone(),
            fields,
        });
    }
    Ok(RecordedIntent {
        stream: ServiceStream {
            service: claim.service.clone(),
            tenant: claim.tenant.clone(),
            realm,
            category,
            key,
        },
        events: domain_events,
        through_version: last,
    })
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
    fn historical_generated_stream_projection_feed_and_effect_vectors_remain_exact() {
        let layout = ProjectionLayout::for_service("fixture");
        assert_eq!(layout.projector_name, "sdk_f16d05ec6b29248d2c61adb1");
        assert_eq!(layout.state.name, "sdk_f16d05ec6b29248d2c61adb1_state");
        assert_eq!(layout.rows.name, "sdk_f16d05ec6b29248d2c61adb1_rows");
        let service = ServiceStream {
            service: "fixture".into(),
            tenant: "tenant-a".into(),
            realm: None,
            category: "document".into(),
            key: "doc-a".into(),
        };
        let stream = durable_stream(&service).unwrap();
        assert_eq!(stream.stream_type(), "generated-service:fixture");
        assert_eq!(stream.stream_id(), "0:|8:document|5:doc-a");
        assert_eq!(
            encode_stream_id(Some("default"), "document", "doc-a"),
            "1:7:default|8:document|5:doc-a"
        );
        let digest = "d631cfee0cf1235d4e100fd0a53b1b0895580356531ea086f5dbc4ffae9f848f";
        assert_eq!(feed_identity("store-a", &stream), digest);
        assert_eq!(
            encode_event_cursor(digest, 42),
            format!("service-event-cursor/1:{digest}:42")
        );
        let effect = effect_stream(&retry_context(None), "fixture", "effect-a").unwrap();
        assert_eq!(effect.stream_type(), "generated-service-effect:fixture");
        assert_eq!(effect.stream_id(), "0:|15:external-effect|8:effect-a");
    }

    #[tokio::test]
    async fn historical_content_digest_and_cross_service_custody_are_unchanged() {
        let store: Arc<dyn DurableEventStore> = Arc::new(
            SqliteEventStore::in_memory("sdk_content_vector")
                .await
                .unwrap(),
        );
        let mut content = EventlogContent::new(store.clone());
        let caller = retry_context(None);
        let staged = content
            .stage(
                &caller,
                "body",
                "same-create-key",
                ContentPayload {
                    media_type: "text/plain",
                    bytes: b"retained content",
                },
            )
            .await
            .unwrap();
        assert_eq!(
            staged.reference,
            "content:sha256:769f7ade05a6173c4b427e086bd860b060d4279e566a3790755910440ab4e971"
        );
        // The existing digest has no service field; changing that would fork retained references.
        let mut another_adapter = EventlogContent::new(store);
        let same = another_adapter
            .stage(
                &caller,
                "body",
                "same-create-key",
                ContentPayload {
                    media_type: "text/plain",
                    bytes: b"retained content",
                },
            )
            .await
            .unwrap();
        assert_eq!(same.reference, staged.reference);
    }

    #[tokio::test]
    async fn recorded_receipt_refuses_gaps_redaction_schema_partition_and_unbounded_batches() {
        let store = SqliteEventStore::in_memory("sdk_receipt_validation")
            .await
            .unwrap();
        let stream = StreamId::new(
            TenantId::new("tenant-a").unwrap(),
            "generated-service:fixture",
            "0:|8:document|5:doc-a",
        )
        .unwrap();
        let event =
            NewEvent::new("fixture.Created", 1, serde_json::json!({"id": "doc-a"})).unwrap();
        let meta = CommandMeta {
            idempotency_key: "old-command".into(),
            request_hash: "old-event-hash".into(),
            subject: "person-a".into(),
            actor: "person-a".into(),
            request_id: "request-a".into(),
            trace_id: "trace-a".into(),
            causation_id: None,
            causation_depth: 0,
            occurred_at: OffsetDateTime::UNIX_EPOCH,
            claim: None,
        };
        let result = store
            .append(&stream, Expected::NoStream, &[event], &meta)
            .await
            .unwrap();
        let claim = IntentClaimRequest {
            tenant: "tenant-a".into(),
            service: "fixture".into(),
            realm: None,
            category: "document".into(),
            stream_key: Some("doc-a".into()),
            key: "original-intent".into(),
            digest: "a".repeat(64),
        };
        assert!(
            store
                .recorded_claim(stream.tenant(), &intent_claim(&claim).unwrap())
                .await
                .unwrap()
                .is_none(),
            "legacy records must not acquire fabricated original-input receipts"
        );
        assert!(recorded_intent(&claim, &result.events, 1, 1).is_ok());
        for (first, last) in [(0, 0), (2, 1), (1, 2), (1, u64::MAX)] {
            assert!(recorded_intent(&claim, &result.events, first, last).is_err());
        }
        let mut unsupported = result.events.clone();
        unsupported[0].schema_version = 2;
        assert!(recorded_intent(&claim, &unsupported, 1, 1).is_err());
        let mut foreign = result.events.clone();
        foreign[0].tenant = TenantId::new("tenant-b").unwrap();
        assert!(recorded_intent(&claim, &foreign, 1, 1).is_err());
        let mut oversized = vec![result.events[0].clone(); 1025];
        for (index, event) in oversized.iter_mut().enumerate() {
            event.version = u64::try_from(index).unwrap() + 1;
        }
        assert!(recorded_intent(&claim, &oversized, 1, 1025).is_err());
        let redacted = store.redact(&stream, 1, "fixture erasure").await.unwrap();
        assert!(recorded_intent(&claim, &[redacted], 1, 1).is_err());
    }

    #[tokio::test]
    async fn retries_recheck_current_owner_transfer_and_scheduler_facts() {
        for provider in [
            "sdk.auth.same-partition-owner-transfer/v1",
            "sdk.auth.trusted-scheduler/v1",
        ] {
            let store: Arc<dyn DurableEventStore> =
                Arc::new(SqliteEventStore::in_memory("sdk_more_auth").await.unwrap());
            let mut plan = retry_plan("fixture", true);
            let intent = plan.intents.get_mut("create").unwrap();
            intent.inputs.push(service_engine::InputPlan {
                name: "new_owner".into(),
                type_ref: "String".into(),
                optional: false,
                source: service_engine::InputSource::Command,
            });
            intent.obligations.push(service_engine::ObligationUse {
                provider: provider.into(),
                bindings: BTreeMap::from([
                    ("owner".into(), "fixture.Document.owner".into()),
                    ("capability".into(), "scheduler".into()),
                ]),
            });
            let service = EventlogService::initialize(store.clone(), ServiceEngine::new(plan))
                .await
                .unwrap();
            let mut body: Value = serde_json::from_slice(&create_body(true)).unwrap();
            body["new_owner"] = Value::String("next-owner".into());
            let body = serde_json::to_vec(&body).unwrap();
            let mut allowed = retry_facts();
            allowed.principals.insert("next-owner".into());
            allowed.capabilities.insert("scheduler".into());
            let first = service
                .intent(
                    &retry_context(None),
                    allowed.clone(),
                    RequestMetadata::default(),
                    "create",
                    &body,
                )
                .await
                .unwrap();
            assert!(matches!(
                service.intent(&retry_context(None), retry_facts(), RequestMetadata::default(), "create", &body).await,
                Err(service_engine::ExecutionError::ObligationRefused(ref reason)) if reason == "forbidden"
            ));
            let replay = service
                .intent(
                    &retry_context(None),
                    allowed,
                    RequestMetadata::default(),
                    "create",
                    &body,
                )
                .await
                .unwrap();
            assert!(replay.replayed);
            assert_eq!(replay.events, first.events);
            assert_eq!(
                store
                    .read_feed(&TenantId::new("tenant-a").unwrap(), 0, 100)
                    .await
                    .unwrap()
                    .events
                    .len(),
                1
            );
        }
    }

    // An SDK-owned test plan exercises reusable execution semantics. No application
    // definition or generated application artifact is modified by this fixture.
    use service_engine::{
        ContentPolicyPlan, ContextSource, EventFieldPlan, ExpectedVersionPlan, IdempotencyPlan,
        InputPlan, InputSource, IntentPlan, ObligationUse, OutcomePlan, PlanDelivery,
        PlanRealmPolicy, ProducedEventPlan, REALIZATION_PLAN_FORMAT, ReducerEffect, ReducerPlan,
        ServicePlan, StreamPlan, ValueSource,
    };

    fn retry_input(name: &str, source: InputSource) -> InputPlan {
        InputPlan {
            name: name.to_owned(),
            type_ref: "String".to_owned(),
            optional: false,
            source,
        }
    }
    fn retry_field(name: &str, source: ValueSource) -> EventFieldPlan {
        EventFieldPlan {
            name: name.to_owned(),
            source,
        }
    }

    fn retry_create(generated_stream: bool) -> IntentPlan {
        let aggregate = ObligationUse {
            provider: "sdk.aggregate.event-sourced/v1".to_owned(),
            bindings: BTreeMap::from([("category".to_owned(), "document".to_owned())]),
        };
        let scoped = ObligationUse {
            provider: "sdk.auth.requested-scopes/v1".to_owned(),
            bindings: BTreeMap::from([("scopes".to_owned(), "scopes".to_owned())]),
        };
        let mut create_inputs = vec![
            retry_input("scopes", InputSource::Command),
            retry_input("key", InputSource::Idempotency),
            retry_input(
                "content",
                InputSource::Content {
                    policy: "body".to_owned(),
                    command_field: "content_ref".to_owned(),
                },
            ),
        ];
        if !generated_stream {
            create_inputs.push(retry_input("id", InputSource::Command));
        }
        IntentPlan {
            command: "fixture.Create".to_owned(),
            scope: "documents.manage".to_owned(),
            inputs: create_inputs,
            stream: if generated_stream {
                StreamPlan::GeneratedUuidV7
            } else {
                StreamPlan::CommandField {
                    field: "id".to_owned(),
                }
            },
            expected_version: ExpectedVersionPlan::NoStream,
            idempotency: IdempotencyPlan::OperationField {
                field: "key".to_owned(),
            },
            obligations: vec![aggregate, scoped],
            outcome: OutcomePlan {
                name: "created".to_owned(),
                events: vec![ProducedEventPlan {
                    event: "fixture.Created".to_owned(),
                    fields: vec![
                        retry_field("id", ValueSource::StreamId),
                        retry_field("revision_id", ValueSource::GeneratedUuidV7),
                        retry_field(
                            "owner",
                            ValueSource::Context {
                                value: ContextSource::CurrentAuthority,
                            },
                        ),
                        retry_field(
                            "scopes",
                            ValueSource::Input {
                                field: "scopes".to_owned(),
                            },
                        ),
                        retry_field(
                            "content_ref",
                            ValueSource::Input {
                                field: "content_ref".to_owned(),
                            },
                        ),
                    ],
                }],
            },
            projections: Vec::new(),
        }
    }

    fn retry_close() -> IntentPlan {
        let aggregate = ObligationUse {
            provider: "sdk.aggregate.event-sourced/v1".to_owned(),
            bindings: BTreeMap::from([("category".to_owned(), "document".to_owned())]),
        };
        IntentPlan {
            command: "fixture.Close".to_owned(),
            scope: "documents.manage".to_owned(),
            inputs: vec![
                retry_input("id", InputSource::Command),
                retry_input("key", InputSource::Idempotency),
                retry_input("version", InputSource::ExpectedVersion),
            ],
            stream: StreamPlan::CommandField {
                field: "id".to_owned(),
            },
            expected_version: ExpectedVersionPlan::OperationField {
                field: "version".to_owned(),
            },
            idempotency: IdempotencyPlan::OperationField {
                field: "key".to_owned(),
            },
            obligations: vec![
                aggregate,
                ObligationUse {
                    provider: "sdk.auth.owner-and-conjunctive-scopes/v1".to_owned(),
                    bindings: BTreeMap::from([
                        ("owner".to_owned(), "fixture.Document.owner".to_owned()),
                        ("scopes".to_owned(), "fixture.Document.scopes".to_owned()),
                    ]),
                },
                ObligationUse {
                    provider: "sdk.lifecycle.require-state/v1".to_owned(),
                    bindings: BTreeMap::from([
                        ("entity".to_owned(), "fixture.Document".to_owned()),
                        ("allowed".to_owned(), "Open".to_owned()),
                    ]),
                },
            ],
            outcome: OutcomePlan {
                name: "closed".to_owned(),
                events: vec![ProducedEventPlan {
                    event: "fixture.Closed".to_owned(),
                    fields: vec![
                        retry_field(
                            "id",
                            ValueSource::Input {
                                field: "id".to_owned(),
                            },
                        ),
                        retry_field("revision_id", ValueSource::GeneratedUuidV7),
                    ],
                }],
            },
            projections: Vec::new(),
        }
    }

    fn retry_plan(service: &str, generated_stream: bool) -> service_engine::ServicePlan {
        ServicePlan {
            format: REALIZATION_PLAN_FORMAT.to_owned(),
            service: service.to_owned(),
            delivery: PlanDelivery::ComposedConnector,
            realm: PlanRealmPolicy::Optional,
            ess_source_digest: "a".repeat(64),
            obligation_catalog_digest: "b".repeat(64),
            content: BTreeMap::from([(
                "body".to_owned(),
                ContentPolicyPlan {
                    media_types: BTreeSet::from(["text/plain".to_owned()]),
                    max_bytes: 4096,
                },
            )]),
            intents: BTreeMap::from([
                ("create".to_owned(), retry_create(generated_stream)),
                ("close".to_owned(), retry_close()),
            ]),
            queries: BTreeMap::new(),
            reducers: BTreeMap::from([
                (
                    "fixture.Created".to_owned(),
                    ReducerPlan {
                        entity: "fixture.Document".to_owned(),
                        identity_field: "id".to_owned(),
                        effect: ReducerEffect::Create {
                            initial_state: "Open".to_owned(),
                        },
                        fields: vec![
                            "revision_id".to_owned(),
                            "owner".to_owned(),
                            "scopes".to_owned(),
                            "content_ref".to_owned(),
                        ],
                        inherit: None,
                    },
                ),
                (
                    "fixture.Closed".to_owned(),
                    ReducerPlan {
                        entity: "fixture.Document".to_owned(),
                        identity_field: "id".to_owned(),
                        effect: ReducerEffect::Move {
                            from: BTreeSet::from(["Open".to_owned()]),
                            to: "Closed".to_owned(),
                        },
                        fields: Vec::new(),
                        inherit: None,
                    },
                ),
            ]),
            views: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn original_intent_digest_does_not_redefine_the_historical_event_batch_hash() {
        let store: Arc<dyn DurableEventStore> = Arc::new(
            SqliteEventStore::in_memory("sdk_hash_vector")
                .await
                .unwrap(),
        );
        let stream = ServiceStream {
            service: "fixture".into(),
            tenant: "tenant-a".into(),
            realm: None,
            category: "document".into(),
            key: "doc-a".into(),
        };
        let claim = IntentClaimRequest {
            tenant: "tenant-a".into(),
            service: "fixture".into(),
            realm: None,
            category: "document".into(),
            stream_key: Some("doc-a".into()),
            key: "new-command".into(),
            digest: "a".repeat(64),
        };
        let events = vec![DomainEvent {
            name: "fixture.Created".into(),
            fields: BTreeMap::from([("id".into(), Value::String("doc-a".into()))]),
        }];
        let direct_hash = "03c00c1814d0ba26114ffecb8e792aca1fa95e4adddc5cd84e116d06bed80710";
        assert_eq!(eventlog_core::request_hash(&events).unwrap(), direct_hash);
        // The historical adapter first converts to Value; its object keys are sorted.
        let expected_hash = "c8820f39516d39986f30a25116e21e63aef877a8c061d2ddf2f00c85f5f117ca";
        assert_eq!(
            eventlog_core::request_hash(&serde_json::to_value(&events).unwrap()).unwrap(),
            expected_hash
        );
        let mut adapter =
            EventlogEvents::new(store.clone(), ProjectionLayout::for_service("fixture"));
        adapter
            .append(AppendRequest {
                stream: stream.clone(),
                expected: AppendExpectation::NoStream,
                idempotency_key: claim.key.clone(),
                claim: claim.clone(),
                events,
                metadata: service_engine::AppendMetadata {
                    subject: "person-a".into(),
                    actor: "person-a".into(),
                    request_id: "request-a".into(),
                    trace_id: "trace-a".into(),
                    occurred_at: "1970-01-01T00:00:00Z".into(),
                },
            })
            .await
            .unwrap();
        let durable = durable_stream(&stream).unwrap();
        assert!(
            store
                .recorded_command(&durable, "new-command", expected_hash)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            store
                .recorded_command(&durable, "new-command", &claim.digest)
                .await
                .is_err(),
            "the distinct original-input digest must not replace the old event-batch hash"
        );
        assert!(
            store
                .recorded_claim(durable.tenant(), &intent_claim(&claim).unwrap())
                .await
                .unwrap()
                .is_some()
        );
    }

    fn retry_context(realm: Option<&str>) -> VerifiedAuthContext {
        retry_context_in("tenant-a", realm)
    }

    fn retry_context_in(tenant: &str, realm: Option<&str>) -> VerifiedAuthContext {
        VerifiedAuthContext::from_verified(VerifiedIdentity::after_verification(
            ServiceTenantId::new(tenant).unwrap(),
            AuthorityId::new("person-a").unwrap(),
            UserId::new("person-a").unwrap(),
            None,
            realm.map(|realm| service_runtime::RealmId::new(realm).unwrap()),
        ))
    }

    fn retry_facts() -> AuthorityFacts {
        AuthorityFacts {
            teams: BTreeSet::from(["engineering".to_owned()]),
            ..AuthorityFacts::default()
        }
    }

    fn create_body(generated_stream: bool) -> Vec<u8> {
        let mut body = serde_json::json!({
            "key": "same-create-key",
            "scopes": {"team": "engineering"},
            "content": {"media_type": "text/plain", "text": "retained content"}
        });
        if !generated_stream {
            body["id"] = Value::String("document-a".to_owned());
        }
        serde_json::to_vec(&body).unwrap()
    }

    struct PostCommitReadRefusal {
        inner: EventlogEvents,
        fail_next_load: bool,
        refused: usize,
        appends: usize,
    }
    impl EventStore for PostCommitReadRefusal {
        fn recorded_intent<'a>(
            &'a mut self,
            claim: &'a IntentClaimRequest,
        ) -> BoxFuture<'a, Result<Option<RecordedIntent>, ResourceError>> {
            self.inner.recorded_intent(claim)
        }
        fn authorization_state<'a>(
            &'a mut self,
            stream: &'a ServiceStream,
        ) -> BoxFuture<'a, Result<ProjectionState, ResourceError>> {
            self.inner.authorization_state(stream)
        }
        fn load<'a>(
            &'a mut self,
            stream: &'a ServiceStream,
        ) -> BoxFuture<'a, Result<LoadedStream, ResourceError>> {
            if std::mem::take(&mut self.fail_next_load) {
                self.refused += 1;
                Box::pin(std::future::ready(Err(ResourceError)))
            } else {
                self.inner.load(stream)
            }
        }
        fn append(
            &mut self,
            request: AppendRequest,
        ) -> BoxFuture<'_, Result<AppendReceipt, ResourceError>> {
            Box::pin(async move {
                let result = self.inner.append(request).await?;
                self.appends += 1;
                self.fail_next_load = true;
                Ok(result)
            })
        }
    }

    #[tokio::test]
    async fn first_dispatch_recovers_one_committed_batch_after_post_commit_read_refusal() {
        let store: Arc<dyn DurableEventStore> = Arc::new(
            SqliteEventStore::in_memory("sdk_first_recovered")
                .await
                .unwrap(),
        );
        let service = EventlogService::initialize(
            store.clone(),
            ServiceEngine::new(retry_plan("fixture", true)),
        )
        .await
        .unwrap();
        let mut events = PostCommitReadRefusal {
            inner: EventlogEvents::new(store.clone(), service.projection),
            fail_next_load: false,
            refused: 0,
            appends: 0,
        };
        let mut projections =
            EventlogProjections::new(store.clone(), service.projection.rows, "fixture");
        let mut content = EventlogContent::new(store.clone());
        let mut authority = VerifiedAuthority::new(retry_facts());
        let mut clock = SystemClock;
        let mut ids = UuidV7;
        let mut resources = ServiceResources {
            events: &mut events,
            projections: &mut projections,
            content: &mut content,
            authority: &mut authority,
            clock: &mut clock,
            ids: &mut ids,
        };
        let body = create_body(true);
        let first = service
            .engine
            .intent(
                &mut resources,
                &retry_context(None),
                RequestMetadata::default(),
                "create",
                &body,
            )
            .await
            .unwrap();
        assert!(first.replayed);
        assert_eq!(events.appends, 1);
        assert_eq!(events.refused, 1);
        let tenant = TenantId::new("tenant-a").unwrap();
        let before_retry = store.read_feed(&tenant, 0, 100).await.unwrap().events;
        assert_eq!(before_retry.len(), 1);
        assert_eq!(
            before_retry[0].data,
            serde_json::to_value(&first.events[0].fields).unwrap()
        );
        let digest = first.events[0].fields["content_ref"]
            .as_str()
            .unwrap()
            .strip_prefix("content:")
            .unwrap();
        assert_eq!(
            store.get_blob(&tenant, digest).await.unwrap().unwrap(),
            b"retained content"
        );
        let replay = service
            .intent(
                &retry_context(None),
                retry_facts(),
                RequestMetadata::default(),
                "create",
                &body,
            )
            .await
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.events, first.events);
        assert_eq!(replay.through_version, first.through_version);
        assert_eq!(
            store.read_feed(&tenant, 0, 100).await.unwrap().events,
            before_retry
        );
    }

    #[tokio::test]
    async fn caller_key_create_retry_returns_original_uuid_and_content() {
        let store: Arc<dyn DurableEventStore> = Arc::new(
            SqliteEventStore::in_memory("sdk_retry_fixed")
                .await
                .unwrap(),
        );
        let service = EventlogService::initialize(
            Arc::clone(&store),
            ServiceEngine::new(retry_plan("fixture", false)),
        )
        .await
        .unwrap();
        let context = retry_context(None);
        let body = create_body(false);
        let first = service
            .intent(
                &context,
                retry_facts(),
                RequestMetadata::default(),
                "create",
                &body,
            )
            .await
            .unwrap();
        let replay = service
            .intent(
                &context,
                retry_facts(),
                RequestMetadata {
                    request_id: Some("new-transport-request"),
                },
                "create",
                &body,
            )
            .await
            .expect("lost Create response must be recovered before deciding again");
        assert!(replay.replayed);
        assert_eq!(replay.events, first.events);
        assert_eq!(replay.through_version, first.through_version);
        let reference = first.events[0].fields["content_ref"].as_str().unwrap();
        assert_eq!(
            store
                .get_blob(
                    &TenantId::new("tenant-a").unwrap(),
                    reference.strip_prefix("content:").unwrap()
                )
                .await
                .unwrap()
                .unwrap(),
            b"retained content"
        );
        let stream = durable_stream(&ServiceStream {
            service: "fixture".to_owned(),
            tenant: "tenant-a".to_owned(),
            realm: None,
            category: "document".to_owned(),
            key: "document-a".to_owned(),
        })
        .unwrap();
        assert_eq!(store.stream_version(&stream).await.unwrap(), Some(1));
    }

    #[tokio::test]
    async fn generated_stream_create_retry_does_not_create_another_aggregate() {
        let store: Arc<dyn DurableEventStore> = Arc::new(
            SqliteEventStore::in_memory("sdk_retry_generated")
                .await
                .unwrap(),
        );
        let service = EventlogService::initialize(
            Arc::clone(&store),
            ServiceEngine::new(retry_plan("fixture", true)),
        )
        .await
        .unwrap();
        let context = retry_context(None);
        let body = create_body(true);
        let first = service
            .intent(
                &context,
                retry_facts(),
                RequestMetadata::default(),
                "create",
                &body,
            )
            .await
            .unwrap();
        let replay = service
            .intent(
                &context,
                retry_facts(),
                RequestMetadata::default(),
                "create",
                &body,
            )
            .await
            .unwrap();
        assert!(
            replay.replayed,
            "generated Create retry must resolve the original claim"
        );
        assert_eq!(replay.events, first.events);
        assert_eq!(
            store
                .read_feed(&TenantId::new("tenant-a").unwrap(), 0, 100)
                .await
                .unwrap()
                .events
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn completed_transition_retry_rechecks_authority_and_returns_original_decision() {
        let store: Arc<dyn DurableEventStore> = Arc::new(
            SqliteEventStore::in_memory("sdk_retry_transition")
                .await
                .unwrap(),
        );
        let service = EventlogService::initialize(
            Arc::clone(&store),
            ServiceEngine::new(retry_plan("fixture", false)),
        )
        .await
        .unwrap();
        let context = retry_context(None);
        service
            .intent(
                &context,
                retry_facts(),
                RequestMetadata::default(),
                "create",
                &create_body(false),
            )
            .await
            .unwrap();
        let body = br#"{"id":"document-a","key":"close-key","version":1}"#;
        let first = service
            .intent(
                &context,
                retry_facts(),
                RequestMetadata::default(),
                "close",
                body,
            )
            .await
            .unwrap();
        let revoked = service
            .intent(
                &context,
                AuthorityFacts::default(),
                RequestMetadata::default(),
                "close",
                body,
            )
            .await;
        assert!(
            matches!(revoked, Err(service_engine::ExecutionError::ObligationRefused(ref reason)) if reason == "forbidden")
        );
        let replay = service
            .intent(
                &context,
                retry_facts(),
                RequestMetadata::default(),
                "close",
                body,
            )
            .await
            .expect(
                "completed transition must replay without rerunning its old state precondition",
            );
        assert!(replay.replayed);
        assert_eq!(replay.events, first.events);
        assert_eq!(replay.through_version, 2);
    }

    #[tokio::test]
    async fn same_claim_key_changed_original_input_is_refused_for_generated_create() {
        let store: Arc<dyn DurableEventStore> = Arc::new(
            SqliteEventStore::in_memory("sdk_retry_changed")
                .await
                .unwrap(),
        );
        let service = EventlogService::initialize(
            Arc::clone(&store),
            ServiceEngine::new(retry_plan("fixture", true)),
        )
        .await
        .unwrap();
        let context = retry_context(None);
        service
            .intent(
                &context,
                retry_facts(),
                RequestMetadata::default(),
                "create",
                &create_body(true),
            )
            .await
            .unwrap();
        let mut changed: Value = serde_json::from_slice(&create_body(true)).unwrap();
        changed["content"]["text"] = Value::String("different original intent".to_owned());
        assert!(
            service
                .intent(
                    &context,
                    retry_facts(),
                    RequestMetadata::default(),
                    "create",
                    &serde_json::to_vec(&changed).unwrap()
                )
                .await
                .is_err(),
            "same claim key with changed caller content must refuse before minting another stream"
        );
        assert_eq!(
            store
                .read_feed(&TenantId::new("tenant-a").unwrap(), 0, 100)
                .await
                .unwrap()
                .events
                .len(),
            1
        );
    }

    fn optional_projection_correction_plan(legacy: bool) -> ServicePlan {
        let mut plan = retry_plan("fixture", false);
        plan.content.clear();
        let create = plan.intents.get_mut("create").unwrap();
        create.inputs.retain(|input| input.name != "content");
        create.outcome.events[0]
            .fields
            .retain(|field| field.name != "content_ref");
        let reducer = plan.reducers.get_mut("fixture.Created").unwrap();
        reducer.fields.retain(|field| field != "content_ref");
        for (name, type_ref, optional) in [
            ("title", "String", false),
            ("empty", "Optional<String>", true),
            ("present", "Optional<String>", true),
            ("metadata", "Optional<fixture.Metadata>", true),
            ("items", "List<Optional<String>>", false),
        ] {
            create.inputs.push(InputPlan {
                name: name.to_owned(),
                type_ref: type_ref.to_owned(),
                optional,
                source: InputSource::Command,
            });
            create.outcome.events[0].fields.push(retry_field(
                name,
                ValueSource::Input {
                    field: name.to_owned(),
                },
            ));
            reducer.fields.push(name.to_owned());
        }
        let fields = BTreeMap::from([
            ("id", "String"),
            ("revision_id", "String"),
            ("owner", "String"),
            ("scopes", "fixture.Scopes"),
            ("state", "String"),
            ("title", "String"),
            ("empty", "Optional<String>"),
            ("present", "Optional<String>"),
            ("metadata", "Optional<fixture.Metadata>"),
            ("items", "List<Optional<String>>"),
        ]);
        plan.views.insert(
            "fixture.Documents".to_owned(),
            service_engine::ViewPlan {
                source: "fixture.Document".to_owned(),
                fields: fields.keys().map(|field| (*field).to_owned()).collect(),
                field_types: fields
                    .into_iter()
                    .map(|(field, kind)| (field.to_owned(), kind.to_owned()))
                    .collect(),
                optional_fields: ["empty", "present", "metadata"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                obligations: Vec::new(),
            },
        );
        plan.queries.insert(
            "list_documents".to_owned(),
            service_engine::QueryPlan {
                view: "fixture.Documents".to_owned(),
                scope: "documents.read".to_owned(),
                inputs: Vec::new(),
                obligations: vec![ObligationUse {
                    provider: "sdk.projection.auth-partitioned-visibility/v1".to_owned(),
                    bindings: BTreeMap::from([
                        ("owner".to_owned(), "owner".to_owned()),
                        ("scopes".to_owned(), "scopes".to_owned()),
                    ]),
                }],
            },
        );
        let mut encoded = serde_json::to_value(plan).unwrap();
        if legacy {
            encoded["format"] = Value::String("service-realization-plan/2".to_owned());
            let view = encoded["views"]["fixture.Documents"]
                .as_object_mut()
                .unwrap();
            view.remove("field_types");
            view.remove("optional_fields");
        }
        ServicePlan::from_json(&serde_json::to_string(&encoded).unwrap()).unwrap()
    }

    fn optional_projection_correction_body() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "id": "document-a", "key": "optional-create", "scopes": {"team": "engineering"},
            "title": "required title", "empty": null, "present": "retained",
            "metadata": {"nested": null, "value": "kept"}, "items": [null, "second"]
        }))
        .unwrap()
    }

    fn optional_projection_correction_view(first: &IntentResult, legacy: bool) -> Value {
        // Only the actual opaque revision is supplied by the outcome; the complete public
        // shape and approved format-specific difference are fixed expectations.
        let revision = &first.events[0].fields["revision_id"];
        if legacy {
            serde_json::json!([{
                "id": "document-a", "revision_id": revision, "owner": "person-a",
                "scopes": {"team": "engineering"}, "state": "Open", "title": "required title",
                "empty": null, "present": "retained", "metadata": {"nested": null, "value": "kept"},
                "items": [null, "second"]
            }])
        } else {
            serde_json::json!([{
                "id": "document-a", "revision_id": revision, "owner": "person-a",
                "scopes": {"team": "engineering"}, "state": "Open", "title": "required title",
                "present": "retained", "metadata": {"nested": null, "value": "kept"},
                "items": [null, "second"]
            }])
        }
    }

    struct OptionalProjectionClaimRecorder {
        inner: EventlogEvents,
        claims: Vec<IntentClaimRequest>,
        appends: usize,
    }

    impl EventStore for OptionalProjectionClaimRecorder {
        fn recorded_intent<'a>(
            &'a mut self,
            claim: &'a IntentClaimRequest,
        ) -> BoxFuture<'a, Result<Option<RecordedIntent>, ResourceError>> {
            self.claims.push(claim.clone());
            self.inner.recorded_intent(claim)
        }
        fn authorization_state<'a>(
            &'a mut self,
            stream: &'a ServiceStream,
        ) -> BoxFuture<'a, Result<ProjectionState, ResourceError>> {
            self.inner.authorization_state(stream)
        }
        fn load<'a>(
            &'a mut self,
            stream: &'a ServiceStream,
        ) -> BoxFuture<'a, Result<LoadedStream, ResourceError>> {
            self.inner.load(stream)
        }
        fn append(
            &mut self,
            request: AppendRequest,
        ) -> BoxFuture<'_, Result<AppendReceipt, ResourceError>> {
            self.appends += 1;
            self.inner.append(request)
        }
    }

    async fn optional_projection_correction_recorded_claim(
        service: &EventlogService,
        auth: &VerifiedAuthContext,
        first: &IntentResult,
    ) -> eventlog_core::Claim {
        // Observe the real engine's claim on a genuine retry; do not reproduce its digest.
        let mut events = OptionalProjectionClaimRecorder {
            inner: EventlogEvents::new(Arc::clone(&service.store), service.projection),
            claims: Vec::new(),
            appends: 0,
        };
        let mut projections = EventlogProjections::new(
            Arc::clone(&service.store),
            service.projection.rows,
            &service.plan().service,
        );
        let mut content = EventlogContent::new(Arc::clone(&service.store));
        let mut authority = VerifiedAuthority::new(retry_facts());
        let mut clock = SystemClock;
        let mut ids = UuidV7;
        let mut resources = ServiceResources {
            events: &mut events,
            projections: &mut projections,
            content: &mut content,
            authority: &mut authority,
            clock: &mut clock,
            ids: &mut ids,
        };
        let replay = service
            .engine
            .intent(
                &mut resources,
                auth,
                RequestMetadata::default(),
                "create",
                &optional_projection_correction_body(),
            )
            .await
            .unwrap();
        let mut expected = first.clone();
        expected.replayed = true;
        assert_eq!(replay, expected);
        assert_eq!(events.appends, 0, "claim observation must not append");
        assert_eq!(events.claims.len(), 1);
        intent_claim(&events.claims[0]).unwrap()
    }

    #[derive(Debug, Eq, PartialEq)]
    struct OptionalProjectionCustody {
        identity: String,
        feed: eventlog_core::FeedPage,
        stream: eventlog_core::StreamSlice,
        version: u64,
        command: eventlog_core::AppendResult,
        claim: eventlog_core::ClaimedCommand,
        fold: Vec<(String, Value)>,
    }

    async fn optional_projection_correction_custody(
        store: &dyn DurableEventStore,
        tenant: &TenantId,
        first: &IntentResult,
        claim: &eventlog_core::Claim,
    ) -> OptionalProjectionCustody {
        let feed = store.read_feed(tenant, 0, 100).await.unwrap();
        assert!(!feed.has_more);
        assert_eq!(feed.events.len(), 1);
        let stream_id = feed.events[0].stream().unwrap();
        let stream = store.read_stream(&stream_id, 0, 100).await.unwrap();
        assert!(stream.end_of_stream);
        assert_eq!(stream.events, feed.events);
        let hash =
            eventlog_core::request_hash(&serde_json::to_value(&first.events).unwrap()).unwrap();
        let command = store
            .recorded_command(&stream_id, "optional-create", &hash)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(command.events, feed.events);
        let recorded_claim = store.recorded_claim(tenant, claim).await.unwrap().unwrap();
        assert_eq!(recorded_claim.stream, stream_id);
        assert_eq!(recorded_claim.first_version, command.first_version);
        assert_eq!(recorded_claim.last_version, command.last_version);
        let (_, specs) = EventlogService::projection_declaration("fixture");
        let state = specs
            .iter()
            .find(|spec| spec.name.ends_with("_state"))
            .unwrap();
        OptionalProjectionCustody {
            identity: store.stream_identity(tenant).await.unwrap(),
            feed,
            stream,
            version: store
                .stream_version(&stream_id)
                .await
                .unwrap()
                .expect("the populated stream must retain its head"),
            command,
            claim: recorded_claim,
            fold: store
                .projection_list(state, tenant, None, 100)
                .await
                .unwrap(),
        }
    }

    fn optional_projection_correction_closed_bytes(database: &std::path::Path) -> Vec<u8> {
        for suffix in ["-wal", "-shm"] {
            assert!(
                !std::path::PathBuf::from(format!("{}{suffix}", database.display())).exists(),
                "all SQLite handles must be closed and WAL custody complete before copying"
            );
        }
        assert!(std::fs::metadata(database).unwrap().len() < 1024 * 1024);
        std::fs::read(database).unwrap()
    }

    #[tokio::test]
    async fn optional_projection_correction_sqlite_intent_keeps_raw_nulls() {
        let store: Arc<dyn DurableEventStore> =
            Arc::new(SqliteEventStore::in_memory("sdk_optional").await.unwrap());
        let service = EventlogService::initialize(
            Arc::clone(&store),
            ServiceEngine::new(optional_projection_correction_plan(false)),
        )
        .await
        .unwrap();
        let context = retry_context(None);
        let body = optional_projection_correction_body();
        let before_body = body.clone();
        let first = service
            .intent(
                &context,
                retry_facts(),
                RequestMetadata::default(),
                "create",
                &body,
            )
            .await
            .unwrap();
        assert_eq!(body, before_body);
        assert!(!first.replayed);
        assert_eq!(first.events.len(), 1);
        let fields = &first.events[0].fields;
        let expected_fields = serde_json::json!({
            "id": "document-a", "revision_id": fields["revision_id"], "owner": "person-a",
            "scopes": {"team": "engineering"}, "title": "required title", "empty": null,
            "present": "retained", "metadata": {"nested": null, "value": "kept"},
            "items": [null, "second"]
        });
        assert_eq!(serde_json::to_value(fields).unwrap(), expected_fields);
        let tenant = TenantId::new("tenant-a").unwrap();
        let claim = optional_projection_correction_recorded_claim(&service, &context, &first).await;
        let custody =
            optional_projection_correction_custody(store.as_ref(), &tenant, &first, &claim).await;
        assert_eq!(custody.feed.events[0].data, expected_fields);
        let fold = store
            .projection_get(
                service.projection.state,
                &tenant,
                &custody.feed.events[0].stream_id,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            fold,
            serde_json::json!({"entities": {"fixture.Document": {
                "document-a": {"state": "Open", "fields": expected_fields}
            }}})
        );
        let query = service
            .query(&context, retry_facts(), "list_documents", b"{}")
            .await
            .expect("explicit-null command must produce a queryable canonical plan/3 row");
        assert_eq!(
            serde_json::to_value(query).unwrap(),
            optional_projection_correction_view(&first, false)
        );
        assert_eq!(
            optional_projection_correction_custody(store.as_ref(), &tenant, &first, &claim).await,
            custody
        );
    }

    #[tokio::test]
    async fn optional_projection_correction_projector_is_explicit_and_inline_rebuild_refuses() {
        let store = SqliteEventStore::in_memory("sdk_optional_api")
            .await
            .unwrap();
        let tenant = TenantId::new("tenant-a").unwrap();
        let projector = EventlogService::projector(ServiceEngine::new(
            optional_projection_correction_plan(false),
        ));
        let declaration = EventlogService::projection_declaration("fixture");
        assert_eq!(projector.name(), declaration.0);
        assert_eq!(projector.projections(), declaration.1);
        assert!(!store.is_inline(projector.name()).await);
        assert!(
            store
                .projection_list(&projector.projections()[0], &tenant, None, 100)
                .await
                .is_err()
        );
        store
            .create_projections(Arc::clone(&projector))
            .await
            .unwrap();
        assert!(!store.is_inline(projector.name()).await);
        for spec in projector.projections() {
            assert_eq!(
                store
                    .projection_list(spec, &tenant, None, 100)
                    .await
                    .unwrap(),
                Vec::new()
            );
        }
        let store: Arc<dyn DurableEventStore> = Arc::new(store);
        let _service = EventlogService::initialize(
            Arc::clone(&store),
            ServiceEngine::new(optional_projection_correction_plan(false)),
        )
        .await
        .unwrap();
        assert!(store.is_inline(projector.name()).await);
        assert!(matches!(store.rebuild_projection(projector, &tenant).await,
            Err(EventLogError::Invalid(message)) if message == "rebuild requires a catch-up projection"
        ));
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn optional_projection_correction_plan2_copy_rebuild_preserves_custody_and_foreign_tenant()
     {
        // Current SDK + parsed old-format plan; this is not historical SDK0118 population.
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/sdk-persistence-tests")
            .join(Uuid::now_v7().to_string());
        std::fs::create_dir_all(&directory).unwrap();
        let original = directory.join("original.sqlite3");
        let candidate = directory.join("candidate.sqlite3");
        let tenant = TenantId::new("tenant-a").unwrap();
        let foreign = TenantId::new("tenant-b").unwrap();
        let context = retry_context(None);
        let foreign_context = retry_context_in("tenant-b", Some("default"));
        let projector = EventlogService::projector(ServiceEngine::new(
            optional_projection_correction_plan(false),
        ));
        let (
            first,
            foreign_first,
            claim,
            foreign_claim,
            before,
            foreign_before,
            foreign_projections,
        ) = {
            let store: Arc<dyn DurableEventStore> = Arc::new(
                SqliteEventStore::open(original.to_str().unwrap(), "sdk_optional_restore")
                    .await
                    .unwrap(),
            );
            let plan = optional_projection_correction_plan(true);
            assert_eq!(plan.format, "service-realization-plan/2");
            assert!(plan.views["fixture.Documents"].optional_fields.is_empty());
            let service = EventlogService::initialize(Arc::clone(&store), ServiceEngine::new(plan))
                .await
                .unwrap();
            let first = service
                .intent(
                    &context,
                    retry_facts(),
                    RequestMetadata::default(),
                    "create",
                    &optional_projection_correction_body(),
                )
                .await
                .unwrap();
            let foreign_first = service
                .intent(
                    &foreign_context,
                    retry_facts(),
                    RequestMetadata::default(),
                    "create",
                    &optional_projection_correction_body(),
                )
                .await
                .unwrap();
            let old_views = service
                .query(&context, retry_facts(), "list_documents", b"{}")
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(&old_views).unwrap(),
                optional_projection_correction_view(&first, true)
            );
            assert_eq!(
                serde_json::to_value(
                    service
                        .query(&foreign_context, retry_facts(), "list_documents", b"{}")
                        .await
                        .unwrap()
                )
                .unwrap(),
                optional_projection_correction_view(&foreign_first, true)
            );
            std::fs::write(
                directory.join("original-views.json"),
                serde_json::to_vec_pretty(&old_views).unwrap(),
            )
            .unwrap();
            let claim =
                optional_projection_correction_recorded_claim(&service, &context, &first).await;
            let foreign_claim = optional_projection_correction_recorded_claim(
                &service,
                &foreign_context,
                &foreign_first,
            )
            .await;
            let before =
                optional_projection_correction_custody(store.as_ref(), &tenant, &first, &claim)
                    .await;
            let foreign_before = optional_projection_correction_custody(
                store.as_ref(),
                &foreign,
                &foreign_first,
                &foreign_claim,
            )
            .await;
            let mut foreign_projections = Vec::new();
            for spec in projector.projections() {
                foreign_projections.push(
                    store
                        .projection_list(spec, &foreign, None, 100)
                        .await
                        .unwrap(),
                );
            }
            (
                first,
                foreign_first,
                claim,
                foreign_claim,
                before,
                foreign_before,
                foreign_projections,
            )
        };
        let original_bytes = optional_projection_correction_closed_bytes(&original);
        let original_hash = hex_digest(&original_bytes);
        std::fs::write(
            directory.join("original.sha256"),
            format!("{original_hash}  original.sqlite3\n"),
        )
        .unwrap();
        std::fs::copy(&original, &candidate).unwrap();
        assert_eq!(
            optional_projection_correction_closed_bytes(&candidate),
            original_bytes
        );
        {
            let store: Arc<dyn DurableEventStore> = Arc::new(
                SqliteEventStore::open(candidate.to_str().unwrap(), "sdk_optional_restore")
                    .await
                    .unwrap(),
            );
            let service = EventlogService::initialize(
                Arc::clone(&store),
                ServiceEngine::new(optional_projection_correction_plan(false)),
            )
            .await
            .unwrap();
            assert!(matches!(
                service
                    .query(&context, retry_facts(), "list_documents", b"{}")
                    .await,
                Err(service_engine::ExecutionError::InvalidProjection)
            ));
            assert!(
                matches!(store.rebuild_projection(Arc::clone(&projector), &tenant).await,
                Err(EventLogError::Invalid(message)) if message == "rebuild requires a catch-up projection")
            );
            assert_eq!(
                optional_projection_correction_custody(store.as_ref(), &tenant, &first, &claim)
                    .await,
                before
            );
        }
        // Every writer above has been dropped. The fresh handle's local flag is only a
        // registration check; the test's ownership/scopes establish the actual writer fence.
        {
            let store = SqliteEventStore::open(candidate.to_str().unwrap(), "sdk_optional_restore")
                .await
                .unwrap();
            assert!(!store.is_inline(projector.name()).await);
            store
                .create_projections(Arc::clone(&projector))
                .await
                .unwrap();
            assert!(!store.is_inline(projector.name()).await);
            assert_eq!(
                store
                    .rebuild_projection(Arc::clone(&projector), &tenant)
                    .await
                    .unwrap(),
                before.feed.events.len() as u64
            );
            assert_eq!(
                optional_projection_correction_custody(&store, &tenant, &first, &claim).await,
                before
            );
            assert_eq!(
                optional_projection_correction_custody(
                    &store,
                    &foreign,
                    &foreign_first,
                    &foreign_claim
                )
                .await,
                foreign_before
            );
            for (spec, expected) in projector.projections().iter().zip(&foreign_projections) {
                assert_eq!(
                    &store
                        .projection_list(spec, &foreign, None, 100)
                        .await
                        .unwrap(),
                    expected
                );
            }
        }
        assert_eq!(
            optional_projection_correction_closed_bytes(&original),
            original_bytes
        );
        assert_eq!(
            hex_digest(&std::fs::read(&original).unwrap()),
            original_hash
        );
        {
            let store: Arc<dyn DurableEventStore> = Arc::new(
                SqliteEventStore::open(candidate.to_str().unwrap(), "sdk_optional_restore")
                    .await
                    .unwrap(),
            );
            let service = EventlogService::initialize(
                Arc::clone(&store),
                ServiceEngine::new(optional_projection_correction_plan(false)),
            )
            .await
            .unwrap();
            assert_eq!(
                optional_projection_correction_custody(store.as_ref(), &tenant, &first, &claim)
                    .await,
                before
            );
            assert_eq!(
                optional_projection_correction_custody(
                    store.as_ref(),
                    &foreign,
                    &foreign_first,
                    &foreign_claim
                )
                .await,
                foreign_before
            );
            assert!(
                matches!(
                    service
                        .query(&foreign_context, retry_facts(), "list_documents", b"{}")
                        .await,
                    Err(service_engine::ExecutionError::InvalidProjection)
                ),
                "the foreign old row is still unmigrated"
            );
            assert_eq!(
                service
                    .query(
                        &retry_context(Some("default")),
                        retry_facts(),
                        "list_documents",
                        b"{}"
                    )
                    .await
                    .unwrap(),
                Vec::new()
            );
            let query = service
                .query(&context, retry_facts(), "list_documents", b"{}")
                .await
                .expect("offline original-projector rebuild must produce canonical plan/3 rows");
            assert_eq!(
                serde_json::to_value(query).unwrap(),
                optional_projection_correction_view(&first, false)
            );
        }
        assert_eq!(
            optional_projection_correction_closed_bytes(&original),
            original_bytes
        );
    }

    #[tokio::test]
    async fn generated_create_claim_survives_file_sqlite_reopen() {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/sdk-persistence-tests")
            .join(Uuid::now_v7().to_string());
        std::fs::create_dir_all(&directory).unwrap();
        let database = directory.join("retry.sqlite3");
        let context = retry_context(Some("default"));
        let body = create_body(true);
        let first = {
            let store: Arc<dyn DurableEventStore> = Arc::new(
                SqliteEventStore::open(database.to_str().unwrap(), "sdk_restart")
                    .await
                    .unwrap(),
            );
            let service =
                EventlogService::initialize(store, ServiceEngine::new(retry_plan("fixture", true)))
                    .await
                    .unwrap();
            service
                .intent(
                    &context,
                    retry_facts(),
                    RequestMetadata::default(),
                    "create",
                    &body,
                )
                .await
                .unwrap()
        };
        let store: Arc<dyn DurableEventStore> = Arc::new(
            SqliteEventStore::open(database.to_str().unwrap(), "sdk_restart")
                .await
                .unwrap(),
        );
        let service = EventlogService::initialize(
            Arc::clone(&store),
            ServiceEngine::new(retry_plan("fixture", true)),
        )
        .await
        .unwrap();
        let replay = service
            .intent(
                &context,
                retry_facts(),
                RequestMetadata::default(),
                "create",
                &body,
            )
            .await
            .unwrap();
        assert!(
            replay.replayed,
            "file-backed retry evidence must survive reopening every SDK resource"
        );
        assert_eq!(replay.events, first.events);
        let reference = replay.events[0].fields["content_ref"].as_str().unwrap();
        assert_eq!(
            store
                .get_blob(
                    &TenantId::new("tenant-a").unwrap(),
                    reference.strip_prefix("content:").unwrap()
                )
                .await
                .unwrap()
                .unwrap(),
            b"retained content"
        );
        assert_eq!(
            store
                .read_feed(&TenantId::new("tenant-a").unwrap(), 0, 100)
                .await
                .unwrap()
                .events
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn concurrent_generated_creates_return_one_original_batch() {
        let store: Arc<dyn DurableEventStore> =
            Arc::new(SqliteEventStore::in_memory("sdk_claim_race").await.unwrap());
        let service = EventlogService::initialize(
            Arc::clone(&store),
            ServiceEngine::new(retry_plan("fixture", true)),
        )
        .await
        .unwrap();
        let context = retry_context(None);
        let body = create_body(true);
        let (left, right) = tokio::join!(
            service.intent(
                &context,
                retry_facts(),
                RequestMetadata::default(),
                "create",
                &body
            ),
            service.intent(
                &context,
                retry_facts(),
                RequestMetadata::default(),
                "create",
                &body
            ),
        );
        let left = left.unwrap();
        let right = right.unwrap();
        assert_ne!(
            left.replayed, right.replayed,
            "one contender commits and the other resolves its claim"
        );
        assert_eq!(left.events, right.events);
        assert_eq!(left.through_version, right.through_version);
        assert_eq!(
            store
                .read_feed(&TenantId::new("tenant-a").unwrap(), 0, 100)
                .await
                .unwrap()
                .events
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn tenant_service_and_exact_optional_realm_keep_retry_claims_separate() {
        let store: Arc<dyn DurableEventStore> = Arc::new(
            SqliteEventStore::in_memory("sdk_claim_partitions")
                .await
                .unwrap(),
        );
        let first = EventlogService::initialize(
            Arc::clone(&store),
            ServiceEngine::new(retry_plan("first", false)),
        )
        .await
        .unwrap();
        let second = EventlogService::initialize(
            Arc::clone(&store),
            ServiceEngine::new(retry_plan("second", false)),
        )
        .await
        .unwrap();
        let body = create_body(false);
        let mut identities = BTreeSet::new();
        for tenant in ["tenant-a", "tenant-b"] {
            for service in [&first, &second] {
                for realm in [None, Some("default")] {
                    let context = retry_context_in(tenant, realm);
                    let committed = service
                        .intent(
                            &context,
                            retry_facts(),
                            RequestMetadata::default(),
                            "create",
                            &body,
                        )
                        .await
                        .unwrap();
                    assert!(!committed.replayed);
                    assert!(
                        identities.insert(
                            committed.events[0].fields["revision_id"]
                                .as_str()
                                .unwrap()
                                .to_owned()
                        )
                    );
                    let replay = service
                        .intent(
                            &context,
                            retry_facts(),
                            RequestMetadata::default(),
                            "create",
                            &body,
                        )
                        .await
                        .unwrap();
                    assert!(replay.replayed);
                    assert_eq!(replay.events, committed.events);
                }
            }
            assert_eq!(
                store
                    .read_feed(&TenantId::new(tenant).unwrap(), 0, 100)
                    .await
                    .unwrap()
                    .events
                    .len(),
                4
            );
        }
        assert_eq!(identities.len(), 8);
    }

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

    #[tokio::test]
    async fn scoped_read_visibility_does_not_confer_aggregate_ownership() {
        let mut authority = VerifiedAuthority::new(AuthorityFacts {
            principals: BTreeSet::from(["person:bob".to_owned()]),
            teams: BTreeSet::from(["engineering".to_owned()]),
            ..AuthorityFacts::default()
        });
        let context = VerifiedAuthContext::from_verified(VerifiedIdentity::after_verification(
            ServiceTenantId::new("tenant-a").unwrap(),
            AuthorityId::new("person:bob").unwrap(),
            UserId::new("person:bob").unwrap(),
            None,
            None,
        ));
        let admitted = serde_json::json!({
            "principal": null,
            "team": "engineering",
            "project": null,
            "extension": null
        });

        assert!(
            authority
                .allows(
                    &context,
                    AuthorityCheck::RequestedScopes {
                        scopes: admitted.clone(),
                    },
                )
                .await
                .unwrap()
        );
        assert!(
            !authority
                .allows(
                    &context,
                    AuthorityCheck::OwnerAndScopes {
                        owner: Value::String("person:alice".to_owned()),
                        scopes: admitted,
                    },
                )
                .await
                .unwrap()
        );
        assert!(
            !authority
                .allows(
                    &context,
                    AuthorityCheck::RequestedScopes {
                        scopes: serde_json::json!({"team": "security"}),
                    },
                )
                .await
                .unwrap()
        );
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
