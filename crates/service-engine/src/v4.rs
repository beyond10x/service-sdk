//! Entity Runtime delegated execution for the closed `service-realization-plan/4` format.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use entity_core::{
    Decision, Evaluation, LoadedDecision, OperationFieldAction, PreloadDecision, Registry, Runtime,
};
use entity_store::asynchronous::{
    AppendMember, AppendOutcome, AppendRequest, BatchKey, CommitReceipt, RecordedEntry,
    StoredBatch, Subject, original_request_comparison_bytes,
};
use entity_store::{Expect, RecordedCommit, RecordedObservation, Recording};
use ess_primitives::facts::{FactPath, FactStore, FactValue};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use service_definition::v4::{OperationFieldPolicy, SlotValueSource};
use service_definition::{ContextValue, QuerySort};
use service_runtime::{RealmPolicy, VerifiedAuthContext};
use service_runtime_ir::v4::{
    CommandBindingDocument, EntityRuntimeBinding, IdentityValueDocument, InstanceBindingDocument,
    OperationFieldCoordinateDocument, PresenceDocument, ResolvedOperationFieldPolicy,
    ResolvedSlotPolicy, SlotCoordinateDocument,
};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::{
    ContentPolicyPlan, ExpectedVersionPlan, IdempotencyPlan, InputPlan, ObligationUse,
    PlanDelivery, ServicePlan, StreamPlan,
};

/// The only realization-plan discriminator which delegates domain semantics to Entity Runtime.
pub const REALIZATION_PLAN_FORMAT_V4: &str = "service-realization-plan/4";

/// Complete generated plan for one ER-delegated service.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServicePlanV4 {
    /// Exact format discriminator.
    pub format: String,
    /// Stable generated service identity.
    pub service: String,
    /// Existing public delivery boundary.
    pub delivery: PlanDelivery,
    /// Exact realm admission policy.
    pub realm: crate::PlanRealmPolicy,
    /// Compiler-minted ESS semantic digest.
    pub ess_source_digest: String,
    /// Exact digest of this plan's semantic inputs.
    pub plan_digest: String,
    /// Complete validated ER definitions and lowerer bindings.
    pub er: EntityRuntimeBinding,
    /// Exact ordinary slot policies.
    #[serde(with = "ordered_map_as_pairs")]
    pub slots: BTreeMap<SlotCoordinateDocument, ResolvedSlotPolicy>,
    /// Exact selected operation-field policies.
    #[serde(with = "ordered_map_as_pairs")]
    pub operation_fields: BTreeMap<OperationFieldCoordinateDocument, ResolvedOperationFieldPolicy>,
    /// SDK-owned mutation admission and host annotations without an SDK outcome selector.
    pub intents: BTreeMap<String, IntentPlanV4>,
    /// SDK-owned projection queries.
    pub queries: BTreeMap<String, QueryPlanV4>,
    /// SDK-owned content policies.
    pub content: BTreeMap<String, ContentPolicyPlan>,
    /// SDK-owned view materialization metadata.
    pub views: BTreeMap<String, ViewPlanV4>,
}

/// Host-owned data needed around one ER command.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntentPlanV4 {
    /// Exact ESS command name.
    pub command: String,
    /// Scope enforced before operation input decoding.
    pub scope: String,
    /// Closed public input inventory.
    pub inputs: Vec<InputPlan>,
    /// Existing claim namespace selector.
    pub stream: StreamPlan,
    /// Public ER subject-revision expectation.
    pub expected_version: ExpectedVersionPlan,
    /// Existing idempotency source.
    pub idempotency: IdempotencyPlan,
    /// SDK host obligations used by admission, effects, and projections.
    pub obligations: Vec<ObligationUse>,
    /// Projections updated after a durable decision.
    pub projections: Vec<String>,
}

/// One authenticated projection read realization with its source ordering.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QueryPlanV4 {
    /// ESS view identity.
    pub view: String,
    /// Exact OAuth scope checked before the request body is decoded.
    pub scope: String,
    /// Closed caller selector inventory.
    pub inputs: Vec<InputPlan>,
    /// SDK implementations applied to returned rows.
    pub obligations: Vec<ObligationUse>,
    /// Complete stable ordering authored for this query.
    pub sort: Vec<QuerySort>,
}

/// Projection row realization with its source-owned filter semantics.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ViewPlanV4 {
    /// Source entity.
    pub source: String,
    /// Public row fields.
    pub fields: Vec<String>,
    /// Canonical ESS type spelling for every public row field.
    pub field_types: BTreeMap<String, String>,
    /// Public row fields that may be absent from the ESS object shape.
    pub optional_fields: BTreeSet<String>,
    /// SDK implementations governing materialization and visibility.
    pub obligations: Vec<ObligationUse>,
    /// Exact validated ESS predicate; absent includes every instance.
    pub filter: Option<ess_primitives::predicate::Predicate>,
}

/// One bounded deterministic projection result produced from authoritative ER instances.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectedQueryV4 {
    /// Visible rows in authored sort order.
    pub items: Vec<Value>,
    /// Subject revision when the page contains exactly one subject.
    pub through_version: Option<u64>,
    /// Opaque cursor for the next raw window.
    pub next_cursor: Option<String>,
    /// Whether another raw window remains.
    pub partial: bool,
}

/// Opaque lifecycle data returned by an injected external content store.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StagedContentV4 {
    /// Opaque immutable reference safe for ER records.
    pub reference: String,
    /// Adapter token consumed by accept or abandon.
    pub token: String,
}

