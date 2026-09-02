//! Generated service contributions and their SDK-owned Connector runtime.
//!
//! This crate deliberately does not bind endpoints, credentials, grants, exposure, or deployment
//! policy. A generated service owns a [`ConnectorServiceFactoryDescriptor`]; this crate turns it
//! and the realization plan into the only supported Connector backend. Generated application code
//! injects Eventlog storage, while the composing product supplies the explicit deployment overlay.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
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
use service_engine::{ExecutionError, InputSource, RequestMetadata, ServiceEngine, ServicePlan};
use service_eventlog::EventlogService;
use service_runtime::{
    AuthorityId, ExecutorId, RealmId, TenantId, UserId, VerifiedAuthContext, VerifiedIdentity,
};
use sha2::{Digest as _, Sha256};

/// Durable persistence port injected into every generated Connector factory.
pub use eventlog_core::EventStore as DurableEventStore;
/// Receiver-verified authority facts consumed by generated SDK obligations.
pub use service_eventlog::AuthorityFacts;

/// Why an authenticated authority-facts adapter could not resolve the current principal.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AuthorityFactsError {
    /// The verified principal has no admitted service authority.
    #[error("the verified principal has no service authority facts")]
    Refused,
    /// The deployment authority source is temporarily unavailable.
    #[error("the service authority facts source is unavailable")]
    Unavailable,
}

/// Deployment-injected source of receiver-verified project, scope and capability facts.
///
/// Implementations receive [`PrincipalContext`] only after the Connector transport has admitted
/// authentication. Operation bodies never participate in authority-fact resolution.
#[async_trait]
pub trait AuthorityFactsResolver: Send + Sync + 'static {
    /// Resolve the complete fact set for one already-verified Connector principal.
    async fn resolve(
        &self,
        context: &PrincipalContext,
    ) -> Result<AuthorityFacts, AuthorityFactsError>;
}

struct PrincipalAuthorityFacts;

#[async_trait]
impl AuthorityFactsResolver for PrincipalAuthorityFacts {
    async fn resolve(
        &self,
        context: &PrincipalContext,
    ) -> Result<AuthorityFacts, AuthorityFactsError> {
        Ok(AuthorityFacts {
            principals: BTreeSet::from([context.subject().to_owned()]),
            teams: context.verified_groups().clone(),
            projects: BTreeSet::new(),
            extensions: BTreeSet::new(),
            capabilities: BTreeSet::new(),
        })
    }
}

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

/// A validated generated factory backed exclusively by SDK execution and Eventlog resources.
pub struct GeneratedConnectorFactory {
    descriptor: ConnectorServiceFactoryDescriptor,
    plan: ServicePlan,
    store: Arc<dyn DurableEventStore>,
    authority: Arc<dyn AuthorityFactsResolver>,
}

/// Why generated artifacts cannot become one operational Connector factory.
#[derive(Debug, thiserror::Error)]
pub enum GeneratedFactoryError {
    /// Connector contribution bytes are malformed or fail their closed validation.
    #[error("generated Connector contribution is invalid")]
    Contribution,
    /// SDK realization-plan bytes are malformed or fail their closed validation.
    #[error("generated service realization plan is invalid")]
    Plan,
    /// Independently generated artifacts disagree on identity or operation inventory.
    #[error("generated Connector contribution and realization plan disagree")]
    Drift,
}

impl GeneratedConnectorFactory {
    /// Admit generated bytes with the safe principal/group authority adapter.
    pub fn from_json(
        realization_plan_json: &str,
        contribution_json: &str,
        store: Arc<dyn DurableEventStore>,
    ) -> Result<Self, GeneratedFactoryError> {
        Self::from_json_with_authority(
            realization_plan_json,
            contribution_json,
            store,
            Arc::new(PrincipalAuthorityFacts),
        )
    }

