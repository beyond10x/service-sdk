//! SDK-owned execution for generated, data-only service realization plans.
//!
//! Application repositories select versioned obligations and commit a generated plan. This crate
//! owns their executable meaning. Deployment supplies storage, content, authority, clock, and ID
//! ports; it does not reimplement service behavior.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use service_runtime::{RealmPolicy, VerifiedAuthContext};
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// The realization-plan format executed by this engine.
pub const REALIZATION_PLAN_FORMAT: &str = "service-realization-plan/1";

/// Allocation-explicit future returned by injected resource ports.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// One completely resolved generated service plan.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServicePlan {
    /// Format discriminator.
    pub format: String,
    /// Stable service identity.
    pub service: String,
    /// Exact optional-realm admission policy.
    pub realm: PlanRealmPolicy,
    /// Compiler-minted ESS semantic digest.
    pub ess_source_digest: String,
    /// SDK obligation catalog digest.
    pub obligation_catalog_digest: String,
    /// Closed plaintext-content policies enforced before the storage adapter sees bytes.
    pub content: BTreeMap<String, ContentPolicyPlan>,
    /// Authenticated mutations by operation identity.
    pub intents: BTreeMap<String, IntentPlan>,
    /// Authenticated projection reads by operation identity.
    pub queries: BTreeMap<String, QueryPlan>,
    /// Event reducers derived from ESS outcomes.
    pub reducers: BTreeMap<String, ReducerPlan>,
    /// Projection row plans derived from ESS views.
    pub views: BTreeMap<String, ViewPlan>,
}

/// One generated plaintext-content admission policy.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContentPolicyPlan {
    /// Exact media types accepted by the definition.
    pub media_types: BTreeSet<String>,
    /// Maximum UTF-8 payload size in bytes.
    pub max_bytes: u64,
}

impl ServicePlan {
    /// Parses strict generated JSON and verifies its immutable format and digests.
    pub fn from_json(text: &str) -> Result<Self, PlanError> {
        let plan: Self = serde_json::from_str(text).map_err(PlanError::Json)?;
        if plan.format != REALIZATION_PLAN_FORMAT {
            return Err(PlanError::UnsupportedFormat(plan.format));
        }
        if !is_digest(&plan.ess_source_digest) || !is_digest(&plan.obligation_catalog_digest) {
            return Err(PlanError::InvalidDigest);
        }
        if plan.service.trim().is_empty() {
            return Err(PlanError::EmptyService);
        }
        Ok(plan)
    }

    /// Canonical generated JSON with a trailing newline.
    pub fn to_canonical_json(&self) -> String {
        let mut output = serde_json::to_string_pretty(self)
            .unwrap_or_else(|error| panic!("a validated realization plan serializes: {error}"));
        output.push('\n');
        output
    }
}

/// Realm policy copied from the strict runtime definition.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanRealmPolicy {
    /// Authentication must supply a realm.
    Required,
    /// Authentication may omit or supply a realm.
    Optional,
    /// Authentication must omit realm.
    Forbidden,
}

impl PlanRealmPolicy {
    fn runtime(self) -> RealmPolicy {
        match self {
            Self::Required => RealmPolicy::Required,
            Self::Optional => RealmPolicy::Optional,
            Self::Forbidden => RealmPolicy::Forbidden,
        }
    }
}

/// One public operation input.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InputPlan {
    /// Stable field name.
    pub name: String,
    /// Resolved ESS or runtime type spelling.
    pub type_ref: String,
    /// Whether callers may omit it.
    pub optional: bool,
    /// How execution consumes the field.
    pub source: InputSource,
}

/// Why a caller-visible field exists.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputSource {
    /// ESS command field.
    Command,
    /// Exact optimistic-concurrency version.
    ExpectedVersion,
    /// Idempotency key.
    Idempotency,
    /// Plaintext content consumed by a staging policy.
    Content {
        /// Content policy identity.
        policy: String,
        /// Semantic command field receiving the opaque reference.
        command_field: String,
    },
    /// Projection selector.
    Selector {
        /// View field selected by this input.
        view_field: String,
    },
}

/// Stream identity derivation for a mutation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StreamPlan {
    /// A command field supplies the aggregate key.
    CommandField {
        /// Semantic command field carrying the stream key.
        field: String,
    },
    /// The engine mints the aggregate key.
    GeneratedUuidV7,
}

/// Optimistic-concurrency source.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExpectedVersionPlan {
    /// Creation requires an empty stream.
    NoStream,
    /// Public operation field carries the exact version.
    OperationField {
        /// Public envelope field carrying the exact version.
        field: String,
    },
}

/// Idempotency source.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum IdempotencyPlan {
    /// Public operation field carries the key.
    OperationField {
        /// Public envelope field carrying the idempotency key.
        field: String,
    },
    /// Trusted transport metadata carries the key.
    RequestId,
}

/// One exact SDK implementation selected for an operation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObligationUse {
    /// Versioned SDK provider identity.
    pub provider: String,
    /// Provider-validated semantic bindings.
    pub bindings: BTreeMap<String, String>,
}

/// One authenticated mutation realization.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntentPlan {
    /// ESS command identity.
    pub command: String,
    /// Closed caller input inventory.
    pub inputs: Vec<InputPlan>,
    /// Aggregate stream key derivation.
    pub stream: StreamPlan,
    /// Optimistic-concurrency source.
    pub expected_version: ExpectedVersionPlan,
    /// Idempotency source.
    pub idempotency: IdempotencyPlan,
    /// SDK implementations applied before decision.
    pub obligations: Vec<ObligationUse>,
    /// Successful semantic outcome; wrong-state is derived from its transition.
    pub outcome: OutcomePlan,
    /// Projections updated after append.
    pub projections: Vec<String>,
}

/// One authenticated projection read realization.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QueryPlan {
    /// ESS view identity.
    pub view: String,
    /// Closed caller selector inventory.
    pub inputs: Vec<InputPlan>,
    /// SDK implementations applied to returned rows.
    pub obligations: Vec<ObligationUse>,
}

/// One deterministic successful command outcome.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OutcomePlan {
    /// Stable ESS outcome name.
    pub name: String,
    /// Events emitted in semantic order.
    pub events: Vec<ProducedEventPlan>,
}

/// One generated event and the source of every declared field.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProducedEventPlan {
    /// ESS event identity.
    pub event: String,
    /// Declared event fields in declaration order.
    pub fields: Vec<EventFieldPlan>,
}

/// Source of one generated event field.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EventFieldPlan {
    /// Event field name.
    pub name: String,
    /// Runtime source.
    pub source: ValueSource,
}

/// Closed event-value derivation vocabulary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ValueSource {
    /// A semantic command field.
    Input {
        /// Semantic command field.
        field: String,
    },
    /// Aggregate stream key.
    StreamId,
    /// Verified authority value.
    Context {
        /// Verified context coordinate.
        value: ContextSource,
    },
    /// Fresh `UUIDv7` from deployment's ID port.
    GeneratedUuidV7,
    /// Literal admitted by ESS.
    Literal {
        /// ESS-admitted literal rendering.
        value: String,
    },
    /// Value derived by a selected SDK provider.
    Obligation {
        /// Exact SDK provider.
        provider: String,
        /// Exact folded entity selected by the provider binding.
        entity: String,
        /// Provider-bound source field.
        field: String,
    },
}

/// Verified-context value available to generated event binding.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextSource {
    /// Tenant ID.
    TenantId,
    /// Exact optional realm.
    RealmIdOptional,
    /// User ID.
    UserId,
    /// Current authority.
    CurrentAuthority,
    /// Optional executor.
    ExecutorOptional,
}

/// How one event reduces aggregate state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReducerPlan {
    /// Entity affected by this event.
    pub entity: String,
    /// Entity identity field.
    pub identity_field: String,
    /// State/data mutation.
    pub effect: ReducerEffect,
    /// Entity fields copied from event fields with matching names.
    pub fields: Vec<String>,
    /// Optional parent-authority derivation for a nested create.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inherit: Option<InheritancePlan>,
}

/// Event reduction effect.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReducerEffect {
    /// Create in the lifecycle's ESS initial state.
    Create {
        /// ESS lifecycle initial state.
        initial_state: String,
    },
    /// Update fields without moving lifecycle state.
    Update,
    /// Move only from one of the ESS-declared source states.
    Move {
        /// ESS-declared source states.
        from: BTreeSet<String>,
        /// ESS-declared destination state.
        to: String,
    },
}

