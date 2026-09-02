//! Exact generated-service catalog contract and its ordinary external Connector service.
//!
//! The catalog combines ESS-owned semantic bytes with SDK-owned operation bindings. It contains
//! no deployment policy and no authentication coordinates. [`ServiceCatalogFactory`] contributes
//! read-only discovery through the same external factory seam as every other Connector service;
//! Connectors itself has no knowledge of this contract.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use connectors_protocol::operation::{
    ApprovalPosture, ConnectionSummary, DescribeRequest, InvocationResult, InvokeRequest,
    OperationDescription, OperationError, OperationErrorCode, OperationRequest, OperationResult,
    OperationSummary, SearchRequest,
};
use connectors_service::{
    BackendCapabilities, BackendReadinessError, ConnectorBackend, ConnectorServiceFactory,
    OperationEffect as ConnectorEffect, PrincipalContext, ServiceDeployment, ServiceDispatch,
    ServiceFactoryBindError, ServiceManifest, ServiceOperation, ServiceProviderMetadata,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

/// Exact application-facing service catalog format.
pub const SERVICE_CATALOG_FORMAT: &str = "service-catalog/1";
/// Exact ESS-owned catalog embedded without semantic reinterpretation.
pub const ESS_BROWSER_CATALOG_FORMAT: &str = "ess-browser-catalog/1";
/// Stable Connector service identity of the catalog overlay.
pub const CATALOG_SERVICE_REF: &str = "service:service-catalog";
/// Read-only operation listing available service catalogs.
pub const LIST_SERVICES_OPERATION: &str = "service_catalog.list_services";
/// Read-only operation retrieving one exact service catalog.
pub const GET_SERVICE_OPERATION: &str = "service_catalog.get_service";

/// Complete generated catalog consumed by docs, reusable widgets and product bindings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceCatalog {
    /// Exact format discriminator.
    pub format: String,
    /// Stable generated service identity.
    pub service_ref: String,
    /// Human-readable service name.
    pub display_name: String,
    /// Human-readable service purpose.
    pub description: String,
    /// Exact ESS-owned `ess-browser-catalog/1` document.
    pub semantic_catalog: Value,
    /// Session authentication policy. Coordinates are intentionally absent.
    pub authentication: CatalogAuthentication,
    /// Realm-free operations in stable Connector identity order.
    pub operations: Vec<CatalogOperation>,
}

/// Login-supplied authentication policy carried as metadata, never an operation input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogAuthentication {
    /// Always `session`.
    pub source: String,
    /// Whether login must, may, or must not supply a realm.
    pub realm_policy: RealmPolicy,
}

/// Realm admission policy; the value itself is supplied only by authentication.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RealmPolicy {
    /// Login must supply a realm.
    Required,
    /// Login may supply a realm; absence remains `None`.
    Optional,
    /// Login must not supply a realm.
    Forbidden,
}

/// One operation rendered and invoked by a generic service console.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogOperation {
    /// Service-local operation name.
    pub name: String,
    /// Exact Connector operation identity after composition.
    pub operation_ref: String,
    /// Exact ESS command or view identity.
    pub semantic_ref: String,
    /// Operation role.
    pub kind: CatalogOperationKind,
    /// Observable operation effect.
    pub effect: CatalogOperationEffect,
    /// Closed, realm-free JSON Schema for caller input.
    pub input_schema: Value,
    /// JSON Schema for the operation result.
    pub output_schema: Value,
}

/// Whether an operation sends intent or reads a query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogOperationKind {
    /// State-changing intent.
    Intent,
    /// Projection query.
    Query,
}

/// Effect used by the widget's confirmation boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogOperationEffect {
    /// Read-only operation.
    Read,
    /// State-changing operation.
    Write,
}

/// Why generated catalog bytes are not safe to publish.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CatalogError {
    /// The contract discriminator is unsupported.
    #[error("unsupported service catalog format `{0}`")]
    UnsupportedFormat(String),
    /// Required metadata is empty or malformed.
    #[error("invalid service catalog field `{0}`")]
    Invalid(&'static str),
    /// A stable operation identity appears more than once.
    #[error("duplicate catalog operation `{0}`")]
    DuplicateOperation(String),
    /// An operation schema tried to expose authentication authority as caller input.
    #[error("operation `{operation}` exposes authentication coordinate `{input}`")]
    AuthenticationCoordinate {
        /// Offending operation.
        operation: String,
        /// Offending input property.
        input: String,
    },
}

