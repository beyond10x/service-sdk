//! Closed, persisted runtime realization IR derived from ESS and `service-definition/1`.
//!
//! The only constructor is [`compile`]. A persisted document can be read only through
//! [`ServiceRuntimeIr::from_json_bound`], which recompiles it against the supplied compiler-minted
//! [`EssIr`] and ESS [`SynthesisPlan`]. This makes both source and synthesis digests enforceable
//! bindings instead of decorative provenance.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use ess_compiler::ir::{EssIr, ResolvedBody, ResolvedCommand, ResolvedTypeRef, ResolvedView};
use ess_synth::{CapabilityKind, SynthesisDisposition, SynthesisPlan};
use service_definition::{
    ContentDefinition, DefinitionId, EventBindingSource, ExpectedVersionSource, GuardRead,
    IdempotencySource, IntentDefinition, ProjectionDefinition, ProjectionDelivery, QueryDefinition,
    QueryDelivery, ServiceDefinition, StreamIdSource,
};
use sha2::{Digest as _, Sha256};

/// The only persisted runtime-IR format understood by this crate.
pub const RUNTIME_IR_FORMAT: &str = "service-runtime-ir/1";

/// Security-sensitive admission order at every generated operation boundary.
pub const ADMISSION_PIPELINE: [AdmissionStage; 4] = [
    AdmissionStage::VerifyCredential,
    AdmissionStage::EnforceRealmPolicy,
    AdmissionStage::DecodeOperation,
    AdmissionStage::BindAndDispatch,
];

/// Required mutation order. There is no path from decoded input directly to append.
pub const INTENT_PIPELINE: [IntentStage; 13] = [
    IntentStage::ResolveIdempotency,
    IntentStage::ResolveStream,
    IntentStage::Load,
    IntentStage::Fold,
    IntentStage::Guards,
    IntentStage::StageContent,
    IntentStage::ConstructCommand,
    IntentStage::Validate,
    IntentStage::Decide,
    IntentStage::GuardedAppend,
    IntentStage::Reduce,
    IntentStage::Project,
    IntentStage::AcceptContent,
];

/// Required query order.
pub const QUERY_PIPELINE: [QueryStage; 3] = [
    QueryStage::Guards,
    QueryStage::BindSelectors,
    QueryStage::ReadProjection,
];

/// A compiler-minted runtime document.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(transparent)]
pub struct ServiceRuntimeIr(RuntimeDocument);

impl ServiceRuntimeIr {
    /// Reads strict JSON, then recompiles and byte-compares it against exact ESS and synthesis input.
    pub fn from_json_bound(
        text: &str,
        ess: &EssIr,
        synthesis: &SynthesisPlan,
    ) -> Result<Self, RuntimeDiagnostics> {
        let persisted: RuntimeDocument = serde_json::from_str(text).map_err(|error| {
            RuntimeDiagnostics::one(
                RuntimeCode::InvalidPersistedIr,
                "document",
                error.to_string(),
            )
        })?;
        let rebuilt = compile(ess, synthesis, &persisted.definition)?;
        if persisted == rebuilt.0 {
            Ok(Self(persisted))
        } else {
            Err(RuntimeDiagnostics::one(
                RuntimeCode::PersistedIrMismatch,
                "document",
                "persisted runtime IR does not equal a fresh compilation against the supplied ESS model and synthesis plan",
            ))
        }
    }

    /// Canonical pretty JSON with sorted maps and a trailing newline.
    pub fn to_canonical_json(&self) -> String {
        let mut output = serde_json::to_string_pretty(self)
            .unwrap_or_else(|error| panic!("validated service runtime IR serializes: {error}"));
        output.push('\n');
        output
    }

    /// The validated, losslessly preserved author definition.
    pub const fn definition(&self) -> &ServiceDefinition {
        &self.0.definition
    }

    /// Exact digest minted by [`EssIr::source_digest`].
    pub fn ess_source_digest(&self) -> &str {
        &self.0.ess.source_digest
    }

    /// Exact digest of [`SynthesisPlan::to_canonical_json`].
    pub fn synthesis_digest(&self) -> &str {
        &self.0.synthesis.digest
    }

