//! Entity Runtime delegated execution for the closed `service-realization-plan/4` format.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use entity_core::{
    Decision, DecisionCommand, Evaluation, LoadedDecision, OperationFieldAction, PreloadDecision,
    Registry, Runtime,
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
    ACCEPTED_ENTITY_RUNTIME_REVISION, ADAPTER_ENTITY_RUNTIME_REVISION, CommandBindingDocument,
    EntityRuntimeBinding, IdentityValueDocument, InstanceBindingDocument,
    OperationFieldCoordinateDocument, PresenceDocument, ResolvedOperationFieldPolicy,
    ResolvedSlotPolicy, SlotCoordinateDocument,
};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::{
    AuthorityCheck, ContentPolicyPlan, ExpectedVersionPlan, IdempotencyPlan, InputPlan,
    ObligationUse, PlanDelivery, ServicePlan, StreamPlan,
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
        if self.er.target_revision != ACCEPTED_ENTITY_RUNTIME_REVISION
            || self.er.target_revision != ADAPTER_ENTITY_RUNTIME_REVISION
        {
            return Err(PlanV4Error::IncompatibleEntityRuntimeTarget(
                self.er.target_revision.clone(),
            ));
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
    /// The document names an ER semantic target other than this SDK's exact admitted target.
    #[error("incompatible Entity Runtime target revision {0:?}")]
    IncompatibleEntityRuntimeTarget(String),
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
    /// Exact public optimistic-concurrency precondition for this attempt.
    ///
    /// Candidate-era observations omitted this field. Their original comparison semantics did
    /// not bind the precondition, so recovery preserves that meaning while every new observation
    /// records and compares it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_version: Option<u64>,
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
    /// Returns the complete authenticated-partition terminal state needed by cross-entity SDK
    /// obligation providers. Implementations without such a provider retain an empty state; any
    /// selected provider that needs it then refuses explicitly.
    fn obligation_instances(&mut self) -> Result<Vec<entity_core::EntityInstance>, String> {
        Ok(Vec::new())
    }
    /// Evaluates one closed SDK authority question against receiver-verified facts.
    /// Implementations without verified facts deny the question rather than admitting it.
    fn obligation_authority(
        &mut self,
        _context: &VerifiedAuthContext,
        _check: AuthorityCheck,
    ) -> Result<bool, String> {
        Ok(false)
    }
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
    /// Idempotently accepts committed staged content from its durable record-safe coordinates.
    fn accept_recorded_content(
        &mut self,
        context: &VerifiedAuthContext,
        policy: &str,
        idempotency_key: &str,
        reference: &str,
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
        context: &VerifiedAuthContext,
        intent: &IntentPlanV4,
        decision: &Decision,
        receipt: &CommitReceipt,
    ) -> Result<(), String>;
    /// Runs declared external effects from the durable selected result.
    fn effects(
        &mut self,
        context: &VerifiedAuthContext,
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
    /// A selected SDK intent obligation refused the admitted command.
    #[error("service obligation refused the operation: {0}")]
    ObligationRefused(String),
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

    /// Repairs projection and durable effect delivery from one authoritative committed batch.
    ///
    /// The token selects no application input. The recorded original intent, selected ER decision,
    /// immutable receipt, and complete subject history remain the only delivery authority.
    pub fn repair(
        &self,
        context: &VerifiedAuthContext,
        token: &str,
        resources: &mut dyn ResourcesV4,
    ) -> Result<MutationResultV4, ExecutionErrorV4> {
        let (operation, claim) = repair_coordinates(token)
            .ok_or_else(|| ExecutionErrorV4::Input("invalid repair token".to_owned()))?;
        let intent = self.plan.intents.get(&operation).ok_or_else(|| {
            ExecutionErrorV4::Binding(format!("unknown repair operation {operation:?}"))
        })?;
        resources
            .authorize(context, &intent.scope)
            .map_err(ExecutionErrorV4::Admission)?;
        enforce_realm(self.plan.realm, context)?;
        let partition = AuthenticatedPartition::derive(&self.plan.service, context);
        resources
            .bind_partition(&partition)
            .map_err(ExecutionErrorV4::Admission)?;
        let batch_key = BatchKey::Named(claim.clone());
        let batch = resources
            .lookup_batch(&batch_key)
            .map_err(ExecutionErrorV4::Persistence)?
            .ok_or_else(|| ExecutionErrorV4::Persistence("repair batch is absent".to_owned()))?;
        if batch.key != batch_key {
            return Err(ExecutionErrorV4::Integrity(
                "repair lookup returned a different batch key".to_owned(),
            ));
        }
        let [_, observation_record] = batch.records.as_slice() else {
            return Err(ExecutionErrorV4::Integrity(
                "intent batch must contain exactly decision and observation".to_owned(),
            ));
        };
        let RecordedEntry::Observation(observed) = &observation_record.entry else {
            return Err(ExecutionErrorV4::Integrity(
                "second intent batch member is not an observation".to_owned(),
            ));
        };
        let observation: IntentObservationV4 =
            serde_json::from_value(observed.envelope.record.clone())
                .map_err(|error| ExecutionErrorV4::Integrity(error.to_string()))?;
        if observation.format != "service-intent-observation/4"
            || observation.intent_digest != digest_json(&observation.intent)
        {
            return Err(ExecutionErrorV4::Integrity(
                "intent observation digest or format is invalid".to_owned(),
            ));
        }
        let original = &observation.intent;
        let binding = self
            .plan
            .er
            .bindings
            .commands
            .get(&intent.command)
            .ok_or_else(|| {
                ExecutionErrorV4::Binding(format!("missing command {}", intent.command))
            })?;
        if original.plan_digest != self.plan.plan_digest
            || original.operation != operation
            || original.service != self.plan.service
            || original.tenant != context.tenant().as_str()
            || original.realm.as_deref() != context.realm().map(service_runtime::RealmId::as_str)
            || original.authority != context.authority().as_str()
            || original.user != context.user().as_str()
            || original.executor.as_deref()
                != context.executor().map(service_runtime::ExecutorId::as_str)
            || original.category != binding.entity
            || claim_key(&partition, original) != claim
        {
            return Err(ExecutionErrorV4::Integrity(
                "repair token does not match the recorded authenticated intent".to_owned(),
            ));
        }
        let recovered = recover(resources, &batch, original)?;
        let MutationResultV4::Committed {
            decision, receipt, ..
        } = recovered
        else {
            return Err(ExecutionErrorV4::Integrity(
                "repair batch does not contain a committed decision".to_owned(),
            ));
        };
        let recorded_content = recorded_content_acceptances(intent, &decision, original)?;
        deliver_after_commit(
            resources, context, &operation, intent, &decision, &receipt, &claim,
        )?;
        for (policy, reference) in recorded_content {
            resources
                .accept_recorded_content(context, &policy, &original.idempotency_key, &reference)
                .map_err(|cause| ExecutionErrorV4::CommittedAftercare {
                    receipt: Box::new(receipt.clone()),
                    repair_token: repair_token(&operation, &claim),
                    cause,
                })?;
        }
        Ok(MutationResultV4::Committed {
            decision,
            receipt,
            replayed: true,
        })
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
            expected_version: Some(expected_version),
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
                    enforce_intent_obligations(
                        self.plan, resources, context, intent, &input, None,
                    )?;
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
                    enforce_intent_obligations(
                        self.plan, resources, context, intent, &input, None,
                    )?;
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
                    enforce_intent_obligations(
                        self.plan,
                        resources,
                        context,
                        intent,
                        &input,
                        Some(&loaded),
                    )?;
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
            deliver_after_commit(
                resources, context, operation, intent, &decision, &receipt, &claim,
            )?;
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
                            repair_token: repair_token(operation, &claim),
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

fn recorded_content_acceptances(
    intent: &IntentPlanV4,
    decision: &Decision,
    original: &OriginalIntentV4,
) -> Result<Vec<(String, String)>, ExecutionErrorV4> {
    let original_input = original.input.as_object().ok_or_else(|| {
        ExecutionErrorV4::Integrity("recorded original intent input is not an object".to_owned())
    })?;
    let arguments = match &decision.record.command {
        DecisionCommand::Create { arguments, .. } | DecisionCommand::Execute { arguments, .. } => {
            arguments
        }
        DecisionCommand::LegacyImport => {
            return Err(ExecutionErrorV4::Integrity(
                "committed service intent cannot be a legacy import".to_owned(),
            ));
        }
    };
    let decided_input = arguments
        .get("input")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            ExecutionErrorV4::Integrity(
                "recorded service decision omits normalized input".to_owned(),
            )
        })?;
    let mut recorded = Vec::new();
    for field in &intent.inputs {
        let crate::InputSource::Content {
            policy,
            command_field,
        } = &field.source
        else {
            continue;
        };
        if !original_input.contains_key(&field.name) {
            continue;
        }
        let reference = decided_input
            .get(command_field)
            .and_then(Value::as_str)
            .filter(|reference| !reference.trim().is_empty())
            .ok_or_else(|| {
                ExecutionErrorV4::Integrity(format!(
                    "recorded content input {:?} omits committed reference {command_field:?}",
                    field.name
                ))
            })?;
        recorded.push((policy.clone(), reference.to_owned()));
    }
    Ok(recorded)
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

fn enforce_intent_obligations(
    plan: &ServicePlanV4,
    resources: &mut dyn ResourcesV4,
    context: &VerifiedAuthContext,
    intent: &IntentPlanV4,
    input: &Map<String, Value>,
    current: Option<&entity_core::EntityInstance>,
) -> Result<(), ExecutionErrorV4> {
    let needs_state = intent.obligations.iter().any(obligation_needs_instances_v4);
    let mut instances = if needs_state {
        resources
            .obligation_instances()
            .map_err(ExecutionErrorV4::Fulfillment)?
    } else {
        Vec::new()
    };
    if let Some(current) = current {
        instances.retain(|item| item.entity != current.entity || item.id != current.id);
        instances.push(current.clone());
    }
    for obligation in &intent.obligations {
        enforce_intent_obligation_v4(
            plan, resources, context, &instances, current, obligation, input,
        )?;
    }
    Ok(())
}

fn obligation_needs_instances_v4(obligation: &ObligationUse) -> bool {
    matches!(
        obligation.provider.as_str(),
        "sdk.lifecycle.require-state/v1"
            | "sdk.auth.owner-and-conjunctive-scopes/v1"
            | "sdk.lifecycle.expiry-due/v1"
            | "sdk.lifecycle.expiring-parent-child/v1"
            | "sdk.aggregate.nested-entity/v1"
            | "sdk.aggregate.owned-revision/v1"
            | "sdk.graph.connect-dag/v1"
            | "sdk.graph.node-unreferenced/v1"
            | "sdk.graph.publish-snapshot/v1"
    )
}

fn enforce_intent_obligation_v4(
    plan: &ServicePlanV4,
    resources: &mut dyn ResourcesV4,
    context: &VerifiedAuthContext,
    instances: &[entity_core::EntityInstance],
    current: Option<&entity_core::EntityInstance>,
    obligation: &ObligationUse,
    input: &Map<String, Value>,
) -> Result<(), ExecutionErrorV4> {
    match obligation.provider.as_str() {
        "sdk.auth.owner-and-conjunctive-scopes/v1"
        | "sdk.auth.requested-scopes/v1"
        | "sdk.auth.same-partition-owner-transfer/v1"
        | "sdk.auth.trusted-scheduler/v1" => {
            enforce_authority_obligation_v4(resources, context, current, obligation, input)
        }
        "sdk.lifecycle.require-state/v1"
        | "sdk.lifecycle.bounded-future/v1"
        | "sdk.lifecycle.expiry-due/v1"
        | "sdk.lifecycle.expiring-parent-child/v1" => {
            enforce_lifecycle_obligation_v4(plan, resources, instances, current, obligation, input)
        }
        "sdk.aggregate.nested-entity/v1" | "sdk.aggregate.owned-revision/v1" => {
            enforce_aggregate_obligation_v4(plan, instances, obligation, input)
        }
        "sdk.graph.connect-dag/v1"
        | "sdk.graph.node-unreferenced/v1"
        | "sdk.graph.publish-snapshot/v1" => {
            validate_graph_obligation_v4(plan, instances, obligation, input)
        }
        "sdk.aggregate.event-sourced/v1"
        | "sdk.content.external-erasable/v1"
        | "sdk.derive.tagged-value/v1"
        | "sdk.derive.trusted-clock/v1"
        | "sdk.effect.email/v1"
        | "sdk.derive.inherit-parent-authority/v1"
        | "sdk.derive.inherit-parent-authority/v2"
        | "sdk.projection.auth-partitioned-visibility/v1"
        | "sdk.projection.conjunctive-scopes-visibility/v1"
        | "sdk.projection.hide-terminal-parent/v1" => Ok(()),
        other => Err(ExecutionErrorV4::Binding(format!(
            "SDK obligation provider {other:?} is not executable"
        ))),
    }
}

fn enforce_authority_obligation_v4(
    resources: &mut dyn ResourcesV4,
    context: &VerifiedAuthContext,
    current: Option<&entity_core::EntityInstance>,
    obligation: &ObligationUse,
    input: &Map<String, Value>,
) -> Result<(), ExecutionErrorV4> {
    let check = match obligation.provider.as_str() {
        "sdk.auth.owner-and-conjunctive-scopes/v1" => {
            let owner = bound_instance_field(current, obligation_binding_v4(obligation, "owner")?)?;
            let scopes =
                bound_instance_field(current, obligation_binding_v4(obligation, "scopes")?)?;
            AuthorityCheck::OwnerAndScopes { owner, scopes }
        }
        "sdk.auth.requested-scopes/v1" => {
            let field = obligation_binding_v4(obligation, "scopes")?;
            let scopes = input.get(field).cloned().ok_or_else(|| {
                ExecutionErrorV4::Input(format!("missing obligation input {field:?}"))
            })?;
            AuthorityCheck::RequestedScopes { scopes }
        }
        "sdk.auth.same-partition-owner-transfer/v1" => {
            let field = obligation
                .bindings
                .get("new_owner")
                .map_or("new_owner", String::as_str);
            let new_owner = input.get(field).cloned().ok_or_else(|| {
                ExecutionErrorV4::Input(format!("missing obligation input {field:?}"))
            })?;
            AuthorityCheck::OwnerTransfer { new_owner }
        }
        "sdk.auth.trusted-scheduler/v1" => AuthorityCheck::Capability {
            capability: obligation_binding_v4(obligation, "capability")?.to_owned(),
        },
        _ => unreachable!("caller admits only authority obligations"),
    };
    require_obligation_authority(resources, context, check)
}

fn enforce_lifecycle_obligation_v4(
    plan: &ServicePlanV4,
    resources: &mut dyn ResourcesV4,
    instances: &[entity_core::EntityInstance],
    current: Option<&entity_core::EntityInstance>,
    obligation: &ObligationUse,
    input: &Map<String, Value>,
) -> Result<(), ExecutionErrorV4> {
    match obligation.provider.as_str() {
        "sdk.lifecycle.require-state/v1" => {
            let entity = bound_instance(
                plan, instances, current, obligation, input, "entity", "identity",
            )?;
            if !obligation_binding_v4(obligation, "allowed")?
                .split(',')
                .map(str::trim)
                .any(|allowed| allowed == entity.lifecycle_state)
            {
                return Err(ExecutionErrorV4::ObligationRefused("wrong_state".into()));
            }
        }
        "sdk.lifecycle.bounded-future/v1" => {
            let field = obligation_binding_v4(obligation, "lifetime")?;
            let candidate = input
                .get(field)
                .and_then(lifetime_instant_v4)
                .ok_or_else(|| ExecutionErrorV4::Input(field.to_owned()))?;
            let candidate = parse_instant_v4(candidate)?;
            let now = resources
                .trusted_clock()
                .map_err(ExecutionErrorV4::Fulfillment)?;
            let now = now.as_str().ok_or_else(|| {
                ExecutionErrorV4::Fulfillment("trusted clock did not return a string".into())
            })?;
            if candidate <= parse_instant_v4(now)? {
                return Err(ExecutionErrorV4::ObligationRefused(
                    "invalid_lifetime".into(),
                ));
            }
        }
        "sdk.lifecycle.expiry-due/v1" => {
            let entity = bound_instance(
                plan, instances, current, obligation, input, "entity", "identity",
            )?;
            let expiry = entity
                .fields
                .get(obligation_binding_v4(obligation, "lifetime")?)
                .and_then(lifetime_instant_v4)
                .ok_or_else(|| ExecutionErrorV4::Binding("entity lifetime".into()))?;
            let now = resources
                .trusted_clock()
                .map_err(ExecutionErrorV4::Fulfillment)?;
            let now = now.as_str().ok_or_else(|| {
                ExecutionErrorV4::Fulfillment("trusted clock did not return a string".into())
            })?;
            if parse_instant_v4(expiry)? > parse_instant_v4(now)? {
                return Err(ExecutionErrorV4::ObligationRefused("expiry_not_due".into()));
            }
        }
        "sdk.lifecycle.expiring-parent-child/v1" => {
            enforce_parent_child_lifetime_v4(resources, instances, obligation, input)?;
        }
        _ => unreachable!("caller admits only lifecycle obligations"),
    }
    Ok(())
}

fn enforce_parent_child_lifetime_v4(
    resources: &mut dyn ResourcesV4,
    instances: &[entity_core::EntityInstance],
    obligation: &ObligationUse,
    input: &Map<String, Value>,
) -> Result<(), ExecutionErrorV4> {
    let child_field = obligation_binding_v4(obligation, "child_lifetime")?;
    let candidate = input
        .get(child_field)
        .and_then(lifetime_instant_v4)
        .ok_or_else(|| ExecutionErrorV4::ObligationRefused("invalid_lifetime".into()))?;
    let parent = instances
        .iter()
        .find(|item| item.entity == obligation_binding_v4(obligation, "parent").unwrap_or_default())
        .ok_or_else(|| ExecutionErrorV4::ObligationRefused("parent_not_found".into()))?;
    let parent_expiry = parent
        .fields
        .get(obligation_binding_v4(obligation, "parent_lifetime")?)
        .and_then(lifetime_instant_v4)
        .ok_or_else(|| ExecutionErrorV4::ObligationRefused("parent_not_found".into()))?;
    let now = resources
        .trusted_clock()
        .map_err(ExecutionErrorV4::Fulfillment)?;
    if parse_instant_v4(candidate)? <= parse_instant_v4(now.as_str().unwrap_or(""))?
        || parse_instant_v4(candidate)? > parse_instant_v4(parent_expiry)?
    {
        return Err(ExecutionErrorV4::ObligationRefused(
            "invalid_lifetime".into(),
        ));
    }
    Ok(())
}

fn enforce_aggregate_obligation_v4(
    plan: &ServicePlanV4,
    instances: &[entity_core::EntityInstance],
    obligation: &ObligationUse,
    input: &Map<String, Value>,
) -> Result<(), ExecutionErrorV4> {
    match obligation.provider.as_str() {
        "sdk.aggregate.nested-entity/v1" => {
            let parent_entity = obligation_binding_v4(obligation, "parent")?;
            let parent_identity = obligation_identity_address_v4(
                plan,
                obligation,
                input,
                "parent",
                "parent_identity",
            )?;
            if !instances
                .iter()
                .any(|item| item.entity == parent_entity && item.id == parent_identity)
            {
                return Err(ExecutionErrorV4::ObligationRefused(
                    "parent_not_found".into(),
                ));
            }
            if obligation.bindings.contains_key("child_identity") {
                let child_entity = obligation_binding_v4(obligation, "child")?;
                let child_identity = obligation_identity_address_v4(
                    plan,
                    obligation,
                    input,
                    "child",
                    "child_identity",
                )?;
                if !instances
                    .iter()
                    .any(|item| item.entity == child_entity && item.id == child_identity)
                {
                    return Err(ExecutionErrorV4::ObligationRefused("not_found".into()));
                }
            }
        }
        "sdk.aggregate.owned-revision/v1" => {
            let revision_entity = obligation_binding_v4(obligation, "revision")?;
            let identity = obligation_identity_address_v4(
                plan,
                obligation,
                input,
                "revision",
                "revision_identity",
            )?;
            let revision = instances
                .iter()
                .find(|item| item.entity == revision_entity && item.id == identity)
                .ok_or_else(|| ExecutionErrorV4::ObligationRefused("revision_not_found".into()))?;
            if let Some(allowed) = obligation.bindings.get("allowed")
                && !allowed
                    .split(',')
                    .map(str::trim)
                    .any(|state| state == revision.lifecycle_state)
            {
                return Err(ExecutionErrorV4::ObligationRefused(
                    "revision_not_publishable".into(),
                ));
            }
        }
        _ => unreachable!("caller admits only aggregate obligations"),
    }
    Ok(())
}

fn obligation_identity_address_v4(
    plan: &ServicePlanV4,
    obligation: &ObligationUse,
    input: &Map<String, Value>,
    entity_binding: &str,
    identity_binding: &str,
) -> Result<String, ExecutionErrorV4> {
    let entity = obligation_binding_v4(obligation, entity_binding)?;
    let field = obligation_binding_v4(obligation, identity_binding)?;
    let logical = input
        .get(field)
        .ok_or_else(|| ExecutionErrorV4::Input(field.to_owned()))?;
    address_for_entity(plan, entity, logical)
}

fn obligation_binding_v4<'a>(
    obligation: &'a ObligationUse,
    name: &str,
) -> Result<&'a str, ExecutionErrorV4> {
    obligation
        .bindings
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| {
            ExecutionErrorV4::Binding(format!("{}.bindings.{name}", obligation.provider))
        })
}