impl ServiceCatalog {
    /// Validates and deterministically orders one generated catalog.
    pub fn new(mut catalog: Self) -> Result<Self, CatalogError> {
        if catalog.format != SERVICE_CATALOG_FORMAT {
            return Err(CatalogError::UnsupportedFormat(catalog.format));
        }
        if !valid_service_ref(&catalog.service_ref)
            || catalog.display_name.trim().is_empty()
            || catalog.description.trim().is_empty()
            || catalog.authentication.source != "session"
            || catalog
                .semantic_catalog
                .get("format")
                .and_then(Value::as_str)
                != Some(ESS_BROWSER_CATALOG_FORMAT)
        {
            return Err(CatalogError::Invalid("metadata"));
        }
        let operation_prefix = format!(
            "{}.",
            catalog
                .service_ref
                .strip_prefix("service:")
                .ok_or(CatalogError::Invalid("service_ref"))?
        );
        catalog
            .operations
            .sort_by(|left, right| left.operation_ref.cmp(&right.operation_ref));
        let mut operations = BTreeSet::new();
        for operation in &catalog.operations {
            if operation.name.trim().is_empty()
                || operation.semantic_ref.trim().is_empty()
                || !operation.operation_ref.starts_with(&operation_prefix)
                || !operations.insert(operation.operation_ref.clone())
                || operation.input_schema.get("type").and_then(Value::as_str) != Some("object")
            {
                return Err(CatalogError::DuplicateOperation(
                    operation.operation_ref.clone(),
                ));
            }
            if let Some(properties) = operation
                .input_schema
                .get("properties")
                .and_then(Value::as_object)
                && let Some(input) = properties.keys().find(|input| is_auth_coordinate(input))
            {
                return Err(CatalogError::AuthenticationCoordinate {
                    operation: operation.operation_ref.clone(),
                    input: input.clone(),
                });
            }
        }
        Ok(catalog)
    }

    /// Parses and validates exact generated JSON.
    pub fn from_json(source: &str) -> Result<Self, CatalogError> {
        let catalog = serde_json::from_str(source).map_err(|_| CatalogError::Invalid("json"))?;
        Self::new(catalog)
    }

    /// Canonical pretty JSON with one trailing newline.
    pub fn to_canonical_json(&self) -> String {
        let mut output = serde_json::to_string_pretty(self)
            .unwrap_or_else(|error| panic!("validated service catalog serializes: {error}"));
        output.push('\n');
        output
    }

    /// Digest of the exact canonical catalog bytes.
    pub fn digest(&self) -> String {
        hex::encode(Sha256::digest(self.to_canonical_json().as_bytes()))
    }
}

/// External, read-only catalog service factory registered by a composition root.
pub struct ServiceCatalogFactory {
    catalogs: BTreeMap<String, ServiceCatalog>,
}

/// Why a catalog inventory cannot become a factory.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CatalogFactoryError {
    /// Two generated services claim the same stable identity.
    #[error("duplicate generated service catalog `{0}`")]
    DuplicateService(String),
    /// One catalog exceeds the bounded Connector result surface.
    #[error("generated service catalog `{0}` is too large for Connector invocation")]
    CatalogTooLarge(String),
}

impl ServiceCatalogFactory {
    /// Seals a deterministic inventory of already validated generated catalogs.
    pub fn new(
        catalogs: impl IntoIterator<Item = ServiceCatalog>,
    ) -> Result<Self, CatalogFactoryError> {
        let mut inventory = BTreeMap::new();
        for catalog in catalogs {
            let service_ref = catalog.service_ref.clone();
            let bytes = serde_json::to_vec(&catalog)
                .unwrap_or_else(|error| panic!("validated service catalog serializes: {error}"));
            if bytes.len() > 240 * 1024 {
                return Err(CatalogFactoryError::CatalogTooLarge(service_ref));
            }
            if inventory.insert(service_ref.clone(), catalog).is_some() {
                return Err(CatalogFactoryError::DuplicateService(service_ref));
            }
        }
        Ok(Self {
            catalogs: inventory,
        })
    }