/// Projects one query from a complete authoritative ER snapshot.
///
/// The visibility callback is evaluated before any row is returned. Pagination counts the
/// source-filtered rows rather than only visible rows so withheld rows cannot be inferred from a
/// full visible page.
#[allow(clippy::too_many_lines)]
pub fn project_query_v4(
    plan: &ServicePlanV4,
    operation: &str,
    raw_input: &[u8],
    instances: impl IntoIterator<Item = entity_core::EntityInstance>,
    cursor: Option<&str>,
    limit: usize,
    mut visible: impl FnMut(&entity_core::EntityInstance, &[ObligationUse]) -> Result<bool, String>,
) -> Result<ProjectedQueryV4, ExecutionErrorV4> {
    if limit == 0 || limit > 1_000 {
        return Err(ExecutionErrorV4::Input(
            "query page limit must be between 1 and 1000".to_owned(),
        ));
    }
    let query = plan.queries.get(operation).ok_or_else(|| {
        ExecutionErrorV4::Binding(format!("unknown query operation {operation:?}"))
    })?;
    let view = plan
        .views
        .get(&query.view)
        .ok_or_else(|| ExecutionErrorV4::Binding(format!("missing view {:?}", query.view)))?;
    let input = serde_json::from_slice::<Value>(raw_input)
        .map_err(|error| ExecutionErrorV4::Input(error.to_string()))?;
    let input = input
        .as_object()
        .ok_or_else(|| ExecutionErrorV4::Input("query input must be an object".to_owned()))?;
    let declared = query
        .inputs
        .iter()
        .map(|field| field.name.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(extra) = input
        .keys()
        .find(|field| !declared.contains(field.as_str()))
    {
        return Err(ExecutionErrorV4::Input(format!(
            "query input contains undeclared field {extra:?}"
        )));
    }
    if let Some(missing) = query
        .inputs
        .iter()
        .find(|field| !field.optional && !input.contains_key(&field.name))
    {
        return Err(ExecutionErrorV4::Input(format!(
            "query input omits required field {:?}",
            missing.name
        )));
    }
    let selectors = query
        .inputs
        .iter()
        .filter_map(|field| match &field.source {
            crate::InputSource::Selector { view_field } => input
                .get(&field.name)
                .map(|value| (view_field.as_str(), value)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut rows = Vec::new();
    for instance in instances {
        if instance.entity != view.source {
            continue;
        }
        if view
            .filter
            .as_ref()
            .is_some_and(|filter| !filter.evaluate(&instance_facts(&instance)).is_satisfied())
        {
            continue;
        }
        let mut row = Map::new();
        for field in &view.fields {
            let value = if field == "state" {
                Some(Value::String(instance.lifecycle_state.clone()))
            } else {
                instance.fields.get(field).cloned()
            };
            match value {
                Some(value) => {
                    row.insert(field.clone(), value);
                }
                None if view.optional_fields.contains(field) => {}
                None => {
                    return Err(ExecutionErrorV4::Integrity(format!(
                        "authoritative instance {}/{} omits required view field {field:?}",
                        instance.entity, instance.id
                    )));
                }
            }
        }
        if selectors
            .iter()
            .any(|(field, expected)| row.get(*field) != Some(*expected))
        {
            continue;
        }
        rows.push((instance, Value::Object(row)));
    }
    rows.sort_by(|(left_instance, left), (right_instance, right)| {
        for sort in &query.sort {
            let ordering = compare_json(left.get(&sort.view_field), right.get(&sort.view_field));
            let ordering = match sort.direction {
                service_definition::SortDirection::Ascending => ordering,
                service_definition::SortDirection::Descending => ordering.reverse(),
            };
            if ordering != Ordering::Equal {
                return ordering;
            }
        }
        (&left_instance.entity, &left_instance.id)
            .cmp(&(&right_instance.entity, &right_instance.id))
    });
    let start = decode_query_cursor(cursor, rows.len())?;
    let end = start.saturating_add(limit).min(rows.len());
    let partial = end < rows.len();
    let next_cursor = partial.then(|| format!("sdk-query-v4:{end}"));
    let mut items = Vec::new();
    let mut subjects = BTreeMap::new();
    for (instance, row) in &rows[start..end] {
        if visible(instance, &view.obligations).map_err(ExecutionErrorV4::Projection)? {
            subjects.insert(
                (instance.entity.clone(), instance.id.clone()),
                instance.revision,
            );
            items.push(row.clone());
        }
    }
    let through_version = (subjects.len() == 1)
        .then(|| subjects.values().next().copied())
        .flatten();
    Ok(ProjectedQueryV4 {
        items,
        through_version,
        next_cursor,
        partial,
    })
}

fn decode_query_cursor(cursor: Option<&str>, row_count: usize) -> Result<usize, ExecutionErrorV4> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };
    let offset = cursor
        .strip_prefix("sdk-query-v4:")
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|offset| *offset <= row_count)
        .ok_or_else(|| ExecutionErrorV4::Input("query cursor is invalid".to_owned()))?;
    Ok(offset)
}

fn instance_facts(instance: &entity_core::EntityInstance) -> FactStore {
    let mut facts = FactStore::new();
    facts.set_path("state", FactValue::text(instance.lifecycle_state.clone()));
    for (field, value) in &instance.fields {
        flatten_fact(&mut facts, field, value);
    }
    facts
}

fn flatten_fact(facts: &mut FactStore, path: &str, value: &Value) {
    match value {
        Value::Null => {}
        Value::Bool(value) => set_fact(facts, path, FactValue::bool(*value)),
        Value::Number(value) => {
            set_fact(facts, path, FactValue::parse_literal(&value.to_string()));
        }
        Value::String(value) => set_fact(facts, path, FactValue::text(value.clone())),
        Value::Array(values) => {
            set_fact(
                facts,
                &format!("{path}.count"),
                FactValue::count(values.len()),
            );
            for (index, value) in values.iter().enumerate() {
                flatten_fact(facts, &format!("{path}.{index}"), value);
            }
        }
        Value::Object(values) => {
            for (name, value) in values {
                flatten_fact(facts, &format!("{path}.{name}"), value);
            }
        }
    }
}

fn set_fact(facts: &mut FactStore, path: &str, value: FactValue) {
    if let Ok(path) = FactPath::new(path) {
        facts.set(path, value);
    }
}

fn compare_json(left: Option<&Value>, right: Option<&Value>) -> Ordering {
    match (left, right) {
        (None, None) | (Some(Value::Null), Some(Value::Null)) => Ordering::Equal,
        (None | Some(Value::Null), Some(_)) => Ordering::Less,
        (Some(_), None | Some(Value::Null)) => Ordering::Greater,
        (Some(Value::Bool(left)), Some(Value::Bool(right))) => left.cmp(right),
        (Some(Value::Number(left)), Some(Value::Number(right))) => left
            .as_f64()
            .partial_cmp(&right.as_f64())
            .unwrap_or(Ordering::Equal),
        (Some(Value::String(left)), Some(Value::String(right))) => left.cmp(right),
        (Some(left), Some(right)) => {
            let rank = |value: &Value| match value {
                Value::Null => 0,
                Value::Bool(_) => 1,
                Value::Number(_) => 2,
                Value::String(_) => 3,
                Value::Array(_) => 4,
                Value::Object(_) => 5,
            };
            rank(left)
                .cmp(&rank(right))
                .then_with(|| left.to_string().cmp(&right.to_string()))
        }
    }
}

impl ServicePlanV4 {
    /// Builds a `/4` plan from the validated runtime IR and retained SDK host plan.
    pub fn from_runtime(
        runtime: &service_runtime_ir::v4::ServiceRuntimeIrV4,
        host: &ServicePlan,
    ) -> Result<Self, PlanV4Error> {
        let service = runtime.definition().service.as_str().to_owned();
        if host.service != service {
            return Err(PlanV4Error::HostMismatch(format!(
                "host service {:?} differs from /4 service {service:?}",
                host.service
            )));
        }
        if host.ess_source_digest != runtime.entity_runtime().source_digest {
            return Err(PlanV4Error::HostMismatch(
                "host and ER plans name different ESS source digests".to_owned(),
            ));
        }
        let intents = host
            .intents
            .iter()
            .map(|(name, intent)| {
                (
                    name.clone(),
                    IntentPlanV4 {
                        command: intent.command.clone(),
                        scope: intent.scope.clone(),
                        inputs: intent.inputs.clone(),
                        stream: intent.stream.clone(),
                        expected_version: intent.expected_version.clone(),
                        idempotency: intent.idempotency.clone(),
                        obligations: intent.obligations.clone(),
                        projections: intent.projections.clone(),
                    },
                )
            })
            .collect();
        let queries = host
            .queries
            .iter()
            .map(|(name, query)| {
                let source = runtime
                    .definition()
                    .queries
                    .iter()
                    .find(|source| source.name.as_str() == name)
                    .ok_or_else(|| {
                        PlanV4Error::HostMismatch(format!(
                            "query {name:?} lost its source definition"
                        ))
                    })?;
                Ok((
                    name.clone(),
                    QueryPlanV4 {
                        view: query.view.clone(),
                        scope: query.scope.clone(),
                        inputs: query.inputs.clone(),
                        obligations: query.obligations.clone(),
                        sort: source.sort.clone(),
                    },
                ))
            })
            .collect::<Result<_, PlanV4Error>>()?;
        let views = host
            .views
            .iter()
            .map(|(name, view)| {
                let source = runtime.views().get(name).ok_or_else(|| {
                    PlanV4Error::HostMismatch(format!("view {name:?} lost its source semantics"))
                })?;
                Ok((
                    name.clone(),
                    ViewPlanV4 {
                        source: source.source.clone(),
                        fields: view.fields.clone(),
                        field_types: view.field_types.clone(),
                        optional_fields: view.optional_fields.clone(),
                        obligations: view.obligations.clone(),
                        filter: source.filter.clone(),
                    },
                ))
            })
            .collect::<Result<_, PlanV4Error>>()?;
        let mut plan = Self {
            format: REALIZATION_PLAN_FORMAT_V4.to_owned(),
            service,
            delivery: host.delivery.clone(),
            realm: host.realm,
            ess_source_digest: host.ess_source_digest.clone(),
            plan_digest: String::new(),
            er: runtime.entity_runtime().clone(),
            slots: runtime.slots().clone(),
            operation_fields: runtime.operation_fields().clone(),
            intents,
            queries,
            content: host.content.clone(),
            views,
        };
        plan.plan_digest = plan.semantic_digest();
        plan.validate()?;
        Ok(plan)
    }

    /// Strictly reads and validates canonical generated JSON.
    pub fn from_json(text: &str) -> Result<Self, PlanV4Error> {
        let plan: Self = serde_json::from_str(text).map_err(PlanV4Error::Json)?;
        plan.validate()?;
        if plan.to_canonical_json() != text {
            return Err(PlanV4Error::NonCanonical);
        }
        Ok(plan)
    }

    /// Canonical JSON with one trailing newline.
    pub fn to_canonical_json(&self) -> String {
        let mut output = serde_json::to_string_pretty(self).unwrap_or_else(|error| {
            panic!("validated service-realization-plan/4 serializes: {error}")
        });
        output.push('\n');
        output
    }

    fn validate(&self) -> Result<(), PlanV4Error> {
        if self.format != REALIZATION_PLAN_FORMAT_V4 {
            return Err(PlanV4Error::UnsupportedFormat(self.format.clone()));
        }
        if self.service.trim().is_empty() || self.plan_digest != self.semantic_digest() {
            return Err(PlanV4Error::InvalidDigest);
        }
        let mut registry = Registry::new();
        for definition in self.er.definitions.values() {
            registry
                .register(definition.clone())
                .map_err(|error| PlanV4Error::Registry(error.to_string()))?;
        }
        registry
            .validate_all()
            .map_err(|error| PlanV4Error::Registry(error.to_string()))?;
        for intent in self.intents.values() {
            if !self.er.bindings.commands.contains_key(&intent.command) {
                return Err(PlanV4Error::MissingCommand(intent.command.clone()));
            }
        }
        for (name, query) in &self.queries {
            if !self.views.contains_key(&query.view) {
                return Err(PlanV4Error::MissingView {
                    query: name.clone(),
                    view: query.view.clone(),
                });
            }
        }
        Ok(())
    }

    fn semantic_digest(&self) -> String {
        let mut semantic = self.clone();
        semantic.plan_digest.clear();
        let bytes = serde_json::to_vec(&semantic)
            .expect("closed realization plan semantic inputs serialize");
        hex::encode(Sha256::digest(bytes))
    }
}

mod ordered_map_as_pairs {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

    pub(super) fn serialize<K, V, S>(map: &BTreeMap<K, V>, serializer: S) -> Result<S::Ok, S::Error>
    where
        K: Serialize,
        V: Serialize,
        S: Serializer,
    {
        map.iter().collect::<Vec<_>>().serialize(serializer)
    }

    pub(super) fn deserialize<'de, K, V, D>(deserializer: D) -> Result<BTreeMap<K, V>, D::Error>
    where
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        let pairs = Vec::<(K, V)>::deserialize(deserializer)?;
        let expected = pairs.len();
        let map = pairs.into_iter().collect::<BTreeMap<_, _>>();
        if map.len() != expected {
            return Err(D::Error::custom(
                "duplicate coordinate in ordered policy table",
            ));
        }
        Ok(map)
    }
}