fn require_obligation_authority(
    resources: &mut dyn ResourcesV4,
    context: &VerifiedAuthContext,
    check: AuthorityCheck,
) -> Result<(), ExecutionErrorV4> {
    if resources
        .obligation_authority(context, check)
        .map_err(ExecutionErrorV4::Fulfillment)?
    {
        Ok(())
    } else {
        Err(ExecutionErrorV4::ObligationRefused("forbidden".into()))
    }
}

fn bound_instance<'a>(
    plan: &ServicePlanV4,
    instances: &'a [entity_core::EntityInstance],
    current: Option<&'a entity_core::EntityInstance>,
    obligation: &ObligationUse,
    input: &Map<String, Value>,
    entity_binding: &str,
    identity_binding: &str,
) -> Result<&'a entity_core::EntityInstance, ExecutionErrorV4> {
    let entity = obligation.bindings.get(entity_binding).ok_or_else(|| {
        ExecutionErrorV4::Binding(format!("{}.bindings.{entity_binding}", obligation.provider))
    })?;
    let identity = obligation
        .bindings
        .get(identity_binding)
        .map(|field| {
            let logical = input
                .get(field)
                .ok_or_else(|| ExecutionErrorV4::Input(field.clone()))?;
            address_for_entity(plan, entity, logical)
        })
        .transpose()?;
    identity
        .as_ref()
        .map_or_else(
            || current.filter(|item| item.entity == *entity),
            |identity| {
                instances
                    .iter()
                    .find(|item| item.entity == *entity && item.id == *identity)
            },
        )
        .ok_or_else(|| ExecutionErrorV4::ObligationRefused("not_found".into()))
}