    /// Returns the exact sealed inventory.
    pub fn catalogs(&self) -> &BTreeMap<String, ServiceCatalog> {
        &self.catalogs
    }
}

#[async_trait]
impl ConnectorServiceFactory for ServiceCatalogFactory {
    fn manifest(&self) -> ServiceManifest {
        ServiceManifest {
            service_ref: CATALOG_SERVICE_REF.to_owned(),
            provider: ServiceProviderMetadata {
                display_name: "Generated service catalog".to_owned(),
                description: "Exact SDK-generated service catalogs for authenticated products"
                    .to_owned(),
            },
            operations: vec![list_operation(), get_operation()],
        }
    }

    async fn bind(
        &self,
        deployment: &ServiceDeployment,
    ) -> Result<ServiceDispatch, ServiceFactoryBindError> {
        let manifest = self.manifest();
        let expected = manifest
            .operations
            .iter()
            .map(|operation| operation.operation_ref.clone())
            .collect::<BTreeSet<_>>();
        let deployed = deployment
            .operations
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        if deployment.service_ref != CATALOG_SERVICE_REF || deployed != expected {
            return Err(ServiceFactoryBindError);
        }
        let backend = CatalogBackend {
            catalogs: self.catalogs.clone(),
            connection_ref: deployment.provider.connection_ref.clone(),
            provider_ref: deployment.provider.provider_ref.clone(),
            operations: manifest
                .operations
                .into_iter()
                .map(|operation| (operation.operation_ref.clone(), operation))
                .collect(),
        };
        Ok(ServiceDispatch::new(Arc::new(backend), expected))
    }
}

fn list_operation() -> ServiceOperation {
    ServiceOperation {
        operation_ref: LIST_SERVICES_OPERATION.to_owned(),
        title: "List generated services".to_owned(),
        description: "List exact generated service catalog identities and digests".to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
        output_schema: json!({
            "type": "object",
            "properties": {
                "services": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "service_ref": {"type": "string"},
                            "display_name": {"type": "string"},
                            "description": {"type": "string"},
                            "digest": {"type": "string"}
                        },
                        "required": ["service_ref", "display_name", "description", "digest"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["services"],
            "additionalProperties": false
        }),
        effect: ConnectorEffect::ReadOnly,
    }
}

fn get_operation() -> ServiceOperation {
    ServiceOperation {
        operation_ref: GET_SERVICE_OPERATION.to_owned(),
        title: "Get generated service".to_owned(),
        description: "Retrieve one exact service-catalog/1 document".to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": {"service_ref": {"type": "string", "minLength": 1}},
            "required": ["service_ref"],
            "additionalProperties": false
        }),
        output_schema: json!({
            "type": "object",
            "description": "Exact service-catalog/1 document"
        }),
        effect: ConnectorEffect::ReadOnly,
    }
}

struct CatalogBackend {
    catalogs: BTreeMap<String, ServiceCatalog>,
    connection_ref: String,
    provider_ref: String,
    operations: BTreeMap<String, ServiceOperation>,
}

impl CatalogBackend {
    fn connection(&self) -> ConnectionSummary {
        ConnectionSummary {
            connection_ref: self.connection_ref.clone(),
            label: "generated services".to_owned(),
            provider: self.provider_ref.clone(),
            audiences: Vec::new(),
            purpose: Some("generated service discovery".to_owned()),
        }
    }

    fn description_ref(&self, context: &PrincipalContext, operation_ref: &str) -> String {
        let authority = context.stable_authority_seed();
        digest_ref(
            "description",
            [
                authority.as_slice(),
                self.connection_ref.as_bytes(),
                operation_ref.as_bytes(),
            ],
        )
    }