/// A malformed or internally inconsistent generated `/4` plan.
#[derive(Debug, Error)]
pub enum PlanV4Error {
    /// Strict JSON decoding failed.
    #[error("invalid service-realization-plan/4 JSON: {0}")]
    Json(serde_json::Error),
    /// The discriminator is not `/4`.
    #[error("unsupported realization plan format {0:?}")]
    UnsupportedFormat(String),
    /// The complete semantic digest is malformed or stale.
    #[error("realization plan digest is invalid")]
    InvalidDigest,
    /// The retained host plan does not describe the same service.
    #[error("retained host plan mismatch: {0}")]
    HostMismatch(String),
    /// An ER definition or its closure is invalid.
    #[error("invalid Entity Runtime registry: {0}")]
    Registry(String),
    /// A host intent lost its lowerer command.
    #[error("host intent names absent ER command {0:?}")]
    MissingCommand(String),
    /// A query names no retained source view.
    #[error("query {query:?} references missing view {view:?}")]
    MissingView {
        /// Query name.
        query: String,
        /// Missing view name.
        view: String,
    },
    /// Persisted JSON is semantically valid but not canonical bytes.
    #[error("service-realization-plan/4 JSON is not canonical")]
    NonCanonical,
}

/// Exact authenticated partition used for one `/4` Eventlog authority.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticatedPartition {
    /// Complete injective logical scope.
    pub logical_scope: String,
    /// Bounded domain-separated physical Eventlog tenant.
    pub physical_tenant: String,
}

impl AuthenticatedPartition {
    /// Derives the exact partition without normalizing the optional realm.
    pub fn derive(service: &str, context: &VerifiedAuthContext) -> Self {
        let tenant = context.tenant().as_str();
        let mut logical_scope = format!(
            "sdk-er-partition/1|{}:{}|{}:{}|",
            service.len(),
            service,
            tenant.len(),
            tenant
        );
        match context.realm() {
            None => logical_scope.push('0'),
            Some(realm) => {
                let realm = realm.as_str();
                logical_scope.push_str("1|");
                logical_scope.push_str(&realm.len().to_string());
                logical_scope.push(':');
                logical_scope.push_str(realm);
            }
        }
        let physical_tenant = format!(
            "sdk-er-v4-{}",
            hex::encode(Sha256::digest(logical_scope.as_bytes()))
        );
        Self {
            logical_scope,
            physical_tenant,
        }
    }
}

/// Canonical SDK-owned original intent committed beside the ER decision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OriginalIntentV4 {
    /// Exact realization plan digest.
    pub plan_digest: String,
    /// Public operation name.
    pub operation: String,
    /// Exact admitted normalized command input.
    pub input: Value,
    /// Generated service identity.
    pub service: String,
    /// Verified tenant.
    pub tenant: String,
    /// Exact optional verified realm.
    pub realm: Option<String>,
    /// Verified authority.
    pub authority: String,
    /// Verified user.
    pub user: String,
    /// Optional verified executor.
    pub executor: Option<String>,
    /// Caller-stable idempotency identity.
    pub idempotency_key: String,
    /// Existing compiled aggregate category.
    pub category: String,
    /// Exact existing stream-selector namespace.
    pub selector: ClaimSelectorV4,
}

/// Existing stream selector retained in the `/4` claim namespace.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClaimSelectorV4 {
    /// Exact selected command-field stream key.
    CommandField {
        /// Exact admitted selected stream key.
        value: String,
    },
    /// Stable creation selector before any UUID is minted.
    GeneratedUuidV7,
}

