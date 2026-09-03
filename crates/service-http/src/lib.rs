//! Identity-authenticated HTTP delivery for generated services.
//!
//! The server verifies the opaque bearer credential and its exact operation scope before parsing
//! application JSON. The client binds one generated service origin and audience and never accepts
//! tenant, actor, or realm as operation inputs.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

pub use axum::Router as HttpRouter;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use eventlog_core::EventStore as DurableEventStore;
pub use identity_client::AccessCredential;
use identity_client::{AccessAuthority, ClientError as IdentityError, IdentityClient};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use service_engine::{ExecutionError, PlanDelivery, RequestMetadata, ServiceEngine};
use service_eventlog::{AuthorityFacts, EventlogService, PageRequest};
use service_runtime::{AuthorityId, TenantId, UserId, VerifiedAuthContext, VerifiedIdentity};

const PAGE_FIELD: &str = "$page";

/// A stable RFC 9457-style problem response returned by generated services.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Problem {
    /// Stable machine-readable refusal category.
    pub code: String,
    /// HTTP status repeated in the body for non-browser clients.
    pub status: u16,
    /// Non-sensitive explanation suitable for operator logs.
    pub detail: String,
}

/// Accepted generated mutation receipt.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MutationReceipt {
    /// ESS outcome selected by the service.
    pub outcome: String,
    /// Exact accepted domain events.
    pub events: Vec<service_engine::DomainEvent>,
    /// Aggregate stream version after the append.
    pub through_version: u64,
    /// Whether idempotency replayed an earlier commit.
    pub replayed: bool,
}

/// One typed page returned by a generated query client.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QueryPage<T> {
    /// Visible typed rows.
    pub items: Vec<T>,
    /// Aggregate version when all rows share one stream.
    pub through_version: Option<u64>,
    /// Opaque continuation cursor.
    pub next_cursor: Option<String>,
    /// Whether another raw projection window exists.
    pub partial: bool,
}

/// Caller-selected bounded query page.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    /// Opaque cursor returned by an earlier request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// Maximum raw rows inspected, from 1 through 1000.
    pub limit: usize,
}

