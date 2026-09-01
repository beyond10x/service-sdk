//! Inert service contributions for Connector composition.
//!
//! This crate deliberately does not bind endpoints, credentials, grants, exposure, or deployment
//! policy. A generated service owns a [`ConnectorServiceFactoryDescriptor`] and generated adapter
//! code implements Connectors' `ConnectorServiceFactory` from it. The composing product only
//! registers that factory and supplies the explicit `ServiceDeployment` accepted by
//! `ServiceBundleBuilder`.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// Persisted contribution format.
pub const CONTRIBUTION_FORMAT: &str = "service-connector-contribution/1";

/// One inert service contribution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceContribution {
    /// Format discriminator.
    pub format: String,
    /// Stable service key within a product bundle.
    pub service: String,
    /// Exact ESS semantic source digest this contribution describes.
    pub ess_source_digest: String,
    /// Operations contributed by the service.
    pub operations: Vec<OperationContribution>,
}

/// One Connector-visible operation, without deployment binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationContribution {
    /// Stable operation name.
    pub operation: String,
    /// ESS command or view this operation realizes.
    pub semantic_ref: String,
    /// Whether the operation accepts an intent or executes a query.
    pub kind: OperationKind,
    /// Observable effect used by deployment approval policy.
    pub effect: OperationEffect,
    /// Application inputs. Authentication coordinates are forbidden here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<OperationInput>,
}

/// The operation's runtime role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    /// Authenticated intent that may decide a semantic command.
    Intent,
    /// Authenticated projection query.
    Query,
}

/// The operation effect Connectors exposes to approval policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationEffect {
    /// Reads service state only.
    Read,
    /// May append domain events.
    Write,
}

/// One application-level operation input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationInput {
    /// Stable wire name.
    pub name: String,
    /// ESS type reference rendered canonically.
    pub type_ref: String,
    /// Whether absence is accepted.
    #[serde(default)]
    pub optional: bool,
}

/// A validated inert factory descriptor generated into one service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorServiceFactoryDescriptor {
    contribution: ServiceContribution,
    digest: String,
}

/// Why an inert contribution cannot become a factory descriptor.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContributionError {
    /// The persisted format is not one this build reads.
    #[error("unsupported contribution format `{0}`")]
    UnsupportedFormat(String),
    /// A stable identifier is absent.
    #[error("{0} must not be empty")]
    Empty(&'static str),
    /// One operation was declared more than once.
    #[error("operation `{0}` is declared more than once")]
    DuplicateOperation(String),
    /// One input was declared more than once on an operation.
    #[error("input `{input}` is declared more than once on operation `{operation}`")]
    DuplicateInput {
        /// Operation carrying the duplicate.
        operation: String,
        /// Duplicated input.
        input: String,
    },
    /// An application operation tried to accept authentication authority as input.
    #[error(
        "operation `{operation}` declares reserved authentication coordinate `{input}`; it must come from verified authentication context"
    )]
    AuthenticationCoordinate {
        /// Operation carrying the invalid input.
        operation: String,
        /// Reserved coordinate.
        input: String,
    },
    /// The source digest is not a full lowercase SHA-256 value.
    #[error("ESS source digest must be 64 lowercase hexadecimal characters")]
    InvalidSourceDigest,
}

impl ConnectorServiceFactoryDescriptor {
    /// Validates and seals an inert contribution.
    pub fn new(contribution: ServiceContribution) -> Result<Self, ContributionError> {
        validate(&contribution)?;
        let bytes = serde_json::to_vec(&contribution)
            .unwrap_or_else(|error| panic!("validated contribution serializes: {error}"));
        let hash = Sha256::digest(bytes);
        let mut digest = String::with_capacity(64);
        for byte in hash {
            let _ = write!(digest, "{byte:02x}");
        }
        Ok(Self {
            contribution,
            digest,
        })
    }

    /// The inert contribution consumed by generated factory adapter code.
    pub fn contribution(&self) -> &ServiceContribution {
        &self.contribution
    }