/// SDK-only observation atomically committed after the complete ER decision.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntentObservationV4 {
    /// Exact strict observation format.
    pub format: String,
    /// Complete original intent.
    pub intent: OriginalIntentV4,
    /// SHA-256 over canonical original-intent bytes.
    pub intent_digest: String,
    /// ER-selected outcome.
    pub selected_outcome: Option<String>,
    /// ER-selected response.
    pub response: Option<Map<String, Value>>,
    /// Resulting public ER subject revision.
    pub through_version: u64,
}

/// One successful or refusing `/4` mutation response.
#[derive(Clone, Debug, PartialEq)]
pub enum MutationResultV4 {
    /// ER selected a declared refusing branch; no store or post-commit effect ran.
    Refused(entity_core::Refusal),
    /// A fresh or recovered complete decision was durably verified.
    Committed {
        /// Complete ER decision.
        decision: Box<Decision>,
        /// Original immutable atomic batch receipt.
        receipt: CommitReceipt,
        /// Whether the response recovered previously committed evidence.
        replayed: bool,
    },
}

/// SDK resources around the pure ER decision boundary.
pub trait ResourcesV4 {
    /// Enforces the operation scope before application input is decoded.
    fn authorize(&mut self, context: &VerifiedAuthContext, scope: &str) -> Result<(), String>;
    /// Opens or verifies the exact immutable partition binding.
    fn bind_partition(&mut self, partition: &AuthenticatedPartition) -> Result<(), String>;
    /// Looks up one original named batch before any business side effect.
    fn lookup_batch(&mut self, key: &BatchKey) -> Result<Option<StoredBatch>, String>;
    /// Loads only the exact subject requested by ER's opaque continuation.
    fn load(&mut self, subject: &Subject) -> Result<Option<entity_core::EntityInstance>, String>;
    /// Returns complete decision history for replay verification.
    fn history(&mut self, subject: &Subject) -> Result<Vec<entity_core::DecisionRecord>, String>;
    /// Atomically appends the complete ER decision and SDK observation.
    fn append(&mut self, request: AppendRequest) -> Result<AppendOutcome, String>;
    /// Supplies the trusted clock value for one slot.
    fn trusted_clock(&mut self) -> Result<Value, String>;
    /// Supplies one `UUIDv7` logical value for one slot.
    fn uuid_v7(&mut self) -> Result<Value, String>;
    /// Runs one declared ordinary-slot obligation once.
    fn slot_obligation(
        &mut self,
        name: &str,
        input: &Map<String, Value>,
        context: &VerifiedAuthContext,
    ) -> Result<Option<Value>, String>;
    /// Runs one selected operation-field obligation once.
    fn operation_field_obligation(
        &mut self,
        name: &str,
        field: &str,
        input: &Map<String, Value>,
        loaded: &entity_core::EntityInstance,
        context: &VerifiedAuthContext,
    ) -> Result<Value, String>;
    /// Stages validated plaintext and returns only an opaque record-safe reference.
    fn stage_content(
        &mut self,
        context: &VerifiedAuthContext,
        policy: &str,
        idempotency_key: &str,
        media_type: &str,
        bytes: &[u8],
    ) -> Result<StagedContentV4, String>;
    /// Accepts content after its decision, projections, and effects are durable.
    fn accept_content(
        &mut self,
        context: &VerifiedAuthContext,
        token: String,
    ) -> Result<(), String>;
    /// Abandons content after a conclusively uncommitted refusal or failure.
    fn abandon_content(
        &mut self,
        context: &VerifiedAuthContext,
        token: String,
    ) -> Result<(), String>;
    /// Applies declared projections from the durable authoritative result.
    fn project(
        &mut self,
        intent: &IntentPlanV4,
        decision: &Decision,
        receipt: &CommitReceipt,
    ) -> Result<(), String>;
    /// Runs declared external effects from the durable selected result.
    fn effects(
        &mut self,
        intent: &IntentPlanV4,
        decision: &Decision,
        receipt: &CommitReceipt,
    ) -> Result<(), String>;
}

/// A typed admission, decision, persistence, or committed-aftercare failure.
#[derive(Debug, Error)]
pub enum ExecutionErrorV4 {
    /// Authentication or authorization refused before input decoding.
    #[error("admission refused: {0}")]
    Admission(String),
    /// Application input is invalid JSON or not an object.
    #[error("invalid operation input: {0}")]
    Input(String),
    /// The operation or lowerer binding is absent or inconsistent.
    #[error("invalid generated binding: {0}")]
    Binding(String),
    /// A host slot or selected fulfillment could not be supplied.
    #[error("host fulfillment failed: {0}")]
    Fulfillment(String),
    /// Entity Runtime refused malformed or invalid semantics.
    #[error("Entity Runtime failed: {0}")]
    Runtime(#[from] entity_core::CoreError),
    /// The exact ER subject is absent.
    #[error("unknown subject {entity} {id}")]
    UnknownSubject {
        /// Exact entity name.
        entity: String,
        /// Exact derived subject identity.
        id: String,
    },
    /// The public expected subject revision does not match authority.
    #[error("subject revision conflict: expected {expected}, found {found:?}")]
    RevisionConflict {
        /// Caller-supplied expected subject revision.
        expected: u64,
        /// Authoritative subject revision, when the subject exists.
        found: Option<u64>,
    },
    /// Same claim key was used for different original intent.
    #[error("idempotency conflict")]
    IdempotencyConflict,
    /// Durable evidence is partial, malformed, or fails replay.
    #[error("durable intent evidence is corrupt: {0}")]
    Integrity(String),
    /// Persistence failed or remained uncertain without conclusive recovery.
    #[error("persistence failed or remained uncertain: {0}")]
    Persistence(String),
    /// Authoritative projection repair or visibility evaluation failed.
    #[error("projection failed: {0}")]
    Projection(String),
    /// Projection or effect failed after commit; the receipt remains authoritative.
    #[error("post-commit delivery failed: {cause}")]
    CommittedAftercare {
        /// Original immutable durable receipt.
        receipt: Box<CommitReceipt>,
        /// Stable repair token derived from the claim key.
        repair_token: String,
        /// Projection or effect failure.
        cause: String,
    },
}

/// Pure decision coordinator over one validated generated `/4` plan.
pub struct EngineV4<'a> {
    plan: &'a ServicePlanV4,
    registry: Registry,
}

impl<'a> EngineV4<'a> {
    /// Validates and registers the complete generated definition closure.
    pub fn new(plan: &'a ServicePlanV4) -> Result<Self, PlanV4Error> {
        plan.validate()?;
        let mut registry = Registry::new();
        for definition in plan.er.definitions.values() {
            registry
                .register(definition.clone())
                .map_err(|error| PlanV4Error::Registry(error.to_string()))?;
        }
        registry
            .validate_all()
            .map_err(|error| PlanV4Error::Registry(error.to_string()))?;
        Ok(Self { plan, registry })
    }

    /// Admits, decides, atomically records, and delivers one mutation.
    #[allow(clippy::too_many_arguments)]
    pub fn execute_json(
        &self,
        context: &VerifiedAuthContext,
        operation: &str,
        raw_input: &[u8],
        expected_version: u64,
        idempotency_key: &str,
        recording: Recording,
        resources: &mut dyn ResourcesV4,
    ) -> Result<MutationResultV4, ExecutionErrorV4> {
        let intent =
            self.plan.intents.get(operation).ok_or_else(|| {
                ExecutionErrorV4::Binding(format!("unknown operation {operation:?}"))
            })?;
        resources
            .authorize(context, &intent.scope)
            .map_err(ExecutionErrorV4::Admission)?;
        enforce_realm(self.plan.realm, context)?;
        let input = serde_json::from_slice::<Value>(raw_input)
            .map_err(|error| ExecutionErrorV4::Input(error.to_string()))?;
        let input = input
            .as_object()
            .cloned()
            .ok_or_else(|| ExecutionErrorV4::Input("command input must be an object".to_owned()))?;
        self.execute_admitted(
            context,
            operation,
            intent,
            input,
            expected_version,
            idempotency_key,
            recording,
            resources,
        )
    }