/// Parent-to-child authority derivation supplied by the SDK.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InheritancePlan {
    /// Parent entity in the same aggregate stream.
    pub parent: String,
    /// Parent owner field.
    pub parent_owner: String,
    /// Parent scopes field.
    pub parent_scopes: String,
    /// Child owner field.
    pub child_owner: String,
    /// Child scopes field.
    pub child_scopes: String,
}

/// Projection row realization derived from one ESS view.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ViewPlan {
    /// Source entity.
    pub source: String,
    /// Public row fields.
    pub fields: Vec<String>,
    /// SDK implementations governing materialization and visibility.
    pub obligations: Vec<ObligationUse>,
}

/// Persisted dynamic domain event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DomainEvent {
    /// ESS event identity.
    pub name: String,
    /// Complete event payload.
    pub fields: BTreeMap<String, Value>,
}

/// One versioned event loaded from a deployment adapter.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoredEvent {
    /// One-based stream version.
    pub version: u64,
    /// Domain event.
    pub event: DomainEvent,
}

/// Loaded aggregate stream.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LoadedStream {
    /// Current stream version, zero when absent.
    pub version: u64,
    /// Complete ordered event history.
    pub events: Vec<StoredEvent>,
}

/// Structured aggregate identity handed to persistence adapters.
///
/// Tenant and realm are authenticated partition facts, never reconstructed from a route or a
/// delimiter-bearing string. In particular, `None` and `Some("default")` remain distinct values.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceStream {
    /// Generated service identity.
    pub service: String,
    /// Hidden authenticated tenant partition.
    pub tenant: String,
    /// Hidden exact optional realm partition.
    pub realm: Option<String>,
    /// Generated aggregate category.
    pub category: String,
    /// Aggregate identity inside the category.
    pub key: String,
}

/// Append precondition sent unchanged to the adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppendExpectation {
    /// Stream must not exist.
    NoStream,
    /// Stream must be at this exact version.
    Exact(u64),
}

/// One idempotent guarded append request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppendRequest {
    /// Fully partitioned storage key.
    pub stream: ServiceStream,
    /// Exact precondition.
    pub expected: AppendExpectation,
    /// Mutation idempotency identity.
    pub idempotency_key: String,
    /// Non-empty event batch.
    pub events: Vec<DomainEvent>,
    /// Receiver-derived audit metadata.
    pub metadata: AppendMetadata,
}

/// Audit facts derived by the SDK before an Eventlog adapter sees an append.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppendMetadata {
    /// Opaque authority identity recorded as the subject.
    pub subject: String,
    /// Opaque executor identity, or the authority when execution was direct.
    pub actor: String,
    /// Receiver request identity, falling back to the mutation idempotency identity.
    pub request_id: String,
    /// Trace join for this standalone invocation.
    pub trace_id: String,
    /// Trusted RFC3339 occurrence instant.
    pub occurred_at: String,
}

/// Whether an append committed or replayed the original receipt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppendDisposition {
    /// Events were newly committed.
    Committed,
    /// The same idempotency key and payload had already committed.
    Replayed,
}

/// Successful guarded append receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppendReceipt {
    /// Commit/replay distinction.
    pub disposition: AppendDisposition,
    /// Stream version through the accepted event batch.
    pub through_version: u64,
}

/// Event-log resource selected by deployment.
pub trait EventStore: Send {
    /// Loads one complete stream.
    fn load<'a>(
        &'a mut self,
        stream: &'a ServiceStream,
    ) -> BoxFuture<'a, Result<LoadedStream, ResourceError>>;
    /// Atomically enforces version and idempotency while appending.
    fn append(
        &mut self,
        request: AppendRequest,
    ) -> BoxFuture<'_, Result<AppendReceipt, ResourceError>>;
}

/// One projection row with hidden authenticated partition metadata.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionRow {
    /// ESS view identity.
    pub view: String,
    /// Hidden tenant partition.
    pub tenant: String,
    /// Hidden exact optional realm partition.
    pub realm: Option<String>,
    /// Hidden aggregate source used for authorized revision and event-feed resolution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_stream: Option<ServiceStream>,
    /// Public row value.
    pub value: BTreeMap<String, Value>,
}

/// One projection row after partition, shape, selector, and authority checks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizedProjectionRow {
    /// Public row value.
    pub value: BTreeMap<String, Value>,
    /// Hidden aggregate source, when the projection adapter can establish it.
    pub source_stream: Option<ServiceStream>,
}

/// Projection row paired with its source entity identity for deterministic adapter keys.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeyedProjectionRow {
    /// Source entity identity inside the aggregate.
    pub entity_key: String,
    /// Complete authenticated projection row.
    pub row: ProjectionRow,
}

/// Projection write after a guarded append.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionWrite {
    /// Aggregate stream.
    pub stream: ServiceStream,
    /// Version rows represent.
    pub through_version: u64,
    /// Complete rows for this stream, replacing older rows idempotently.
    pub rows: Vec<ProjectionRow>,
}

/// Partitioned projection query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionRead {
    /// ESS view identity.
    pub view: String,
    /// Hidden tenant partition.
    pub tenant: String,
    /// Hidden exact optional realm partition.
    pub realm: Option<String>,
    /// Domain selectors only.
    pub selectors: BTreeMap<String, Value>,
}

/// Projection resource selected by deployment.
pub trait ProjectionStore: Send {
    /// Idempotently replaces one stream's rows and makes them visible before returning.
    fn project(&mut self, write: ProjectionWrite) -> BoxFuture<'_, Result<(), ResourceError>>;
    /// Reads only the exact hidden authentication partition and explicit domain selectors.
    fn query(
        &mut self,
        read: ProjectionRead,
    ) -> BoxFuture<'_, Result<Vec<ProjectionRow>, ResourceError>>;
}

/// Staged content token returned by the injected content adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StagedContent {
    /// Opaque immutable reference safe for events.
    pub reference: String,
    /// Adapter lifecycle token consumed by accept/abandon.
    pub token: String,
}

/// Validated plaintext passed to a deployment-selected content store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContentPayload<'a> {
    /// Definition-admitted media type.
    pub media_type: &'a str,
    /// Plaintext bytes, already checked against the policy limit.
    pub bytes: &'a [u8],
}

/// Content resource selected by deployment.
pub trait ContentStore: Send {
    /// Stages caller plaintext idempotently and returns an opaque reference.
    fn stage<'a>(
        &'a mut self,
        context: &'a VerifiedAuthContext,
        policy: &'a str,
        idempotency_key: &'a str,
        payload: ContentPayload<'a>,
    ) -> BoxFuture<'a, Result<StagedContent, ResourceError>>;
    /// Accepts a staged object after append and projection succeed.
    fn accept<'a>(
        &'a mut self,
        context: &'a VerifiedAuthContext,
        token: String,
    ) -> BoxFuture<'a, Result<(), ResourceError>>;
    /// Abandons a staged object when no referencing event becomes visible.
    fn abandon<'a>(
        &'a mut self,
        context: &'a VerifiedAuthContext,
        token: String,
    ) -> BoxFuture<'a, Result<(), ResourceError>>;
}

/// Closed authority question constructed by an SDK obligation implementation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthorityCheck {
    /// Current authority must see the owner and every populated scope axis.
    OwnerAndScopes {
        /// Folded owner value.
        owner: Value,
        /// Folded conjunctive scopes.
        scopes: Value,
    },
    /// Requested scope bindings must be granted by verified authority.
    RequestedScopes {
        /// Caller-requested scopes to compare with verified grants.
        scopes: Value,
    },
    /// New owner must be valid in the same authenticated partition.
    OwnerTransfer {
        /// Proposed owner in the current authenticated partition.
        new_owner: Value,
    },
    /// Verified identity must carry a deployment-known capability.
    Capability {
        /// Deployment-known capability name.
        capability: String,
    },
}

/// Authority-fact resource selected by deployment.
pub trait AuthorityEvaluator: Send {
    /// Evaluates one SDK-constructed, non-extensible question.
    fn allows<'a>(
        &'a mut self,
        context: &'a VerifiedAuthContext,
        check: AuthorityCheck,
    ) -> BoxFuture<'a, Result<bool, ResourceError>>;
}

/// Trusted clock selected by deployment.
pub trait Clock: Send {
    /// Current RFC3339 instant.
    fn now(&mut self) -> Result<String, ResourceError>;
}

/// `UUIDv7` source selected by deployment.
pub trait IdGenerator: Send {
    /// Mints a canonical `UUIDv7`.
    fn uuid_v7(&mut self) -> Result<String, ResourceError>;
}

