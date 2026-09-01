//! Deterministic transport-neutral client and Connector operation plans.
//!
//! Authentication establishes trusted context before any operation is decoded. Consequently this
//! module derives only application inputs and rejects tenant, realm, user, authority, and executor
//! coordinates even if an upstream validation boundary accidentally admits one.
//!
//! [`ClientPlan`] is the generation plan for one standalone service. Selection and client
//! generation for a composed component surface remain the responsibility of ESS composition.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use service_connectors::{
    CONTRIBUTION_FORMAT, ConnectorServiceFactoryDescriptor, ContributionError,
    OperationContribution, OperationEffect, OperationInput, OperationKind, ServiceContribution,
};
use service_definition::{
    ExpectedVersionSource, IdempotencySource, IntentDefinition, QueryDefinition, RealmPolicy,
};
use service_runtime_ir::{ResolvedIntent, ResolvedQuery, ServiceRuntimeIr};

/// The only client-plan format emitted by this version of the builder.
pub const CLIENT_PLAN_FORMAT: &str = "service-client-plan/1";

/// A complete transport-neutral service client surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientPlan {
    /// Format discriminator.
    pub format: String,
    /// Stable service identity.
    pub service: String,
    /// Exact compiler-minted ESS source digest.
    pub ess_source_digest: String,
    /// Admission policy carried as client-generation metadata, never as an operation input.
    pub realm_policy: RealmPolicy,
    /// Authentication source for every operation. It is deliberately not an operation argument.
    pub authentication: ClientAuthentication,
    /// Operations in stable identity order.
    pub operations: Vec<ClientOperation>,
}

/// Where a generated client obtains authentication.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientAuthentication {
    /// Use the already authenticated login session supplied to the client implementation.
    Session,
}

/// One generated client operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientOperation {
    /// Stable operation name from `service-definition/1`.
    pub operation: String,
    /// Exact ESS command or view reference.
    pub semantic_ref: String,
    /// Operation role.
    pub kind: ClientOperationKind,
    /// Caller-controlled application inputs in stable name order.
    pub inputs: Vec<ClientInput>,
    /// Typed result surface.
    pub result: ClientResult,
}

/// Whether a client operation mutates or reads service state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientOperationKind {
    /// An authenticated intent backed by one ESS command.
    Intent,
    /// An authenticated query backed by one ESS view.
    Query,
}

/// One caller-controlled client input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientInput {
    /// Stable operation-local wire name.
    pub name: String,
    /// Canonical ESS type spelling, including runtime envelope primitives.
    pub type_ref: String,
    /// Whether the type permits absence.
    pub optional: bool,
    /// Why this field is part of the operation.
    pub source: ClientInputSource,
}

/// Origin of one generated client input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClientInputSource {
    /// Input to the resolved ESS command.
    Command,
    /// Explicit optimistic-concurrency input from the operation envelope.
    ExpectedVersion,
    /// Explicit idempotency input from the operation envelope.
    Idempotency,
    /// Plaintext staged through a named external-content policy.
    Content {
        /// Content-policy name from `service-definition/1`.
        policy: String,
    },
    /// Query selector mapped to a resolved view field.
    Selector {
        /// Resolved ESS view field selected by this parameter.
        view_field: String,
    },
}

/// Typed result of a generated client operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClientResult {
    /// Intent acceptance may append these exact ESS event types.
    Intent {
        /// All events declared by the command's outcomes, in ESS order.
        emitted_events: Vec<String>,
    },
    /// Query success returns this resolved ESS view row.
    Query {
        /// Fields in stable name order.
        fields: BTreeMap<String, String>,
    },
}

/// A runtime IR invariant was insufficient to mint a safe public operation surface.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClientPlanError {
    /// A resolved operation has no matching losslessly embedded annotation.
    #[error("resolved {kind} operation `{operation}` has no embedded definition")]
    MissingDefinition {
        /// Operation role.
        kind: &'static str,
        /// Operation identity.
        operation: String,
    },
    /// A content binding names a command input absent from the resolved operation.
    #[error("intent `{operation}` stages content for missing command field `{command_field}`")]
    MissingContentField {
        /// Intent identity.
        operation: String,
        /// Bound command field.
        command_field: String,
    },
    /// A query selector names a field absent from the resolved view.
    #[error("query `{operation}` selects missing view field `{view_field}`")]
    MissingSelectorField {
        /// Query identity.
        operation: String,
        /// Bound view field.
        view_field: String,
    },
    /// Two sources claim the same public input name.
    #[error("operation `{operation}` derives input `{input}` more than once")]
    DuplicateInput {
        /// Operation identity.
        operation: String,
        /// Duplicated input.
        input: String,
    },
    /// Trusted authority context was about to become caller-controlled input.
    #[error(
        "operation `{operation}` exposes reserved authentication coordinate `{input}` as caller input"
    )]
    AuthenticationCoordinate {
        /// Operation identity.
        operation: String,
        /// Forbidden input.
        input: String,
    },
    /// The derived contribution failed the independent Connector validation boundary.
    #[error(transparent)]
    Connector(#[from] ContributionError),
}

