//! Closed persisted `/4` IR compiled through the official ESS to Entity Runtime lowerer.

use std::collections::{BTreeMap, BTreeSet};

use entity_core::EntityDefinition;
use ess_compiler::ir::{EssIr, ResolvedField, ResolvedTypeRef};
use ess_domain::name::Naming;
use ess_entity_runtime as lowerer;
use ess_service_contract::extract;
use ess_synth::{
    CapabilityKind, ObligationReason, PlannedCapability, RefusalReason, RefusalStage,
    SynthesisDisposition, SynthesisPlan,
};
use serde_json::Value;
use service_definition::v4::{OperationFieldPolicy, ServiceDefinitionV4, SlotValueSource};

use crate::{
    RuntimeCode, RuntimeDiagnostic, RuntimeDiagnostics, RuntimeDocument, ServiceRuntimeIr, compile,
};

/// Exact persisted format for an ER-delegated runtime IR.
pub const RUNTIME_IR_FORMAT_V4: &str = "service-runtime-ir/4";

/// Exact accepted ER semantic target named by the lowerer candidate.
pub const ACCEPTED_ENTITY_RUNTIME_REVISION: &str = "7fd93ef43d4a91c460c7305f9e3be90d7b0a4c11";

/// Exact `entity-eventlog` revision; it shares the accepted `entity-core` crate identity.
pub const ADAPTER_ENTITY_RUNTIME_REVISION: &str = "7fd93ef43d4a91c460c7305f9e3be90d7b0a4c11";

/// A compiler-minted `/4` runtime document.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct ServiceRuntimeIrV4(RuntimeDocumentV4);

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeDocumentV4 {
    format: String,
    definition: ServiceDefinitionV4,
    host: RuntimeDocument,
    er: EntityRuntimeBinding,
    views: BTreeMap<String, ProjectionSemanticsDocument>,
    #[serde(with = "ordered_map_as_pairs")]
    slots: BTreeMap<SlotCoordinateDocument, ResolvedSlotPolicy>,
    #[serde(with = "ordered_map_as_pairs")]
    operation_fields: BTreeMap<OperationFieldCoordinateDocument, ResolvedOperationFieldPolicy>,
}

/// Source-owned ESS view predicate retained for authoritative projection repair.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionSemanticsDocument {
    /// Source entity projected by the view.
    pub source: String,
    /// Exact validated ESS predicate; absent includes every instance.
    pub filter: Option<ess_primitives::predicate::Predicate>,
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

/// Exact complete lowerer result persisted beside retained SDK host plans.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityRuntimeBinding {
    /// Selected ESS component.
    pub component: String,
    /// Exact ESS source digest observed by the lowerer.
    pub source_digest: String,
    /// Exact selected synthesis digest observed by the lowerer.
    pub synthesis_digest: String,
    /// Exact accepted semantic target revision.
    pub target_revision: String,
    /// Adapter-side strict definitions after canonical-byte conversion.
    pub definitions: BTreeMap<String, EntityDefinition>,
    /// Complete typed binding-plan mirror.
    pub bindings: BindingPlanDocument,
}

/// Complete persisted lowerer binding plan.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingPlanDocument {
    /// Commands in qualified-name order.
    pub commands: BTreeMap<String, CommandBindingDocument>,
    /// Complete ordered lowerer obligations.
    pub requirements: Vec<BindingRequirementDocument>,
    /// Complete ordered selected synthesis capabilities.
    pub source_capabilities: Vec<CapabilityDocument>,
}

/// One command's target, entrypoint, subject binding, slots, and fulfillments.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandBindingDocument {
    /// Target entity.
    pub entity: String,
    /// Nonzero target definition version.
    pub version: u32,
    /// ER entrypoint.
    pub entrypoint: EntrypointDocument,
    /// Logical instance binding.
    pub instance: InstanceBindingDocument,
    /// Canonical lowerer slots.
    pub slots: BTreeMap<u32, BoundValueDocument>,
    /// Selected-outcome field requirements.
    #[serde(with = "ordered_map_as_pairs")]
    pub operation_fields: BTreeMap<OperationFieldDocument, OperationFieldFulfillmentDocument>,
}

/// Runtime entrypoint selected for one ESS command.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EntrypointDocument {
    /// Entity creation.
    Create,
    /// Existing-entity operation.
    Operation {
        /// Exact ER operation name.
        name: String,
    },
}

/// Logical subject binding for one command.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InstanceBindingDocument {
    /// Creation identity and its observed event field.
    Created {
        /// Logical identity source.
        logical_identity: IdentityValueDocument,
        /// Event coordinate observing the identity.
        observed_at: EventFieldDocument,
    },
    /// ER selects one accepting branch and derives its address from that branch's identity.
    SelectedOutcome {
        /// Complete ordered outcome-to-identity binding.
        identities: BTreeMap<String, SelectedCreationIdentityDocument>,
    },
    /// Existing logical identity from normalized input.
    Supplied {
        /// Command input field.
        input_field: String,
    },
}

/// One accepting creation outcome's exact identity source and observation coordinate.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectedCreationIdentityDocument {
    /// Logical identity source for this outcome.
    pub logical_identity: IdentityValueDocument,
    /// Exact event member that observes the identity.
    pub observed_at: EventFieldDocument,
}

/// One logical identity source.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum IdentityValueDocument {
    /// Normalized input field.
    InputField {
        /// Exact normalized input field.
        field: String,
    },
    /// Exact literal.
    Literal {
        /// Exact authored value.
        value: Value,
    },
    /// One canonical lowerer slot.
    Bound {
        /// Canonical lowerer slot index.
        slot: u32,
    },
}

/// One event-field coordinate.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventFieldDocument {
    /// Source outcome.
    pub outcome: String,
    /// Zero-based occurrence within the outcome.
    pub occurrence: usize,
    /// Exact event name.
    pub event: String,
    /// Event field.
    pub field: String,
}

/// One lowerer slot and its source-owned semantics.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundValueDocument {
    /// Semantic target.
    pub target: BoundTargetDocument,
    /// Exact resolved ESS type spelling.
    pub type_ref: String,
    /// Source semantics.
    pub source: BoundSourceDocument,
    /// Whether the source value may be absent.
    pub presence: PresenceDocument,
}