    /// Admit generated bytes with deployment-selected persistence and authority resources.
    pub fn from_json_with_authority(
        realization_plan_json: &str,
        contribution_json: &str,
        store: Arc<dyn DurableEventStore>,
        authority: Arc<dyn AuthorityFactsResolver>,
    ) -> Result<Self, GeneratedFactoryError> {
        let contribution = serde_json::from_str::<ServiceContribution>(contribution_json)
            .map_err(|_| GeneratedFactoryError::Contribution)?;
        let descriptor = ConnectorServiceFactoryDescriptor::new(contribution)
            .map_err(|_| GeneratedFactoryError::Contribution)?;
        let plan = ServicePlan::from_json(realization_plan_json)
            .map_err(|_| GeneratedFactoryError::Plan)?;
        if descriptor.contribution().service != plan.service
            || descriptor.contribution().ess_source_digest != plan.ess_source_digest
            || !operations_match(descriptor.contribution(), &plan)
        {
            return Err(GeneratedFactoryError::Drift);
        }
        Ok(Self {
            descriptor,
            plan,
            store,
            authority,
        })
    }
}

#[async_trait]
impl ConnectorServiceFactory for GeneratedConnectorFactory {
    fn manifest(&self) -> ServiceManifest {
        manifest(&self.descriptor, &self.plan)
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
        if deployment.service_ref != manifest.service_ref || deployed != expected {
            return Err(ServiceFactoryBindError);
        }
        let runtime = EventlogService::initialize(
            Arc::clone(&self.store),
            ServiceEngine::new(self.plan.clone()),
        )
        .await
        .map_err(|_| ServiceFactoryBindError)?;
        let backend = GeneratedBackend {
            runtime,
            service: self.plan.service.clone(),
            connection_ref: deployment.provider.connection_ref.clone(),
            provider_ref: deployment.provider.provider_ref.clone(),
            authority: Arc::clone(&self.authority),
            operations: manifest
                .operations
                .into_iter()
                .map(|operation| (operation.operation_ref.clone(), operation))
                .collect(),
        };
        Ok(ServiceDispatch::new(Arc::new(backend), expected))
    }
}

fn operations_match(contribution: &ServiceContribution, plan: &ServicePlan) -> bool {
    let expected = contribution
        .operations
        .iter()
        .map(|operation| {
            let executable = match operation.kind {
                OperationKind::Intent => plan.intents.contains_key(&operation.operation),
                OperationKind::Query => plan.queries.contains_key(&operation.operation),
            };
            (operation.operation.as_str(), executable)
        })
        .collect::<BTreeMap<_, _>>();
    expected.values().all(|executable| *executable)
        && expected.len() == plan.intents.len() + plan.queries.len()
}

fn manifest(descriptor: &ConnectorServiceFactoryDescriptor, plan: &ServicePlan) -> ServiceManifest {
    let contribution = descriptor.contribution();
    ServiceManifest {
        service_ref: format!("service:{}", contribution.service),
        provider: ServiceProviderMetadata {
            display_name: contribution.service.clone(),
            description: format!("Generated {} service", contribution.service),
        },
        operations: contribution
            .operations
            .iter()
            .map(|operation| ServiceOperation {
                operation_ref: format!("{}.{}", contribution.service, operation.operation),
                title: operation.operation.clone(),
                description: format!(
                    "{} realized from {}",
                    operation.operation, operation.semantic_ref
                ),
                input_schema: input_schema(plan, operation),
                output_schema: output_schema(plan, operation),
                effect: match operation.effect {
                    OperationEffect::Read => ConnectorEffect::ReadOnly,
                    OperationEffect::Write => ConnectorEffect::Mutating,
                },
            })
            .collect(),
    }
}