impl ClientPlan {
    /// Derives the complete client surface from compiler-minted runtime IR.
    pub fn from_runtime(runtime: &ServiceRuntimeIr) -> Result<Self, ClientPlanError> {
        let definition = runtime.definition();
        let mut operations = Vec::with_capacity(runtime.intents().len() + runtime.queries().len());

        for (name, resolved) in runtime.intents() {
            let annotation = definition
                .intents
                .iter()
                .find(|candidate| candidate.name == *name)
                .ok_or_else(|| ClientPlanError::MissingDefinition {
                    kind: "intent",
                    operation: name.to_string(),
                })?;
            operations.push(intent_operation(name.as_str(), resolved, annotation)?);
        }

        for (name, resolved) in runtime.queries() {
            let annotation = definition
                .queries
                .iter()
                .find(|candidate| candidate.name == *name)
                .ok_or_else(|| ClientPlanError::MissingDefinition {
                    kind: "query",
                    operation: name.to_string(),
                })?;
            operations.push(query_operation(name.as_str(), resolved, annotation)?);
        }

        operations.sort_by(|left, right| left.operation.cmp(&right.operation));
        Ok(Self {
            format: CLIENT_PLAN_FORMAT.to_owned(),
            service: definition.service.to_string(),
            ess_source_digest: runtime.ess_source_digest().to_owned(),
            realm_policy: definition.realm,
            authentication: ClientAuthentication::Session,
            operations,
        })
    }

    /// Canonical pretty JSON with a trailing newline.
    pub fn to_canonical_json(&self) -> String {
        let mut output = serde_json::to_string_pretty(self)
            .unwrap_or_else(|error| panic!("a derived client plan serializes: {error}"));
        output.push('\n');
        output
    }

    /// Mints the inert Connector factory descriptor from this exact operation inventory.
    pub fn connector_descriptor(
        &self,
    ) -> Result<ConnectorServiceFactoryDescriptor, ClientPlanError> {
        let operations = self
            .operations
            .iter()
            .map(|operation| OperationContribution {
                operation: operation.operation.clone(),
                semantic_ref: operation.semantic_ref.clone(),
                kind: match operation.kind {
                    ClientOperationKind::Intent => OperationKind::Intent,
                    ClientOperationKind::Query => OperationKind::Query,
                },
                effect: match operation.kind {
                    ClientOperationKind::Intent => OperationEffect::Write,
                    ClientOperationKind::Query => OperationEffect::Read,
                },
                inputs: operation
                    .inputs
                    .iter()
                    .map(|input| OperationInput {
                        name: input.name.clone(),
                        type_ref: input.type_ref.clone(),
                        optional: input.optional,
                    })
                    .collect(),
            })
            .collect();
        let contribution = ServiceContribution {
            format: CONTRIBUTION_FORMAT.to_owned(),
            service: self.service.clone(),
            ess_source_digest: self.ess_source_digest.clone(),
            operations,
        };
        ConnectorServiceFactoryDescriptor::new(contribution).map_err(Into::into)
    }
}

