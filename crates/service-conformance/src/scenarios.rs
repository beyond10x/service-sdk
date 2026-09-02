//! Execution of generated declarative scenarios through the generated Connector seam.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use connectors_protocol::operation::{
    DescribeRequest, InvokeRequest, OperationRequest, OperationResult,
};
use connectors_service::{
    ConnectorServiceFactory, DeploymentApproval, DeploymentRisk, OperationDeployment,
    PrincipalContext, ProviderIdentity, ServiceDeployment,
};
use eventlog_sqlite::SqliteEventStore;
use serde::Deserialize;
use serde_json::{Map, Value};
use service_connectors::{
    AuthorityFacts, AuthorityFactsError, AuthorityFactsResolver, GeneratedConnectorFactory,
};

/// One deterministic generated-scenario conformance refusal.
#[derive(Debug, thiserror::Error)]
#[error("generated Connector scenario conformance failed: {message}")]
pub struct ScenarioConformanceError {
    message: String,
}

impl ScenarioConformanceError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioDocument {
    format: String,
    service: String,
    scenarios: Vec<ScenarioCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioCase {
    name: String,
    given: ScenarioGiven,
    #[serde(default)]
    when: Vec<ScenarioIntent>,
    then: Vec<ScenarioAssertion>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioGiven {
    auth: ScenarioAuth,
    #[serde(default)]
    other_auth: Option<ScenarioAuth>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioAuth {
    tenant: String,
    realm: Option<String>,
    authority: String,
    user: String,
    #[serde(default)]
    executor: Option<String>,
    #[serde(default)]
    facts: ScenarioFacts,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioFacts {
    #[serde(default)]
    principals: BTreeSet<String>,
    #[serde(default)]
    teams: BTreeSet<String>,
    #[serde(default)]
    projects: BTreeSet<String>,
    #[serde(default)]
    extensions: BTreeSet<String>,
    #[serde(default)]
    capabilities: BTreeSet<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioIntent {
    intent: String,
    input: BTreeMap<String, Value>,
    #[serde(default = "default_auth_fixture")]
    using: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ScenarioAssertion {
    Query(ScenarioQuery),
    Partitions(ScenarioPartitions),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioQuery {
    query: String,
    input: BTreeMap<String, Value>,
    count: usize,
    #[serde(default = "default_auth_fixture")]
    using: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioPartitions {
    partitions_are_distinct: [String; 2],
}

fn default_auth_fixture() -> String {
    "auth".to_owned()
}

struct FixtureAuthority {
    facts: BTreeMap<Vec<u8>, AuthorityFacts>,
}

#[async_trait]
impl AuthorityFactsResolver for FixtureAuthority {
    async fn resolve(
        &self,
        context: &PrincipalContext,
    ) -> Result<AuthorityFacts, AuthorityFactsError> {
        self.facts
            .get(&context.stable_authority_seed())
            .cloned()
            .ok_or(AuthorityFactsError::Refused)
    }
}

/// Execute every committed scenario fixture through the operational generated Connector factory.
///
/// The supplied tuples are `(generated-relative path, YAML bytes)`. Authentication coordinates
/// and authority facts come only from scenario fixtures standing in for a verified receiver; they
/// are never merged into operation input.
pub async fn run_connector_scenarios(
    realization_plan_json: &str,
    contribution_json: &str,
    sources: &[(&str, &str)],
) -> Result<(), ScenarioConformanceError> {
    if sources.is_empty() {
        return Err(ScenarioConformanceError::new(
            "no generated scenario fixtures were supplied",
        ));
    }
    for (path, source) in sources {
        let document: ScenarioDocument = serde_yaml::from_str(source).map_err(|error| {
            ScenarioConformanceError::new(format!("{path}: invalid scenario YAML: {error}"))
        })?;
        if document.format != "service-scenarios/1" {
            return Err(ScenarioConformanceError::new(format!(
                "{path}: unsupported scenario format {:?}",
                document.format
            )));
        }
        for case in document.scenarios {
            run_case(
                path,
                &document.service,
                realization_plan_json,
                contribution_json,
                case,
            )
            .await?;
        }
    }
    Ok(())
}

async fn run_case(
    path: &str,
    service: &str,
    realization_plan_json: &str,
    contribution_json: &str,
    case: ScenarioCase,
) -> Result<(), ScenarioConformanceError> {
    let runtime = setup_case(
        path,
        service,
        realization_plan_json,
        contribution_json,
        &case,
    )
    .await?;
    let contexts = runtime.contexts;
    let backend = runtime.backend;
    let connection_ref = runtime.connection_ref;
    let mut captures = BTreeMap::<String, BTreeMap<String, Value>>::new();

    for step in &case.when {
        let fixture = contexts.get(&step.using).ok_or_else(|| {
            scenario_error(
                path,
                &case.name,
                format!("unknown auth fixture {:?}", step.using),
            )
        })?;
        let input = resolve_object(&step.input, &captures)
            .map_err(|message| scenario_error(path, &case.name, message))?;
        let output = invoke(
            &backend,
            &fixture.context,
            service,
            &connection_ref,
            &step.intent,
            input,
        )
        .await
        .map_err(|message| scenario_error(path, &case.name, message))?;
        capture_intent(&step.intent, &output, &mut captures)
            .map_err(|message| scenario_error(path, &case.name, message))?;
    }

    assert_case(
        path,
        service,
        &case,
        &contexts,
        &backend,
        &connection_ref,
        &captures,
    )
    .await
}

struct ScenarioRuntime {
    contexts: BTreeMap<String, AuthFixture>,
    backend: Arc<dyn connectors_service::ConnectorBackend>,
    connection_ref: String,
}

async fn setup_case(
    path: &str,
    service: &str,
    realization_plan_json: &str,
    contribution_json: &str,
    case: &ScenarioCase,
) -> Result<ScenarioRuntime, ScenarioConformanceError> {
    let contexts = fixture_contexts(path, case)?;
    let authority = Arc::new(FixtureAuthority {
        facts: contexts
            .values()
            .map(|fixture| {
                (
                    fixture.context.stable_authority_seed(),
                    fixture.facts.clone(),
                )
            })
            .collect(),
    });
    let store = Arc::new(
        SqliteEventStore::in_memory("service_scenario")
            .await
            .map_err(|_| scenario_error(path, &case.name, "could not create Eventlog SQLite"))?,
    );
    let factory = GeneratedConnectorFactory::from_json_with_authority(
        realization_plan_json,
        contribution_json,
        store,
        authority,
    )
    .map_err(|_| scenario_error(path, &case.name, "generated factory artifacts were refused"))?;
    let manifest = factory.manifest();
    let expected_service_ref = format!("service:{service}");
    if manifest.service_ref != expected_service_ref {
        return Err(scenario_error(
            path,
            &case.name,
            "scenario service differs from the generated factory",
        ));
    }
    let connection_ref = "connection:generated-scenario".to_owned();
    let deployment = ServiceDeployment {
        service_ref: manifest.service_ref,
        provider: ProviderIdentity {
            provider_ref: "provider:generated-scenario".to_owned(),
            authority: "dev.b10x.generated-scenario".to_owned(),
            connection_ref: connection_ref.clone(),
        },
        operations: manifest
            .operations
            .iter()
            .map(|operation| {
                (
                    operation.operation_ref.clone(),
                    OperationDeployment {
                        expose: true,
                        risk: DeploymentRisk::Low,
                        approval: DeploymentApproval::NotRequired,
                        endpoint_bindings: BTreeMap::new(),
                        credential_bindings: BTreeMap::new(),
                        grant_refs: BTreeSet::new(),
                    },
                )
            })
            .collect(),
    };
    let dispatch = factory
        .bind(&deployment)
        .await
        .map_err(|_| scenario_error(path, &case.name, "generated factory could not bind"))?;
    let (backend, _) = dispatch.into_parts();
    Ok(ScenarioRuntime {
        contexts,
        backend,
        connection_ref,
    })
}

async fn assert_case(
    path: &str,
    service: &str,
    case: &ScenarioCase,
    contexts: &BTreeMap<String, AuthFixture>,
    backend: &Arc<dyn connectors_service::ConnectorBackend>,
    connection_ref: &str,
    captures: &BTreeMap<String, BTreeMap<String, Value>>,
) -> Result<(), ScenarioConformanceError> {
    for assertion in &case.then {
        match assertion {
            ScenarioAssertion::Query(query) => {
                let fixture = contexts.get(&query.using).ok_or_else(|| {
                    scenario_error(
                        path,
                        &case.name,
                        format!("unknown auth fixture {:?}", query.using),
                    )
                })?;
                let input = resolve_object(&query.input, captures)
                    .map_err(|message| scenario_error(path, &case.name, message))?;
                let output = invoke(
                    backend,
                    &fixture.context,
                    service,
                    connection_ref,
                    &query.query,
                    input,
                )
                .await
                .map_err(|message| scenario_error(path, &case.name, message))?;
                let rows = output.as_array().ok_or_else(|| {
                    scenario_error(path, &case.name, "query output was not an array")
                })?;
                if rows.len() != query.count {
                    return Err(scenario_error(
                        path,
                        &case.name,
                        format!(
                            "query {:?} returned {} rows; expected {}",
                            query.query,
                            rows.len(),
                            query.count
                        ),
                    ));
                }
            }
            ScenarioAssertion::Partitions(partitions) => {
                let [left, right] = &partitions.partitions_are_distinct;
                let left = contexts.get(left).ok_or_else(|| {
                    scenario_error(path, &case.name, "left partition fixture is absent")
                })?;
                let right = contexts.get(right).ok_or_else(|| {
                    scenario_error(path, &case.name, "right partition fixture is absent")
                })?;
                if left.context.stable_authority_seed() == right.context.stable_authority_seed() {
                    return Err(scenario_error(
                        path,
                        &case.name,
                        "partition fixtures collapse to one Connector authority",
                    ));
                }
            }
        }
    }
    Ok(())
}

struct AuthFixture {
    context: PrincipalContext,
    facts: AuthorityFacts,
}

fn fixture_contexts(
    path: &str,
    case: &ScenarioCase,
) -> Result<BTreeMap<String, AuthFixture>, ScenarioConformanceError> {
    let mut fixtures = BTreeMap::new();
    fixtures.insert(
        "auth".to_owned(),
        auth_fixture(path, &case.name, "auth", &case.given.auth)?,
    );
    if let Some(auth) = &case.given.other_auth {
        fixtures.insert(
            "other_auth".to_owned(),
            auth_fixture(path, &case.name, "other_auth", auth)?,
        );
    }
    Ok(fixtures)
}

fn auth_fixture(
    path: &str,
    case: &str,
    name: &str,
    auth: &ScenarioAuth,
) -> Result<AuthFixture, ScenarioConformanceError> {
    if auth.user != auth.authority {
        return Err(scenario_error(
            path,
            case,
            format!("{name} user and current authority differ at the Connector seam"),
        ));
    }
    let actor = auth.executor.as_deref().unwrap_or(&auth.authority);
    let context = PrincipalContext::hosted(
        auth.tenant.clone(),
        auth.authority.clone(),
        actor.to_owned(),
        None,
        format!("scenario-{name}"),
        "a".repeat(64),
    )
    .and_then(|context| context.with_verified_realm(auth.realm.clone()))
    .map_err(|_| scenario_error(path, case, format!("{name} authentication is invalid")))?;
    let mut principals = auth.facts.principals.clone();
    principals.insert(auth.authority.clone());
    Ok(AuthFixture {
        context,
        facts: AuthorityFacts {
            principals,
            teams: auth.facts.teams.clone(),
            projects: auth.facts.projects.clone(),
            extensions: auth.facts.extensions.clone(),
            capabilities: auth.facts.capabilities.clone(),
        },
    })
}

async fn invoke(
    backend: &Arc<dyn connectors_service::ConnectorBackend>,
    context: &PrincipalContext,
    service: &str,
    connection_ref: &str,
    operation: &str,
    input: Map<String, Value>,
) -> Result<Value, String> {
    let operation_ref = format!("{service}.{operation}");
    let described = backend
        .handle(
            context,
            OperationRequest::Describe(DescribeRequest {
                operation_ref: operation_ref.clone(),
            }),
        )
        .await
        .map_err(|error| format!("describe {operation:?} failed: {:?}", error.code))?;
    let OperationResult::Describe(description) = described else {
        return Err(format!("describe {operation:?} returned the wrong result"));
    };
    let invoked = backend
        .handle(
            context,
            OperationRequest::Invoke(InvokeRequest {
                operation_ref,
                connection_ref: connection_ref.to_owned(),
                description_ref: description.description_ref,
                input: Value::Object(input),
                approval_evidence_ref: None,
            }),
        )
        .await
        .map_err(|error| format!("invoke {operation:?} failed: {:?}", error.code))?;
    let OperationResult::Invoke(result) = invoked else {
        return Err(format!("invoke {operation:?} returned the wrong result"));
    };
    Ok(result.output)
}

fn resolve_object(
    input: &BTreeMap<String, Value>,
    captures: &BTreeMap<String, BTreeMap<String, Value>>,
) -> Result<Map<String, Value>, String> {
    input
        .iter()
        .map(|(name, value)| Ok((name.clone(), resolve_value(value, captures)?)))
        .collect()
}

fn resolve_value(
    value: &Value,
    captures: &BTreeMap<String, BTreeMap<String, Value>>,
) -> Result<Value, String> {
    match value {
        Value::String(reference) if reference.starts_with('$') => {
            let (step, field) = reference[1..]
                .split_once('.')
                .ok_or_else(|| format!("invalid scenario reference {reference:?}"))?;
            captures
                .get(step)
                .and_then(|fields| fields.get(field))
                .cloned()
                .ok_or_else(|| format!("unresolved scenario reference {reference:?}"))
        }
        Value::Array(items) => items
            .iter()
            .map(|item| resolve_value(item, captures))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(fields) => fields
            .iter()
            .map(|(name, value)| Ok((name.clone(), resolve_value(value, captures)?)))
            .collect::<Result<Map<_, _>, _>>()
            .map(Value::Object),
        _ => Ok(value.clone()),
    }
}

fn capture_intent(
    operation: &str,
    output: &Value,
    captures: &mut BTreeMap<String, BTreeMap<String, Value>>,
) -> Result<(), String> {
    let object = output
        .as_object()
        .ok_or_else(|| format!("intent {operation:?} output was not an object"))?;
    let events = object
        .get("events")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("intent {operation:?} output omitted events"))?;
    let fields = captures.entry(operation.to_owned()).or_default();
    for event in events {
        let event_fields = event
            .get("fields")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("intent {operation:?} emitted an invalid event"))?;
        for (name, value) in event_fields {
            fields.insert(name.clone(), value.clone());
        }
    }
    if let Some(version) = object.get("through_version") {
        fields.insert("through_version".to_owned(), version.clone());
    }
    Ok(())
}

fn scenario_error(path: &str, case: &str, message: impl fmt::Display) -> ScenarioConformanceError {
    ScenarioConformanceError::new(format!("{path} scenario {case:?}: {message}"))
}