    /// Resolved intent plans in stable identity order.
    pub const fn intents(&self) -> &BTreeMap<DefinitionId, ResolvedIntent> {
        &self.0.intents
    }

    /// Resolved projection plans in stable identity order.
    pub const fn projections(&self) -> &BTreeMap<DefinitionId, ResolvedProjection> {
        &self.0.projections
    }

    /// Resolved query plans in stable identity order.
    pub const fn queries(&self) -> &BTreeMap<DefinitionId, ResolvedQuery> {
        &self.0.queries
    }

    /// Resolved content policies in stable identity order.
    pub const fn content(&self) -> &BTreeMap<DefinitionId, ResolvedContent> {
        &self.0.content
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeDocument {
    format: String,
    ess: EssBinding,
    synthesis: SynthesisBinding,
    admission: Vec<AdmissionStage>,
    definition: ServiceDefinition,
    intents: BTreeMap<DefinitionId, ResolvedIntent>,
    projections: BTreeMap<DefinitionId, ResolvedProjection>,
    queries: BTreeMap<DefinitionId, ResolvedQuery>,
    content: BTreeMap<DefinitionId, ResolvedContent>,
}

/// Exact compiler model binding carried by a runtime document.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EssBinding {
    /// ESS system name.
    pub system: String,
    /// ESS specification version.
    pub version: String,
    /// Full canonical semantic SHA-256 from the compiler.
    pub source_digest: String,
}

/// Exact ESS synthesis-plan binding carried by a runtime document.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SynthesisBinding {
    /// Synthesis scope profile.
    pub profile: String,
    /// Full SHA-256 of the plan's canonical JSON.
    pub digest: String,
    /// Number of capabilities in the exact bound plan.
    pub capabilities: usize,
}

/// Authentication and decoding stage.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionStage {
    /// Verify credentials into trusted authority facts.
    VerifyCredential,
    /// Apply required, optional, or forbidden realm policy to trusted facts.
    EnforceRealmPolicy,
    /// Decode caller-controlled fields only after authentication succeeds.
    DecodeOperation,
    /// Bind validated values and dispatch the operation.
    BindAndDispatch,
}

/// Ordered mutation stage.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum IntentStage {
    /// Resolve the idempotency identity.
    ResolveIdempotency,
    /// Resolve the aggregate stream identity.
    ResolveStream,
    /// Load the tenant-scoped event stream.
    Load,
    /// Fold events into current aggregate state.
    Fold,
    /// Evaluate declared guards.
    Guards,
    /// Stage plaintext content externally.
    StageContent,
    /// Construct an ESS command value.
    ConstructCommand,
    /// Apply ESS and handwritten validation.
    Validate,
    /// Decide one declared ESS outcome.
    Decide,
    /// Conditionally and idempotently append events.
    GuardedAppend,
    /// Reduce newly accepted events.
    Reduce,
    /// Apply or schedule declared projections.
    Project,
    /// Accept content references only after their event append commits.
    AcceptContent,
}

/// Ordered query stage.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum QueryStage {
    /// Evaluate authorization and business guards.
    Guards,
    /// Bind validated selectors to resolved view fields.
    BindSelectors,
    /// Read the projection under its declared delivery guarantee.
    ReadProjection,
}

/// ESS synthesis disposition required by a runtime semantic reference.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "disposition", rename_all = "snake_case", deny_unknown_fields)]
pub enum RequiredDisposition {
    /// Fully generated by ESS synthesis.
    Generated,
    /// Handwritten implementation is required against this exact ESS contract.
    Obligation {
        /// Stable human-readable ESS reason.
        reason: String,
        /// Exact ESS obligation contract.
        contract: String,
    },
}

/// Resolved runtime intent.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedIntent {
    /// ESS command name.
    pub command: String,
    /// Command input fields and their resolved type spellings.
    pub input_fields: BTreeMap<String, String>,
    /// Every event an outcome may emit, in ESS order.
    pub emitted_events: Vec<String>,
    /// ESS command-contract disposition.
    pub contract: RequiredDisposition,
    /// ESS command-behavior disposition.
    pub behavior: RequiredDisposition,
    /// Closed execution order.
    pub pipeline: Vec<IntentStage>,
}