fn input_schema(plan: &ServicePlan, operation: &OperationContribution) -> Value {
    let inputs = match operation.kind {
        OperationKind::Intent => plan
            .intents
            .get(&operation.operation)
            .map(|item| &item.inputs),
        OperationKind::Query => plan
            .queries
            .get(&operation.operation)
            .map(|item| &item.inputs),
    };
    let properties = operation
        .inputs
        .iter()
        .map(|input| {
            let generated =
                inputs.and_then(|inputs| inputs.iter().find(|item| item.name == input.name));
            let schema = match generated.map(|item| &item.source) {
                Some(InputSource::Content { policy, .. }) => plan.content.get(policy).map_or_else(
                    || json!({"title": input.type_ref}),
                    |policy| {
                        json!({
                            "type": "object",
                            "properties": {
                                "media_type": {"type": "string", "enum": policy.media_types},
                                "text": {"type": "string"}
                            },
                            "required": ["media_type", "text"],
                            "additionalProperties": false
                        })
                    },
                ),
                Some(InputSource::ExpectedVersion) => {
                    json!({"type": "integer", "minimum": 0})
                }
                Some(InputSource::Idempotency) => json!({"type": "string", "minLength": 1}),
                _ => json!({"title": input.type_ref}),
            };
            (input.name.clone(), schema)
        })
        .collect::<serde_json::Map<_, _>>();
    let required = operation
        .inputs
        .iter()
        .filter(|input| !input.optional)
        .map(|input| input.name.clone())
        .collect::<Vec<_>>();
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

fn output_schema(plan: &ServicePlan, operation: &OperationContribution) -> Value {
    match operation.kind {
        OperationKind::Intent => json!({
            "type": "object",
            "properties": {
                "outcome": {"type": "string"},
                "events": {"type": "array"},
                "through_version": {"type": "integer", "minimum": 1},
                "replayed": {"type": "boolean"}
            },
            "required": ["outcome", "events", "through_version", "replayed"],
            "additionalProperties": false
        }),
        OperationKind::Query => {
            let properties = plan
                .queries
                .get(&operation.operation)
                .and_then(|query| plan.views.get(&query.view))
                .map(|view| {
                    view.fields
                        .iter()
                        .map(|field| (field.clone(), json!({})))
                        .collect::<serde_json::Map<_, _>>()
                })
                .unwrap_or_default();
            json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": properties,
                    "additionalProperties": false
                }
            })
        }
    }
}

struct GeneratedBackend {
    runtime: EventlogService,
    service: String,
    connection_ref: String,
    provider_ref: String,
    authority: Arc<dyn AuthorityFactsResolver>,
    operations: BTreeMap<String, ServiceOperation>,
}

impl GeneratedBackend {
    fn connection(&self) -> ConnectionSummary {
        ConnectionSummary {
            connection_ref: self.connection_ref.clone(),
            label: self.service.clone(),
            provider: self.provider_ref.clone(),
            audiences: Vec::new(),
            purpose: Some("generated service state".to_owned()),
        }
    }

    fn summary(&self, operation: &ServiceOperation) -> OperationSummary {
        OperationSummary {
            operation_ref: operation.operation_ref.clone(),
            title: operation.title.clone(),
            effect: operation.effect,
            approval: ApprovalPosture::NotRequired,
            connections: vec![self.connection()],
        }
    }

    fn description_ref(&self, context: &PrincipalContext, operation_ref: &str) -> String {
        let authority = context.stable_authority_seed();
        digest_ref(
            "description",
            [
                authority.as_slice(),
                self.service.as_bytes(),
                self.connection_ref.as_bytes(),
                operation_ref.as_bytes(),
            ],
        )
    }