fn intent_operation(
    operation: &str,
    resolved: &ResolvedIntent,
    annotation: &IntentDefinition,
) -> Result<ClientOperation, ClientPlanError> {
    let mut inputs = resolved
        .input_fields
        .iter()
        .map(|(name, type_ref)| {
            (
                name.clone(),
                ClientInput {
                    name: name.clone(),
                    type_ref: type_ref.clone(),
                    optional: is_optional(type_ref),
                    source: ClientInputSource::Command,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    for binding in &annotation.content {
        if inputs.remove(&binding.command_reference_field).is_none() {
            return Err(ClientPlanError::MissingContentField {
                operation: operation.to_owned(),
                command_field: binding.command_reference_field.clone(),
            });
        }
        insert_input(
            operation,
            &mut inputs,
            ClientInput {
                name: binding.input_field.clone(),
                type_ref: "Bytes".to_owned(),
                optional: false,
                source: ClientInputSource::Content {
                    policy: binding.content.to_string(),
                },
            },
        )?;
    }

    if let ExpectedVersionSource::OperationField { field } = &annotation.expected_version {
        insert_input(
            operation,
            &mut inputs,
            ClientInput {
                name: field.clone(),
                type_ref: "Integer".to_owned(),
                optional: false,
                source: ClientInputSource::ExpectedVersion,
            },
        )?;
    }
    if let IdempotencySource::OperationField { field } = &annotation.idempotency {
        insert_input(
            operation,
            &mut inputs,
            ClientInput {
                name: field.clone(),
                type_ref: "String".to_owned(),
                optional: false,
                source: ClientInputSource::Idempotency,
            },
        )?;
    }

    reject_authentication_inputs(operation, inputs.keys().map(String::as_str))?;
    Ok(ClientOperation {
        operation: operation.to_owned(),
        semantic_ref: resolved.command.clone(),
        kind: ClientOperationKind::Intent,
        inputs: inputs.into_values().collect(),
        result: ClientResult::Intent {
            emitted_events: resolved.emitted_events.clone(),
        },
    })
}

fn query_operation(
    operation: &str,
    resolved: &ResolvedQuery,
    annotation: &QueryDefinition,
) -> Result<ClientOperation, ClientPlanError> {
    let mut inputs = BTreeMap::new();
    for selector in &annotation.selectors {
        let type_ref = resolved.fields.get(&selector.view_field).ok_or_else(|| {
            ClientPlanError::MissingSelectorField {
                operation: operation.to_owned(),
                view_field: selector.view_field.clone(),
            }
        })?;
        insert_input(
            operation,
            &mut inputs,
            ClientInput {
                name: selector.parameter.clone(),
                type_ref: type_ref.clone(),
                optional: is_optional(type_ref),
                source: ClientInputSource::Selector {
                    view_field: selector.view_field.clone(),
                },
            },
        )?;
    }

    reject_authentication_inputs(operation, inputs.keys().map(String::as_str))?;
    Ok(ClientOperation {
        operation: operation.to_owned(),
        semantic_ref: resolved.view.clone(),
        kind: ClientOperationKind::Query,
        inputs: inputs.into_values().collect(),
        result: ClientResult::Query {
            fields: resolved.fields.clone(),
        },
    })
}

fn insert_input(
    operation: &str,
    inputs: &mut BTreeMap<String, ClientInput>,
    input: ClientInput,
) -> Result<(), ClientPlanError> {
    let name = input.name.clone();
    if inputs.insert(name.clone(), input).is_some() {
        return Err(ClientPlanError::DuplicateInput {
            operation: operation.to_owned(),
            input: name,
        });
    }
    Ok(())
}

fn reject_authentication_inputs<'a>(
    operation: &str,
    mut inputs: impl Iterator<Item = &'a str>,
) -> Result<(), ClientPlanError> {
    if let Some(input) = inputs.find(|input| is_authentication_coordinate(input)) {
        return Err(ClientPlanError::AuthenticationCoordinate {
            operation: operation.to_owned(),
            input: input.to_owned(),
        });
    }
    Ok(())
}

fn is_authentication_coordinate(input: &str) -> bool {
    matches!(
        input.trim().to_ascii_lowercase().as_str(),
        "tenant"
            | "tenant_id"
            | "tenantid"
            | "realm"
            | "realm_id"
            | "realmid"
            | "user"
            | "user_id"
            | "userid"
            | "current_user"
            | "principal"
            | "principal_id"
            | "principalid"
            | "authority"
            | "authority_id"
            | "authorityid"
            | "current_authority"
            | "executor"
            | "executor_id"
            | "executorid"
    )
}

fn is_optional(type_ref: &str) -> bool {
    type_ref.starts_with("Optional<")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_complete_authentication_coordinate_set_is_reserved() {
        for input in [
            "tenant_id",
            "realm",
            "user_id",
            "principal_id",
            "current_authority",
            "executor_id",
        ] {
            assert!(is_authentication_coordinate(input));
        }
        assert!(!is_authentication_coordinate("owner"));
        assert!(!is_authentication_coordinate("project_id"));
    }

    #[test]
    fn optionality_comes_from_the_resolved_type() {
        assert!(is_optional("Optional<String>"));
        assert!(!is_optional("String"));
        assert!(!is_optional("List<Optional<String>>"));
    }
}