/// Lowerer bound target.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BoundTargetDocument {
    /// Outcome selected by external evidence.
    ExternalEvidence {
        /// Outcome whose evidence selects the branch.
        outcome: String,
    },
    /// Logical identity at one event field.
    LogicalIdentity {
        /// Event field observing the identity.
        at: EventFieldDocument,
    },
    /// Entity field.
    EntityField {
        /// Selected outcome.
        outcome: String,
        /// Entity field.
        field: String,
    },
    /// Event field occurrence.
    EventField {
        /// Selected outcome.
        outcome: String,
        /// Zero-based event occurrence.
        occurrence: usize,
        /// Exact event name.
        event: String,
        /// Event field.
        field: String,
    },
    /// Response field.
    ResponseField {
        /// Selected outcome.
        outcome: String,
        /// Response field.
        field: String,
    },
}

/// Lowerer bound source.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BoundSourceDocument {
    /// Host supplies creation identity.
    Identity,
    /// Declared command response field.
    ResponseField {
        /// Declared response field.
        field: String,
    },
    /// Independently generated value.
    Generated,
    /// Otherwise undetermined value.
    Undetermined,
    /// Declared conversion obligation.
    Conversion {
        /// Source type spelling.
        from: String,
        /// Authored conversion reason.
        because: String,
    },
    /// External branch-selection evidence.
    External {
        /// Authored external selection cause.
        cause: String,
    },
}

/// Source-owned slot presence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceDocument {
    /// Must be supplied.
    Required,
    /// May be absent without becoming JSON null.
    Optional,
}

/// One operation-field coordinate within one command.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationFieldDocument {
    /// Selected outcome.
    pub outcome: String,
    /// Entity field.
    pub field: String,
}

/// Exact admitted actions for an omitted entity field.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationFieldFulfillmentDocument {
    /// Exact resolved ESS type spelling.
    pub type_ref: String,
    /// Required fields admit set/preserve; optional fields also admit remove.
    pub optional: bool,
}

/// Definition coordinate used by an authored slot policy.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotCoordinateDocument {
    /// Exact command.
    pub command: String,
    /// Canonical lowerer slot.
    pub slot: u32,
}

/// Validated SDK policy retained beside one exact lowerer slot.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedSlotPolicy {
    /// Exact source-owned type spelling.
    pub type_ref: String,
    /// Exact source-owned presence.
    pub presence: PresenceDocument,
    /// Authored closed SDK source.
    pub source: SlotValueSource,
}

/// Full operation-field coordinate including command.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationFieldCoordinateDocument {
    /// Exact command.
    pub command: String,
    /// Selected outcome.
    pub outcome: String,
    /// Entity field.
    pub field: String,
}

/// Validated policy retained beside one exact fulfillment requirement.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedOperationFieldPolicy {
    /// Exact field type spelling.
    pub type_ref: String,
    /// Whether remove is admitted.
    pub optional: bool,
    /// Explicit authored policy.
    pub policy: OperationFieldPolicy,
}

/// Complete ordered lowerer obligation mirror.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BindingRequirementDocument {
    /// Creation identity and observation.
    IdentitySupplied {
        /// Exact command.
        command: String,
        /// Identity source.
        value: IdentityValueDocument,
        /// Event field observing it.
        observed_at: EventFieldDocument,
    },
    /// Declared response value.
    ResponseFieldSupplied {
        /// Exact command.
        command: String,
        /// Selected outcome.
        outcome: String,
        /// Response field.
        field: String,
        /// Canonical source slot.
        slot: u32,
    },
    /// Independently generated target value.
    GeneratedFieldSupplied {
        /// Exact command.
        command: String,
        /// Generated target.
        target: BoundTargetDocument,
        /// Canonical source slot.
        slot: u32,
    },
    /// Otherwise undetermined target value.
    UndeterminedFieldSupplied {
        /// Exact command.
        command: String,
        /// Undetermined target.
        target: BoundTargetDocument,
        /// Canonical source slot.
        slot: u32,
    },
    /// Declared conversion value.
    ConversionSupplied {
        /// Exact command.
        command: String,
        /// Conversion target.
        target: BoundTargetDocument,
        /// Canonical source slot.
        slot: u32,
        /// Source type spelling.
        from: String,
        /// Target type spelling.
        to: String,
        /// Authored conversion reason.
        because: String,
    },
    /// External branch evidence.
    ExternalEvidenceSupplied {
        /// Exact command.
        command: String,
        /// Selected outcome.
        outcome: String,
        /// Authored external cause.
        cause: String,
        /// Canonical evidence slot.
        slot: u32,
    },
    /// Selected-outcome operation-field action.
    OperationFieldPolicySupplied {
        /// Exact command.
        command: String,
        /// Selected operation field.
        target: OperationFieldDocument,
        /// Exact admitted action set.
        fulfillment: OperationFieldFulfillmentDocument,
    },
    /// Host-controlled timestamp spelling.
    TimestampSpelling {
        /// Primitive spelling family.
        primitive: String,
        /// Semantic use location.
        at: SemanticLocationDocument,
    },
    /// Host-controlled UUID spelling.
    UuidSpelling {
        /// Semantic use location.
        at: SemanticLocationDocument,
    },
    /// Host-controlled bytes spelling.
    BytesSpelling {
        /// Semantic use location.
        at: SemanticLocationDocument,
    },
    /// Host-controlled decimal spelling.
    DecimalSpelling {
        /// Semantic use location.
        at: SemanticLocationDocument,
    },
    /// Host-controlled binary64 spelling.
    Binary64Spelling {
        /// Semantic use location.
        at: SemanticLocationDocument,
    },
    /// Ordered scale required by ER comparison.
    ScaleDeclaration {
        /// Entity using the scale.
        entity: String,
        /// Scale name.
        name: String,
        /// Ordered scale values.
        values: Vec<String>,
    },
    /// Host-enforced relation existence.
    RelationExistence {
        /// Exact relation coordinate.
        relation: RelationDocument,
    },
    /// Host-enforced relation cardinality.
    RelationCardinality {
        /// Exact relation coordinate.
        relation: RelationDocument,
    },
    /// Host-enforced relation ownership.
    RelationOwnership {
        /// Exact relation coordinate.
        relation: RelationDocument,
    },
    /// Declared refusal payload.
    ErrorPayload {
        /// Exact command.
        command: String,
        /// Refusing outcome.
        outcome: String,
        /// Declared error type.
        error: String,
        /// Complete error fields.
        fields: Vec<ResolvedFieldDocument>,
    },
    /// Exact subject revision expectation.
    RevisionExpectation {
        /// Exact command.
        command: String,
        /// Exact entity.
        entity: String,
    },
}