/// Resolved runtime projection.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedProjection {
    /// ESS view name.
    pub view: String,
    /// Reused row shape when the ESS view declares one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
    /// Checked view fields and their resolved type spellings.
    pub fields: BTreeMap<String, String>,
    /// ESS consistency spelling.
    pub consistency: String,
    /// ESS view-type disposition.
    pub view_type: RequiredDisposition,
    /// ESS view-query disposition.
    pub view_query: RequiredDisposition,
}

/// Resolved runtime query.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedQuery {
    /// ESS view name.
    pub view: String,
    /// Projection identity used by this query.
    pub projection: DefinitionId,
    /// Checked view fields and their resolved type spellings.
    pub fields: BTreeMap<String, String>,
    /// ESS view-type disposition.
    pub view_type: RequiredDisposition,
    /// ESS view-query disposition.
    pub view_query: RequiredDisposition,
    /// Closed execution order.
    pub pipeline: Vec<QueryStage>,
}

/// Resolved external content policy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedContent {
    /// Exact ESS newtype name.
    pub reference_type: String,
    /// Resolved underlying type spelling.
    pub representation: String,
}

/// Resolves one validated service definition against exact ESS semantics and synthesis decisions.
pub fn compile(
    ess: &EssIr,
    synthesis: &SynthesisPlan,
    definition: &ServiceDefinition,
) -> Result<ServiceRuntimeIr, RuntimeDiagnostics> {
    if let Err(errors) = definition.validate() {
        return Err(RuntimeDiagnostics::one(
            RuntimeCode::InvalidDefinition,
            "definition",
            errors.to_string(),
        ));
    }

    let mut diagnostics = Vec::new();
    let source_digest = ess.source_digest();
    if synthesis.provenance.source_digest != source_digest {
        diagnostics.push(RuntimeDiagnostic::new(
            RuntimeCode::SynthesisModelMismatch,
            "synthesis.provenance.source_digest",
            format!(
                "plan binds {}, but the supplied EssIr binds {source_digest}",
                synthesis.provenance.source_digest
            ),
        ));
    }
    if synthesis.provenance.system != ess.system().to_string()
        || synthesis.provenance.specification_version != ess.version().to_string()
    {
        diagnostics.push(RuntimeDiagnostic::new(
            RuntimeCode::SynthesisModelMismatch,
            "synthesis.provenance",
            "plan system/version do not match the supplied EssIr",
        ));
    }

    let mut content = BTreeMap::new();
    for (index, annotation) in definition.content.iter().enumerate() {
        if let Some(resolved) = resolve_content(ess, index, annotation, &mut diagnostics) {
            content.insert(annotation.name.clone(), resolved);
        }
    }

    let mut projections = BTreeMap::new();
    for (index, annotation) in definition.projections.iter().enumerate() {
        if let Some(resolved) =
            resolve_projection(ess, synthesis, index, annotation, &mut diagnostics)
        {
            projections.insert(annotation.name.clone(), resolved);
        }
    }

    let content_definitions: BTreeMap<_, _> = definition
        .content
        .iter()
        .map(|item| (&item.name, item))
        .collect();
    let mut intents = BTreeMap::new();
    for (index, annotation) in definition.intents.iter().enumerate() {
        if let Some(resolved) = resolve_intent(
            ess,
            synthesis,
            index,
            annotation,
            &content_definitions,
            &mut diagnostics,
        ) {
            intents.insert(annotation.name.clone(), resolved);
        }
    }

    let mut queries = BTreeMap::new();
    for (index, annotation) in definition.queries.iter().enumerate() {
        if let Some(resolved) = resolve_query(ess, synthesis, index, annotation, &mut diagnostics) {
            queries.insert(annotation.name.clone(), resolved);
        }
    }

    if !diagnostics.is_empty() {
        return Err(RuntimeDiagnostics(diagnostics));
    }

    Ok(ServiceRuntimeIr(RuntimeDocument {
        format: RUNTIME_IR_FORMAT.to_owned(),
        ess: EssBinding {
            system: ess.system().to_string(),
            version: ess.version().to_string(),
            source_digest,
        },
        synthesis: SynthesisBinding {
            profile: synthesis.scope.profile.clone(),
            digest: sha256(synthesis.to_canonical_json().as_bytes()),
            capabilities: synthesis.capabilities.len(),
        },
        admission: ADMISSION_PIPELINE.to_vec(),
        definition: definition.clone(),
        intents,
        projections,
        queries,
        content,
    }))
}