/// Complete deployment-selected resource set. No resource chooses service policy.
pub struct ServiceResources<'a> {
    /// Eventlog adapter.
    pub events: &'a mut dyn EventStore,
    /// Projection adapter.
    pub projections: &'a mut dyn ProjectionStore,
    /// External content adapter.
    pub content: &'a mut dyn ContentStore,
    /// Verified authority facts adapter.
    pub authority: &'a mut dyn AuthorityEvaluator,
    /// Trusted time.
    pub clock: &'a mut dyn Clock,
    /// `UUIDv7` generator.
    pub ids: &'a mut dyn IdGenerator,
}

/// Trusted transport metadata, kept outside operation bodies.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RequestMetadata<'a> {
    /// Receiver-minted request identity.
    pub request_id: Option<&'a str>,
}

/// Successful mutation result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntentResult {
    /// ESS outcome name.
    pub outcome: String,
    /// Accepted event batch.
    pub events: Vec<DomainEvent>,
    /// Resulting stream version.
    pub through_version: u64,
    /// Whether the append committed or replayed.
    pub replayed: bool,
}

/// SDK-owned generated service executor.
#[derive(Clone)]
pub struct ServiceEngine {
    plan: ServicePlan,
}

impl ServiceEngine {
    /// Constructs an executor from generated plan bytes.
    pub fn new(plan: ServicePlan) -> Self {
        Self { plan }
    }

    /// Returns the exact generated plan.
    pub const fn plan(&self) -> &ServicePlan {
        &self.plan
    }

    /// Apply one already-recorded domain event to SDK projection fold state.
    ///
    /// Eventlog adapters use this exact reducer rather than reimplementing generated semantics.
    pub fn apply_projection_event(
        &self,
        state: &mut ProjectionState,
        event: &DomainEvent,
    ) -> Result<(), ExecutionError> {
        reduce(&self.plan, state, event)
    }

    /// Materialize every generated view row for one aggregate fold and exact auth partition.
    #[must_use]
    pub fn projection_rows(
        &self,
        tenant: &str,
        realm: Option<&str>,
        state: &ProjectionState,
    ) -> Vec<KeyedProjectionRow> {
        projection_rows_for_partition(&self.plan, tenant, realm, state)
    }

    /// Executes an authenticated intent. Context policy is enforced before body decoding.
    #[allow(clippy::too_many_lines)]
    pub async fn intent(
        &self,
        resources: &mut ServiceResources<'_>,
        context: &VerifiedAuthContext,
        metadata: RequestMetadata<'_>,
        operation: &str,
        body: &[u8],
    ) -> Result<IntentResult, ExecutionError> {
        self.plan.realm.runtime().enforce(context)?;
        let plan = self
            .plan
            .intents
            .get(operation)
            .ok_or_else(|| ExecutionError::UnknownOperation(operation.to_owned()))?;
        let decoded = decode_inputs(&plan.inputs, body)?;
        let mut command = command_inputs(&plan.inputs, &decoded);
        let idempotency_key = idempotency(&plan.idempotency, &decoded, metadata)?;
        let expected = expected_version(&plan.expected_version, &decoded)?;
        let stream_key = match &plan.stream {
            StreamPlan::CommandField { field } => scalar_string(
                command
                    .get(field)
                    .ok_or_else(|| ExecutionError::MissingInput(field.clone()))?,
                field,
            )?,
            StreamPlan::GeneratedUuidV7 => resources.ids.uuid_v7()?,
        };
        let stream = ServiceStream {
            service: self.plan.service.clone(),
            tenant: context.tenant().as_str().to_owned(),
            realm: context.realm().map(|realm| realm.as_str().to_owned()),
            category: plan
                .obligations
                .iter()
                .find(|item| item.provider == "sdk.aggregate.event-sourced/v1")
                .and_then(|item| item.bindings.get("category"))
                .map_or_else(|| "aggregate".to_owned(), Clone::clone),
            key: stream_key.clone(),
        };
        let history = resources.events.load(&stream).await?;
        let state = fold(&self.plan, &history)?;
        run_intent_obligations(resources, context, plan, &state, &command).await?;

        let mut staged = Vec::new();
        for input in &plan.inputs {
            if let InputSource::Content {
                policy,
                command_field,
            } = &input.source
            {
                let value = decoded
                    .get(&input.name)
                    .ok_or_else(|| ExecutionError::MissingInput(input.name.clone()))?;
                let payload = content_payload(&self.plan, policy, &input.name, value)?;
                match resources
                    .content
                    .stage(context, policy, &idempotency_key, payload)
                    .await
                {
                    Ok(staged_item) => {
                        command.insert(
                            command_field.clone(),
                            Value::String(staged_item.reference.clone()),
                        );
                        staged.push(staged_item);
                    }
                    Err(error) => {
                        abandon_all(resources.content, context, staged).await;
                        return Err(error.into());
                    }
                }
            }
        }

        let events = match produce_events(
            &self.plan,
            resources.ids,
            context,
            &stream_key,
            &command,
            plan,
            &state,
        ) {
            Ok(events) => events,
            Err(error) => {
                abandon_all(resources.content, context, staged).await;
                return Err(error);
            }
        };
        let occurred_at = resources.clock.now()?;
        let request_id = metadata.request_id.unwrap_or(&idempotency_key).to_owned();
        let receipt = match resources
            .events
            .append(AppendRequest {
                stream: stream.clone(),
                expected,
                idempotency_key,
                events: events.clone(),
                metadata: AppendMetadata {
                    subject: context.authority().as_str().to_owned(),
                    actor: context.executor().map_or_else(
                        || context.authority().as_str().to_owned(),
                        |executor| executor.as_str().to_owned(),
                    ),
                    trace_id: request_id.clone(),
                    request_id,
                    occurred_at,
                },
            })
            .await
        {
            Ok(receipt) => receipt,
            Err(error) => {
                abandon_all(resources.content, context, staged).await;
                return Err(error.into());
            }
        };

        let committed = resources.events.load(&stream).await?;
        let state = fold(&self.plan, &committed)?;
        let rows = projection_rows_for_partition(
            &self.plan,
            context.tenant().as_str(),
            context.realm().map(service_runtime::RealmId::as_str),
            &state,
        )
        .into_iter()
        .map(|row| row.row)
        .collect();
        if let Err(error) = resources
            .projections
            .project(ProjectionWrite {
                stream,
                through_version: receipt.through_version,
                rows,
            })
            .await
        {
            // The event batch already committed. Leave staged content recoverable by the
            // idempotent retry rather than deleting an object now referenced by Eventlog.
            return Err(error.into());
        }
        for content in staged {
            resources.content.accept(context, content.token).await?;
        }
        Ok(IntentResult {
            outcome: plan.outcome.name.clone(),
            events,
            through_version: receipt.through_version,
            replayed: receipt.disposition == AppendDisposition::Replayed,
        })
    }

    /// Executes an authenticated query. Realm remains hidden partition context, never a selector.
    pub async fn query(
        &self,
        resources: &mut ServiceResources<'_>,
        context: &VerifiedAuthContext,
        operation: &str,
        body: &[u8],
    ) -> Result<Vec<BTreeMap<String, Value>>, ExecutionError> {
        self.query_rows(resources, context, operation, body)
            .await
            .map(|rows| rows.into_iter().map(|row| row.value).collect())
    }

    /// Executes an authenticated query while retaining hidden aggregate source metadata.
    ///
    /// Callers must never serialize `source_stream` as application data. It exists so trusted
    /// transports can resolve the exact revision of an already-authorized aggregate.
    pub async fn query_rows(
        &self,
        resources: &mut ServiceResources<'_>,
        context: &VerifiedAuthContext,
        operation: &str,
        body: &[u8],
    ) -> Result<Vec<AuthorizedProjectionRow>, ExecutionError> {
        self.plan.realm.runtime().enforce(context)?;
        let plan = self
            .plan
            .queries
            .get(operation)
            .ok_or_else(|| ExecutionError::UnknownOperation(operation.to_owned()))?;
        let decoded = decode_inputs(&plan.inputs, body)?;
        let selectors: BTreeMap<String, Value> = plan
            .inputs
            .iter()
            .filter_map(|input| match &input.source {
                InputSource::Selector { view_field } => decoded
                    .get(&input.name)
                    .cloned()
                    .map(|value| (view_field.clone(), value)),
                _ => None,
            })
            .collect();
        let rows = resources
            .projections
            .query(ProjectionRead {
                view: plan.view.clone(),
                tenant: context.tenant().as_str().to_owned(),
                realm: context.realm().map(|realm| realm.as_str().to_owned()),
                selectors: selectors.clone(),
            })
            .await?;
        let view = self
            .plan
            .views
            .get(&plan.view)
            .ok_or_else(|| ExecutionError::InvalidPlan(plan.view.clone()))?;
        let mut visible = Vec::new();
        for row in rows {
            validate_projection_row(context, plan, view, &selectors, &row)?;
            if query_visible(resources.authority, context, plan, &row.value).await? {
                let source_stream = row.source_stream.or_else(|| {
                    projection_source_from_value(&self.plan, context, view, &row.value)
                });
                visible.push(AuthorizedProjectionRow {
                    value: row.value,
                    source_stream,
                });
            }
        }
        Ok(visible)
    }
}