/// Semantic location of a spelling obligation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SemanticLocationDocument {
    /// Entity identity.
    EntityIdentity {
        /// Exact entity.
        entity: String,
        /// Identity field.
        field: String,
    },
    /// Entity field.
    EntityField {
        /// Exact entity.
        entity: String,
        /// Entity field.
        field: String,
    },
    /// Command input.
    CommandInput {
        /// Exact command.
        command: String,
        /// Input field.
        field: String,
    },
    /// Command response.
    CommandResponse {
        /// Exact command.
        command: String,
        /// Response field.
        field: String,
    },
    /// Event field.
    EventField {
        /// Exact event.
        event: String,
        /// Event field.
        field: String,
    },
    /// Lowerer binding target.
    BindingTarget {
        /// Exact command.
        command: String,
        /// Exact lowerer target.
        target: BoundTargetDocument,
    },
}

/// Relation coordinate retained without reinterpretation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationDocument {
    /// Source entity.
    pub source: String,
    /// Relation name.
    pub relation: String,
    /// Target entity.
    pub target: String,
    /// Source field carrying the relation.
    pub via: String,
}

/// Resolved field retained in an error-payload obligation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedFieldDocument {
    /// Field name.
    pub name: String,
    /// Exact resolved type spelling.
    pub type_ref: String,
    /// Exact authored naming metadata.
    pub naming: Naming,
}

/// Complete selected synthesis capability.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityDocument {
    /// Closed capability kind.
    pub kind: CapabilityKindDocument,
    /// Exact source construct.
    pub source: String,
    /// Exact synthesis disposition.
    pub disposition: DispositionDocument,
}

/// Closed mirror of ESS synthesis capability kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKindDocument {
    /// Domain type projection.
    DomainType,
    /// Entity lifecycle semantics.
    EntityLifecycle,
    /// Command input and outcome contract.
    CommandContract,
    /// Command decision behavior.
    CommandBehavior,
    /// Event type projection.
    EventType,
    /// Error type projection.
    ErrorType,
    /// View row type.
    ViewType,
    /// View query behavior.
    ViewQuery,
    /// Declared type conversion.
    Conversion,
    /// Actor grant semantics.
    ActorGrants,
    /// Binding transformation.
    BindingTransformation,
    /// Binding delivery.
    BindingDelivery,
    /// Binding escalation.
    BindingEscalation,
    /// Component port.
    ComponentPort,
    /// Component transport.
    ComponentTransport,
    /// Deployable workload.
    Workload,
}

/// Closed mirror of synthesis dispositions.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DispositionDocument {
    /// Capability is generated directly.
    Generated,
    /// Capability is assigned to a host obligation.
    Obligation {
        /// Closed obligation reason.
        reason: ObligationReasonDocument,
        /// Exact host contract.
        contract: String,
    },
    /// Capability is refused by synthesis.
    Refused {
        /// Closed refusal reason.
        reason: RefusalReasonDocument,
        /// Stage that refused it.
        stage: RefusalStageDocument,
        /// Exact refusal detail.
        detail: String,
    },
}

/// Closed mirror of synthesis obligation reasons.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ObligationReasonDocument {
    /// External evidence or integration is required.
    External {
        /// Exact cause.
        cause: String,
    },
    /// No deterministic generation algorithm is selected.
    UnspecifiedAlgorithm,
    /// Projection maintenance remains host-owned.
    ProjectionMaintenance,
}

/// Closed mirror of synthesis refusal reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalReasonDocument {
    /// Caller identity is unavailable at synthesis.
    NeedsCallerIdentity,
    /// Acceptor-owned value remains undetermined.
    AcceptorUndetermined,
    /// Topology must be selected by deployment.
    TopologyDeferred,
    /// Periodic execution requires a host.
    PeriodicHostRequired,
}

/// Closed mirror of the refusing synthesis stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalStageDocument {
    /// Structural planning stage.
    Planning,
    /// Target-specific generation stage.
    Target,
}

impl ServiceRuntimeIrV4 {
    /// Reads canonical strict JSON, recompiles against the exact ESS inputs, and compares all data.
    pub fn from_json_bound(
        text: &str,
        ess: &EssIr,
        synthesis: &SynthesisPlan,
    ) -> Result<Self, RuntimeDiagnostics> {
        let persisted: RuntimeDocumentV4 = serde_json::from_str(text).map_err(|error| {
            RuntimeDiagnostics::one(
                RuntimeCode::InvalidPersistedIr,
                "document",
                error.to_string(),
            )
        })?;
        let persisted_ir = Self(persisted.clone());
        if persisted_ir.to_canonical_json() != text {
            return Err(RuntimeDiagnostics::one(
                RuntimeCode::InvalidPersistedIr,
                "document",
                "persisted service-runtime-ir/4 is not canonical JSON",
            ));
        }
        let rebuilt = compile_v4(ess, synthesis, &persisted.definition)?;
        if persisted == rebuilt.0 {
            Ok(persisted_ir)
        } else {
            Err(RuntimeDiagnostics::one(
                RuntimeCode::PersistedIrMismatch,
                "document",
                "persisted service-runtime-ir/4 does not equal a fresh complete compilation",
            ))
        }
    }

    /// Canonical pretty JSON with stable map order and one trailing newline.
    pub fn to_canonical_json(&self) -> String {
        let mut output = serde_json::to_string_pretty(self)
            .unwrap_or_else(|error| panic!("validated service-runtime-ir/4 serializes: {error}"));
        output.push('\n');
        output
    }

    /// Exact author-owned `/4` definition.
    pub const fn definition(&self) -> &ServiceDefinitionV4 {
        &self.0.definition
    }

    /// Complete validated Entity Runtime definitions and lowerer binding plan.
    pub const fn entity_runtime(&self) -> &EntityRuntimeBinding {
        &self.0.er
    }

    /// Complete validated ordinary-slot policies.
    pub const fn slots(&self) -> &BTreeMap<SlotCoordinateDocument, ResolvedSlotPolicy> {
        &self.0.slots
    }