fn resolve_content(
    ess: &EssIr,
    index: usize,
    annotation: &ContentDefinition,
    diagnostics: &mut Vec<RuntimeDiagnostic>,
) -> Option<ResolvedContent> {
    let path = format!("definition.content[{index}].reference_type");
    let Some(resolved) = ess.types().get(annotation.reference_type.name()) else {
        missing_semantic(
            diagnostics,
            &path,
            "type",
            &annotation.reference_type.to_string(),
        );
        return None;
    };
    let ResolvedBody::Newtype { of, .. } = &resolved.body else {
        diagnostics.push(RuntimeDiagnostic::new(
            RuntimeCode::UnsupportedAnnotation,
            path,
            format!(
                "content reference {} must be an ESS newtype, not a structured or enum value",
                annotation.reference_type
            ),
        ));
        return None;
    };
    Some(ResolvedContent {
        reference_type: annotation.reference_type.to_string(),
        representation: of.to_string(),
    })
}

fn resolve_projection(
    ess: &EssIr,
    synthesis: &SynthesisPlan,
    index: usize,
    annotation: &ProjectionDefinition,
    diagnostics: &mut Vec<RuntimeDiagnostic>,
) -> Option<ResolvedProjection> {
    let path = format!("definition.projections[{index}]");
    let view = resolve_view(ess, &annotation.view.to_string(), &path, diagnostics)?;
    let consistency = view_consistency(view, &path, diagnostics)?;
    let expected_delivery = match consistency.as_str() {
        "read_your_writes" => ProjectionDelivery::InlineTransactional,
        "eventual" => ProjectionDelivery::CatchUp,
        _ => return None,
    };
    if annotation.delivery != expected_delivery {
        diagnostics.push(RuntimeDiagnostic::new(
            RuntimeCode::IncompatibleDelivery,
            format!("{path}.delivery"),
            format!(
                "{} requires {:?}, but the annotation declares {:?}",
                annotation.view, expected_delivery, annotation.delivery
            ),
        ));
    }
    let view_type = required_disposition(
        synthesis,
        CapabilityKind::ViewType,
        &annotation.view.to_string(),
        &format!("{path}.view"),
        diagnostics,
    );
    let view_query = required_disposition(
        synthesis,
        CapabilityKind::ViewQuery,
        &annotation.view.to_string(),
        &format!("{path}.view"),
        diagnostics,
    );
    Some(ResolvedProjection {
        view: annotation.view.to_string(),
        shape: view.shape.as_ref().map(ToString::to_string),
        fields: resolved_view_fields(view),
        consistency,
        view_type: view_type?,
        view_query: view_query?,
    })
}

fn resolve_intent(
    ess: &EssIr,
    synthesis: &SynthesisPlan,
    index: usize,
    annotation: &IntentDefinition,
    contents: &BTreeMap<&DefinitionId, &ContentDefinition>,
    diagnostics: &mut Vec<RuntimeDiagnostic>,
) -> Option<ResolvedIntent> {
    let path = format!("definition.intents[{index}]");
    let Some(command) = ess.commands().get(annotation.command.name()) else {
        missing_semantic(
            diagnostics,
            &format!("{path}.command"),
            "command",
            &annotation.command.to_string(),
        );
        return None;
    };

    validate_intent_inputs(command, annotation, &path, diagnostics);
    validate_intent_content(command, annotation, contents, &path, diagnostics);
    validate_event_bindings(ess, command, annotation, &path, diagnostics);

    let contract = required_disposition(
        synthesis,
        CapabilityKind::CommandContract,
        &annotation.command.to_string(),
        &format!("{path}.command"),
        diagnostics,
    );
    let behavior = required_disposition(
        synthesis,
        CapabilityKind::CommandBehavior,
        &annotation.command.to_string(),
        &format!("{path}.command"),
        diagnostics,
    );
    Some(ResolvedIntent {
        command: annotation.command.to_string(),
        input_fields: resolved_command_fields(command),
        emitted_events: command.emits().map(ToString::to_string).collect(),
        contract: contract?,
        behavior: behavior?,
        pipeline: INTENT_PIPELINE.to_vec(),
    })
}