    /// Executes one generated public operation envelope, removing host-only concurrency fields.
    pub fn execute_public_json(
        &self,
        context: &VerifiedAuthContext,
        operation: &str,
        raw_input: &[u8],
        recording: Recording,
        resources: &mut dyn ResourcesV4,
    ) -> Result<MutationResultV4, ExecutionErrorV4> {
        let intent =
            self.plan.intents.get(operation).ok_or_else(|| {
                ExecutionErrorV4::Binding(format!("unknown operation {operation:?}"))
            })?;
        resources
            .authorize(context, &intent.scope)
            .map_err(ExecutionErrorV4::Admission)?;
        enforce_realm(self.plan.realm, context)?;
        let mut input = serde_json::from_slice::<Value>(raw_input)
            .map_err(|error| ExecutionErrorV4::Input(error.to_string()))?
            .as_object()
            .cloned()
            .ok_or_else(|| ExecutionErrorV4::Input("command input must be an object".to_owned()))?;
        let declared = intent
            .inputs
            .iter()
            .map(|field| field.name.as_str())
            .collect::<BTreeSet<_>>();
        if let Some(extra) = input
            .keys()
            .find(|field| !declared.contains(field.as_str()))
        {
            return Err(ExecutionErrorV4::Input(format!(
                "operation input contains undeclared field {extra:?}"
            )));
        }
        if let Some(missing) = intent
            .inputs
            .iter()
            .find(|field| !field.optional && !input.contains_key(&field.name))
        {
            return Err(ExecutionErrorV4::Input(format!(
                "operation input omits required field {:?}",
                missing.name
            )));
        }
        let expected_version = match &intent.expected_version {
            ExpectedVersionPlan::NoStream => 0,
            ExpectedVersionPlan::OperationField { field } => input
                .remove(field)
                .and_then(|value| value.as_u64())
                .ok_or_else(|| {
                    ExecutionErrorV4::Input(format!(
                        "expected-version field {field:?} must be a nonnegative integer"
                    ))
                })?,
        };
        let idempotency_key = match &intent.idempotency {
            IdempotencyPlan::OperationField { field } => input
                .remove(field)
                .and_then(|value| value.as_str().map(str::to_owned))
                .ok_or_else(|| {
                    ExecutionErrorV4::Input(format!("idempotency field {field:?} must be a string"))
                })?,
            IdempotencyPlan::RequestId => recording.record_id.clone(),
        };
        self.execute_admitted(
            context,
            operation,
            intent,
            input,
            expected_version,
            &idempotency_key,
            recording,
            resources,
        )
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn execute_admitted(
        &self,
        context: &VerifiedAuthContext,
        operation: &str,
        intent: &IntentPlanV4,
        mut input: Map<String, Value>,
        expected_version: u64,
        idempotency_key: &str,
        recording: Recording,
        resources: &mut dyn ResourcesV4,
    ) -> Result<MutationResultV4, ExecutionErrorV4> {
        let partition = AuthenticatedPartition::derive(&self.plan.service, context);
        resources
            .bind_partition(&partition)
            .map_err(ExecutionErrorV4::Admission)?;
        let binding = self
            .plan
            .er
            .bindings
            .commands
            .get(&intent.command)
            .ok_or_else(|| {
                ExecutionErrorV4::Binding(format!("missing command {}", intent.command))
            })?;
        let selector = claim_selector(&intent.stream, &input)?;
        let (claim_input, pending_content) = prepare_content(&self.plan.content, intent, &input)?;
        let original = OriginalIntentV4 {
            plan_digest: self.plan.plan_digest.clone(),
            operation: operation.to_owned(),
            input: Value::Object(claim_input),
            service: self.plan.service.clone(),
            tenant: context.tenant().as_str().to_owned(),
            realm: context.realm().map(|value| value.as_str().to_owned()),
            authority: context.authority().as_str().to_owned(),
            user: context.user().as_str().to_owned(),
            executor: context.executor().map(|value| value.as_str().to_owned()),
            idempotency_key: idempotency_key.to_owned(),
            category: binding.entity.clone(),
            selector,
        };
        let claim = claim_key(&partition, &original);
        let batch_key = BatchKey::Named(claim.clone());
        if let Some(batch) = resources
            .lookup_batch(&batch_key)
            .map_err(ExecutionErrorV4::Persistence)?
        {
            return recover(resources, &batch, &original);
        }

        let mut staged_content = Vec::new();
        for pending in pending_content {
            let staged = match resources.stage_content(
                context,
                &pending.policy,
                idempotency_key,
                &pending.media_type,
                &pending.bytes,
            ) {
                Ok(staged) => staged,
                Err(cause) => {
                    abandon_content(resources, context, staged_content);
                    return Err(ExecutionErrorV4::Fulfillment(cause));
                }
            };
            input.remove(&pending.input_field);
            input.insert(
                pending.command_field,
                Value::String(staged.reference.clone()),
            );
            staged_content.push(staged);
        }

        let mut append_dispatched = false;
        let execution = (|| {
            let bound = resolve_slots(self.plan, &intent.command, &input, context, resources)?;
            let arguments = serde_json::json!({"input": input, "bound": bound});
            let runtime = Runtime::new(&self.registry);
            let (decision, expect) = match &binding.instance {
                InstanceBindingDocument::Created {
                    logical_identity, ..
                } => {
                    if expected_version != 0 {
                        return Err(ExecutionErrorV4::RevisionConflict {
                            expected: expected_version,
                            found: None,
                        });
                    }
                    let logical = identity_value(logical_identity, &arguments)?;
                    let id = address(self.plan, binding, &logical)?;
                    match runtime.decide_create(
                        &binding.entity,
                        binding.version,
                        id,
                        arguments.clone(),
                    )? {
                        Evaluation::Refused(refusal) => {
                            return Ok(MutationResultV4::Refused(refusal));
                        }
                        Evaluation::Accepted(decision) => (decision, Expect::Absent),
                    }
                }
                InstanceBindingDocument::SelectedOutcome { .. } => {
                    if expected_version != 0 {
                        return Err(ExecutionErrorV4::RevisionConflict {
                            expected: expected_version,
                            found: None,
                        });
                    }
                    match runtime.decide_create_derived(
                        &binding.entity,
                        binding.version,
                        arguments.clone(),
                    )? {
                        Evaluation::Refused(refusal) => {
                            return Ok(MutationResultV4::Refused(refusal));
                        }
                        Evaluation::Accepted(decision) => (decision, Expect::Absent),
                    }
                }
                InstanceBindingDocument::Supplied { input_field } => {
                    if expected_version == 0 {
                        return Err(ExecutionErrorV4::RevisionConflict {
                            expected: 0,
                            found: None,
                        });
                    }
                    let logical =
                        arguments["input"]
                            .get(input_field)
                            .cloned()
                            .ok_or_else(|| {
                                ExecutionErrorV4::Input(format!(
                                    "missing subject field {input_field:?}"
                                ))
                            })?;
                    let id = address(self.plan, binding, &logical)?;
                    let prepared = match &binding.entrypoint {
                        service_runtime_ir::v4::EntrypointDocument::Operation { name } => runtime
                            .decide_before_load(
                            &binding.entity,
                            binding.version,
                            id,
                            name,
                            arguments.clone(),
                        )?,
                        service_runtime_ir::v4::EntrypointDocument::Create => {
                            return Err(ExecutionErrorV4::Binding(
                                "supplied instance uses create entrypoint".to_owned(),
                            ));
                        }
                    };
                    let prepared = match prepared {
                        PreloadDecision::Refused(refusal) => {
                            return Ok(MutationResultV4::Refused(refusal));
                        }
                        PreloadDecision::Load(prepared) => prepared,
                    };
                    let subject =
                        Subject::new(prepared.subject().entity(), prepared.subject().id())
                            .map_err(|error| ExecutionErrorV4::Binding(error.to_string()))?;
                    let loaded = resources
                        .load(&subject)
                        .map_err(ExecutionErrorV4::Persistence)?
                        .ok_or_else(|| ExecutionErrorV4::UnknownSubject {
                            entity: subject.entity.clone(),
                            id: subject.id.clone(),
                        })?;
                    if loaded.revision != expected_version {
                        return Err(ExecutionErrorV4::RevisionConflict {
                            expected: expected_version,
                            found: Some(loaded.revision),
                        });
                    }
                    let evaluation = match prepared.select_with(&loaded)? {
                        LoadedDecision::Complete(evaluation) => evaluation,
                        LoadedDecision::NeedsFulfillment(selected) => {
                            let actions = resolve_operation_fields(
                                self.plan,
                                &intent.command,
                                selected.outcome(),
                                selected.requirements().keys(),
                                arguments["input"].as_object().expect("input object"),
                                &loaded,
                                context,
                                resources,
                            )?;
                            selected.complete(actions)?
                        }
                    };
                    match evaluation {
                        Evaluation::Refused(refusal) => {
                            return Ok(MutationResultV4::Refused(refusal));
                        }
                        Evaluation::Accepted(decision) => {
                            (decision, Expect::Revision(expected_version))
                        }
                    }
                }
            };
            let intent_digest = digest_json(&original);
            let observation = IntentObservationV4 {
                format: "service-intent-observation/4".to_owned(),
                intent: original.clone(),
                intent_digest,
                selected_outcome: decision.record.outcome.clone(),
                response: decision.record.response.clone(),
                through_version: decision.instance.revision,
            };
            let commit = RecordedCommit::new(decision.clone(), &recording)
                .map_err(|error| ExecutionErrorV4::Binding(error.to_string()))?;
            let Recording {
                record_id,
                recorded_at,
                correlation,
                causation: _,
                actor,
            } = recording;
            let observation_recording = Recording {
                record_id: format!("sdk-intent-observation-{claim}"),
                recorded_at,
                correlation,
                causation: Some(record_id),
                actor,
            };
            let observed = RecordedObservation {
                entity: decision.instance.entity.clone(),
                id: decision.instance.id.clone(),
                revision: decision.instance.revision,
                envelope: observation_recording
                    .seal(
                        serde_json::to_value(&observation).expect("closed observation serializes"),
                    )
                    .map_err(|error| ExecutionErrorV4::Binding(error.to_string()))?,
            };
            let decision_entry = RecordedEntry::Decision(commit);
            let observation_entry = RecordedEntry::Observation(observed);
            let request = AppendRequest::new(
                batch_key.clone(),
                vec![
                    AppendMember::new(
                        expect,
                        decision_entry.clone(),
                        original_request_comparison_bytes(&decision_entry)
                            .map_err(|error| ExecutionErrorV4::Binding(error.to_string()))?,
                    ),
                    AppendMember::new(
                        Expect::Revision(decision.instance.revision),
                        observation_entry.clone(),
                        original_request_comparison_bytes(&observation_entry)
                            .map_err(|error| ExecutionErrorV4::Binding(error.to_string()))?,
                    ),
                ],
            )
            .map_err(|error| ExecutionErrorV4::Binding(error.to_string()))?;
            append_dispatched = true;
            let outcome = match resources.append(request) {
                Ok(outcome) => outcome,
                Err(cause) => match resources.lookup_batch(&batch_key) {
                    Ok(Some(batch)) => return recover(resources, &batch, &original),
                    Ok(None) | Err(_) => return Err(ExecutionErrorV4::Persistence(cause)),
                },
            };
            let receipt = outcome.receipt().cloned().ok_or_else(|| {
                ExecutionErrorV4::Integrity("append returned no committed receipt".to_owned())
            })?;
            deliver_after_commit(resources, intent, &decision, &receipt, &claim)?;
            Ok(MutationResultV4::Committed {
                decision: Box::new(decision),
                receipt,
                replayed: outcome.replayed(),
            })
        })();
        match execution {
            Ok(MutationResultV4::Committed {
                decision,
                receipt,
                replayed,
            }) => {
                for content in staged_content {
                    resources
                        .accept_content(context, content.token)
                        .map_err(|cause| ExecutionErrorV4::CommittedAftercare {
                            receipt: Box::new(receipt.clone()),
                            repair_token: format!("sdk-er-repair-{claim}"),
                            cause,
                        })?;
                }
                Ok(MutationResultV4::Committed {
                    decision,
                    receipt,
                    replayed,
                })
            }
            Ok(refused @ MutationResultV4::Refused(_)) => {
                abandon_content(resources, context, staged_content);
                Ok(refused)
            }
            Err(error @ ExecutionErrorV4::CommittedAftercare { .. }) => Err(error),
            Err(error @ ExecutionErrorV4::Persistence(_)) if append_dispatched => Err(error),
            Err(error) => {
                abandon_content(resources, context, staged_content);
                Err(error)
            }
        }
    }
}

fn enforce_realm(
    policy: crate::PlanRealmPolicy,
    context: &VerifiedAuthContext,
) -> Result<(), ExecutionErrorV4> {
    let runtime = match policy {
        crate::PlanRealmPolicy::Required => RealmPolicy::Required,
        crate::PlanRealmPolicy::Optional => RealmPolicy::Optional,
        crate::PlanRealmPolicy::Forbidden => RealmPolicy::Forbidden,
    };
    runtime
        .enforce(context)
        .map_err(|error| ExecutionErrorV4::Admission(error.to_string()))
}

struct PendingContentV4 {
    input_field: String,
    command_field: String,
    policy: String,
    media_type: String,
    bytes: Vec<u8>,
}

fn prepare_content(
    policies: &BTreeMap<String, crate::ContentPolicyPlan>,
    intent: &IntentPlanV4,
    input: &Map<String, Value>,
) -> Result<(Map<String, Value>, Vec<PendingContentV4>), ExecutionErrorV4> {
    let mut claim_input = input.clone();
    let mut pending = Vec::new();
    for field in &intent.inputs {
        let crate::InputSource::Content {
            policy,
            command_field,
        } = &field.source
        else {
            continue;
        };
        let Some(value) = input.get(&field.name) else {
            if field.optional {
                continue;
            }
            return Err(ExecutionErrorV4::Input(format!(
                "missing content input {:?}",
                field.name
            )));
        };
        let content_policy = policies.get(policy).ok_or_else(|| {
            ExecutionErrorV4::Binding(format!("missing content policy {policy:?}"))
        })?;
        let object = value.as_object().ok_or_else(|| {
            ExecutionErrorV4::Input(format!("content input {:?} must be an object", field.name))
        })?;
        if object.len() != 2 || !object.contains_key("media_type") || !object.contains_key("text") {
            return Err(ExecutionErrorV4::Input(format!(
                "content input {:?} must contain exactly media_type and text",
                field.name
            )));
        }
        let media_type = object["media_type"].as_str().ok_or_else(|| {
            ExecutionErrorV4::Input(format!(
                "content input {:?} has invalid media_type",
                field.name
            ))
        })?;
        let text = object["text"].as_str().ok_or_else(|| {
            ExecutionErrorV4::Input(format!("content input {:?} has invalid text", field.name))
        })?;
        if !content_policy.media_types.contains(media_type)
            || u64::try_from(text.len()).map_or(true, |length| length > content_policy.max_bytes)
        {
            return Err(ExecutionErrorV4::Input(format!(
                "content input {:?} violates its policy",
                field.name
            )));
        }
        let mut digest = Sha256::new();
        digest.update(b"service-content-claim/4");
        digest.update((media_type.len() as u64).to_be_bytes());
        digest.update(media_type.as_bytes());
        digest.update((text.len() as u64).to_be_bytes());
        digest.update(text.as_bytes());
        claim_input.insert(
            field.name.clone(),
            serde_json::json!({
                "media_type": media_type,
                "sha256": hex::encode(digest.finalize()),
            }),
        );
        pending.push(PendingContentV4 {
            input_field: field.name.clone(),
            command_field: command_field.clone(),
            policy: policy.clone(),
            media_type: media_type.to_owned(),
            bytes: text.as_bytes().to_vec(),
        });
    }
    Ok((claim_input, pending))
}

fn abandon_content(
    resources: &mut dyn ResourcesV4,
    context: &VerifiedAuthContext,
    staged: Vec<StagedContentV4>,
) {
    for content in staged {
        let _ = resources.abandon_content(context, content.token);
    }
}

fn claim_selector(
    stream: &StreamPlan,
    input: &Map<String, Value>,
) -> Result<ClaimSelectorV4, ExecutionErrorV4> {
    match stream {
        StreamPlan::CommandField { field } => input
            .get(field)
            .and_then(Value::as_str)
            .map(|value| ClaimSelectorV4::CommandField {
                value: value.to_owned(),
            })
            .ok_or_else(|| {
                ExecutionErrorV4::Input(format!("stream selector field {field:?} must be a string"))
            }),
        StreamPlan::GeneratedUuidV7 => Ok(ClaimSelectorV4::GeneratedUuidV7),
    }
}

fn claim_key(partition: &AuthenticatedPartition, intent: &OriginalIntentV4) -> String {
    let bytes = serde_json::to_vec(&(
        "service-intent-claim/4",
        &partition.logical_scope,
        &intent.category,
        &intent.selector,
        &intent.idempotency_key,
    ))
    .expect("closed claim key serializes");
    hex::encode(Sha256::digest(bytes))
}

fn resolve_slots(
    plan: &ServicePlanV4,
    command: &str,
    input: &Map<String, Value>,
    context: &VerifiedAuthContext,
    resources: &mut dyn ResourcesV4,
) -> Result<Map<String, Value>, ExecutionErrorV4> {
    let binding = plan
        .er
        .bindings
        .commands
        .get(command)
        .ok_or_else(|| ExecutionErrorV4::Binding(format!("missing command {command}")))?;
    let mut bound = Map::new();
    for (slot, lowerer_value) in &binding.slots {
        let coordinate = SlotCoordinateDocument {
            command: command.to_owned(),
            slot: *slot,
        };
        let policy = plan.slots.get(&coordinate).ok_or_else(|| {
            ExecutionErrorV4::Binding(format!("missing slot policy {command}/{slot}"))
        })?;
        let value = resolve_slot(policy, input, context, resources)?;
        match (value, lowerer_value.presence) {
            (Some(value), _) => {
                bound.insert(format!("b{slot:08}"), value);
            }
            (None, PresenceDocument::Optional) => {}
            (None, PresenceDocument::Required) => {
                return Err(ExecutionErrorV4::Fulfillment(format!(
                    "required slot {command}/{slot} was absent"
                )));
            }
        }
    }
    Ok(bound)
}

fn resolve_slot(
    policy: &ResolvedSlotPolicy,
    input: &Map<String, Value>,
    context: &VerifiedAuthContext,
    resources: &mut dyn ResourcesV4,
) -> Result<Option<Value>, ExecutionErrorV4> {
    let value = match &policy.source {
        SlotValueSource::Absent => None,
        SlotValueSource::VerifiedContext { value } => match value {
            ContextValue::TenantId => Some(Value::String(context.tenant().as_str().to_owned())),
            ContextValue::RealmIdOptional => context
                .realm()
                .map(|value| Value::String(value.as_str().to_owned())),
            ContextValue::UserId => Some(Value::String(context.user().as_str().to_owned())),
            ContextValue::CurrentAuthority => {
                Some(Value::String(context.authority().as_str().to_owned()))
            }
            ContextValue::ExecutorOptional => context
                .executor()
                .map(|value| Value::String(value.as_str().to_owned())),
        },
        SlotValueSource::CommandField { field } => input.get(field).cloned(),
        SlotValueSource::TrustedClock => Some(
            resources
                .trusted_clock()
                .map_err(ExecutionErrorV4::Fulfillment)?,
        ),
        SlotValueSource::GeneratedUuidV7 => {
            Some(resources.uuid_v7().map_err(ExecutionErrorV4::Fulfillment)?)
        }
        SlotValueSource::Obligation { name } => resources
            .slot_obligation(name.as_str(), input, context)
            .map_err(ExecutionErrorV4::Fulfillment)?,
        SlotValueSource::Literal { value } => Some(value.clone()),
    };
    Ok(value)
}

#[allow(clippy::too_many_arguments)]
fn resolve_operation_fields<'a>(
    plan: &ServicePlanV4,
    command: &str,
    outcome: &str,
    requirements: impl Iterator<Item = &'a String>,
    input: &Map<String, Value>,
    loaded: &entity_core::EntityInstance,
    context: &VerifiedAuthContext,
    resources: &mut dyn ResourcesV4,
) -> Result<BTreeMap<String, OperationFieldAction>, ExecutionErrorV4> {
    let mut actions = BTreeMap::new();
    for field in requirements {
        let coordinate = OperationFieldCoordinateDocument {
            command: command.to_owned(),
            outcome: outcome.to_owned(),
            field: field.clone(),
        };
        let resolved = plan.operation_fields.get(&coordinate).ok_or_else(|| {
            ExecutionErrorV4::Binding(format!(
                "missing operation-field policy {command}/{outcome}/{field}"
            ))
        })?;
        let action = match &resolved.policy {
            OperationFieldPolicy::Preserve => OperationFieldAction::Preserve,
            OperationFieldPolicy::Remove => OperationFieldAction::Remove,
            OperationFieldPolicy::CommandField { field: source } => OperationFieldAction::Set {
                value: input.get(source).cloned().ok_or_else(|| {
                    ExecutionErrorV4::Fulfillment(format!(
                        "missing command field {source:?} for {field:?}"
                    ))
                })?,
            },
            OperationFieldPolicy::Obligation { name } => OperationFieldAction::Set {
                value: resources
                    .operation_field_obligation(name.as_str(), field, input, loaded, context)
                    .map_err(ExecutionErrorV4::Fulfillment)?,
            },
        };
        actions.insert(field.clone(), action);
    }
    Ok(actions)
}