    /// Complete validated selected operation-field policies.
    pub const fn operation_fields(
        &self,
    ) -> &BTreeMap<OperationFieldCoordinateDocument, ResolvedOperationFieldPolicy> {
        &self.0.operation_fields
    }

    /// Complete source-owned predicates for every declared projection view.
    pub const fn views(&self) -> &BTreeMap<String, ProjectionSemanticsDocument> {
        &self.0.views
    }

    /// Reconstructs the validated retained `/3` host plan used only for SDK-owned concerns.
    pub fn retained_host_ir(&self) -> ServiceRuntimeIr {
        ServiceRuntimeIr(self.0.host.clone())
    }
}

/// Compiles one strict `/4` definition through the official component extractor and ER lowerer.
#[allow(clippy::too_many_lines)]
pub fn compile_v4(
    ess: &EssIr,
    synthesis: &SynthesisPlan,
    definition: &ServiceDefinitionV4,
) -> Result<ServiceRuntimeIrV4, RuntimeDiagnostics> {
    definition.validate().map_err(|errors| {
        RuntimeDiagnostics(
            errors
                .diagnostics()
                .iter()
                .map(|diagnostic| {
                    RuntimeDiagnostic::new(
                        RuntimeCode::InvalidDefinition,
                        format!("definition.{}", diagnostic.path),
                        format!("{:?}: {}", diagnostic.code, diagnostic.message),
                    )
                })
                .collect(),
        )
    })?;
    let component =
        ess_domain::component::ComponentName::new(&definition.component).map_err(|error| {
            RuntimeDiagnostics::one(
                RuntimeCode::InvalidDefinition,
                "definition.component",
                error.to_string(),
            )
        })?;
    let service = extract(ess, synthesis, &component).map_err(|errors| {
        RuntimeDiagnostics::one(
            RuntimeCode::MissingSemantic,
            "definition.component",
            format!("{errors:?}"),
        )
    })?;
    let lowered = lowerer::lower(
        &service,
        &lowerer::LoweringOptions {
            definition_versions: definition.entity_versions.clone(),
            scales: definition.scales.clone(),
        },
    )
    .map_err(|errors| {
        RuntimeDiagnostics(
            errors
                .as_slice()
                .iter()
                .map(|diagnostic| {
                    RuntimeDiagnostic::new(
                        RuntimeCode::UnsupportedAnnotation,
                        format!("lowerer.{}", diagnostic.path),
                        format!("{:?}: {}", diagnostic.code, diagnostic.message),
                    )
                })
                .collect(),
        )
    })?;
    if lowered.target_revision() != ACCEPTED_ENTITY_RUNTIME_REVISION
        || lowered.target_revision() != lowerer::ENTITY_RUNTIME_REVISION
    {
        return Err(RuntimeDiagnostics::one(
            RuntimeCode::PersistedIrMismatch,
            "er.target_revision",
            format!(
                "lowerer targets {:?}, SDK accepts {:?}",
                lowered.target_revision(),
                ACCEPTED_ENTITY_RUNTIME_REVISION
            ),
        ));
    }

    let definitions = lowered
        .definitions()
        .iter()
        .map(|(name, definition)| (name.to_string(), definition.as_definition().clone()))
        .collect::<BTreeMap<_, _>>();
    let mut registry = entity_core::Registry::new();
    for definition in definitions.values() {
        registry.register(definition.clone()).map_err(|errors| {
            RuntimeDiagnostics::one(
                RuntimeCode::UnsupportedAnnotation,
                "er.definitions",
                errors.to_string(),
            )
        })?;
    }
    registry.validate_all().map_err(|errors| {
        RuntimeDiagnostics::one(
            RuntimeCode::UnsupportedAnnotation,
            "er.definitions",
            errors.to_string(),
        )
    })?;

    let bindings = binding_plan(lowered.bindings());
    let (slots, operation_fields) = resolve_policies(ess, definition, lowered.bindings())?;
    let views = definition
        .projections
        .iter()
        .map(|projection| {
            let name = projection.view.to_string();
            let view = ess
                .views()
                .values()
                .find(|view| view.name.to_string() == name)
                .expect("validated projection view remains in compiler IR");
            (
                name,
                ProjectionSemanticsDocument {
                    source: view.source.to_string(),
                    filter: view.filter.clone(),
                },
            )
        })
        .collect();
    let ServiceRuntimeIr(host) = compile(ess, synthesis, &definition.retained_host_definition())?;
    let document = RuntimeDocumentV4 {
        format: RUNTIME_IR_FORMAT_V4.to_owned(),
        definition: definition.clone(),
        host,
        er: EntityRuntimeBinding {
            component: lowered.component().as_str().to_owned(),
            source_digest: lowered.source_digest().to_owned(),
            synthesis_digest: lowered.synthesis_digest().to_owned(),
            target_revision: lowered.target_revision().to_owned(),
            definitions,
            bindings,
        },
        views,
        slots,
        operation_fields,
    };
    Ok(ServiceRuntimeIrV4(document))
}

type ResolvedPolicies = (
    BTreeMap<SlotCoordinateDocument, ResolvedSlotPolicy>,
    BTreeMap<OperationFieldCoordinateDocument, ResolvedOperationFieldPolicy>,
);

