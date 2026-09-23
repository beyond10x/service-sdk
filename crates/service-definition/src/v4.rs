//! Strict opt-in annotations for Entity Runtime delegated services.
//!
//! This is a separate reader and representation. The `/3` document remains byte-for-byte owned by
//! [`crate::ServiceDefinition`] and cannot acquire `/4` fields through compatibility defaults.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU32;

use ess_compiler::refs::CommandRef;
use ess_domain::command::OutcomeName;
use ess_domain::name::QualifiedName;
use serde_json::Value;

use crate::{
    ContentDefinition, DefinitionCode, DefinitionDiagnostic, DefinitionDiagnostics, DefinitionId,
    IntentDefinition, ObligationDefinition, ProjectionDefinition, QueryDefinition, RealmPolicy,
    ServiceDefinition, ServiceDelivery,
};

/// Exact discriminator for the Entity Runtime delegated definition.
pub const SERVICE_DEFINITION_FORMAT_V4: &str = "service-definition/4";

/// One ordinary lowerer slot coordinate within a command binding.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotCoordinate {
    /// Exact ESS command.
    pub command: CommandRef,
    /// Lowerer-assigned canonical slot index.
    pub slot: u32,
}

/// A closed SDK source for one ordinary lowerer slot.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SlotValueSource {
    /// Intentionally omit one optional source value without materializing JSON null.
    Absent,
    /// A value from verified authentication.
    VerifiedContext {
        /// Exact context coordinate.
        value: crate::ContextValue,
    },
    /// A normalized field of the selected ESS command.
    CommandField {
        /// Command field name.
        field: String,
    },
    /// The injected trusted clock supplies the value.
    TrustedClock,
    /// The injected identifier source supplies a `UUIDv7`.
    GeneratedUuidV7,
    /// One named SDK obligation supplies the value.
    Obligation {
        /// Top-level obligation identity.
        name: DefinitionId,
    },
    /// One exact authored JSON value.
    Literal {
        /// Value checked against the lowerer's exact source type during compilation.
        value: Value,
    },
}

/// Binding of one lowerer slot to one SDK-owned value source.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotBindingDefinition {
    /// Exact lowerer coordinate.
    pub coordinate: SlotCoordinate,
    /// SDK source evaluated once per selected command.
    pub source: SlotValueSource,
}

/// One omitted entity-field coordinate selected by Entity Runtime.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationFieldCoordinate {
    /// Exact ESS command.
    pub command: CommandRef,
    /// Exact source outcome.
    pub outcome: OutcomeName,
    /// Entity field name.
    pub field: String,
}

/// Closed SDK policy for one selected operation-field fulfillment.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OperationFieldPolicy {
    /// Preserve the loaded field value.
    Preserve,
    /// Remove an optional field.
    Remove,
    /// Set from one normalized command field.
    CommandField {
        /// Command field name.
        field: String,
    },
    /// Run one named SDK obligation once and use its typed value.
    Obligation {
        /// Top-level obligation identity.
        name: DefinitionId,
    },
}

/// Binding of one selected operation field to one explicit policy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationFieldPolicyDefinition {
    /// Exact lowerer coordinate.
    pub coordinate: OperationFieldCoordinate,
    /// Explicit action source; omission never means `Preserve`.
    pub policy: OperationFieldPolicy,
}

/// Strict `/4` runtime annotations for a service whose domain decisions are delegated to ER.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceDefinitionV4 {
    /// Format discriminator; must equal [`SERVICE_DEFINITION_FORMAT_V4`].
    pub format: String,
    /// Stable SDK service identity.
    pub service: DefinitionId,
    /// Exact authored ESS component selected for this service.
    pub component: String,
    /// Nonzero ER version for every entity in the selected closure.
    pub entity_versions: BTreeMap<QualifiedName, NonZeroU32>,
    /// Exact named ordered scales required by the selected closure.
    #[serde(default)]
    pub scales: BTreeMap<QualifiedName, BTreeMap<String, Vec<String>>>,
    /// Explicit public delivery boundary.
    pub delivery: ServiceDelivery,
    /// Admission policy for authentication's exact optional realm.
    pub realm: RealmPolicy,
    /// Mutation operations and retained SDK host annotations.
    #[serde(default)]
    pub intents: Vec<IntentDefinition>,
    /// Projection materializations.
    #[serde(default)]
    pub projections: Vec<ProjectionDefinition>,
    /// Query operations.
    #[serde(default)]
    pub queries: Vec<QueryDefinition>,
    /// External content policies.
    #[serde(default)]
    pub content: Vec<ContentDefinition>,
    /// Versioned SDK-provided obligation bindings.
    #[serde(default)]
    pub obligations: Vec<ObligationDefinition>,
    /// Complete ordinary lowerer-slot bindings.
    #[serde(default)]
    pub slots: Vec<SlotBindingDefinition>,
    /// Complete operation-field policies.
    #[serde(default)]
    pub operation_fields: Vec<OperationFieldPolicyDefinition>,
}

impl ServiceDefinitionV4 {
    /// Reads and validates strict YAML without falling back to a `/3` reader.
    pub fn from_yaml(text: &str) -> Result<Self, DefinitionDiagnostics> {
        let definition: Self = serde_yaml::from_str(text)
            .map_err(|error| DefinitionDiagnostics::syntax(error.to_string()))?;
        definition.validate()?;
        Ok(definition)
    }

    /// Reads and validates strict JSON without falling back to a `/3` reader.
    pub fn from_json(text: &str) -> Result<Self, DefinitionDiagnostics> {
        let definition: Self = serde_json::from_str(text)
            .map_err(|error| DefinitionDiagnostics::syntax(error.to_string()))?;
        definition.validate()?;
        Ok(definition)
    }