    fn operation_name<'a>(&self, operation_ref: &'a str) -> Option<&'a str> {
        operation_ref.strip_prefix(&format!("{}.", self.service))
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
            .map(|operation| self.summary(operation))
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

    async fn invoke(
        &self,
        context: &PrincipalContext,
        request: &InvokeRequest,
    ) -> Result<OperationResult, OperationError> {
        if request.connection_ref != self.connection_ref {
            return Err(not_found());
        }
        let operation = self
            .operations
            .get(&request.operation_ref)
            .ok_or_else(not_found)?;
        if request.description_ref != self.description_ref(context, &request.operation_ref) {
            return Err(OperationError::new(
                OperationErrorCode::StaleAuthority,
                "generated service description lease is stale",
                false,
            ));
        }
        let auth = verified_context(context)?;
        let facts = self
            .authority
            .resolve(context)
            .await
            .map_err(|error| match error {
                AuthorityFactsError::Refused => OperationError::new(
                    OperationErrorCode::NotGranted,
                    "generated service authority refused the operation",
                    false,
                ),
                AuthorityFactsError::Unavailable => unavailable(),
            })?;
        let body = serde_json::to_vec(&request.input).map_err(|_| invalid_input())?;
        let operation_name = self
            .operation_name(&request.operation_ref)
            .ok_or_else(not_found)?;
        let output = if self.runtime.plan().intents.contains_key(operation_name) {
            serde_json::to_value(
                self.runtime
                    .intent(
                        &auth,
                        facts,
                        RequestMetadata {
                            request_id: context.request_id(),
                        },
                        operation_name,
                        &body,
                    )
                    .await
                    .map_err(|error| execution_error(&error))?,
            )
            .map_err(|_| unavailable())?
        } else {
            serde_json::to_value(
                self.runtime
                    .query(&auth, facts, operation_name, &body)
                    .await
                    .map_err(|error| execution_error(&error))?,
            )
            .map_err(|_| unavailable())?
        };
        let output_bytes = serde_json::to_vec(&output).map_err(|_| unavailable())?;
        let authority = context.stable_authority_seed();
        Ok(OperationResult::Invoke(InvocationResult {
            operation_ref: operation.operation_ref.clone(),
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
impl ConnectorBackend for GeneratedBackend {
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
            OperationRequest::Invoke(request) => self.invoke(context, &request).await,
            _ => Err(not_found()),
        }
    }
}

fn verified_context(context: &PrincipalContext) -> Result<VerifiedAuthContext, OperationError> {
    let tenant = TenantId::new(context.tenant_id()).map_err(|_| unavailable())?;
    let authority = AuthorityId::new(context.subject()).map_err(|_| unavailable())?;
    let user = UserId::new(context.subject()).map_err(|_| unavailable())?;
    let executor = (context.actor_subject() != context.subject())
        .then(|| ExecutorId::new(context.actor_subject()))
        .transpose()
        .map_err(|_| unavailable())?;
    let realm = context
        .realm()
        .map(RealmId::new)
        .transpose()
        .map_err(|_| unavailable())?;
    Ok(VerifiedAuthContext::from_verified(
        VerifiedIdentity::after_verification(tenant, authority, user, executor, realm),
    ))
}

fn execution_error(error: &ExecutionError) -> OperationError {
    match error {
        ExecutionError::Decode(_)
        | ExecutionError::ExpectedObject
        | ExecutionError::UnknownInput(_)
        | ExecutionError::MissingInput(_)
        | ExecutionError::InvalidInput(_) => invalid_input(),
        ExecutionError::ObligationRefused(_) | ExecutionError::Context(_) => OperationError::new(
            OperationErrorCode::NotGranted,
            "generated service authority refused the operation",
            false,
        ),
        ExecutionError::Resource(_) => unavailable(),
        _ => OperationError::new(
            OperationErrorCode::Unavailable,
            "generated service state is inconsistent",
            false,
        ),
    }
}

fn invalid_input() -> OperationError {
    OperationError::new(
        OperationErrorCode::InvalidInput,
        "generated service input is invalid",
        false,
    )
}

fn not_found() -> OperationError {
    OperationError::new(
        OperationErrorCode::NotFound,
        "generated service operation was not found",
        false,
    )
}

fn unavailable() -> OperationError {
    OperationError::new(
        OperationErrorCode::Unavailable,
        "generated service runtime is unavailable",
        true,
    )
}

fn digest_ref<'a>(label: &str, parts: impl IntoIterator<Item = &'a [u8]>) -> String {
    let mut hash = Sha256::new();
    hash.update(b"service-connector-runtime/1");
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    format!("{label}:sha256:{}", hex::encode(hash.finalize()))
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