    fn search(&self, request: &SearchRequest) -> OperationResult {
        let query = request.query.to_ascii_lowercase();
        let operations = self
            .operations
            .values()
            .filter(|operation| {
                query.is_empty()
                    || operation
                        .operation_ref
                        .to_ascii_lowercase()
                        .contains(&query)
                    || operation.title.to_ascii_lowercase().contains(&query)
            })
            .take(usize::from(request.limit))
            .map(|operation| OperationSummary {
                operation_ref: operation.operation_ref.clone(),
                title: operation.title.clone(),
                effect: operation.effect,
                approval: ApprovalPosture::NotRequired,
                connections: vec![self.connection()],
            })
            .collect();
        OperationResult::Search { operations }
    }

    fn describe(
        &self,
        context: &PrincipalContext,
        request: &DescribeRequest,
    ) -> Result<OperationResult, OperationError> {
        let operation = self
            .operations
            .get(&request.operation_ref)
            .ok_or_else(not_found)?;
        Ok(OperationResult::Describe(OperationDescription {
            operation_ref: operation.operation_ref.clone(),
            title: operation.title.clone(),
            description: operation.description.clone(),
            input_schema: operation.input_schema.clone(),
            output_schema: operation.output_schema.clone(),
            effect: operation.effect,
            approval: ApprovalPosture::NotRequired,
            connections: vec![self.connection()],
            description_ref: self.description_ref(context, &operation.operation_ref),
        }))
    }

    fn invoke(
        &self,
        context: &PrincipalContext,
        request: &InvokeRequest,
    ) -> Result<OperationResult, OperationError> {
        if request.connection_ref != self.connection_ref
            || request.description_ref != self.description_ref(context, &request.operation_ref)
        {
            return Err(OperationError::new(
                OperationErrorCode::StaleAuthority,
                "generated service catalog description lease is stale",
                false,
            ));
        }
        let output = match request.operation_ref.as_str() {
            LIST_SERVICES_OPERATION => {
                if request
                    .input
                    .as_object()
                    .is_none_or(|input| !input.is_empty())
                {
                    return Err(invalid_input());
                }
                let services = self
                    .catalogs
                    .values()
                    .map(|catalog| {
                        json!({
                            "service_ref": catalog.service_ref,
                            "display_name": catalog.display_name,
                            "description": catalog.description,
                            "digest": catalog.digest()
                        })
                    })
                    .collect::<Vec<_>>();
                json!({"services": services})
            }
            GET_SERVICE_OPERATION => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct GetInput {
                    service_ref: String,
                }
                let input: GetInput =
                    serde_json::from_value(request.input.clone()).map_err(|_| invalid_input())?;
                serde_json::to_value(
                    self.catalogs
                        .get(&input.service_ref)
                        .ok_or_else(not_found)?,
                )
                .map_err(|_| unavailable())?
            }
            _ => return Err(not_found()),
        };
        let output_bytes = serde_json::to_vec(&output).map_err(|_| unavailable())?;
        let authority = context.stable_authority_seed();
        Ok(OperationResult::Invoke(InvocationResult {
            operation_ref: request.operation_ref.clone(),
            output,
            connector_audit_ref: digest_ref(
                "audit",
                [
                    authority.as_slice(),
                    request.operation_ref.as_bytes(),
                    context.request_id().unwrap_or("").as_bytes(),
                    output_bytes.as_slice(),
                ],
            ),
            execution_ref: None,
        }))
    }
}

#[async_trait]
impl ConnectorBackend for CatalogBackend {
    async fn ready(&self) -> Result<(), BackendReadinessError> {
        Ok(())
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::OPERATIONS
    }

    fn owns_operation(&self, request: &OperationRequest) -> bool {
        match request {
            OperationRequest::Search(_) => true,
            OperationRequest::Describe(request) => {
                self.operations.contains_key(&request.operation_ref)
            }
            OperationRequest::Invoke(request) => {
                request.connection_ref == self.connection_ref
                    && self.operations.contains_key(&request.operation_ref)
            }
            _ => false,
        }
    }

    async fn handle(
        &self,
        context: &PrincipalContext,
        request: OperationRequest,
    ) -> Result<OperationResult, OperationError> {
        match request {
            OperationRequest::Search(request) => Ok(self.search(&request)),
            OperationRequest::Describe(request) => self.describe(context, &request),
            OperationRequest::Invoke(request) => self.invoke(context, &request),
            _ => Err(not_found()),
        }
    }
}