    /// Validates host annotations and every `/4`-owned coordinate deterministically.
    pub fn validate(&self) -> Result<(), DefinitionDiagnostics> {
        let mut diagnostics = Vec::new();
        if self.format != SERVICE_DEFINITION_FORMAT_V4 {
            diagnostics.push(DefinitionDiagnostic::new(
                DefinitionCode::UnsupportedFormat,
                "format",
                format!(
                    "expected {SERVICE_DEFINITION_FORMAT_V4:?}, found {:?}",
                    self.format
                ),
            ));
        }
        if ess_domain::component::ComponentName::new(&self.component).is_err() {
            diagnostics.push(DefinitionDiagnostic::new(
                DefinitionCode::InvalidValue,
                "component",
                "component must be a valid ESS component name",
            ));
        }

        if let Err(host) = self.retained_host_definition().validate() {
            diagnostics.extend(host.0);
        }

        let obligations = self
            .obligations
            .iter()
            .map(|obligation| &obligation.name)
            .collect::<BTreeSet<_>>();
        let mut slots = BTreeSet::new();
        for (index, binding) in self.slots.iter().enumerate() {
            if !slots.insert(binding.coordinate.clone()) {
                diagnostics.push(DefinitionDiagnostic::new(
                    DefinitionCode::Duplicate,
                    format!("slots[{index}].coordinate"),
                    "lowerer slot coordinate is bound more than once",
                ));
            }
            validate_slot_source(&mut diagnostics, index, &binding.source, &obligations);
        }

        let mut fields = BTreeSet::new();
        for (index, binding) in self.operation_fields.iter().enumerate() {
            if !fields.insert(binding.coordinate.clone()) {
                diagnostics.push(DefinitionDiagnostic::new(
                    DefinitionCode::Duplicate,
                    format!("operation_fields[{index}].coordinate"),
                    "operation field coordinate is bound more than once",
                ));
            }
            if binding.coordinate.field.is_empty() {
                diagnostics.push(DefinitionDiagnostic::new(
                    DefinitionCode::InvalidValue,
                    format!("operation_fields[{index}].coordinate.field"),
                    "field must not be empty",
                ));
            }
            validate_operation_policy(&mut diagnostics, index, &binding.policy, &obligations);
        }

        if diagnostics.is_empty() {
            Ok(())
        } else {
            Err(DefinitionDiagnostics(diagnostics))
        }
    }

    /// Canonical pretty JSON with stable field order and a trailing newline.
    pub fn to_canonical_json(&self) -> String {
        let mut output = serde_json::to_string_pretty(self)
            .unwrap_or_else(|error| panic!("validated /4 service definition serializes: {error}"));
        output.push('\n');
        output
    }

    /// Returns the exact retained SDK host annotations in the established `/3` representation.
    ///
    /// This is an in-memory compiler input and never replaces the outer `/4` discriminator.
    pub fn retained_host_definition(&self) -> ServiceDefinition {
        ServiceDefinition {
            format: crate::SERVICE_DEFINITION_FORMAT.to_owned(),
            service: self.service.clone(),
            delivery: self.delivery.clone(),
            realm: self.realm,
            intents: self.intents.clone(),
            projections: self.projections.clone(),
            queries: self.queries.clone(),
            content: self.content.clone(),
            obligations: self.obligations.clone(),
        }
    }
}

fn validate_slot_source(
    diagnostics: &mut Vec<DefinitionDiagnostic>,
    index: usize,
    source: &SlotValueSource,
    obligations: &BTreeSet<&DefinitionId>,
) {
    match source {
        SlotValueSource::CommandField { field } if field.is_empty() => {
            diagnostics.push(DefinitionDiagnostic::new(
                DefinitionCode::InvalidValue,
                format!("slots[{index}].source.field"),
                "field must not be empty",
            ));
        }
        SlotValueSource::Obligation { name } if !obligations.contains(name) => {
            diagnostics.push(DefinitionDiagnostic::new(
                DefinitionCode::InvalidReference,
                format!("slots[{index}].source.name"),
                format!("obligation {name:?} is not declared"),
            ));
        }
        SlotValueSource::VerifiedContext { .. }
        | SlotValueSource::Absent
        | SlotValueSource::CommandField { .. }
        | SlotValueSource::TrustedClock
        | SlotValueSource::GeneratedUuidV7
        | SlotValueSource::Obligation { .. }
        | SlotValueSource::Literal { .. } => {}
    }
}

fn validate_operation_policy(
    diagnostics: &mut Vec<DefinitionDiagnostic>,
    index: usize,
    policy: &OperationFieldPolicy,
    obligations: &BTreeSet<&DefinitionId>,
) {
    match policy {
        OperationFieldPolicy::CommandField { field } if field.is_empty() => {
            diagnostics.push(DefinitionDiagnostic::new(
                DefinitionCode::InvalidValue,
                format!("operation_fields[{index}].policy.field"),
                "field must not be empty",
            ));
        }
        OperationFieldPolicy::Obligation { name } if !obligations.contains(name) => {
            diagnostics.push(DefinitionDiagnostic::new(
                DefinitionCode::InvalidReference,
                format!("operation_fields[{index}].policy.name"),
                format!("obligation {name:?} is not declared"),
            ));
        }
        OperationFieldPolicy::Preserve
        | OperationFieldPolicy::Remove
        | OperationFieldPolicy::CommandField { .. }
        | OperationFieldPolicy::Obligation { .. } => {}
    }
}