fn bound_instance_field(
    current: Option<&entity_core::EntityInstance>,
    path: &str,
) -> Result<Value, ExecutionErrorV4> {
    let (entity, field) = path
        .rsplit_once('.')
        .ok_or_else(|| ExecutionErrorV4::Binding(path.to_owned()))?;
    current
        .filter(|item| item.entity == entity)
        .and_then(|item| item.fields.get(field))
        .cloned()
        .ok_or_else(|| ExecutionErrorV4::ObligationRefused("not_found".into()))
}

fn lifetime_instant_v4(value: &Value) -> Option<&str> {
    value
        .as_object()
        .and_then(|object| object.get("expires_at"))
        .and_then(Value::as_str)
}

fn parse_instant_v4(value: &str) -> Result<OffsetDateTime, ExecutionErrorV4> {
    OffsetDateTime::parse(value, &Rfc3339)
        .map_err(|_| ExecutionErrorV4::ObligationRefused("invalid_lifetime".into()))
}

fn validate_graph_obligation_v4(
    plan: &ServicePlanV4,
    instances: &[entity_core::EntityInstance],
    obligation: &ObligationUse,
    input: &Map<String, Value>,
) -> Result<(), ExecutionErrorV4> {
    let mut graph = graph_state_v4(plan, instances, obligation, input)?;
    match obligation.provider.as_str() {
        "sdk.graph.connect-dag/v1" => {
            let source =
                obligation_identity_address_v4(plan, obligation, input, "nodes", "source")?;
            let target =
                obligation_identity_address_v4(plan, obligation, input, "nodes", "target")?;
            insert_graph_pair_v4(&graph.nodes, &mut graph.pairs, &source, &target)?;
        }
        "sdk.graph.node-unreferenced/v1" => {
            let node = obligation_identity_address_v4(plan, obligation, input, "nodes", "node")?;
            if !graph.nodes.contains(&node) {
                return Err(ExecutionErrorV4::ObligationRefused("node_not_found".into()));
            }
            if graph
                .pairs
                .iter()
                .any(|(source, target)| source == &node || target == &node)
            {
                return Err(ExecutionErrorV4::ObligationRefused(
                    "node_referenced".into(),
                ));
            }
        }
        "sdk.graph.publish-snapshot/v1" => {}
        _ => unreachable!("caller admits only graph providers"),
    }
    if graph.pairs.len() > graph_limit_v4(obligation, "max_edges", 2_000)? {
        return Err(ExecutionErrorV4::ObligationRefused(
            "graph_too_large".into(),
        ));
    }
    if graph_has_cycle_v4(&graph.nodes, &graph.pairs) {
        return Err(ExecutionErrorV4::ObligationRefused("graph_cycle".into()));
    }
    Ok(())
}