impl Default for Page {
    fn default() -> Self {
        Self {
            cursor: None,
            limit: 100,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WirePage {
    #[serde(default)]
    cursor: Option<String>,
    limit: usize,
}

/// Credential-safe client failure.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The configured service origin or audience is invalid.
    #[error("invalid generated service client configuration: {0}")]
    Configuration(&'static str),
    /// The request input could not be encoded.
    #[error("generated service request could not be encoded")]
    Encode(#[source] serde_json::Error),
    /// The HTTP exchange failed.
    #[error("generated service request could not be completed")]
    Transport(#[source] reqwest::Error),
    /// The service returned a stable refusal.
    #[error("generated service refused the operation: {0:?}")]
    Refused(Problem),
    /// The service returned a success body outside the generated contract.
    #[error("generated service returned an invalid success response")]
    InvalidResponse(#[source] reqwest::Error),
}

/// HTTP client bound to one generated service origin and exact resource audience.
#[derive(Clone, Debug)]
pub struct ServiceHttpClient {
    origin: reqwest::Url,
    audience: String,
    http: reqwest::Client,
}

impl ServiceHttpClient {
    /// Creates a redirect-free client for one exact generated service.
    pub fn new(origin: &str, audience: &str) -> Result<Self, ClientError> {
        let origin = reqwest::Url::parse(origin)
            .map_err(|_| ClientError::Configuration("origin must be an absolute URL"))?;
        let internal_http = origin.scheme() == "http"
            && origin.host_str().is_some_and(|host| {
                host == "127.0.0.1" || host == "localhost" || host.ends_with(".svc.cluster.local")
            });
        if !(origin.scheme() == "https" || internal_http)
            || origin.host_str().is_none()
            || !origin.username().is_empty()
            || origin.password().is_some()
            || origin.path() != "/"
            || origin.query().is_some()
            || origin.fragment().is_some()
        {
            return Err(ClientError::Configuration(
                "origin must be an HTTPS origin or an internal cluster HTTP origin",
            ));
        }
        if audience.trim() != audience || !(3..=256).contains(&audience.len()) {
            return Err(ClientError::Configuration("audience is malformed"));
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(ClientError::Transport)?;
        Ok(Self {
            origin,
            audience: audience.to_owned(),
            http,
        })
    }

    /// Returns the immutable audience bound by the generated client constructor.
    #[must_use]
    pub fn audience(&self) -> &str {
        &self.audience
    }

    /// Executes one generated mutation with an Identity-issued access credential.
    pub async fn intent<I: Serialize>(
        &self,
        credential: &AccessCredential,
        operation: &str,
        input: &I,
    ) -> Result<MutationReceipt, ClientError> {
        let response = self
            .http
            .post(self.endpoint(&format!("v1/intents/{operation}"))?)
            .bearer_auth(credential.expose_at_authorization_boundary())
            .json(input)
            .send()
            .await
            .map_err(ClientError::Transport)?;
        decode(response).await
    }

    /// Executes one generated query with typed input and rows.
    pub async fn query<I: Serialize, O: DeserializeOwned>(
        &self,
        credential: &AccessCredential,
        operation: &str,
        input: &I,
        page: Page,
    ) -> Result<QueryPage<O>, ClientError> {
        let mut value = serde_json::to_value(input).map_err(ClientError::Encode)?;
        let object = value.as_object_mut().ok_or(ClientError::Configuration(
            "query input must encode as an object",
        ))?;
        object.insert(
            PAGE_FIELD.to_owned(),
            serde_json::to_value(page).map_err(ClientError::Encode)?,
        );
        let response = self
            .http
            .post(self.endpoint(&format!("v1/queries/{operation}"))?)
            .bearer_auth(credential.expose_at_authorization_boundary())
            .json(&value)
            .send()
            .await
            .map_err(ClientError::Transport)?;
        decode(response).await
    }

    fn endpoint(&self, path: &str) -> Result<reqwest::Url, ClientError> {
        self.origin
            .join(path)
            .map_err(|_| ClientError::Configuration("operation path is invalid"))
    }
}

async fn decode<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, ClientError> {
    if response.status().is_success() {
        response.json().await.map_err(ClientError::InvalidResponse)
    } else {
        let status = response.status().as_u16();
        let problem = response
            .json::<Problem>()
            .await
            .unwrap_or_else(|_| Problem {
                code: "unexpected_status".to_owned(),
                status,
                detail: "generated service returned an unexpected response".to_owned(),
            });
        Err(ClientError::Refused(problem))
    }
}

/// Initialization failure for an Identity HTTP service.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    /// The generated realization plan could not be parsed.
    #[error("generated service plan is invalid")]
    Plan(#[source] service_engine::PlanError),
    /// The generated plan selected a different delivery mode.
    #[error("generated plan does not declare Identity HTTP delivery")]
    Delivery,
    /// Identity client configuration was invalid.
    #[error("Identity client configuration is invalid")]
    Identity(#[source] IdentityError),
    /// Eventlog projector registration failed.
    #[error("generated service persistence initialization failed")]
    Persistence(#[source] eventlog_core::EventLogError),
}

#[derive(Clone)]
struct HttpState {
    service: EventlogService,
    identity: IdentityClient,
    audience: String,
}

/// Initialized generated service HTTP boundary.
#[derive(Clone)]
pub struct IdentityHttpService {
    state: HttpState,
}

impl IdentityHttpService {
    /// Initializes Identity verification and Eventlog-backed execution from generated plan bytes.
    pub async fn initialize(
        store: Arc<dyn DurableEventStore>,
        engine: ServiceEngine,
        identity_origin: &str,
    ) -> Result<Self, ServerError> {
        let PlanDelivery::IdentityHttp { audience } = &engine.plan().delivery else {
            return Err(ServerError::Delivery);
        };
        let audience = audience.clone();
        let identity =
            IdentityClient::new(identity_origin, &audience).map_err(ServerError::Identity)?;
        let service = EventlogService::initialize(store, engine)
            .await
            .map_err(ServerError::Persistence)?;
        Ok(Self {
            state: HttpState {
                service,
                identity,
                audience,
            },
        })
    }

    /// Builds the complete generated router with probes and closed operation routes.
    pub fn router(self) -> Router {
        Router::new()
            .route("/healthz", get(health))
            .route("/readyz", get(health))
            .route("/v1/intents/{operation}", post(intent))
            .route("/v1/queries/{operation}", post(query))
            .with_state(self.state)
    }
}

async fn health() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn intent(
    State(state): State<HttpState>,
    Path(operation): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(plan) = state.service.plan().intents.get(&operation) else {
        return problem(
            StatusCode::NOT_FOUND,
            "unknown_operation",
            "unknown generated operation",
        );
    };
    let (context, facts) = match authorize(&state, &headers, &plan.scope).await {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let request_id = headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok());
    match state
        .service
        .intent(
            &context,
            facts,
            RequestMetadata { request_id },
            &operation,
            &body,
        )
        .await
    {
        Ok(receipt) => Json(receipt).into_response(),
        Err(error) => execution_problem(error),
    }
}

async fn query(
    State(state): State<HttpState>,
    Path(operation): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(plan) = state.service.plan().queries.get(&operation) else {
        return problem(
            StatusCode::NOT_FOUND,
            "unknown_operation",
            "unknown generated operation",
        );
    };
    let (context, facts) = match authorize(&state, &headers, &plan.scope).await {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let mut object = match serde_json::from_slice::<Value>(&body) {
        Ok(Value::Object(object)) => object,
        Ok(_) => {
            return problem(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                "query body must be an object",
            );
        }
        Err(_) => {
            return problem(
                StatusCode::BAD_REQUEST,
                "invalid_json",
                "query body is invalid JSON",
            );
        }
    };
    let page = match object.remove(PAGE_FIELD) {
        Some(value) => match serde_json::from_value::<WirePage>(value) {
            Ok(page) => page,
            Err(_) => {
                return problem(
                    StatusCode::BAD_REQUEST,
                    "invalid_page",
                    "query page is invalid",
                );
            }
        },
        None => WirePage {
            cursor: None,
            limit: 100,
        },
    };
    let Ok(page) = PageRequest::new(page.cursor, page.limit) else {
        return problem(
            StatusCode::BAD_REQUEST,
            "invalid_page",
            "query page is outside supported bounds",
        );
    };
    let Ok(application_body) = serde_json::to_vec(&Value::Object(object)) else {
        return problem(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            "query body could not be decoded",
        );
    };
    match state
        .service
        .query_page(&context, facts, &operation, &application_body, page)
        .await
    {
        Ok(result) => Json(result).into_response(),
        Err(error) => execution_problem(error),
    }
}

async fn authorize(
    state: &HttpState,
    headers: &HeaderMap,
    required_scope: &str,
) -> Result<(VerifiedAuthContext, AuthorityFacts), Box<Response>> {
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            Box::new(problem(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "a bearer credential is required",
            ))
        })?;
    let authority = state
        .identity
        .resolve_access_token(authorization, &state.audience)
        .await
        .map_err(|error| Box::new(identity_problem(&error)))?;
    if !has_scope(&authority.scope, required_scope) {
        return Err(Box::new(problem(
            StatusCode::FORBIDDEN,
            "insufficient_scope",
            "the access credential lacks the operation scope",
        )));
    }
    verified_context(authority)
}

fn has_scope(granted: &str, required: &str) -> bool {
    granted
        .split_ascii_whitespace()
        .any(|scope| scope == required)
}

fn verified_context(
    authority: AccessAuthority,
) -> Result<(VerifiedAuthContext, AuthorityFacts), Box<Response>> {
    let tenant = TenantId::new(&authority.tenant_id).map_err(|_| {
        Box::new(problem(
            StatusCode::UNAUTHORIZED,
            "invalid_authority",
            "Identity returned invalid tenant authority",
        ))
    })?;
    let principal = AuthorityId::new(&authority.subject).map_err(|_| {
        Box::new(problem(
            StatusCode::UNAUTHORIZED,
            "invalid_authority",
            "Identity returned invalid principal authority",
        ))
    })?;
    let user = UserId::new(&authority.actor.subject).map_err(|_| {
        Box::new(problem(
            StatusCode::UNAUTHORIZED,
            "invalid_authority",
            "Identity returned invalid actor authority",
        ))
    })?;
    let mut principals = BTreeSet::from([authority.subject.clone(), authority.actor.subject]);
    if let Some(email) = authority.email {
        principals.insert(email);
    }
    let facts = AuthorityFacts {
        principals,
        teams: authority.groups.into_iter().collect(),
        projects: BTreeSet::new(),
        extensions: BTreeSet::new(),
        capabilities: BTreeSet::new(),
    };
    Ok((
        VerifiedAuthContext::from_verified(VerifiedIdentity::after_verification(
            tenant, principal, user, None, None,
        )),
        facts,
    ))
}

fn identity_problem(error: &IdentityError) -> Response {
    match error {
        IdentityError::Unauthorized => problem(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Identity refused the access credential",
        ),
        IdentityError::Forbidden => problem(
            StatusCode::FORBIDDEN,
            "forbidden",
            "Identity refused the resource audience",
        ),
        IdentityError::Transport(_) | IdentityError::UnexpectedStatus(_) => problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "identity_unavailable",
            "Identity authority is unavailable",
        ),
        IdentityError::Configuration(_) | IdentityError::CacheableCredentialResponse => problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "identity_contract",
            "Identity authority response violated its contract",
        ),
    }
}

fn execution_problem(error: ExecutionError) -> Response {
    match error {
        ExecutionError::Decode(_)
        | ExecutionError::ExpectedObject
        | ExecutionError::UnknownInput(_)
        | ExecutionError::MissingInput(_)
        | ExecutionError::InvalidInput(_) => problem(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            "operation input was refused",
        ),
        ExecutionError::ObligationRefused(code) if code == "not_found" => problem(
            StatusCode::NOT_FOUND,
            "not_found",
            "the requested entity was not found",
        ),
        ExecutionError::ObligationRefused(code) => problem(
            StatusCode::CONFLICT,
            &code,
            "a service invariant refused the operation",
        ),
        ExecutionError::Context(_) => problem(
            StatusCode::FORBIDDEN,
            "context_refused",
            "verified authority does not satisfy the service policy",
        ),
        ExecutionError::UnknownOperation(_) => problem(
            StatusCode::NOT_FOUND,
            "unknown_operation",
            "unknown generated operation",
        ),
        ExecutionError::Resource(_) => problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "service_unavailable",
            "a service resource is unavailable",
        ),
        ExecutionError::InvalidHistory
        | ExecutionError::UnknownEvent(_)
        | ExecutionError::InvalidEvent(_)
        | ExecutionError::InvalidProjection
        | ExecutionError::UnknownProvider(_)
        | ExecutionError::InvalidPlan(_) => problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "service_contract",
            "generated service state violated its contract",
        ),
    }
}

fn problem(status: StatusCode, code: &str, detail: &str) -> Response {
    (
        status,
        Json(Problem {
            code: code.to_owned(),
            status: status.as_u16(),
            detail: detail.to_owned(),
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_scopes_are_exact_tokens() {
        assert!(has_scope(
            "workflows.read workflows.manage",
            "workflows.read"
        ));
        assert!(!has_scope("workflows.read-all", "workflows.read"));
        assert!(!has_scope("WORKFLOWS.READ", "workflows.read"));
    }

    #[test]
    fn remote_plaintext_service_origins_are_refused() {
        assert!(ServiceHttpClient::new("http://example.com/", "urn:b10x:workflow").is_err());
        assert!(ServiceHttpClient::new("http://127.0.0.1:8080/", "urn:b10x:workflow").is_ok());
    }
}