fn validate_intent_inputs(
    command: &ResolvedCommand,
    annotation: &IntentDefinition,
    path: &str,
    diagnostics: &mut Vec<RuntimeDiagnostic>,
) {
    for field in &command.input {
        if authority_coordinate(&field.name) {
            diagnostics.push(RuntimeDiagnostic::new(
                RuntimeCode::AuthorityCoordinate,
                format!("{path}.command.input.{}", field.name),
                format!(
                    "ESS command {} exposes authentication-derived field {:?}; tenant, realm, and user must come only from verified context",
                    annotation.command, field.name
                ),
            ));
        }
    }
    if let StreamIdSource::CommandField { field } = &annotation.stream_id {
        require_command_field(
            command,
            field,
            &format!("{path}.stream_id.field"),
            diagnostics,
        );
    }
    for (guard_index, guard) in annotation.guards.iter().enumerate() {
        for (read_index, read) in guard.reads.iter().enumerate() {
            match read {
                GuardRead::CommandField { field } => {
                    require_command_field(
                        command,
                        field,
                        &format!("{path}.guards[{guard_index}].reads[{read_index}].field"),
                        diagnostics,
                    );
                }
                GuardRead::ViewField { field } => diagnostics.push(RuntimeDiagnostic::new(
                    RuntimeCode::UnsupportedAnnotation,
                    format!("{path}.guards[{guard_index}].reads[{read_index}]"),
                    format!("intent guard cannot read view field {field:?}"),
                )),
                GuardRead::Context { .. } => {}
            }
        }
    }
    let command_fields: BTreeSet<_> = command
        .input
        .iter()
        .map(|field| field.name.as_str())
        .collect();
    for (field_path, field) in intent_envelope_fields(annotation, path) {
        if command_fields.contains(field.as_str()) {
            diagnostics.push(RuntimeDiagnostic::new(
                RuntimeCode::OperationFieldCollision,
                field_path,
                format!("operation-envelope field {field:?} collides with an ESS command field"),
            ));
        }
    }
}

fn validate_intent_content(
    command: &ResolvedCommand,
    annotation: &IntentDefinition,
    contents: &BTreeMap<&DefinitionId, &ContentDefinition>,
    path: &str,
    diagnostics: &mut Vec<RuntimeDiagnostic>,
) {
    for (binding_index, binding) in annotation.content.iter().enumerate() {
        let binding_path = format!("{path}.content[{binding_index}]");
        let Some(policy) = contents.get(&binding.content) else {
            continue;
        };
        let Some(field) = require_command_field(
            command,
            &binding.command_reference_field,
            &format!("{binding_path}.command_reference_field"),
            diagnostics,
        ) else {
            continue;
        };
        match &field.type_ref {
            ResolvedTypeRef::Declared { name } if name.name() == policy.reference_type.name() => {}
            other => diagnostics.push(RuntimeDiagnostic::new(
                RuntimeCode::ContentTypeMismatch,
                format!("{binding_path}.command_reference_field"),
                format!(
                    "{} is {other}, but content policy {} requires exact non-optional type {}",
                    binding.command_reference_field, binding.content, policy.reference_type
                ),
            )),
        }
    }
}