struct GraphStateV4 {
    nodes: BTreeSet<String>,
    pairs: BTreeSet<(String, String)>,
}

fn graph_state_v4(
    plan: &ServicePlanV4,
    instances: &[entity_core::EntityInstance],
    obligation: &ObligationUse,
    input: &Map<String, Value>,
) -> Result<GraphStateV4, ExecutionErrorV4> {
    let partition = obligation_input_string_v4(obligation, input, "partition")?;
    let active = obligation
        .bindings
        .get("active_state")
        .map_or("Active", String::as_str);
    let node_partition = obligation_binding_v4(obligation, "node_partition")?;
    let edge_partition = obligation_binding_v4(obligation, "edge_partition")?;
    let edge_source = obligation_binding_v4(obligation, "edge_source")?;
    let edge_target = obligation_binding_v4(obligation, "edge_target")?;
    let node_identity = obligation_binding_v4(obligation, "node_identity")?;
    let nodes_entity = obligation_binding_v4(obligation, "nodes")?;
    let edges_entity = obligation_binding_v4(obligation, "edges")?;
    let nodes = instances
        .iter()
        .filter(|item| {
            item.entity == nodes_entity
                && item.lifecycle_state == active
                && item.fields.get(node_partition).and_then(Value::as_str)
                    == Some(partition.as_str())
        })
        .map(|item| {
            let logical = item
                .fields
                .get(node_identity)
                .ok_or_else(|| ExecutionErrorV4::Binding(node_identity.to_owned()))?;
            let address = address_for_entity(plan, nodes_entity, logical)?;
            if address != item.id {
                return Err(ExecutionErrorV4::Binding(format!(
                    "node identity {node_identity:?} does not match its ER address"
                )));
            }
            Ok(address)
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if nodes.is_empty() {
        return Err(ExecutionErrorV4::ObligationRefused("empty_graph".into()));
    }
    if nodes.len() > graph_limit_v4(obligation, "max_nodes", 500)? {
        return Err(ExecutionErrorV4::ObligationRefused(
            "graph_too_large".into(),
        ));
    }
    let mut pairs = BTreeSet::new();
    for edge in instances.iter().filter(|item| {
        item.entity == edges_entity
            && item.lifecycle_state == active
            && item.fields.get(edge_partition).and_then(Value::as_str) == Some(partition.as_str())
    }) {
        let source = edge
            .fields
            .get(edge_source)
            .ok_or_else(|| ExecutionErrorV4::Binding(edge_source.to_owned()))?;
        let target = edge
            .fields
            .get(edge_target)
            .ok_or_else(|| ExecutionErrorV4::Binding(edge_target.to_owned()))?;
        let source = address_for_entity(plan, nodes_entity, source)?;
        let target = address_for_entity(plan, nodes_entity, target)?;
        insert_graph_pair_v4(&nodes, &mut pairs, &source, &target)?;
    }
    Ok(GraphStateV4 { nodes, pairs })
}

fn obligation_input_string_v4(
    obligation: &ObligationUse,
    input: &Map<String, Value>,
    name: &str,
) -> Result<String, ExecutionErrorV4> {
    let field = obligation_binding_v4(obligation, name)?;
    input
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ExecutionErrorV4::Input(field.to_owned()))
}

fn graph_limit_v4(
    obligation: &ObligationUse,
    name: &str,
    default: usize,
) -> Result<usize, ExecutionErrorV4> {
    obligation.bindings.get(name).map_or(Ok(default), |value| {
        value
            .parse::<usize>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                ExecutionErrorV4::Binding(format!("{}.bindings.{name}", obligation.provider))
            })
    })
}