/// Serializable fold state used by SDK-owned inline Eventlog projectors.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionState {
    entities: BTreeMap<String, BTreeMap<String, EntityValue>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct EntityValue {
    state: String,
    fields: BTreeMap<String, Value>,
}

fn fold(plan: &ServicePlan, history: &LoadedStream) -> Result<ProjectionState, ExecutionError> {
    if history.events.last().map_or(0, |event| event.version) != history.version
        || history
            .events
            .iter()
            .enumerate()
            .any(|(index, event)| event.version != index as u64 + 1)
    {
        return Err(ExecutionError::InvalidHistory);
    }
    let mut state = ProjectionState::default();
    for stored in &history.events {
        reduce(plan, &mut state, &stored.event)?;
    }
    Ok(state)
}

fn reduce(
    plan: &ServicePlan,
    state: &mut ProjectionState,
    event: &DomainEvent,
) -> Result<(), ExecutionError> {
    let reducer = plan
        .reducers
        .get(&event.name)
        .ok_or_else(|| ExecutionError::UnknownEvent(event.name.clone()))?;
    let identity = scalar_string(
        event
            .fields
            .get(&reducer.identity_field)
            .ok_or_else(|| ExecutionError::InvalidEvent(event.name.clone()))?,
        &reducer.identity_field,
    )?;
    let inherited = if let Some(inherit) = &reducer.inherit {
        let parent = state
            .entities
            .get(&inherit.parent)
            .and_then(|instances| instances.values().next())
            .ok_or_else(|| ExecutionError::ObligationRefused("parent_not_found".into()))?;
        Some((
            parent
                .fields
                .get(&inherit.parent_owner)
                .cloned()
                .ok_or_else(|| ExecutionError::InvalidEvent(event.name.clone()))?,
            parent
                .fields
                .get(&inherit.parent_scopes)
                .cloned()
                .ok_or_else(|| ExecutionError::InvalidEvent(event.name.clone()))?,
        ))
    } else {
        None
    };
    let entity_set = state.entities.entry(reducer.entity.clone()).or_default();
    match &reducer.effect {
        ReducerEffect::Create { initial_state } => {
            if entity_set.contains_key(&identity) {
                return Err(ExecutionError::InvalidEvent(event.name.clone()));
            }
            let mut fields = event_fields(reducer, event);
            if let (Some(inherit), Some((owner, scopes))) = (&reducer.inherit, inherited) {
                fields.insert(inherit.child_owner.clone(), owner);
                fields.insert(inherit.child_scopes.clone(), scopes);
            }
            fields.insert(
                reducer.identity_field.clone(),
                Value::String(identity.clone()),
            );
            entity_set.insert(
                identity,
                EntityValue {
                    state: initial_state.clone(),
                    fields,
                },
            );
        }
        ReducerEffect::Update => {
            let entity = entity_set
                .get_mut(&identity)
                .ok_or_else(|| ExecutionError::InvalidEvent(event.name.clone()))?;
            for (field, value) in event_fields(reducer, event) {
                entity.fields.insert(field, value);
            }
        }
        ReducerEffect::Move { from, to } => {
            let entity = entity_set
                .get_mut(&identity)
                .ok_or_else(|| ExecutionError::InvalidEvent(event.name.clone()))?;
            if !from.contains(&entity.state) {
                return Err(ExecutionError::ObligationRefused("wrong_state".into()));
            }
            entity.state.clone_from(to);
        }
    }
    Ok(())
}

fn event_fields(reducer: &ReducerPlan, event: &DomainEvent) -> BTreeMap<String, Value> {
    reducer
        .fields
        .iter()
        .filter_map(|field| {
            event
                .fields
                .get(field)
                .cloned()
                .map(|value| (field.clone(), value))
        })
        .collect()
}

#[allow(clippy::too_many_lines)]
async fn run_intent_obligations(
    resources: &mut ServiceResources<'_>,
    context: &VerifiedAuthContext,
    plan: &IntentPlan,
    state: &ProjectionState,
    command: &BTreeMap<String, Value>,
) -> Result<(), ExecutionError> {
    for obligation in &plan.obligations {
        match obligation.provider.as_str() {
            "sdk.aggregate.event-sourced/v1"
            | "sdk.content.external-erasable/v1"
            | "sdk.derive.inherit-parent-authority/v1"
            | "sdk.derive.inherit-parent-authority/v2"
            | "sdk.projection.auth-partitioned-visibility/v1"
            | "sdk.projection.hide-terminal-parent/v1" => {}
            "sdk.auth.owner-and-conjunctive-scopes/v1" => {
                let entity = bound_entity(state, obligation, "owner")?;
                let owner = bound_value(entity, obligation, "owner")?;
                let scopes = bound_value(entity, obligation, "scopes")?;
                require_authority(
                    resources.authority,
                    context,
                    AuthorityCheck::OwnerAndScopes { owner, scopes },
                )
                .await?;
            }
            "sdk.auth.requested-scopes/v1" => {
                let field = binding(obligation, "scopes")?;
                let scopes = command
                    .get(field)
                    .cloned()
                    .ok_or_else(|| ExecutionError::MissingInput(field.to_owned()))?;
                require_authority(
                    resources.authority,
                    context,
                    AuthorityCheck::RequestedScopes { scopes },
                )
                .await?;
            }
            "sdk.auth.same-partition-owner-transfer/v1" => {
                let field = obligation
                    .bindings
                    .get("new_owner")
                    .map_or("new_owner", String::as_str);
                let new_owner = command
                    .get(field)
                    .cloned()
                    .ok_or_else(|| ExecutionError::MissingInput(field.to_owned()))?;
                require_authority(
                    resources.authority,
                    context,
                    AuthorityCheck::OwnerTransfer { new_owner },
                )
                .await?;
            }
            "sdk.auth.trusted-scheduler/v1" => {
                require_authority(
                    resources.authority,
                    context,
                    AuthorityCheck::Capability {
                        capability: binding(obligation, "capability")?.to_owned(),
                    },
                )
                .await?;
            }
            "sdk.lifecycle.require-state/v1" => {
                let instances = state
                    .entities
                    .get(binding(obligation, "entity")?)
                    .ok_or_else(|| ExecutionError::ObligationRefused("not_found".into()))?;
                let entity = if let Some(identity_field) = obligation.bindings.get("identity") {
                    let identity = command
                        .get(identity_field)
                        .ok_or_else(|| ExecutionError::MissingInput(identity_field.clone()))?;
                    instances.get(&scalar_string(identity, identity_field)?)
                } else {
                    instances.values().next()
                }
                .ok_or_else(|| ExecutionError::ObligationRefused("not_found".into()))?;
                let allowed = binding(obligation, "allowed")?
                    .split(',')
                    .map(str::trim)
                    .any(|candidate| candidate == entity.state);
                if !allowed {
                    return Err(ExecutionError::ObligationRefused("wrong_state".into()));
                }
            }
            "sdk.aggregate.nested-entity/v1" => {
                let child = binding(obligation, "child")?;
                let identity_field = binding(obligation, "child_identity")?;
                let identity = command
                    .get(identity_field)
                    .ok_or_else(|| ExecutionError::MissingInput(identity_field.to_owned()))?;
                let identity = scalar_string(identity, identity_field)?;
                if !state
                    .entities
                    .get(child)
                    .is_some_and(|instances| instances.contains_key(&identity))
                {
                    return Err(ExecutionError::ObligationRefused("not_found".into()));
                }
            }
            "sdk.lifecycle.expiring-parent-child/v1" => {
                validate_lifetime(resources.clock, obligation, state, command)?;
            }
            "sdk.lifecycle.bounded-future/v1" => {
                validate_future(resources.clock, obligation, command)?;
            }
            "sdk.lifecycle.expiry-due/v1" => {
                validate_expiry_due(resources.clock, obligation, state, command)?;
            }
            other => return Err(ExecutionError::UnknownProvider(other.to_owned())),
        }
    }
    Ok(())
}