fn validate_event_bindings(
    ess: &EssIr,
    command: &ResolvedCommand,
    annotation: &IntentDefinition,
    path: &str,
    diagnostics: &mut Vec<RuntimeDiagnostic>,
) {
    for (binding_index, binding) in annotation.event_bindings.iter().enumerate() {
        let binding_path = format!("{path}.event_bindings[{binding_index}]");
        let emitted = command
            .emits()
            .any(|handle| handle.name() == binding.event.name());
        if !emitted {
            diagnostics.push(RuntimeDiagnostic::new(
                RuntimeCode::InvalidSemanticReference,
                format!("{binding_path}.event"),
                format!(
                    "{} is not emitted by any outcome of {}",
                    binding.event, annotation.command
                ),
            ));
            continue;
        }
        let Some(event) = ess.events().get(binding.event.name()) else {
            missing_semantic(
                diagnostics,
                &format!("{binding_path}.event"),
                "event",
                &binding.event.to_string(),
            );
            continue;
        };
        if event.field(&binding.field).is_none() {
            diagnostics.push(RuntimeDiagnostic::new(
                RuntimeCode::InvalidSemanticReference,
                format!("{binding_path}.field"),
                format!("event {} has no field {:?}", binding.event, binding.field),
            ));
        }
        if let EventBindingSource::CommandField { field } = &binding.source {
            require_command_field(
                command,
                field,
                &format!("{binding_path}.source.field"),
                diagnostics,
            );
        }
    }
}

fn resolve_query(
    ess: &EssIr,
    synthesis: &SynthesisPlan,
    index: usize,
    annotation: &QueryDefinition,
    diagnostics: &mut Vec<RuntimeDiagnostic>,
) -> Option<ResolvedQuery> {
    let path = format!("definition.queries[{index}]");
    let view = resolve_view(ess, &annotation.view.to_string(), &path, diagnostics)?;
    let consistency = view_consistency(view, &path, diagnostics)?;
    let expected_delivery = match consistency.as_str() {
        "read_your_writes" => QueryDelivery::ReadYourWrites,
        "eventual" => QueryDelivery::Eventual,
        _ => return None,
    };
    if annotation.delivery != expected_delivery {
        diagnostics.push(RuntimeDiagnostic::new(
            RuntimeCode::IncompatibleDelivery,
            format!("{path}.delivery"),
            format!(
                "{} requires {:?}, but the query declares {:?}",
                annotation.view, expected_delivery, annotation.delivery
            ),
        ));
    }
    for (selector_index, selector) in annotation.selectors.iter().enumerate() {
        require_view_field(
            view,
            &selector.view_field,
            &format!("{path}.selectors[{selector_index}].view_field"),
            diagnostics,
        );
    }
    for (sort_index, sort) in annotation.sort.iter().enumerate() {
        require_view_field(
            view,
            &sort.view_field,
            &format!("{path}.sort[{sort_index}].view_field"),
            diagnostics,
        );
    }
    for (guard_index, guard) in annotation.guards.iter().enumerate() {
        for (read_index, read) in guard.reads.iter().enumerate() {
            match read {
                GuardRead::ViewField { field } => {
                    require_view_field(
                        view,
                        field,
                        &format!("{path}.guards[{guard_index}].reads[{read_index}].field"),
                        diagnostics,
                    );
                }
                GuardRead::CommandField { field } => diagnostics.push(RuntimeDiagnostic::new(
                    RuntimeCode::UnsupportedAnnotation,
                    format!("{path}.guards[{guard_index}].reads[{read_index}]"),
                    format!("query guard cannot read command field {field:?}"),
                )),
                GuardRead::Context { .. } => {}
            }
        }
    }
    let view_type = required_disposition(
        synthesis,
        CapabilityKind::ViewType,
        &annotation.view.to_string(),
        &format!("{path}.view"),
        diagnostics,
    );
    let view_query = required_disposition(
        synthesis,
        CapabilityKind::ViewQuery,
        &annotation.view.to_string(),
        &format!("{path}.view"),
        diagnostics,
    );
    Some(ResolvedQuery {
        view: annotation.view.to_string(),
        projection: annotation.projection.clone(),
        fields: resolved_view_fields(view),
        view_type: view_type?,
        view_query: view_query?,
        pipeline: QUERY_PIPELINE.to_vec(),
    })
}