fn insert_graph_pair_v4(
    nodes: &BTreeSet<String>,
    pairs: &mut BTreeSet<(String, String)>,
    source: &str,
    target: &str,
) -> Result<(), ExecutionErrorV4> {
    if source == target {
        return Err(ExecutionErrorV4::ObligationRefused("self_edge".into()));
    }
    if !nodes.contains(source) || !nodes.contains(target) {
        return Err(ExecutionErrorV4::ObligationRefused("dangling_edge".into()));
    }
    if !pairs.insert((source.to_owned(), target.to_owned())) {
        return Err(ExecutionErrorV4::ObligationRefused("duplicate_edge".into()));
    }
    Ok(())
}

fn graph_has_cycle_v4(nodes: &BTreeSet<String>, pairs: &BTreeSet<(String, String)>) -> bool {
    fn visit(
        node: &str,
        adjacency: &BTreeMap<String, BTreeSet<String>>,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
    ) -> bool {
        if !visiting.insert(node.to_owned()) {
            return true;
        }
        if visited.contains(node) {
            visiting.remove(node);
            return false;
        }
        if adjacency
            .get(node)
            .into_iter()
            .flatten()
            .any(|target| visit(target, adjacency, visiting, visited))
        {
            return true;
        }
        visiting.remove(node);
        visited.insert(node.to_owned());
        false
    }
    let mut adjacency = nodes
        .iter()
        .map(|node| (node.clone(), BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    for (source, target) in pairs {
        adjacency
            .get_mut(source)
            .expect("validated graph source exists")
            .insert(target.clone());
    }
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    nodes
        .iter()
        .any(|node| visit(node, &adjacency, &mut visiting, &mut visited))
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
    address_for_entity(plan, &binding.entity, logical)
}

fn address_for_entity(
    plan: &ServicePlanV4,
    entity: &str,
    logical: &Value,
) -> Result<String, ExecutionErrorV4> {
    let definition = plan
        .er
        .definitions
        .get(entity)
        .ok_or_else(|| ExecutionErrorV4::Binding(format!("missing definition {entity}")))?;
    let identity = definition
        .identity
        .as_ref()
        .ok_or_else(|| ExecutionErrorV4::Binding(format!("definition {entity} has no identity")))?;
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
    if !original_intents_match(&observation.intent, original) {
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
    let _latest = entity_core::replay(&history)
        .map_err(|error| ExecutionErrorV4::Integrity(error.to_string()))?;
    let target = history
        .iter()
        .position(|record| record.revision == commit.instance.revision)
        .ok_or_else(|| {
            ExecutionErrorV4::Integrity(
                "verified ER history omits the winning decision revision".to_owned(),
            )
        })?;
    if history[target] != commit.envelope.record {
        return Err(ExecutionErrorV4::Integrity(
            "verified ER history differs from the winning decision record".to_owned(),
        ));
    }
    let replayed = entity_core::replay(&history[..=target])
        .map_err(|error| ExecutionErrorV4::Integrity(error.to_string()))?;
    if replayed != commit.instance {
        return Err(ExecutionErrorV4::Integrity(
            "verified ER history prefix does not reproduce the winning instance".to_owned(),
        ));
    }
    Ok(MutationResultV4::Committed {
        decision: Box::new(commit.decision()),
        receipt: batch.receipt.clone(),
        replayed: true,
    })
}

fn original_intents_match(recorded: &OriginalIntentV4, attempted: &OriginalIntentV4) -> bool {
    if recorded.expected_version.is_some() {
        return recorded == attempted;
    }
    let mut compatible_attempt = attempted.clone();
    compatible_attempt.expected_version = None;
    recorded == &compatible_attempt
}

fn deliver_after_commit(
    resources: &mut dyn ResourcesV4,
    context: &VerifiedAuthContext,
    operation: &str,
    intent: &IntentPlanV4,
    decision: &Decision,
    receipt: &CommitReceipt,
    claim: &str,
) -> Result<(), ExecutionErrorV4> {
    resources
        .project(context, intent, decision, receipt)
        .and_then(|()| resources.effects(context, intent, decision, receipt))
        .map_err(|cause| ExecutionErrorV4::CommittedAftercare {
            receipt: Box::new(receipt.clone()),
            repair_token: repair_token(operation, claim),
            cause,
        })
}

fn repair_token(operation: &str, claim: &str) -> String {
    format!("sdk-er-repair-v1-{}-{claim}", hex::encode(operation))
}

fn repair_coordinates(token: &str) -> Option<(String, String)> {
    let encoded = token.strip_prefix("sdk-er-repair-v1-")?;
    let (operation, claim) = encoded.split_once('-')?;
    if operation.is_empty()
        || claim.len() != 64
        || !operation
            .bytes()
            .chain(claim.bytes())
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    let operation = String::from_utf8(hex::decode(operation).ok()?).ok()?;
    if operation.is_empty() {
        return None;
    }
    Some((operation, claim.to_owned()))
}

/// Returns the exact public operation encoded in a closed `/4` repair token.
#[must_use]
pub fn repair_operation(token: &str) -> Option<String> {
    repair_coordinates(token).map(|(operation, _)| operation)
}

fn digest_json(value: &impl Serialize) -> String {
    let bytes = serde_json::to_vec(value).expect("closed SDK observation data serializes");
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContentPolicyPlan, InputSource};

    fn identity_plan() -> ServicePlanV4 {
        let definition = |entity: &str, identity: &str| {
            serde_json::from_value(serde_json::json!({
                "entity": entity,
                "schema": {"fields": {identity: {"type": "string"}}},
                "lifecycle": {"initial": "Active", "states": ["Active", "Draft", "Issued"]},
                "semantics": "service/1",
                "identity": {"field": identity}
            }))
            .unwrap()
        };
        let definitions = BTreeMap::from([
            (
                "demo.Parent".to_owned(),
                definition("demo.Parent", "parent_id"),
            ),
            (
                "demo.Child".to_owned(),
                definition("demo.Child", "child_id"),
            ),
            (
                "demo.Revision".to_owned(),
                definition("demo.Revision", "revision_id"),
            ),
            ("demo.Node".to_owned(), definition("demo.Node", "node_id")),
            ("demo.Edge".to_owned(), definition("demo.Edge", "edge_id")),
        ]);
        ServicePlanV4 {
            format: REALIZATION_PLAN_FORMAT_V4.to_owned(),
            service: "demo".to_owned(),
            delivery: PlanDelivery::ComposedConnector,
            realm: crate::PlanRealmPolicy::Optional,
            ess_source_digest: String::new(),
            plan_digest: String::new(),
            er: EntityRuntimeBinding {
                component: String::new(),
                source_digest: String::new(),
                synthesis_digest: String::new(),
                target_revision: String::new(),
                definitions,
                bindings: service_runtime_ir::v4::BindingPlanDocument {
                    commands: BTreeMap::new(),
                    requirements: Vec::new(),
                    source_capabilities: Vec::new(),
                },
            },
            slots: BTreeMap::new(),
            operation_fields: BTreeMap::new(),
            intents: BTreeMap::new(),
            queries: BTreeMap::new(),
            content: BTreeMap::new(),
            views: BTreeMap::new(),
        }
    }

    fn identity_obligation(provider: &str, bindings: &[(&str, &str)]) -> ObligationUse {
        ObligationUse {
            provider: provider.to_owned(),
            bindings: bindings
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }

    fn identity_instance(
        entity: &str,
        id: &str,
        state: &str,
        fields: Value,
    ) -> entity_core::EntityInstance {
        let Value::Object(fields) = fields else {
            panic!("test instance fields must be an object");
        };
        entity_core::EntityInstance {
            entity: entity.to_owned(),
            version: 1,
            id: id.to_owned(),
            lifecycle_state: state.to_owned(),
            revision: 1,
            fields,
        }
    }

    fn nested_obligation(with_child: bool) -> ObligationUse {
        let mut obligation = identity_obligation(
            "sdk.aggregate.nested-entity/v1",
            &[
                ("parent", "demo.Parent"),
                ("child", "demo.Child"),
                ("parent_identity", "parent_id"),
            ],
        );
        if with_child {
            obligation
                .bindings
                .insert("child_identity".to_owned(), "child_id".to_owned());
        }
        obligation
    }

    fn nested_instances() -> Vec<entity_core::EntityInstance> {
        vec![
            identity_instance(
                "demo.Parent",
                "s:parent-uuid",
                "Active",
                serde_json::json!({"parent_id": "parent-uuid"}),
            ),
            identity_instance(
                "demo.Child",
                "s:child-uuid",
                "Active",
                serde_json::json!({"child_id": "child-uuid"}),
            ),
        ]
    }

    fn graph_obligation_v4(provider: &str) -> ObligationUse {
        identity_obligation(
            provider,
            &[
                ("nodes", "demo.Node"),
                ("edges", "demo.Edge"),
                ("partition", "draft_id"),
                ("node_partition", "draft_id"),
                ("edge_partition", "draft_id"),
                ("node_identity", "node_id"),
                ("edge_identity", "edge_id"),
                ("edge_source", "source_node_id"),
                ("edge_target", "target_node_id"),
                ("source", "source_node_id"),
                ("target", "target_node_id"),
                ("node", "node_id"),
            ],
        )
    }

    fn graph_instances(with_edge: bool) -> Vec<entity_core::EntityInstance> {
        let mut instances = vec![
            identity_instance(
                "demo.Node",
                "s:node-a",
                "Active",
                serde_json::json!({"node_id": "node-a", "draft_id": "draft-a"}),
            ),
            identity_instance(
                "demo.Node",
                "s:node-b",
                "Active",
                serde_json::json!({"node_id": "node-b", "draft_id": "draft-a"}),
            ),
        ];
        if with_edge {
            instances.push(identity_instance(
                "demo.Edge",
                "s:edge-ab",
                "Active",
                serde_json::json!({
                    "edge_id": "edge-ab", "draft_id": "draft-a",
                    "source_node_id": "node-a", "target_node_id": "node-b"
                }),
            ));
        }
        instances
    }

    #[test]
    fn nested_parent_identity_resolves_its_er_address() {
        let plan = identity_plan();
        let obligation = nested_obligation(false);
        let input = serde_json::json!({"parent_id": "parent-uuid"});
        assert!(
            enforce_aggregate_obligation_v4(
                &plan,
                &nested_instances(),
                &obligation,
                input.as_object().unwrap()
            )
            .is_ok(),
            "a present parent with a logical UUID identity must be found"
        );
        let missing = serde_json::json!({"parent_id": "another-parent"});
        assert!(matches!(
            enforce_aggregate_obligation_v4(
                &plan,
                &nested_instances(),
                &obligation,
                missing.as_object().unwrap()
            ),
            Err(ExecutionErrorV4::ObligationRefused(ref reason)) if reason == "parent_not_found"
        ));
    }

    #[test]
    fn nested_child_identity_resolves_its_er_address() {
        let plan = identity_plan();
        let obligation = nested_obligation(true);
        let input = serde_json::json!({"parent_id": "parent-uuid", "child_id": "child-uuid"});
        assert!(
            enforce_aggregate_obligation_v4(
                &plan,
                &nested_instances(),
                &obligation,
                input.as_object().unwrap()
            )
            .is_ok(),
            "both present logical identities must resolve to their own ER addresses"
        );
        let wrong_child = serde_json::json!({
            "parent_id": "parent-uuid", "child_id": "another-child"
        });
        assert!(matches!(
            enforce_aggregate_obligation_v4(
                &plan,
                &nested_instances(),
                &obligation,
                wrong_child.as_object().unwrap()
            ),
            Err(ExecutionErrorV4::ObligationRefused(ref reason)) if reason == "not_found"
        ));
    }

    #[test]
    fn graph_connect_resolves_public_logical_node_ids() {
        let plan = identity_plan();
        let obligation = graph_obligation_v4("sdk.graph.connect-dag/v1");
        let input = serde_json::json!({
            "draft_id": "draft-a", "source_node_id": "node-a", "target_node_id": "node-b"
        });
        assert!(
            validate_graph_obligation_v4(
                &plan,
                &graph_instances(false),
                &obligation,
                input.as_object().unwrap()
            )
            .is_ok(),
            "valid logical node IDs must connect existing ER nodes"
        );
        let missing = serde_json::json!({
            "draft_id": "draft-a", "source_node_id": "node-a", "target_node_id": "missing"
        });
        assert!(matches!(
            validate_graph_obligation_v4(
                &plan,
                &graph_instances(false),
                &obligation,
                missing.as_object().unwrap()
            ),
            Err(ExecutionErrorV4::ObligationRefused(ref reason)) if reason == "dangling_edge"
        ));
    }

    #[test]
    fn graph_node_unreferenced_resolves_public_logical_node_id() {
        let plan = identity_plan();
        let obligation = graph_obligation_v4("sdk.graph.node-unreferenced/v1");
        let input = serde_json::json!({"draft_id": "draft-a", "node_id": "node-a"});
        assert!(
            validate_graph_obligation_v4(
                &plan,
                &graph_instances(false),
                &obligation,
                input.as_object().unwrap()
            )
            .is_ok(),
            "an unreferenced logical node ID must find its ER node"
        );
        assert!(matches!(
            validate_graph_obligation_v4(
                &plan,
                &graph_instances(true),
                &obligation,
                input.as_object().unwrap()
            ),
            Err(ExecutionErrorV4::ObligationRefused(ref reason)) if reason == "node_referenced"
        ));
        let missing = serde_json::json!({"draft_id": "draft-a", "node_id": "missing"});
        assert!(matches!(
            validate_graph_obligation_v4(
                &plan,
                &graph_instances(false),
                &obligation,
                missing.as_object().unwrap()
            ),
            Err(ExecutionErrorV4::ObligationRefused(ref reason)) if reason == "node_not_found"
        ));
    }

    #[test]
    fn graph_publish_resolves_logical_edge_references() {
        let plan = identity_plan();
        let obligation = graph_obligation_v4("sdk.graph.publish-snapshot/v1");
        let input = serde_json::json!({"draft_id": "draft-a"});
        assert!(
            validate_graph_obligation_v4(
                &plan,
                &graph_instances(true),
                &obligation,
                input.as_object().unwrap()
            )
            .is_ok(),
            "valid logical edge references must resolve to existing ER nodes"
        );
        let mut invalid = graph_instances(true);
        invalid[2]
            .fields
            .insert("target_node_id".to_owned(), serde_json::json!("missing"));
        assert!(matches!(
            validate_graph_obligation_v4(&plan, &invalid, &obligation, input.as_object().unwrap()),
            Err(ExecutionErrorV4::ObligationRefused(ref reason)) if reason == "dangling_edge"
        ));
        let mut mismatched = graph_instances(false);
        mismatched[0]
            .fields
            .insert("node_id".to_owned(), serde_json::json!("another-node"));
        assert!(matches!(
            validate_graph_obligation_v4(
                &plan,
                &mismatched,
                &obligation,
                input.as_object().unwrap()
            ),
            Err(ExecutionErrorV4::Binding(_))
        ));
    }

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
    fn committed_content_reference_is_the_only_repair_coordinate() {
        let instance = entity_core::EntityInstance {
            entity: "demo.Document".to_owned(),
            version: 1,
            id: "document-a".to_owned(),
            lifecycle_state: "Created".to_owned(),
            revision: 1,
            fields: Map::new(),
        };
        let mut decision = Decision::legacy_import(instance, Vec::new());
        decision.record.command = DecisionCommand::Execute {
            operation: "demo.document.Create".to_owned(),
            arguments: serde_json::from_value(serde_json::json!({
                "input": {"content_ref": "content:sha256:recorded"},
                "bound": {}
            }))
            .unwrap(),
            fulfillments: BTreeMap::new(),
        };
        let original = OriginalIntentV4 {
            plan_digest: "plan".to_owned(),
            operation: "create_document".to_owned(),
            input: serde_json::json!({
                "content": {"media_type": "text/plain", "sha256": "a".repeat(64)}
            }),
            service: "demo".to_owned(),
            tenant: "tenant-a".to_owned(),
            realm: None,
            authority: "account-a".to_owned(),
            user: "user-a".to_owned(),
            executor: None,
            idempotency_key: "create-1".to_owned(),
            expected_version: Some(0),
            category: "demo.Document".to_owned(),
            selector: ClaimSelectorV4::GeneratedUuidV7,
        };
        assert_eq!(
            recorded_content_acceptances(&content_intent(false), &decision, &original).unwrap(),
            vec![("body".to_owned(), "content:sha256:recorded".to_owned())]
        );

        let mut absent = original.clone();
        absent.input = serde_json::json!({});
        assert!(
            recorded_content_acceptances(&content_intent(true), &decision, &absent)
                .unwrap()
                .is_empty()
        );
        decision.record.command = DecisionCommand::Execute {
            operation: "demo.document.Create".to_owned(),
            arguments: serde_json::from_value(serde_json::json!({"input": {}, "bound": {}}))
                .unwrap(),
            fulfillments: BTreeMap::new(),
        };
        assert!(matches!(
            recorded_content_acceptances(&content_intent(false), &decision, &original),
            Err(ExecutionErrorV4::Integrity(_))
        ));
    }

    #[test]
    fn expected_version_is_part_of_new_intent_identity_without_rewriting_old_meaning() {
        let original = OriginalIntentV4 {
            plan_digest: "plan".to_owned(),
            operation: "revise".to_owned(),
            input: serde_json::json!({"document_id": "document-a"}),
            service: "demo".to_owned(),
            tenant: "tenant-a".to_owned(),
            realm: None,
            authority: "account-a".to_owned(),
            user: "user-a".to_owned(),
            executor: None,
            idempotency_key: "revise-1".to_owned(),
            expected_version: Some(1),
            category: "demo.Document".to_owned(),
            selector: ClaimSelectorV4::CommandField {
                value: "document-a".to_owned(),
            },
        };
        assert!(original_intents_match(&original, &original));
        let mut changed = original.clone();
        changed.expected_version = Some(2);
        assert!(!original_intents_match(&original, &changed));

        let mut candidate_era = original.clone();
        candidate_era.expected_version = None;
        assert!(
            !serde_json::to_string(&candidate_era)
                .unwrap()
                .contains("expected_version")
        );
        assert!(original_intents_match(&candidate_era, &original));
        assert!(original_intents_match(&candidate_era, &changed));
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
