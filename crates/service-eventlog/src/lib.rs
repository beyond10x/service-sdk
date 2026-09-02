//! SDK resource adapters over the organization Eventlog kit.
//!
//! The adapter is generic over Eventlog's one `EventStore` port, so a deployment chooses its
//! existing SQLite or PostgreSQL implementation without changing generated service code.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use eventlog_core::{
    CommandMeta, EventLogError, EventStore as DurableEventStore, Expected, NewEvent,
    ProjectionSpec, ProjectionStore as DurableProjectionStore, Projector, StreamId, TenantId,
};
use serde_json::Value;
use service_engine::{
    AppendDisposition, AppendExpectation, AppendReceipt, AppendRequest, AuthorityCheck,
    AuthorityEvaluator, BoxFuture, Clock, ContentPayload, ContentStore, DomainEvent, EventStore,
    IdGenerator, IntentResult, LoadedStream, ProjectionRead, ProjectionRow, ProjectionState,
    ProjectionStore, ProjectionWrite, RequestMetadata, ResourceError, ServiceEngine,
    ServiceResources, ServiceStream, StagedContent, StoredEvent,
};
use service_runtime::VerifiedAuthContext;
use sha2::{Digest as _, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

const PAGE: usize = 1_000;
const MAX_QUERY_ROWS: usize = 10_000;

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
            let (realm, _, _) = decode_stream_id(&event.stream_id)?;
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
            for row in self
                .engine
                .projection_rows(event.tenant.as_str(), realm.as_deref(), &next)
            {
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

fn durable_stream(stream: &ServiceStream) -> Result<StreamId, EventLogError> {
    StreamId::new(
        TenantId::new(stream.tenant.clone())?,
        stream_type(&stream.service),
        encode_stream_id(stream.realm.as_deref(), &stream.category, &stream.key),
    )
}

fn stream_type(service: &str) -> String {
    format!("generated-service:{service}")
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
}