fn identity_value(
    source: &IdentityValueDocument,
    arguments: &Value,
) -> Result<Value, ExecutionErrorV4> {
    match source {
        IdentityValueDocument::InputField { field } => arguments["input"]
            .get(field)
            .cloned()
            .ok_or_else(|| ExecutionErrorV4::Input(format!("missing identity input {field:?}"))),
        IdentityValueDocument::Literal { value } => Ok(value.clone()),
        IdentityValueDocument::Bound { slot } => arguments["bound"]
            .get(format!("b{slot:08}"))
            .cloned()
            .ok_or_else(|| ExecutionErrorV4::Fulfillment(format!("missing identity slot {slot}"))),
    }
}

fn address(
    plan: &ServicePlanV4,
    binding: &CommandBindingDocument,
    logical: &Value,
) -> Result<String, ExecutionErrorV4> {
    let definition = plan.er.definitions.get(&binding.entity).ok_or_else(|| {
        ExecutionErrorV4::Binding(format!("missing definition {}", binding.entity))
    })?;
    let identity = definition.identity.as_ref().ok_or_else(|| {
        ExecutionErrorV4::Binding(format!("definition {} has no identity", binding.entity))
    })?;
    let field = definition
        .schema
        .fields
        .get(&identity.field)
        .ok_or_else(|| {
            ExecutionErrorV4::Binding(format!(
                "definition identity field {:?} is absent",
                identity.field
            ))
        })?;
    entity_core::identity::address(field.kind, logical)
        .map_err(|error| ExecutionErrorV4::Input(error.to_string()))
}