fn resolve_view<'a>(
    ess: &'a EssIr,
    name: &str,
    path: &str,
    diagnostics: &mut Vec<RuntimeDiagnostic>,
) -> Option<&'a ResolvedView> {
    let resolved = ess
        .views()
        .values()
        .find(|view| view.name.to_string() == name);
    if resolved.is_none() {
        missing_semantic(diagnostics, &format!("{path}.view"), "view", name);
    }
    resolved
}

fn view_consistency(
    view: &ResolvedView,
    path: &str,
    diagnostics: &mut Vec<RuntimeDiagnostic>,
) -> Option<String> {
    let value = serde_json::to_value(view.consistency)
        .unwrap_or_else(|error| panic!("ESS consistency serializes: {error}"));
    let Some(consistency) = value.as_str() else {
        diagnostics.push(RuntimeDiagnostic::new(
            RuntimeCode::UnsupportedAnnotation,
            format!("{path}.view.consistency"),
            "ESS consistency did not serialize as a closed string",
        ));
        return None;
    };
    if !matches!(consistency, "read_your_writes" | "eventual") {
        diagnostics.push(RuntimeDiagnostic::new(
            RuntimeCode::UnsupportedAnnotation,
            format!("{path}.view.consistency"),
            format!("unsupported ESS consistency {consistency:?}"),
        ));
        return None;
    }
    Some(consistency.to_owned())
}

fn require_command_field<'a>(
    command: &'a ResolvedCommand,
    field: &str,
    path: &str,
    diagnostics: &mut Vec<RuntimeDiagnostic>,
) -> Option<&'a ess_compiler::ir::ResolvedField> {
    let resolved = command.input_field(field);
    if resolved.is_none() {
        diagnostics.push(RuntimeDiagnostic::new(
            RuntimeCode::InvalidSemanticReference,
            path,
            format!("command {} has no input field {field:?}", command.name),
        ));
    }
    resolved
}

fn require_view_field<'a>(
    view: &'a ResolvedView,
    field: &str,
    path: &str,
    diagnostics: &mut Vec<RuntimeDiagnostic>,
) -> Option<&'a ess_compiler::ir::ResolvedField> {
    let resolved = view.field(field);
    if resolved.is_none() {
        diagnostics.push(RuntimeDiagnostic::new(
            RuntimeCode::InvalidSemanticReference,
            path,
            format!("view {} has no field {field:?}", view.name),
        ));
    }
    resolved
}

fn required_disposition(
    synthesis: &SynthesisPlan,
    kind: CapabilityKind,
    source: &str,
    path: &str,
    diagnostics: &mut Vec<RuntimeDiagnostic>,
) -> Option<RequiredDisposition> {
    let matches: Vec<_> = synthesis
        .capabilities
        .iter()
        .filter(|planned| planned.capability.kind == kind && planned.capability.source == source)
        .collect();
    let [planned] = matches.as_slice() else {
        diagnostics.push(RuntimeDiagnostic::new(
            RuntimeCode::MissingSynthesisCapability,
            path,
            format!(
                "expected exactly one {kind:?} synthesis capability for {source}, found {}",
                matches.len()
            ),
        ));
        return None;
    };
    match &planned.disposition {
        SynthesisDisposition::Generated => Some(RequiredDisposition::Generated),
        SynthesisDisposition::Obligation(obligation) => Some(RequiredDisposition::Obligation {
            reason: obligation.reason.describes(),
            contract: obligation.contract.clone(),
        }),
        SynthesisDisposition::Refused(refusal) => {
            diagnostics.push(RuntimeDiagnostic::new(
                RuntimeCode::RefusedSynthesisCapability,
                path,
                format!(
                    "ESS synthesis refused {kind:?} for {source}: {}",
                    refusal.detail
                ),
            ));
            None
        }
    }
}

fn resolved_command_fields(command: &ResolvedCommand) -> BTreeMap<String, String> {
    command
        .input
        .iter()
        .map(|field| (field.name.clone(), field.type_ref.to_string()))
        .collect()
}