#[allow(clippy::too_many_lines)]
fn resolve_policies(
    ess: &EssIr,
    definition: &ServiceDefinitionV4,
    bindings: &lowerer::BindingPlan,
) -> Result<ResolvedPolicies, RuntimeDiagnostics> {
    let authored_slots = definition
        .slots
        .iter()
        .map(|binding| {
            (
                SlotCoordinateDocument {
                    command: binding.coordinate.command.to_string(),
                    slot: binding.coordinate.slot,
                },
                &binding.source,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let authored_fields = definition
        .operation_fields
        .iter()
        .map(|binding| {
            (
                OperationFieldCoordinateDocument {
                    command: binding.coordinate.command.to_string(),
                    outcome: binding.coordinate.outcome.as_str().to_owned(),
                    field: binding.coordinate.field.clone(),
                },
                &binding.policy,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut diagnostics = Vec::new();
    let mut slots = BTreeMap::new();
    let mut fields = BTreeMap::new();

    for (command, binding) in bindings.commands() {
        for (slot, value) in &binding.slots {
            let coordinate = SlotCoordinateDocument {
                command: command.to_string(),
                slot: slot.index(),
            };
            let Some(source) = authored_slots.get(&coordinate) else {
                diagnostics.push(RuntimeDiagnostic::new(
                    RuntimeCode::UnsupportedAnnotation,
                    format!("definition.slots.{}.{}", command, slot.index()),
                    "required lowerer slot has no explicit SDK source",
                ));
                continue;
            };
            validate_slot_policy(ess, command, value, source, &mut diagnostics);
            slots.insert(
                coordinate,
                ResolvedSlotPolicy {
                    type_ref: value.type_ref.to_string(),
                    presence: presence(value.presence),
                    source: (*source).clone(),
                },
            );
        }
        for (coordinate, fulfillment) in &binding.operation_fields {
            let document = OperationFieldCoordinateDocument {
                command: command.to_string(),
                outcome: coordinate.outcome.as_str().to_owned(),
                field: coordinate.field.clone(),
            };
            let Some(policy) = authored_fields.get(&document) else {
                diagnostics.push(RuntimeDiagnostic::new(
                    RuntimeCode::UnsupportedAnnotation,
                    format!(
                        "definition.operation_fields.{}.{}.{}",
                        command, coordinate.outcome, coordinate.field
                    ),
                    "required operation field has no explicit SDK policy",
                ));
                continue;
            };
            let optional = matches!(
                fulfillment.actions,
                lowerer::OperationFieldActions::Optional
            );
            if matches!(policy, OperationFieldPolicy::Remove) && !optional {
                diagnostics.push(RuntimeDiagnostic::new(
                    RuntimeCode::UnsupportedAnnotation,
                    format!(
                        "definition.operation_fields.{}.{}.{}",
                        command, coordinate.outcome, coordinate.field
                    ),
                    "Remove is admitted only for an optional entity field",
                ));
            }
            if let OperationFieldPolicy::CommandField { field } = policy {
                let command_definition = ess
                    .commands()
                    .values()
                    .find(|candidate| &candidate.name == command);
                let source = command_definition.and_then(|candidate| {
                    candidate
                        .input
                        .iter()
                        .find(|candidate| candidate.name == *field)
                });
                // Set supplies a present value even when the entity field may be absent.
                // Presence is controlled separately by Preserve and Remove.
                let set_value_type = match &fulfillment.type_ref {
                    ResolvedTypeRef::Optional { of } if optional => of.as_ref(),
                    other => other,
                };
                match source {
                    None => diagnostics.push(RuntimeDiagnostic::new(
                        RuntimeCode::InvalidSemanticReference,
                        format!(
                            "definition.operation_fields.{}.{}.{}",
                            command, coordinate.outcome, coordinate.field
                        ),
                        format!("command has no input field {field:?}"),
                    )),
                    Some(source)
                        if source.type_ref != fulfillment.type_ref
                            && &source.type_ref != set_value_type =>
                    {
                        diagnostics.push(RuntimeDiagnostic::new(
                            RuntimeCode::InvalidSemanticReference,
                            format!(
                                "definition.operation_fields.{}.{}.{}",
                                command, coordinate.outcome, coordinate.field
                            ),
                            format!(
                                "command field type {} is incompatible with operation field type {}",
                                source.type_ref, fulfillment.type_ref
                            ),
                        ));
                    }
                    Some(_) => {}
                }
            }
            fields.insert(
                document,
                ResolvedOperationFieldPolicy {
                    type_ref: fulfillment.type_ref.to_string(),
                    optional,
                    policy: (*policy).clone(),
                },
            );
        }
    }
    for coordinate in authored_slots.keys() {
        if !slots.contains_key(coordinate) {
            diagnostics.push(RuntimeDiagnostic::new(
                RuntimeCode::UnsupportedAnnotation,
                format!(
                    "definition.slots.{}.{}",
                    coordinate.command, coordinate.slot
                ),
                "slot policy does not name a lowerer slot",
            ));
        }
    }
    for coordinate in authored_fields.keys() {
        if !fields.contains_key(coordinate) {
            diagnostics.push(RuntimeDiagnostic::new(
                RuntimeCode::UnsupportedAnnotation,
                format!(
                    "definition.operation_fields.{}.{}.{}",
                    coordinate.command, coordinate.outcome, coordinate.field
                ),
                "operation-field policy does not name a lowerer requirement",
            ));
        }
    }
    if diagnostics.is_empty() {
        Ok((slots, fields))
    } else {
        Err(RuntimeDiagnostics(diagnostics))
    }
}

fn validate_slot_policy(
    ess: &EssIr,
    command_name: &ess_domain::name::QualifiedName,
    value: &lowerer::BoundValue,
    source: &SlotValueSource,
    diagnostics: &mut Vec<RuntimeDiagnostic>,
) {
    let path = format!(
        "definition.slots.{}.{}",
        command_name,
        slot_target(&value.target)
    );
    match source {
        SlotValueSource::Absent if matches!(value.presence, lowerer::BoundPresence::Required) => {
            diagnostics.push(RuntimeDiagnostic::new(
                RuntimeCode::UnsupportedAnnotation,
                path,
                "Absent is admitted only for an optional lowerer slot",
            ));
        }
        SlotValueSource::CommandField { field } => {
            let command = ess
                .commands()
                .values()
                .find(|command| &command.name == command_name);
            let found = command.and_then(|command| {
                command
                    .input
                    .iter()
                    .find(|candidate| candidate.name == *field)
            });
            match found {
                None => diagnostics.push(RuntimeDiagnostic::new(
                    RuntimeCode::InvalidSemanticReference,
                    path,
                    format!("command has no input field {field:?}"),
                )),
                Some(field) if field.type_ref != value.type_ref => {
                    diagnostics.push(RuntimeDiagnostic::new(
                        RuntimeCode::InvalidSemanticReference,
                        path,
                        format!(
                            "command field type {} does not equal slot type {}",
                            field.type_ref, value.type_ref
                        ),
                    ));
                }
                Some(_) => {}
            }
        }
        SlotValueSource::Literal { value: literal }
            if !literal_matches_type(ess, &value.type_ref, literal, &mut BTreeSet::new()) =>
        {
            diagnostics.push(RuntimeDiagnostic::new(
                RuntimeCode::UnsupportedAnnotation,
                path,
                format!("literal does not have required type {}", value.type_ref),
            ));
        }
        SlotValueSource::TrustedClock if value.type_ref.to_string() != "Timestamp" => {
            diagnostics.push(RuntimeDiagnostic::new(
                RuntimeCode::UnsupportedAnnotation,
                path,
                format!("trusted clock cannot supply {}", value.type_ref),
            ));
        }
        SlotValueSource::GeneratedUuidV7
            if !type_resolves_to_primitive(
                ess,
                &value.type_ref,
                ess_domain::types::Primitive::Uuid,
                &mut BTreeSet::new(),
            ) =>
        {
            diagnostics.push(RuntimeDiagnostic::new(
                RuntimeCode::UnsupportedAnnotation,
                path,
                format!("UUIDv7 source cannot supply {}", value.type_ref),
            ));
        }
        SlotValueSource::Absent
        | SlotValueSource::VerifiedContext { .. }
        | SlotValueSource::TrustedClock
        | SlotValueSource::GeneratedUuidV7
        | SlotValueSource::Obligation { .. }
        | SlotValueSource::Literal { .. } => {}
    }
}

fn type_resolves_to_primitive(
    ess: &EssIr,
    type_ref: &ess_compiler::ir::ResolvedTypeRef,
    expected: ess_domain::types::Primitive,
    seen: &mut BTreeSet<String>,
) -> bool {
    use ess_compiler::ir::{ResolvedBody, ResolvedTypeRef};
    match type_ref {
        ResolvedTypeRef::Primitive { name } => *name == expected,
        ResolvedTypeRef::Declared { name } => {
            let key = name.to_string();
            if !seen.insert(key.clone()) {
                return false;
            }
            let result = match &ess.named_type(name).body {
                ResolvedBody::Newtype { of, .. } => {
                    type_resolves_to_primitive(ess, of, expected, seen)
                }
                _ => false,
            };
            seen.remove(&key);
            result
        }
        _ => false,
    }
}

fn literal_matches_type(
    ess: &EssIr,
    type_ref: &ess_compiler::ir::ResolvedTypeRef,
    value: &Value,
    seen: &mut BTreeSet<String>,
) -> bool {
    use ess_compiler::ir::{ResolvedBody, ResolvedTypeRef};
    use ess_domain::types::Primitive;
    match type_ref {
        ResolvedTypeRef::Primitive { name } => match name {
            Primitive::String
            | Primitive::Timestamp
            | Primitive::Duration
            | Primitive::Uuid
            | Primitive::Bytes => value.is_string(),
            Primitive::Integer => value.as_i64().is_some() || value.as_u64().is_some(),
            Primitive::Decimal | Primitive::Binary64 => value.is_number(),
            Primitive::Boolean => value.is_boolean(),
        },
        ResolvedTypeRef::Optional { of } => {
            value.is_null() || literal_matches_type(ess, of, value, seen)
        }
        ResolvedTypeRef::List { of } => value.as_array().is_some_and(|items| {
            items
                .iter()
                .all(|item| literal_matches_type(ess, of, item, seen))
        }),
        ResolvedTypeRef::Map { value: of, .. } => value.as_object().is_some_and(|members| {
            members
                .values()
                .all(|item| literal_matches_type(ess, of, item, seen))
        }),
        ResolvedTypeRef::Declared { name } => {
            let key = name.to_string();
            if !seen.insert(key.clone()) {
                return false;
            }
            let declared = ess.named_type(name);
            let result = match &declared.body {
                ResolvedBody::Newtype { of, .. } => literal_matches_type(ess, of, value, seen),
                ResolvedBody::Enum { variants } => value
                    .as_str()
                    .is_some_and(|value| variants.iter().any(|variant| variant == value)),
                ResolvedBody::Struct { fields, .. } => value.as_object().is_some_and(|members| {
                    members.len() == fields.len()
                        && fields.iter().all(|field| {
                            members.get(&field.name).is_some_and(|item| {
                                literal_matches_type(ess, &field.type_ref, item, seen)
                            })
                        })
                }),
                ResolvedBody::Union { tag, variants } => value.as_object().is_some_and(|members| {
                    members
                        .get(tag)
                        .and_then(Value::as_str)
                        .is_some_and(|variant| {
                            variants
                                .get(variant)
                                .is_some_and(|shape| literal_matches_type(ess, shape, value, seen))
                        })
                }),
            };
            seen.remove(&key);
            result
        }
    }
}

fn binding_plan(plan: &lowerer::BindingPlan) -> BindingPlanDocument {
    BindingPlanDocument {
        commands: plan
            .commands()
            .iter()
            .map(|(name, binding)| (name.to_string(), command_binding(binding)))
            .collect(),
        requirements: plan.requirements().iter().map(requirement).collect(),
        source_capabilities: plan.source_capabilities().iter().map(capability).collect(),
    }
}

fn command_binding(binding: &lowerer::CommandBinding) -> CommandBindingDocument {
    CommandBindingDocument {
        entity: binding.target.entity.to_string(),
        version: binding.target.version.get(),
        entrypoint: match &binding.entrypoint {
            lowerer::RuntimeEntrypoint::Create => EntrypointDocument::Create,
            lowerer::RuntimeEntrypoint::Operation { name } => EntrypointDocument::Operation {
                name: name.to_string(),
            },
        },
        instance: instance_binding(&binding.instance),
        slots: binding
            .slots
            .iter()
            .map(|(slot, value)| {
                (
                    slot.index(),
                    BoundValueDocument {
                        target: bound_target(&value.target),
                        type_ref: value.type_ref.to_string(),
                        source: bound_source(&value.source),
                        presence: presence(value.presence),
                    },
                )
            })
            .collect(),
        operation_fields: binding
            .operation_fields
            .iter()
            .map(|(coordinate, fulfillment)| {
                (
                    operation_field(coordinate),
                    operation_fulfillment(fulfillment),
                )
            })
            .collect(),
    }
}

fn instance_binding(binding: &lowerer::InstanceBinding) -> InstanceBindingDocument {
    match binding {
        lowerer::InstanceBinding::Created {
            logical_identity,
            observed_at,
        } => InstanceBindingDocument::Created {
            logical_identity: identity_value(logical_identity),
            observed_at: event_field(observed_at),
        },
        lowerer::InstanceBinding::SelectedOutcome { identities } => {
            InstanceBindingDocument::SelectedOutcome {
                identities: identities
                    .iter()
                    .map(|(outcome, identity)| {
                        (
                            outcome.as_str().to_owned(),
                            SelectedCreationIdentityDocument {
                                logical_identity: identity_value(&identity.logical_identity),
                                observed_at: event_field(&identity.observed_at),
                            },
                        )
                    })
                    .collect(),
            }
        }
        lowerer::InstanceBinding::Supplied { input_field } => InstanceBindingDocument::Supplied {
            input_field: input_field.clone(),
        },
    }
}

fn identity_value(value: &lowerer::IdentityValue) -> IdentityValueDocument {
    match value {
        lowerer::IdentityValue::InputField { field } => IdentityValueDocument::InputField {
            field: field.clone(),
        },
        lowerer::IdentityValue::Literal { value } => IdentityValueDocument::Literal {
            value: value.clone(),
        },
        lowerer::IdentityValue::Bound { slot } => {
            IdentityValueDocument::Bound { slot: slot.index() }
        }
    }
}

fn event_field(value: &lowerer::EventFieldCoordinate) -> EventFieldDocument {
    EventFieldDocument {
        outcome: value.outcome.as_str().to_owned(),
        occurrence: value.occurrence,
        event: value.event.to_string(),
        field: value.field.clone(),
    }
}

fn presence(value: lowerer::BoundPresence) -> PresenceDocument {
    match value {
        lowerer::BoundPresence::Required => PresenceDocument::Required,
        lowerer::BoundPresence::Optional => PresenceDocument::Optional,
    }
}

fn bound_target(value: &lowerer::BoundTarget) -> BoundTargetDocument {
    match value {
        lowerer::BoundTarget::ExternalEvidence { outcome } => {
            BoundTargetDocument::ExternalEvidence {
                outcome: outcome.as_str().to_owned(),
            }
        }
        lowerer::BoundTarget::LogicalIdentity { at } => BoundTargetDocument::LogicalIdentity {
            at: event_field(at),
        },
        lowerer::BoundTarget::EntityField { outcome, field } => BoundTargetDocument::EntityField {
            outcome: outcome.as_str().to_owned(),
            field: field.clone(),
        },
        lowerer::BoundTarget::EventField {
            outcome,
            occurrence,
            event,
            field,
        } => BoundTargetDocument::EventField {
            outcome: outcome.as_str().to_owned(),
            occurrence: *occurrence,
            event: event.to_string(),
            field: field.clone(),
        },
        lowerer::BoundTarget::ResponseField { outcome, field } => {
            BoundTargetDocument::ResponseField {
                outcome: outcome.as_str().to_owned(),
                field: field.clone(),
            }
        }
    }
}

fn bound_source(value: &lowerer::BoundSource) -> BoundSourceDocument {
    match value {
        lowerer::BoundSource::Identity => BoundSourceDocument::Identity,
        lowerer::BoundSource::ResponseField { field } => BoundSourceDocument::ResponseField {
            field: field.clone(),
        },
        lowerer::BoundSource::Generated => BoundSourceDocument::Generated,
        lowerer::BoundSource::Undetermined => BoundSourceDocument::Undetermined,
        lowerer::BoundSource::Conversion { from, because } => BoundSourceDocument::Conversion {
            from: from.to_string(),
            because: because.clone(),
        },
        lowerer::BoundSource::External { cause } => BoundSourceDocument::External {
            cause: cause.clone(),
        },
    }
}

fn slot_target(value: &lowerer::BoundTarget) -> String {
    serde_json::to_string(&bound_target(value))
        .unwrap_or_else(|error| panic!("closed bound target serializes: {error}"))
}

fn operation_field(value: &lowerer::OperationFieldCoordinate) -> OperationFieldDocument {
    OperationFieldDocument {
        outcome: value.outcome.as_str().to_owned(),
        field: value.field.clone(),
    }
}

fn operation_fulfillment(
    value: &lowerer::OperationFieldFulfillment,
) -> OperationFieldFulfillmentDocument {
    OperationFieldFulfillmentDocument {
        type_ref: value.type_ref.to_string(),
        optional: matches!(value.actions, lowerer::OperationFieldActions::Optional),
    }
}

#[allow(clippy::too_many_lines)]
fn requirement(value: &lowerer::BindingRequirement) -> BindingRequirementDocument {
    use lowerer::BindingRequirement as R;
    match value {
        R::IdentitySupplied {
            command,
            value,
            observed_at,
        } => BindingRequirementDocument::IdentitySupplied {
            command: command.to_string(),
            value: identity_value(value),
            observed_at: event_field(observed_at),
        },
        R::ResponseFieldSupplied {
            command,
            outcome,
            field,
            slot,
        } => BindingRequirementDocument::ResponseFieldSupplied {
            command: command.to_string(),
            outcome: outcome.as_str().to_owned(),
            field: field.clone(),
            slot: slot.index(),
        },
        R::GeneratedFieldSupplied {
            command,
            target,
            slot,
        } => BindingRequirementDocument::GeneratedFieldSupplied {
            command: command.to_string(),
            target: bound_target(target),
            slot: slot.index(),
        },
        R::UndeterminedFieldSupplied {
            command,
            target,
            slot,
        } => BindingRequirementDocument::UndeterminedFieldSupplied {
            command: command.to_string(),
            target: bound_target(target),
            slot: slot.index(),
        },
        R::ConversionSupplied {
            command,
            target,
            slot,
            from,
            to,
            because,
        } => BindingRequirementDocument::ConversionSupplied {
            command: command.to_string(),
            target: bound_target(target),
            slot: slot.index(),
            from: from.to_string(),
            to: to.to_string(),
            because: because.clone(),
        },
        R::ExternalEvidenceSupplied {
            command,
            outcome,
            cause,
            slot,
        } => BindingRequirementDocument::ExternalEvidenceSupplied {
            command: command.to_string(),
            outcome: outcome.as_str().to_owned(),
            cause: cause.clone(),
            slot: slot.index(),
        },
        R::OperationFieldPolicySupplied {
            command,
            target,
            fulfillment,
        } => BindingRequirementDocument::OperationFieldPolicySupplied {
            command: command.to_string(),
            target: operation_field(target),
            fulfillment: operation_fulfillment(fulfillment),
        },
        R::TimestampSpelling { primitive, at } => BindingRequirementDocument::TimestampSpelling {
            primitive: primitive.to_string(),
            at: semantic_location(at),
        },
        R::UuidSpelling { at } => BindingRequirementDocument::UuidSpelling {
            at: semantic_location(at),
        },
        R::BytesSpelling { at } => BindingRequirementDocument::BytesSpelling {
            at: semantic_location(at),
        },
        R::DecimalSpelling { at } => BindingRequirementDocument::DecimalSpelling {
            at: semantic_location(at),
        },
        R::Binary64Spelling { at } => BindingRequirementDocument::Binary64Spelling {
            at: semantic_location(at),
        },
        R::ScaleDeclaration {
            entity,
            name,
            values,
        } => BindingRequirementDocument::ScaleDeclaration {
            entity: entity.to_string(),
            name: name.clone(),
            values: values.clone(),
        },
        R::RelationExistence { relation } => BindingRequirementDocument::RelationExistence {
            relation: relation_document(relation),
        },
        R::RelationCardinality { relation } => BindingRequirementDocument::RelationCardinality {
            relation: relation_document(relation),
        },
        R::RelationOwnership { relation } => BindingRequirementDocument::RelationOwnership {
            relation: relation_document(relation),
        },
        R::ErrorPayload {
            command,
            outcome,
            error,
            fields,
        } => BindingRequirementDocument::ErrorPayload {
            command: command.to_string(),
            outcome: outcome.as_str().to_owned(),
            error: error.to_string(),
            fields: fields.iter().map(resolved_field).collect(),
        },
        R::RevisionExpectation { command, entity } => {
            BindingRequirementDocument::RevisionExpectation {
                command: command.to_string(),
                entity: entity.to_string(),
            }
        }
    }
}

fn semantic_location(value: &lowerer::SemanticLocation) -> SemanticLocationDocument {
    use lowerer::SemanticLocation as L;
    match value {
        L::EntityIdentity { entity, field } => SemanticLocationDocument::EntityIdentity {
            entity: entity.to_string(),
            field: field.clone(),
        },
        L::EntityField { entity, field } => SemanticLocationDocument::EntityField {
            entity: entity.to_string(),
            field: field.clone(),
        },
        L::CommandInput { command, field } => SemanticLocationDocument::CommandInput {
            command: command.to_string(),
            field: field.clone(),
        },
        L::CommandResponse { command, field } => SemanticLocationDocument::CommandResponse {
            command: command.to_string(),
            field: field.clone(),
        },
        L::EventField { event, field } => SemanticLocationDocument::EventField {
            event: event.to_string(),
            field: field.clone(),
        },
        L::BindingTarget { command, target } => SemanticLocationDocument::BindingTarget {
            command: command.to_string(),
            target: bound_target(target),
        },
    }
}

fn relation_document(value: &lowerer::RelationCoordinate) -> RelationDocument {
    RelationDocument {
        source: value.source.to_string(),
        relation: value.relation.clone(),
        target: value.target.to_string(),
        via: value.via.clone(),
    }
}

fn resolved_field(value: &ResolvedField) -> ResolvedFieldDocument {
    ResolvedFieldDocument {
        name: value.name.clone(),
        type_ref: value.type_ref.to_string(),
        naming: value.naming.clone(),
    }
}

fn capability(value: &PlannedCapability) -> CapabilityDocument {
    CapabilityDocument {
        kind: match value.capability.kind {
            CapabilityKind::DomainType => CapabilityKindDocument::DomainType,
            CapabilityKind::EntityLifecycle => CapabilityKindDocument::EntityLifecycle,
            CapabilityKind::CommandContract => CapabilityKindDocument::CommandContract,
            CapabilityKind::CommandBehavior => CapabilityKindDocument::CommandBehavior,
            CapabilityKind::EventType => CapabilityKindDocument::EventType,
            CapabilityKind::ErrorType => CapabilityKindDocument::ErrorType,
            CapabilityKind::ViewType => CapabilityKindDocument::ViewType,
            CapabilityKind::ViewQuery => CapabilityKindDocument::ViewQuery,
            CapabilityKind::Conversion => CapabilityKindDocument::Conversion,
            CapabilityKind::ActorGrants => CapabilityKindDocument::ActorGrants,
            CapabilityKind::BindingTransformation => CapabilityKindDocument::BindingTransformation,
            CapabilityKind::BindingDelivery => CapabilityKindDocument::BindingDelivery,
            CapabilityKind::BindingEscalation => CapabilityKindDocument::BindingEscalation,
            CapabilityKind::ComponentPort => CapabilityKindDocument::ComponentPort,
            CapabilityKind::ComponentTransport => CapabilityKindDocument::ComponentTransport,
            CapabilityKind::Workload => CapabilityKindDocument::Workload,
        },
        source: value.capability.source.clone(),
        disposition: disposition(&value.disposition),
    }
}

fn disposition(value: &SynthesisDisposition) -> DispositionDocument {
    match value {
        SynthesisDisposition::Generated => DispositionDocument::Generated,
        SynthesisDisposition::Obligation(obligation) => DispositionDocument::Obligation {
            reason: match &obligation.reason {
                ObligationReason::External { cause } => ObligationReasonDocument::External {
                    cause: cause.clone(),
                },
                ObligationReason::UnspecifiedAlgorithm => {
                    ObligationReasonDocument::UnspecifiedAlgorithm
                }
                ObligationReason::ProjectionMaintenance => {
                    ObligationReasonDocument::ProjectionMaintenance
                }
            },
            contract: obligation.contract.clone(),
        },
        SynthesisDisposition::Refused(refusal) => DispositionDocument::Refused {
            reason: match refusal.reason {
                RefusalReason::NeedsCallerIdentity => RefusalReasonDocument::NeedsCallerIdentity,
                RefusalReason::AcceptorUndetermined => RefusalReasonDocument::AcceptorUndetermined,
                RefusalReason::TopologyDeferred => RefusalReasonDocument::TopologyDeferred,
                RefusalReason::PeriodicHostRequired => RefusalReasonDocument::PeriodicHostRequired,
            },
            stage: match refusal.stage {
                RefusalStage::Planning => RefusalStageDocument::Planning,
                RefusalStage::Target => RefusalStageDocument::Target,
            },
            detail: refusal.detail.clone(),
        },
    }
}