fn valid_service_ref(service_ref: &str) -> bool {
    service_ref.strip_prefix("service:").is_some_and(|value| {
        !value.is_empty()
            && value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
            })
    })
}

fn is_auth_coordinate(input: &str) -> bool {
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
            | "authority"
            | "authority_id"
            | "authorityid"
            | "principal"
            | "principal_id"
            | "principalid"
            | "executor"
            | "executor_id"
            | "executorid"
    )
}

fn invalid_input() -> OperationError {
    OperationError::new(
        OperationErrorCode::InvalidInput,
        "generated service catalog input is invalid",
        false,
    )
}

fn not_found() -> OperationError {
    OperationError::new(
        OperationErrorCode::NotFound,
        "generated service catalog entry was not found",
        false,
    )
}

fn unavailable() -> OperationError {
    OperationError::new(
        OperationErrorCode::Unavailable,
        "generated service catalog is unavailable",
        true,
    )
}

fn digest_ref<'a>(label: &str, parts: impl IntoIterator<Item = &'a [u8]>) -> String {
    let mut hash = Sha256::new();
    hash.update(b"service-catalog-runtime/1");
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    format!("{label}:sha256:{}", hex::encode(hash.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> ServiceCatalog {
        ServiceCatalog::new(ServiceCatalog {
            format: SERVICE_CATALOG_FORMAT.to_owned(),
            service_ref: "service:todo".to_owned(),
            display_name: "Todo".to_owned(),
            description: "Generated Todo service".to_owned(),
            semantic_catalog: json!({"format": ESS_BROWSER_CATALOG_FORMAT}),
            authentication: CatalogAuthentication {
                source: "session".to_owned(),
                realm_policy: RealmPolicy::Optional,
            },
            operations: vec![CatalogOperation {
                name: "create_list".to_owned(),
                operation_ref: "todo.create_list".to_owned(),
                semantic_ref: "todo.list.CreateList".to_owned(),
                kind: CatalogOperationKind::Intent,
                effect: CatalogOperationEffect::Write,
                input_schema: json!({
                    "type": "object",
                    "properties": {"list_id": {"type": "string"}},
                    "required": ["list_id"],
                    "additionalProperties": false
                }),
                output_schema: json!({"type": "object"}),
            }],
        })
        .unwrap()
    }

    #[test]
    fn canonical_catalog_keeps_realm_as_policy_only() {
        let source = catalog().to_canonical_json();
        assert!(source.contains("\"realm_policy\": \"optional\""));
        assert!(!source.contains("realm_id"));
        assert_eq!(ServiceCatalog::from_json(&source).unwrap(), catalog());
    }

    #[test]
    fn authentication_coordinates_are_refused_as_inputs() {
        let mut invalid = catalog();
        invalid.operations[0].input_schema["properties"] = json!({"realm": {"type": "string"}});
        assert!(matches!(
            ServiceCatalog::new(invalid),
            Err(CatalogError::AuthenticationCoordinate { input, .. }) if input == "realm"
        ));
    }

    #[test]
    fn catalog_factory_is_an_ordinary_external_service() {
        let factory = ServiceCatalogFactory::new([catalog()]).unwrap();
        let manifest = factory.manifest();
        assert_eq!(manifest.service_ref, CATALOG_SERVICE_REF);
        assert_eq!(
            manifest
                .operations
                .iter()
                .map(|operation| operation.operation_ref.as_str())
                .collect::<Vec<_>>(),
            [LIST_SERVICES_OPERATION, GET_SERVICE_OPERATION]
        );
        assert!(
            manifest
                .operations
                .iter()
                .all(|operation| operation.effect == ConnectorEffect::ReadOnly)
        );
    }

    #[test]
    fn duplicate_service_catalogs_are_refused() {
        assert_eq!(
            ServiceCatalogFactory::new([catalog(), catalog()]).err(),
            Some(CatalogFactoryError::DuplicateService(
                "service:todo".to_owned()
            ))
        );
    }
}