fn validate_lifetime(
    clock: &mut dyn Clock,
    obligation: &ObligationUse,
    state: &ProjectionState,
    command: &BTreeMap<String, Value>,
) -> Result<(), ExecutionError> {
    let child_field = binding(obligation, "child_lifetime")?;
    let candidate = command
        .get(child_field)
        .and_then(lifetime_instant)
        .ok_or_else(|| ExecutionError::ObligationRefused("invalid_lifetime".into()))?;
    let candidate = parse_instant(candidate)?;
    if candidate <= parse_instant(&clock.now()?)? {
        return Err(ExecutionError::ObligationRefused("invalid_lifetime".into()));
    }
    let parent_expiry = state
        .entities
        .get(binding(obligation, "parent")?)
        .and_then(|items| items.values().next())
        .and_then(|parent| {
            parent
                .fields
                .get(binding(obligation, "parent_lifetime").ok()?)
        })
        .and_then(lifetime_instant)
        .ok_or_else(|| ExecutionError::ObligationRefused("parent_not_found".into()))?;
    if candidate > parse_instant(parent_expiry)? {
        return Err(ExecutionError::ObligationRefused("invalid_lifetime".into()));
    }
    Ok(())
}

fn validate_future(
    clock: &mut dyn Clock,
    obligation: &ObligationUse,
    command: &BTreeMap<String, Value>,
) -> Result<(), ExecutionError> {
    let field = binding(obligation, "lifetime")?;
    let candidate = command
        .get(field)
        .and_then(lifetime_instant)
        .ok_or_else(|| ExecutionError::InvalidInput(field.to_owned()))?;
    let candidate = OffsetDateTime::parse(candidate, &Rfc3339)
        .map_err(|_| ExecutionError::InvalidInput(field.to_owned()))?;
    if candidate <= parse_instant(&clock.now()?)? {
        return Err(ExecutionError::InvalidInput(field.to_owned()));
    }
    Ok(())
}

fn validate_expiry_due(
    clock: &mut dyn Clock,
    obligation: &ObligationUse,
    state: &ProjectionState,
    command: &BTreeMap<String, Value>,
) -> Result<(), ExecutionError> {
    let instances = state
        .entities
        .get(binding(obligation, "entity")?)
        .ok_or_else(|| ExecutionError::ObligationRefused("not_found".into()))?;
    let entity = if let Some(identity_field) = obligation.bindings.get("identity") {
        let identity = command
            .get(identity_field)
            .ok_or_else(|| ExecutionError::MissingInput(identity_field.clone()))?;
        instances.get(&scalar_string(identity, identity_field)?)
    } else {
        instances.values().next()
    }
    .ok_or_else(|| ExecutionError::ObligationRefused("not_found".into()))?;
    let expiry = entity
        .fields
        .get(binding(obligation, "lifetime")?)
        .and_then(lifetime_instant)
        .ok_or_else(|| ExecutionError::InvalidPlan("entity lifetime".into()))?;
    if parse_instant(expiry)? > parse_instant(&clock.now()?)? {
        return Err(ExecutionError::ObligationRefused("expiry_not_due".into()));
    }
    Ok(())
}

fn lifetime_instant(value: &Value) -> Option<&str> {
    value
        .as_object()
        .and_then(|object| object.get("expires_at"))
        .and_then(Value::as_str)
}

fn parse_instant(value: &str) -> Result<OffsetDateTime, ExecutionError> {
    OffsetDateTime::parse(value, &Rfc3339)
        .map_err(|_| ExecutionError::ObligationRefused("invalid_lifetime".into()))
}

async fn require_authority(
    authority: &mut dyn AuthorityEvaluator,
    context: &VerifiedAuthContext,
    check: AuthorityCheck,
) -> Result<(), ExecutionError> {
    if authority.allows(context, check).await? {
        Ok(())
    } else {
        Err(ExecutionError::ObligationRefused("forbidden".into()))
    }
}

async fn query_visible(
    authority: &mut dyn AuthorityEvaluator,
    context: &VerifiedAuthContext,
    plan: &QueryPlan,
    row: &BTreeMap<String, Value>,
) -> Result<bool, ExecutionError> {
    for obligation in &plan.obligations {
        match obligation.provider.as_str() {
            "sdk.projection.auth-partitioned-visibility/v1"
            | "sdk.auth.owner-and-conjunctive-scopes/v1" => {
                let owner = row
                    .get(binding(obligation, "owner")?)
                    .cloned()
                    .ok_or_else(|| ExecutionError::InvalidPlan("projection owner".into()))?;
                let scopes = row
                    .get(binding(obligation, "scopes")?)
                    .cloned()
                    .ok_or_else(|| ExecutionError::InvalidPlan("projection scopes".into()))?;
                if !authority
                    .allows(context, AuthorityCheck::OwnerAndScopes { owner, scopes })
                    .await?
                {
                    return Ok(false);
                }
            }
            "sdk.projection.hide-terminal-parent/v1" => {
                if row
                    .get("parent_state")
                    .and_then(Value::as_str)
                    .is_some_and(|state| state == "Archived" || state == "Expired")
                {
                    return Ok(false);
                }
            }
            other => return Err(ExecutionError::UnknownProvider(other.to_owned())),
        }
    }
    Ok(true)
}

fn produce_events(
    service: &ServicePlan,
    ids: &mut dyn IdGenerator,
    context: &VerifiedAuthContext,
    stream_id: &str,
    command: &BTreeMap<String, Value>,
    plan: &IntentPlan,
    state: &ProjectionState,
) -> Result<Vec<DomainEvent>, ExecutionError> {
    let mut events = Vec::with_capacity(plan.outcome.events.len());
    for event in &plan.outcome.events {
        let mut fields = BTreeMap::new();
        for field in &event.fields {
            let value = match &field.source {
                ValueSource::Input { field } => command
                    .get(field)
                    .cloned()
                    .ok_or_else(|| ExecutionError::MissingInput(field.clone()))?,
                ValueSource::StreamId => Value::String(stream_id.to_owned()),
                ValueSource::Context { value } => context_value(context, *value),
                ValueSource::GeneratedUuidV7 => Value::String(ids.uuid_v7()?),
                ValueSource::Literal { value } => Value::String(value.clone()),
                ValueSource::Obligation {
                    provider,
                    entity,
                    field,
                } => obligation_value(provider, entity, field, state)?,
            };
            fields.insert(field.name.clone(), value);
        }
        events.push(DomainEvent {
            name: event.event.clone(),
            fields,
        });
    }
    if events.is_empty() {
        return Err(ExecutionError::InvalidPlan("empty decision".into()));
    }
    let mut preview = state.clone();
    for event in &events {
        reduce(service, &mut preview, event)?;
    }
    Ok(events)
}

fn obligation_value(
    provider: &str,
    entity: &str,
    field: &str,
    state: &ProjectionState,
) -> Result<Value, ExecutionError> {
    if !matches!(
        provider,
        "sdk.derive.inherit-parent-authority/v1" | "sdk.derive.inherit-parent-authority/v2"
    ) {
        return Err(ExecutionError::UnknownProvider(provider.to_owned()));
    }
    state
        .entities
        .get(entity)
        .and_then(|instances| instances.values().next())
        .and_then(|entity| entity.fields.get(field))
        .cloned()
        .ok_or_else(|| ExecutionError::ObligationRefused("parent_not_found".into()))
}

fn validate_projection_row(
    context: &VerifiedAuthContext,
    query: &QueryPlan,
    view: &ViewPlan,
    selectors: &BTreeMap<String, Value>,
    row: &ProjectionRow,
) -> Result<(), ExecutionError> {
    let exact_partition = row.view == query.view
        && row.tenant == context.tenant().as_str()
        && row.realm.as_deref() == context.realm().map(service_runtime::RealmId::as_str);
    let exact_shape = row.value.len() == view.fields.len()
        && view
            .fields
            .iter()
            .all(|field| row.value.contains_key(field));
    let exact_selection = selectors
        .iter()
        .all(|(field, expected)| row.value.get(field) == Some(expected));
    if exact_partition && exact_shape && exact_selection {
        Ok(())
    } else {
        Err(ExecutionError::InvalidProjection)
    }
}