fn recover(
    resources: &mut dyn ResourcesV4,
    batch: &StoredBatch,
    original: &OriginalIntentV4,
) -> Result<MutationResultV4, ExecutionErrorV4> {
    let [decision_record, observation_record] = batch.records.as_slice() else {
        return Err(ExecutionErrorV4::Integrity(
            "intent batch must contain exactly decision and observation".to_owned(),
        ));
    };
    let RecordedEntry::Decision(commit) = &decision_record.entry else {
        return Err(ExecutionErrorV4::Integrity(
            "first intent batch member is not a decision".to_owned(),
        ));
    };
    let RecordedEntry::Observation(observed) = &observation_record.entry else {
        return Err(ExecutionErrorV4::Integrity(
            "second intent batch member is not an observation".to_owned(),
        ));
    };
    if observed.entity != commit.instance.entity
        || observed.id != commit.instance.id
        || observed.revision != commit.instance.revision
    {
        return Err(ExecutionErrorV4::Integrity(
            "intent observation does not describe the winning decision revision".to_owned(),
        ));
    }
    let observation: IntentObservationV4 = serde_json::from_value(observed.envelope.record.clone())
        .map_err(|error| ExecutionErrorV4::Integrity(error.to_string()))?;
    if observation.format != "service-intent-observation/4"
        || observation.intent_digest != digest_json(&observation.intent)
    {
        return Err(ExecutionErrorV4::Integrity(
            "intent observation digest or format is invalid".to_owned(),
        ));
    }
    if &observation.intent != original {
        return Err(ExecutionErrorV4::IdempotencyConflict);
    }
    if observation.selected_outcome != commit.envelope.record.outcome
        || observation.response != commit.envelope.record.response
        || observation.through_version != commit.instance.revision
    {
        return Err(ExecutionErrorV4::Integrity(
            "intent observation selected result differs from the ER record".to_owned(),
        ));
    }
    let subject = Subject::new(&commit.instance.entity, &commit.instance.id)
        .map_err(|error| ExecutionErrorV4::Integrity(error.to_string()))?;
    let history = resources
        .history(&subject)
        .map_err(ExecutionErrorV4::Persistence)?;
    let replayed = entity_core::replay(&history)
        .map_err(|error| ExecutionErrorV4::Integrity(error.to_string()))?;
    if replayed != commit.instance {
        return Err(ExecutionErrorV4::Integrity(
            "verified ER history does not reproduce the winning instance".to_owned(),
        ));
    }
    Ok(MutationResultV4::Committed {
        decision: Box::new(commit.decision()),
        receipt: batch.receipt.clone(),
        replayed: true,
    })
}