    /// Digest of the complete canonical contribution payload.
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Canonical JSON persisted by a builder.
    pub fn to_canonical_json(&self) -> String {
        let mut json = serde_json::to_string_pretty(&self.contribution)
            .unwrap_or_else(|error| panic!("validated contribution serializes: {error}"));
        json.push('\n');
        json
    }
}

fn validate(contribution: &ServiceContribution) -> Result<(), ContributionError> {
    if contribution.format != CONTRIBUTION_FORMAT {
        return Err(ContributionError::UnsupportedFormat(
            contribution.format.clone(),
        ));
    }
    if contribution.service.trim().is_empty() {
        return Err(ContributionError::Empty("service"));
    }
    if contribution.ess_source_digest.len() != 64
        || !contribution
            .ess_source_digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ContributionError::InvalidSourceDigest);
    }

    let mut operations = BTreeSet::new();
    for operation in &contribution.operations {
        if operation.operation.trim().is_empty() {
            return Err(ContributionError::Empty("operation"));
        }
        if operation.semantic_ref.trim().is_empty() {
            return Err(ContributionError::Empty("semantic_ref"));
        }
        if !operations.insert(&operation.operation) {
            return Err(ContributionError::DuplicateOperation(
                operation.operation.clone(),
            ));
        }

        let mut inputs = BTreeSet::new();
        for input in &operation.inputs {
            if input.name.trim().is_empty() {
                return Err(ContributionError::Empty("operation input name"));
            }
            if input.type_ref.trim().is_empty() {
                return Err(ContributionError::Empty("operation input type_ref"));
            }
            if !inputs.insert(&input.name) {
                return Err(ContributionError::DuplicateInput {
                    operation: operation.operation.clone(),
                    input: input.name.clone(),
                });
            }
            if is_authentication_coordinate(&input.name) {
                return Err(ContributionError::AuthenticationCoordinate {
                    operation: operation.operation.clone(),
                    input: input.name.clone(),
                });
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn contribution() -> ServiceContribution {
        ServiceContribution {
            format: CONTRIBUTION_FORMAT.into(),
            service: "todo".into(),
            ess_source_digest: "a".repeat(64),
            operations: vec![OperationContribution {
                operation: "add-item".into(),
                semantic_ref: "todo.list.AddItem".into(),
                kind: OperationKind::Intent,
                effect: OperationEffect::Write,
                inputs: vec![OperationInput {
                    name: "list_id".into(),
                    type_ref: "todo.list.ListId".into(),
                    optional: false,
                }],
            }],
        }
    }

    #[test]
    fn a_descriptor_is_deterministic_and_inert() {
        let left = ConnectorServiceFactoryDescriptor::new(contribution()).unwrap();
        let right = ConnectorServiceFactoryDescriptor::new(contribution()).unwrap();

        assert_eq!(left.digest(), right.digest());
        assert_eq!(left.to_canonical_json(), right.to_canonical_json());
        assert!(!left.to_canonical_json().contains("endpoint"));
        assert!(!left.to_canonical_json().contains("credential"));
    }

    #[test]
    fn authentication_coordinates_never_become_operation_arguments() {
        for reserved in [
            "tenant_id",
            "tenantId",
            "realm",
            "realmId",
            "user_id",
            "authority",
            "principal_id",
            "executor",
        ] {
            let mut invalid = contribution();
            invalid.operations[0].inputs[0].name = reserved.into();
            assert!(matches!(
                ConnectorServiceFactoryDescriptor::new(invalid),
                Err(ContributionError::AuthenticationCoordinate { input, .. }) if input == reserved
            ));
        }
    }

    #[test]
    fn duplicate_operations_are_refused_before_bundle_composition() {
        let mut invalid = contribution();
        invalid.operations.push(invalid.operations[0].clone());
        assert_eq!(
            ConnectorServiceFactoryDescriptor::new(invalid),
            Err(ContributionError::DuplicateOperation("add-item".into()))
        );
    }
}