fn projection_source_from_value(
    service: &ServicePlan,
    context: &VerifiedAuthContext,
    view: &ViewPlan,
    value: &BTreeMap<String, Value>,
) -> Option<ServiceStream> {
    let mut candidates = BTreeSet::new();
    for intent in service.intents.values() {
        let category = intent
            .obligations
            .iter()
            .find(|item| item.provider == "sdk.aggregate.event-sourced/v1")
            .and_then(|item| item.bindings.get("category"))
            .map_or("aggregate", String::as_str);
        for produced in &intent.outcome.events {
            let Some(reducer) = service.reducers.get(&produced.event) else {
                continue;
            };
            if reducer.entity != view.source {
                continue;
            }
            let key_field = match &intent.stream {
                StreamPlan::CommandField { field } => field,
                StreamPlan::GeneratedUuidV7 => &reducer.identity_field,
            };
            if let Some(key) = value.get(key_field).and_then(Value::as_str) {
                candidates.insert((category.to_owned(), key.to_owned()));
            }
        }
    }
    let mut candidates = candidates.into_iter();
    let (category, key) = candidates.next()?;
    if candidates.next().is_some() {
        return None;
    }
    Some(ServiceStream {
        service: service.service.clone(),
        tenant: context.tenant().as_str().to_owned(),
        realm: context.realm().map(|realm| realm.as_str().to_owned()),
        category,
        key,
    })
}

fn context_value(context: &VerifiedAuthContext, source: ContextSource) -> Value {
    match source {
        ContextSource::TenantId => Value::String(context.tenant().as_str().to_owned()),
        ContextSource::RealmIdOptional => context.realm().map_or(Value::Null, |realm| {
            Value::String(realm.as_str().to_owned())
        }),
        ContextSource::UserId => Value::String(context.user().as_str().to_owned()),
        ContextSource::CurrentAuthority => Value::String(context.authority().as_str().to_owned()),
        ContextSource::ExecutorOptional => context.executor().map_or(Value::Null, |executor| {
            Value::String(executor.as_str().to_owned())
        }),
    }
}

fn projection_rows_for_partition(
    plan: &ServicePlan,
    tenant: &str,
    realm: Option<&str>,
    state: &ProjectionState,
) -> Vec<KeyedProjectionRow> {
    let mut rows = Vec::new();
    for (view_name, view) in &plan.views {
        let hidden_by_terminal_parent = view.obligations.iter().any(|obligation| {
            obligation.provider == "sdk.projection.hide-terminal-parent/v1"
                && obligation
                    .bindings
                    .get("parent")
                    .and_then(|parent| state.entities.get(parent))
                    .is_some_and(|instances| {
                        instances.values().any(|entity| {
                            obligation
                                .bindings
                                .get("terminal_states")
                                .map_or("Archived,Expired", String::as_str)
                                .split(',')
                                .map(str::trim)
                                .any(|terminal| terminal == entity.state)
                        })
                    })
        });
        if hidden_by_terminal_parent {
            continue;
        }
        if let Some(instances) = state.entities.get(&view.source) {
            for (entity_key, entity) in instances {
                let mut value = BTreeMap::new();
                for field in &view.fields {
                    if field == "state" {
                        value.insert(field.clone(), Value::String(entity.state.clone()));
                    } else if let Some(field_value) = entity.fields.get(field) {
                        value.insert(field.clone(), field_value.clone());
                    }
                }
                if let Some(obligation) = view.obligations.iter().find(|obligation| {
                    obligation.provider == "sdk.derive.inherit-parent-authority/v2"
                }) {
                    let parent = obligation
                        .bindings
                        .get("parent")
                        .and_then(|parent| state.entities.get(parent))
                        .and_then(|instances| instances.values().next());
                    let inherited = parent.and_then(|parent| {
                        let parent_owner = binding_field(obligation, "parent_owner")?;
                        let parent_scopes = binding_field(obligation, "parent_scopes")?;
                        let child_owner = binding_field(obligation, "child_owner")?;
                        let child_scopes = binding_field(obligation, "child_scopes")?;
                        Some((
                            child_owner,
                            parent.fields.get(parent_owner)?.clone(),
                            child_scopes,
                            parent.fields.get(parent_scopes)?.clone(),
                        ))
                    });
                    let Some((child_owner, owner, child_scopes, scopes)) = inherited else {
                        // Live inheritance fails closed: an inconsistent child never becomes a
                        // projection row with stale authority copied at creation time.
                        continue;
                    };
                    value.insert(child_owner.to_owned(), owner);
                    value.insert(child_scopes.to_owned(), scopes);
                }
                rows.push(KeyedProjectionRow {
                    entity_key: entity_key.clone(),
                    row: ProjectionRow {
                        view: view_name.clone(),
                        tenant: tenant.to_owned(),
                        realm: realm.map(str::to_owned),
                        source_stream: None,
                        value,
                    },
                });
            }
        }
    }
    rows
}

fn binding_field<'a>(obligation: &'a ObligationUse, name: &str) -> Option<&'a str> {
    obligation
        .bindings
        .get(name)
        .map(String::as_str)
        .map(|path| path.rsplit_once('.').map_or(path, |(_, field)| field))
}

fn decode_inputs(
    plan: &[InputPlan],
    body: &[u8],
) -> Result<BTreeMap<String, Value>, ExecutionError> {
    let value: Value = serde_json::from_slice(body).map_err(ExecutionError::Decode)?;
    let object = value.as_object().ok_or(ExecutionError::ExpectedObject)?;
    let allowed = plan
        .iter()
        .map(|input| input.name.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(unknown) = object.keys().find(|name| !allowed.contains(name.as_str())) {
        return Err(ExecutionError::UnknownInput(unknown.clone()));
    }
    for input in plan {
        if !input.optional && !object.contains_key(&input.name) {
            return Err(ExecutionError::MissingInput(input.name.clone()));
        }
    }
    Ok(object
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect())
}

fn command_inputs(
    plan: &[InputPlan],
    decoded: &BTreeMap<String, Value>,
) -> BTreeMap<String, Value> {
    plan.iter()
        .filter(|input| matches!(input.source, InputSource::Command))
        .filter_map(|input| {
            decoded
                .get(&input.name)
                .cloned()
                .map(|value| (input.name.clone(), value))
        })
        .collect()
}

fn expected_version(
    plan: &ExpectedVersionPlan,
    decoded: &BTreeMap<String, Value>,
) -> Result<AppendExpectation, ExecutionError> {
    match plan {
        ExpectedVersionPlan::NoStream => Ok(AppendExpectation::NoStream),
        ExpectedVersionPlan::OperationField { field } => decoded
            .get(field)
            .and_then(Value::as_u64)
            .map(AppendExpectation::Exact)
            .ok_or_else(|| ExecutionError::InvalidInput(field.clone())),
    }
}

fn idempotency(
    plan: &IdempotencyPlan,
    decoded: &BTreeMap<String, Value>,
    metadata: RequestMetadata<'_>,
) -> Result<String, ExecutionError> {
    let value = match plan {
        IdempotencyPlan::OperationField { field } => decoded
            .get(field)
            .and_then(Value::as_str)
            .ok_or_else(|| ExecutionError::InvalidInput(field.clone()))?,
        IdempotencyPlan::RequestId => metadata
            .request_id
            .ok_or_else(|| ExecutionError::InvalidInput("request_id".into()))?,
    };
    if value.trim().is_empty() {
        Err(ExecutionError::InvalidInput("idempotency_key".into()))
    } else {
        Ok(value.to_owned())
    }
}

fn bound_entity<'a>(
    state: &'a ProjectionState,
    obligation: &ObligationUse,
    binding_name: &str,
) -> Result<&'a EntityValue, ExecutionError> {
    let path = binding(obligation, binding_name)?;
    let (entity, _) = path
        .rsplit_once('.')
        .ok_or_else(|| ExecutionError::InvalidPlan(path.to_owned()))?;
    state
        .entities
        .get(entity)
        .and_then(|instances| instances.values().next())
        .ok_or_else(|| ExecutionError::ObligationRefused("not_found".into()))
}

fn bound_value(
    entity: &EntityValue,
    obligation: &ObligationUse,
    binding_name: &str,
) -> Result<Value, ExecutionError> {
    let path = binding(obligation, binding_name)?;
    let field = path.rsplit_once('.').map_or(path, |(_, field)| field);
    entity
        .fields
        .get(field)
        .cloned()
        .ok_or_else(|| ExecutionError::InvalidPlan(path.to_owned()))
}

fn binding<'a>(obligation: &'a ObligationUse, name: &str) -> Result<&'a str, ExecutionError> {
    obligation
        .bindings
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| {
            ExecutionError::InvalidPlan(format!("{}.bindings.{name}", obligation.provider))
        })
}