fn deliver_after_commit(
    resources: &mut dyn ResourcesV4,
    intent: &IntentPlanV4,
    decision: &Decision,
    receipt: &CommitReceipt,
    claim: &str,
) -> Result<(), ExecutionErrorV4> {
    resources
        .project(intent, decision, receipt)
        .and_then(|()| resources.effects(intent, decision, receipt))
        .map_err(|cause| ExecutionErrorV4::CommittedAftercare {
            receipt: Box::new(receipt.clone()),
            repair_token: format!("sdk-er-repair-{claim}"),
            cause,
        })
}

fn digest_json(value: &impl Serialize) -> String {
    let bytes = serde_json::to_vec(value).expect("closed SDK observation data serializes");
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContentPolicyPlan, InputSource};

    fn content_intent(optional: bool) -> IntentPlanV4 {
        IntentPlanV4 {
            command: "demo.document.Create".to_owned(),
            scope: "documents.manage".to_owned(),
            inputs: vec![InputPlan {
                name: "content".to_owned(),
                type_ref: "Bytes".to_owned(),
                optional,
                source: InputSource::Content {
                    policy: "body".to_owned(),
                    command_field: "content_ref".to_owned(),
                },
            }],
            stream: StreamPlan::GeneratedUuidV7,
            expected_version: ExpectedVersionPlan::NoStream,
            idempotency: IdempotencyPlan::RequestId,
            obligations: Vec::new(),
            projections: Vec::new(),
        }
    }

    fn content_policies() -> BTreeMap<String, ContentPolicyPlan> {
        BTreeMap::from([(
            "body".to_owned(),
            ContentPolicyPlan {
                media_types: BTreeSet::from(["text/plain".to_owned()]),
                max_bytes: 16,
            },
        )])
    }

    #[test]
    fn content_preparation_keeps_plaintext_out_of_the_original_intent() {
        let input = serde_json::json!({
            "content": {"media_type": "text/plain", "text": "private body"}
        })
        .as_object()
        .expect("object")
        .clone();
        let (claim, pending) =
            prepare_content(&content_policies(), &content_intent(false), &input).unwrap();

        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].policy, "body");
        assert_eq!(pending[0].command_field, "content_ref");
        assert_eq!(pending[0].media_type, "text/plain");
        assert_eq!(pending[0].bytes, b"private body");
        assert_eq!(claim["content"]["media_type"], "text/plain");
        assert_eq!(claim["content"]["sha256"].as_str().unwrap().len(), 64);
        assert!(
            !serde_json::to_string(&claim)
                .unwrap()
                .contains("private body")
        );
    }

    #[test]
    fn content_preparation_enforces_shape_policy_size_and_optional_absence() {
        let policy = content_policies();
        let required = content_intent(false);
        for invalid in [
            serde_json::json!({"content": "plaintext"}),
            serde_json::json!({"content": {"media_type": "text/html", "text": "body"}}),
            serde_json::json!({"content": {"media_type": "text/plain", "text": "a body longer than sixteen bytes"}}),
            serde_json::json!({"content": {"media_type": "text/plain", "text": "body", "extra": true}}),
        ] {
            assert!(prepare_content(&policy, &required, invalid.as_object().unwrap()).is_err());
        }
        assert!(prepare_content(&policy, &required, &Map::new()).is_err());
        let (claim, pending) =
            prepare_content(&policy, &content_intent(true), &Map::new()).unwrap();
        assert!(claim.is_empty());
        assert!(pending.is_empty());
    }
}