fn resolved_view_fields(view: &ResolvedView) -> BTreeMap<String, String> {
    view.fields
        .iter()
        .map(|field| (field.name.clone(), field.type_ref.to_string()))
        .collect()
}

fn intent_envelope_fields(annotation: &IntentDefinition, path: &str) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    if let ExpectedVersionSource::OperationField { field } = &annotation.expected_version {
        fields.push((format!("{path}.expected_version.field"), field.clone()));
    }
    if let IdempotencySource::OperationField { field } = &annotation.idempotency {
        fields.push((format!("{path}.idempotency.field"), field.clone()));
    }
    fields.extend(
        annotation
            .content
            .iter()
            .enumerate()
            .map(|(index, binding)| {
                (
                    format!("{path}.content[{index}].input_field"),
                    binding.input_field.clone(),
                )
            }),
    );
    fields
}

fn authority_coordinate(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "realm"
            | "realm_id"
            | "realmid"
            | "tenant"
            | "tenant_id"
            | "tenantid"
            | "user"
            | "user_id"
            | "userid"
            | "current_user"
            | "authority"
            | "authority_id"
            | "authorityid"
            | "current_authority"
            | "principal"
            | "principal_id"
            | "principalid"
            | "executor"
            | "executor_id"
            | "executorid"
    )
}

fn missing_semantic(diagnostics: &mut Vec<RuntimeDiagnostic>, path: &str, kind: &str, name: &str) {
    diagnostics.push(RuntimeDiagnostic::new(
        RuntimeCode::MissingSemantic,
        path,
        format!("ESS {kind} {name:?} does not exist in the supplied EssIr"),
    ));
}

fn sha256(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let hash = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in hash {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

/// Stable runtime compiler diagnostic code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCode {
    /// The embedded definition is invalid.
    InvalidDefinition,
    /// Persisted JSON is malformed or contains unknown fields.
    InvalidPersistedIr,
    /// Persisted JSON differs from a fresh compilation.
    PersistedIrMismatch,
    /// Synthesis provenance does not bind the supplied ESS model.
    SynthesisModelMismatch,
    /// A command, event, view, or type does not exist.
    MissingSemantic,
    /// A field or emitted-event relationship does not exist.
    InvalidSemanticReference,
    /// ESS synthesis omitted a capability required by an annotation.
    MissingSynthesisCapability,
    /// ESS synthesis explicitly refused a capability required by an annotation.
    RefusedSynthesisCapability,
    /// The annotation form is closed but unsupported for this semantic location.
    UnsupportedAnnotation,
    /// Runtime delivery cannot satisfy ESS consistency.
    IncompatibleDelivery,
    /// A content reference does not exactly match its ESS command field.
    ContentTypeMismatch,
    /// Runtime envelope input collides with semantic command input.
    OperationFieldCollision,
    /// ESS attempts to expose authentication-derived authority as operation input.
    AuthorityCoordinate,
}

/// One structured runtime compilation diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RuntimeDiagnostic {
    /// Stable diagnostic code.
    pub code: RuntimeCode,
    /// Definition or persisted-document path.
    pub path: String,
    /// Human-readable repair guidance.
    pub message: String,
}

impl RuntimeDiagnostic {
    fn new(code: RuntimeCode, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            path: path.into(),
            message: message.into(),
        }
    }
}

/// All runtime compiler errors found in one pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDiagnostics(Vec<RuntimeDiagnostic>);

impl RuntimeDiagnostics {
    fn one(code: RuntimeCode, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self(vec![RuntimeDiagnostic::new(code, path, message)])
    }

    /// Structured diagnostics in deterministic discovery order.
    pub fn diagnostics(&self) -> &[RuntimeDiagnostic] {
        &self.0
    }
}

impl fmt::Display for RuntimeDiagnostics {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("service runtime IR compilation was refused")?;
        for diagnostic in &self.0 {
            write!(
                formatter,
                "\n- {:?} at {}: {}",
                diagnostic.code, diagnostic.path, diagnostic.message
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for RuntimeDiagnostics {}