fn scalar_string(value: &Value, field: &str) -> Result<String, ExecutionError> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| ExecutionError::InvalidInput(field.to_owned()))
}

fn content_payload<'a>(
    service: &ServicePlan,
    policy: &str,
    input: &str,
    value: &'a Value,
) -> Result<ContentPayload<'a>, ExecutionError> {
    let policy = service
        .content
        .get(policy)
        .ok_or_else(|| ExecutionError::InvalidPlan(format!("content.{policy}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| ExecutionError::InvalidInput(input.to_owned()))?;
    if object.len() != 2 || !object.contains_key("media_type") || !object.contains_key("text") {
        return Err(ExecutionError::InvalidInput(input.to_owned()));
    }
    let media_type = object["media_type"]
        .as_str()
        .ok_or_else(|| ExecutionError::InvalidInput(input.to_owned()))?;
    let text = object["text"]
        .as_str()
        .ok_or_else(|| ExecutionError::InvalidInput(input.to_owned()))?;
    if !policy.media_types.contains(media_type)
        || u64::try_from(text.len()).map_or(true, |length| length > policy.max_bytes)
    {
        return Err(ExecutionError::InvalidInput(input.to_owned()));
    }
    Ok(ContentPayload {
        media_type,
        bytes: text.as_bytes(),
    })
}

async fn abandon_all(
    store: &mut dyn ContentStore,
    context: &VerifiedAuthContext,
    staged: Vec<StagedContent>,
) {
    for item in staged {
        let _ = store.abandon(context, item.token).await;
    }
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Generated plan could not be admitted.
#[derive(Debug, Error)]
pub enum PlanError {
    /// Plan JSON is malformed or contains an unknown field.
    #[error("invalid realization plan JSON: {0}")]
    Json(serde_json::Error),
    /// Format marker is unsupported.
    #[error("unsupported realization plan format {0:?}")]
    UnsupportedFormat(String),
    /// Service identity is empty.
    #[error("realization plan service is empty")]
    EmptyService,
    /// A semantic or catalog digest is malformed.
    #[error("realization plan digest is malformed")]
    InvalidDigest,
}

/// Value-free resource adapter refusal.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("a generated service resource refused the operation")]
pub struct ResourceError;

/// Generated operation execution failed at a closed boundary.
#[derive(Debug, Error)]
pub enum ExecutionError {
    /// Verified context violates realm policy.
    #[error(transparent)]
    Context(#[from] service_runtime::ContextPolicyViolation),
    /// Operation is not generated into this service.
    #[error("unknown generated operation {0:?}")]
    UnknownOperation(String),
    /// Event history contains an impossible version sequence.
    #[error("event history violates stream invariants")]
    InvalidHistory,
    /// Persisted event is not in the generated reducer set.
    #[error("unknown persisted event {0:?}")]
    UnknownEvent(String),
    /// Persisted event does not satisfy its generated reducer contract.
    #[error("persisted event {0:?} is invalid")]
    InvalidEvent(String),
    /// A projection adapter returned a row outside the generated partition, selector, or shape.
    #[error("projection resource returned a row outside the generated query contract")]
    InvalidProjection,
    /// Generated plan names a provider absent from this engine.
    #[error("SDK obligation provider {0:?} is not executable")]
    UnknownProvider(String),
    /// Generated plan contains an inconsistent binding.
    #[error("generated realization plan is inconsistent at {0}")]
    InvalidPlan(String),
    /// JSON decoding failed after authentication.
    #[error("operation body is invalid JSON: {0}")]
    Decode(serde_json::Error),
    /// Operation body must be an object.
    #[error("operation body must be a JSON object")]
    ExpectedObject,
    /// Caller supplied a field outside the generated operation contract.
    #[error("unknown operation input {0:?}")]
    UnknownInput(String),
    /// Required operation input is absent.
    #[error("missing operation input {0:?}")]
    MissingInput(String),
    /// Operation input has the wrong representation.
    #[error("invalid operation input {0:?}")]
    InvalidInput(String),
    /// One SDK obligation refused the operation.
    #[error("service obligation refused the operation: {0}")]
    ObligationRefused(String),
    /// Deployment resource failed without leaking adapter details.
    #[error(transparent)]
    Resource(#[from] ResourceError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_runtime::{AuthorityId, RealmId, TenantId, UserId, VerifiedIdentity};

    struct FixedIds;

    struct FixedClock;

    impl Clock for FixedClock {
        fn now(&mut self) -> Result<String, ResourceError> {
            Ok("2026-09-02T10:00:00Z".to_owned())
        }
    }

    impl IdGenerator for FixedIds {
        fn uuid_v7(&mut self) -> Result<String, ResourceError> {
            Ok("01991f7e-2d66-7000-8000-000000000001".to_owned())
        }
    }

    fn context(realm: Option<&str>) -> VerifiedAuthContext {
        VerifiedAuthContext::from_verified(VerifiedIdentity::after_verification(
            TenantId::new("tenant-a").expect("tenant is valid"),
            AuthorityId::new("person-a").expect("authority is valid"),
            UserId::new("person-a").expect("user is valid"),
            None,
            realm.map(|value| RealmId::new(value).expect("realm is valid")),
        ))
    }

    fn two_event_plan() -> (ServicePlan, IntentPlan) {
        let create = ProducedEventPlan {
            event: "demo.Created".to_owned(),
            fields: vec![
                EventFieldPlan {
                    name: "id".to_owned(),
                    source: ValueSource::Input {
                        field: "id".to_owned(),
                    },
                },
                EventFieldPlan {
                    name: "value".to_owned(),
                    source: ValueSource::Literal {
                        value: "created".to_owned(),
                    },
                },
            ],
        };
        let update = ProducedEventPlan {
            event: "demo.Updated".to_owned(),
            fields: vec![
                EventFieldPlan {
                    name: "id".to_owned(),
                    source: ValueSource::Input {
                        field: "id".to_owned(),
                    },
                },
                EventFieldPlan {
                    name: "value".to_owned(),
                    source: ValueSource::Literal {
                        value: "updated".to_owned(),
                    },
                },
            ],
        };
        let intent = IntentPlan {
            command: "demo.CreateAndUpdate".to_owned(),
            inputs: Vec::new(),
            stream: StreamPlan::GeneratedUuidV7,
            expected_version: ExpectedVersionPlan::NoStream,
            idempotency: IdempotencyPlan::RequestId,
            obligations: Vec::new(),
            outcome: OutcomePlan {
                name: "accepted".to_owned(),
                events: vec![create, update],
            },
            projections: Vec::new(),
        };
        let plan = ServicePlan {
            format: REALIZATION_PLAN_FORMAT.to_owned(),
            service: "demo".to_owned(),
            realm: PlanRealmPolicy::Optional,
            ess_source_digest: "a".repeat(64),
            obligation_catalog_digest: "b".repeat(64),
            content: BTreeMap::new(),
            intents: BTreeMap::new(),
            queries: BTreeMap::new(),
            reducers: BTreeMap::from([
                (
                    "demo.Created".to_owned(),
                    ReducerPlan {
                        entity: "demo.Item".to_owned(),
                        identity_field: "id".to_owned(),
                        effect: ReducerEffect::Create {
                            initial_state: "Active".to_owned(),
                        },
                        fields: vec!["value".to_owned()],
                        inherit: None,
                    },
                ),
                (
                    "demo.Updated".to_owned(),
                    ReducerPlan {
                        entity: "demo.Item".to_owned(),
                        identity_field: "id".to_owned(),
                        effect: ReducerEffect::Update,
                        fields: vec!["value".to_owned()],
                        inherit: None,
                    },
                ),
            ]),
            views: BTreeMap::new(),
        };
        (plan, intent)
    }

    #[test]
    fn multi_event_decisions_validate_against_each_preceding_event() {
        let (plan, intent) = two_event_plan();
        let events = produce_events(
            &plan,
            &mut FixedIds,
            &context(None),
            "item-a",
            &BTreeMap::from([("id".to_owned(), Value::String("item-a".to_owned()))]),
            &intent,
            &ProjectionState::default(),
        )
        .expect("the update sees the create before it");

        assert_eq!(events.len(), 2);
    }

    #[test]
    fn projection_rows_must_match_the_exact_hidden_partition_selector_and_shape() {
        let query = QueryPlan {
            view: "demo.ItemById".to_owned(),
            inputs: Vec::new(),
            obligations: Vec::new(),
        };
        let view = ViewPlan {
            source: "demo.Item".to_owned(),
            fields: vec!["id".to_owned()],
            obligations: Vec::new(),
        };
        let selectors = BTreeMap::from([("id".to_owned(), Value::String("item-a".to_owned()))]);
        let row = ProjectionRow {
            view: query.view.clone(),
            tenant: "tenant-a".to_owned(),
            realm: None,
            source_stream: None,
            value: selectors.clone(),
        };

        validate_projection_row(&context(None), &query, &view, &selectors, &row)
            .expect("an exact row is accepted");
        assert!(matches!(
            validate_projection_row(&context(Some("default")), &query, &view, &selectors, &row),
            Err(ExecutionError::InvalidProjection)
        ));

        let mut extra = row;
        extra.value.insert("secret".to_owned(), Value::Bool(true));
        assert!(matches!(
            validate_projection_row(&context(None), &query, &view, &selectors, &extra),
            Err(ExecutionError::InvalidProjection)
        ));
    }

    #[test]
    fn legacy_projection_rows_recover_their_aggregate_source_from_the_plan() {
        let (mut plan, intent) = two_event_plan();
        plan.intents.insert("create".to_owned(), intent);
        let view = ViewPlan {
            source: "demo.Item".to_owned(),
            fields: vec!["id".to_owned(), "value".to_owned()],
            obligations: Vec::new(),
        };
        let value = BTreeMap::from([
            ("id".to_owned(), Value::String("item-a".to_owned())),
            ("value".to_owned(), Value::String("updated".to_owned())),
        ]);

        assert_eq!(
            projection_source_from_value(&plan, &context(None), &view, &value),
            Some(ServiceStream {
                service: "demo".to_owned(),
                tenant: "tenant-a".to_owned(),
                realm: None,
                category: "aggregate".to_owned(),
                key: "item-a".to_owned(),
            })
        );
    }

    #[test]
    fn absent_and_default_realms_are_distinct_stream_partitions() {
        assert_ne!(
            ServiceStream {
                service: "demo".to_owned(),
                tenant: "tenant-a".to_owned(),
                realm: None,
                category: "item".to_owned(),
                key: "item-a".to_owned(),
            },
            ServiceStream {
                service: "demo".to_owned(),
                tenant: "tenant-a".to_owned(),
                realm: Some("default".to_owned()),
                category: "item".to_owned(),
                key: "item-a".to_owned(),
            }
        );
    }

    #[test]
    fn malformed_or_elapsed_bounded_lifetimes_are_invalid_input() {
        let obligation = ObligationUse {
            provider: "sdk.lifecycle.bounded-future/v1".to_owned(),
            bindings: BTreeMap::from([("lifetime".to_owned(), "lifetime".to_owned())]),
        };
        for lifetime in [
            serde_json::json!("persistent"),
            serde_json::json!({"kind": "session"}),
            serde_json::json!({"expires_at": "not-a-timestamp"}),
            serde_json::json!({"expires_at": "2026-09-02T09:59:59Z"}),
        ] {
            let command = BTreeMap::from([("lifetime".to_owned(), lifetime)]);
            assert!(matches!(
                validate_future(&mut FixedClock, &obligation, &command),
                Err(ExecutionError::InvalidInput(field)) if field == "lifetime"
            ));
        }

        let valid = BTreeMap::from([(
            "lifetime".to_owned(),
            serde_json::json!({"expires_at": "2026-09-03T10:00:00Z"}),
        )]);
        assert!(validate_future(&mut FixedClock, &obligation, &valid).is_ok());
    }

    fn live_parent_authority_plan() -> ServicePlan {
        let inherit = InheritancePlan {
            parent: "demo.List".to_owned(),
            parent_owner: "owner".to_owned(),
            parent_scopes: "scopes".to_owned(),
            child_owner: "owner".to_owned(),
            child_scopes: "scopes".to_owned(),
        };
        ServicePlan {
            format: REALIZATION_PLAN_FORMAT.to_owned(),
            service: "demo".to_owned(),
            realm: PlanRealmPolicy::Optional,
            ess_source_digest: "a".repeat(64),
            obligation_catalog_digest: "b".repeat(64),
            content: BTreeMap::new(),
            intents: BTreeMap::new(),
            queries: BTreeMap::new(),
            reducers: BTreeMap::from([
                (
                    "demo.ListCreated".to_owned(),
                    ReducerPlan {
                        entity: "demo.List".to_owned(),
                        identity_field: "list_id".to_owned(),
                        effect: ReducerEffect::Create {
                            initial_state: "Active".to_owned(),
                        },
                        fields: vec!["owner".to_owned(), "scopes".to_owned()],
                        inherit: None,
                    },
                ),
                (
                    "demo.ItemAdded".to_owned(),
                    ReducerPlan {
                        entity: "demo.Item".to_owned(),
                        identity_field: "item_id".to_owned(),
                        effect: ReducerEffect::Create {
                            initial_state: "Open".to_owned(),
                        },
                        fields: vec![
                            "list_id".to_owned(),
                            "owner".to_owned(),
                            "scopes".to_owned(),
                        ],
                        inherit: Some(inherit),
                    },
                ),
                (
                    "demo.ListTransferred".to_owned(),
                    ReducerPlan {
                        entity: "demo.List".to_owned(),
                        identity_field: "list_id".to_owned(),
                        effect: ReducerEffect::Update,
                        fields: vec!["owner".to_owned()],
                        inherit: None,
                    },
                ),
            ]),
            views: BTreeMap::from([(
                "demo.Items".to_owned(),
                ViewPlan {
                    source: "demo.Item".to_owned(),
                    fields: vec![
                        "item_id".to_owned(),
                        "list_id".to_owned(),
                        "owner".to_owned(),
                        "scopes".to_owned(),
                        "state".to_owned(),
                    ],
                    obligations: vec![ObligationUse {
                        provider: "sdk.derive.inherit-parent-authority/v2".to_owned(),
                        bindings: BTreeMap::from([
                            ("parent".to_owned(), "demo.List".to_owned()),
                            ("child".to_owned(), "demo.Item".to_owned()),
                            ("parent_owner".to_owned(), "demo.List.owner".to_owned()),
                            ("parent_scopes".to_owned(), "demo.List.scopes".to_owned()),
                            ("child_owner".to_owned(), "demo.Item.owner".to_owned()),
                            ("child_scopes".to_owned(), "demo.Item.scopes".to_owned()),
                        ]),
                    }],
                },
            )]),
        }
    }

    #[test]
    fn live_parent_authority_replaces_child_projection_authority_after_transfer() {
        let mut plan = live_parent_authority_plan();
        let scopes = serde_json::json!({
            "principal": null,
            "team": null,
            "project": null,
            "extension": null
        });
        let mut state = ProjectionState::default();
        for event in [
            DomainEvent {
                name: "demo.ListCreated".to_owned(),
                fields: BTreeMap::from([
                    ("list_id".to_owned(), Value::String("list-a".to_owned())),
                    ("owner".to_owned(), Value::String("person-a".to_owned())),
                    ("scopes".to_owned(), scopes.clone()),
                ]),
            },
            DomainEvent {
                name: "demo.ItemAdded".to_owned(),
                fields: BTreeMap::from([
                    ("item_id".to_owned(), Value::String("item-a".to_owned())),
                    ("list_id".to_owned(), Value::String("list-a".to_owned())),
                ]),
            },
            DomainEvent {
                name: "demo.ListTransferred".to_owned(),
                fields: BTreeMap::from([
                    ("list_id".to_owned(), Value::String("list-a".to_owned())),
                    ("owner".to_owned(), Value::String("person-b".to_owned())),
                ]),
            },
        ] {
            reduce(&plan, &mut state, &event).expect("the event is valid");
        }

        let rows = projection_rows_for_partition(&plan, "tenant-a", None, &state);
        let item = &rows
            .iter()
            .find(|row| row.row.view == "demo.Items")
            .expect("the child row remains materialized")
            .row
            .value;
        assert_eq!(item["owner"], Value::String("person-b".to_owned()));
        assert_eq!(item["scopes"], scopes);

        plan.views
            .get_mut("demo.Items")
            .expect("view exists")
            .obligations[0]
            .provider = "sdk.derive.inherit-parent-authority/v1".to_owned();
        let legacy = projection_rows_for_partition(&plan, "tenant-a", None, &state);
        assert_eq!(
            legacy[0].row.value["owner"],
            Value::String("person-a".to_owned()),
            "v1 remains frozen as creation-time inheritance"
        );
    }
}
