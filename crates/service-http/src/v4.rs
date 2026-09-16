//! Identity HTTP boundary for generated Entity Runtime delegated services.

use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use entity_store::{Recording, asynchronous::CommitReceipt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use service_engine::{
    PlanRealmPolicy,
    v4::{
        AuthenticatedPartition, EngineV4, ExecutionErrorV4, MutationResultV4, ServicePlanV4,
        project_query_v4,
    },
};
use service_eventlog::{
    AuthorityFacts, PageRequest,
    v4::{EventlogResourcesV4, HostResourcesV4},
};
use service_runtime::{RealmPolicy, VerifiedAuthContext};
use sha2::{Digest as _, Sha256};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    IdentityClient, Problem, ServerError, authorize_identity, execution_problem_v4, problem,
};

/// Public mutation receipt derived only from the selected durable ER result.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MutationReceiptV4 {
    /// ER-selected accepting outcome, when the command declared one.
    pub outcome: Option<String>,
    /// ER-selected response fields.
    pub response: Option<serde_json::Map<String, Value>>,
    /// Exact ordered domain events from the selected branch.
    pub events: Vec<entity_core::DomainEvent>,
    /// Resulting ER subject revision.
    pub through_version: u64,
    /// Original physical append receipt.
    pub commit: CommitReceipt,
    /// Whether this response recovered the original committed batch.
    pub replayed: bool,
}

/// Query result supplied by the SDK-owned projection boundary.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QueryResultV4 {
    /// Visible projection rows.
    pub items: Vec<Value>,
    /// Authority revision covered by the result, when one aggregate is selected.
    pub through_version: Option<u64>,
    /// Opaque continuation cursor.
    pub next_cursor: Option<String>,
    /// Whether more projection rows remain.
    pub partial: bool,
}

/// A durable mutation whose projection or external effect requires repair.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommittedAftercareV4 {
    /// Original immutable physical commit receipt.
    pub commit: CommitReceipt,
    /// Stable authority-derived repair token.
    pub repair_token: String,
    /// Stable non-secret failure category.
    pub code: String,
}

/// Operational backend for one exact generated `/4` service plan.
pub trait ServiceBackendV4: Send + Sync {
    /// Performs a bounded durable readiness read.
    fn readiness(&self) -> Result<(), String>;
    /// Executes one authenticated public mutation envelope.
    fn intent(
        &self,
        context: &VerifiedAuthContext,
        facts: AuthorityFacts,
        operation: &str,
        body: &[u8],
        recording: Recording,
    ) -> Result<MutationResultV4, ExecutionErrorV4>;
    /// Reads one authenticated SDK-owned projection.
    fn query(
        &self,
        context: &VerifiedAuthContext,
        facts: AuthorityFacts,
        operation: &str,
        body: &[u8],
        page: PageRequest,
    ) -> Result<QueryResultV4, String>;
}

/// A complete request backend over one already provisioned authenticated Eventlog authority.
///
/// Provider administration remains outside this type. A deployment opens and provisions the
/// bridge, then transfers its sole owner here for mutation, replay, snapshot query, and restart.
pub struct RecordedServiceBackendV4<H> {
    plan: ServicePlanV4,
    bridge: entity_eventlog::sync::RecordedEventlogBridge,
    authority: entity_eventlog::Authority,
    wait: entity_eventlog::sync::CallWait,
    host: Mutex<H>,
}

impl<H> RecordedServiceBackendV4<H> {
    /// Binds a validated generated plan to one exact opened authority.
    pub fn new(
        plan: ServicePlanV4,
        bridge: entity_eventlog::sync::RecordedEventlogBridge,
        authority: entity_eventlog::Authority,
        wait: entity_eventlog::sync::CallWait,
        host: H,
    ) -> Result<Self, service_engine::v4::PlanV4Error> {
        let _ = EngineV4::new(&plan)?;
        Ok(Self {
            plan,
            bridge,
            authority,
            wait,
            host: Mutex::new(host),
        })
    }

    /// Returns the exact generated plan bound to this backend.
    pub const fn plan(&self) -> &ServicePlanV4 {
        &self.plan
    }

    /// Borrows host resources for operational inspection after a request completes.
    pub fn host(&self) -> Result<std::sync::MutexGuard<'_, H>, String> {
        self.host
            .lock()
            .map_err(|_| "host resource lock poisoned".to_owned())
    }
}

impl<H: HostResourcesV4 + Send> ServiceBackendV4 for RecordedServiceBackendV4<H> {
    fn readiness(&self) -> Result<(), String> {
        self.bridge
            .complete_snapshot(&self.authority.logical_scope, self.wait)
            .map(|_| ())
            .map_err(|error| format!("{error:?}"))
    }

    fn intent(
        &self,
        context: &VerifiedAuthContext,
        _: AuthorityFacts,
        operation: &str,
        body: &[u8],
        recording: Recording,
    ) -> Result<MutationResultV4, ExecutionErrorV4> {
        let mut host = self
            .host
            .lock()
            .map_err(|_| ExecutionErrorV4::Persistence("host resource lock poisoned".into()))?;
        let occurred_at = OffsetDateTime::parse(&recording.recorded_at, &Rfc3339)
            .map_err(|error| ExecutionErrorV4::Input(error.to_string()))?;
        let eventlog_operation = entity_eventlog::EventlogOperationContext {
            subject: context.user().as_str().to_owned(),
            actor: recording
                .actor
                .clone()
                .unwrap_or_else(|| context.user().as_str().to_owned()),
            request_id: recording.record_id.clone(),
            trace_id: recording
                .correlation
                .clone()
                .unwrap_or_else(|| recording.record_id.clone()),
            causation_id: recording.causation.clone(),
            causation_depth: 0,
            occurred_at,
        };
        let mut resources = EventlogResourcesV4::new(
            &self.bridge,
            &self.authority,
            eventlog_operation,
            self.wait,
            &mut *host,
        );
        EngineV4::new(&self.plan)
            .map_err(|error| ExecutionErrorV4::Binding(error.to_string()))?
            .execute_public_json(context, operation, body, recording, &mut resources)
    }

    fn query(
        &self,
        context: &VerifiedAuthContext,
        facts: AuthorityFacts,
        operation: &str,
        body: &[u8],
        page: PageRequest,
    ) -> Result<QueryResultV4, String> {
        let query = self
            .plan
            .queries
            .get(operation)
            .ok_or_else(|| format!("unknown query operation {operation:?}"))?;
        let mut host = self
            .host
            .lock()
            .map_err(|_| "host resource lock poisoned".to_owned())?;
        host.authorize(context, &query.scope)?;
        match self.plan.realm {
            PlanRealmPolicy::Required => RealmPolicy::Required,
            PlanRealmPolicy::Optional => RealmPolicy::Optional,
            PlanRealmPolicy::Forbidden => RealmPolicy::Forbidden,
        }
        .enforce(context)
        .map_err(|error| error.to_string())?;
        let partition = AuthenticatedPartition::derive(&self.plan.service, context);
        if self.authority.logical_scope != partition.logical_scope
            || self.authority.tenant != partition.physical_tenant
        {
            return Err("opened Eventlog authority does not match authenticated partition".into());
        }
        let snapshot = self
            .bridge
            .complete_snapshot(&self.authority.logical_scope, self.wait)
            .map_err(|error| format!("{error:?}"))?;
        let result = project_query_v4(
            &self.plan,
            operation,
            body,
            snapshot
                .histories
                .into_iter()
                .map(|history| history.terminal),
            page.cursor(),
            page.limit(),
            |instance, obligations| host.visible(instance, obligations, context, &facts),
        )
        .map_err(|error| error.to_string())?;
        Ok(QueryResultV4 {
            items: result.items,
            through_version: result.through_version,
            next_cursor: result.next_cursor,
            partial: result.partial,
        })
    }
}

#[derive(Clone)]
struct HttpStateV4 {
    plan: Arc<ServicePlanV4>,
    backend: Arc<dyn ServiceBackendV4>,
    identity: IdentityClient,
    audience: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WirePageV4 {
    #[serde(default)]
    cursor: Option<String>,
    limit: usize,
}

/// Initialized generated `/4` Identity HTTP service.
#[derive(Clone)]
pub struct IdentityHttpServiceV4 {
    state: HttpStateV4,
}

impl IdentityHttpServiceV4 {
    /// Validates the exact generated plan and binds Identity verification to its audience.
    pub fn initialize(
        plan: ServicePlanV4,
        backend: Arc<dyn ServiceBackendV4>,
        identity_origin: &str,
        audience: &str,
    ) -> Result<Self, ServerError> {
        let identity =
            IdentityClient::new(identity_origin, audience).map_err(ServerError::Identity)?;
        Ok(Self {
            state: HttpStateV4 {
                plan: Arc::new(plan),
                backend,
                identity,
                audience: audience.to_owned(),
            },
        })
    }

    /// Builds probes and the unchanged generated operation routes.
    pub fn router(self) -> Router {
        Router::new()
            .route("/healthz", get(health))
            .route("/readyz", get(ready))
            .route("/v1/intents/{operation}", post(intent))
            .route("/v1/queries/{operation}", post(query))
            .with_state(self.state)
    }
}

async fn health() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn ready(State(state): State<HttpStateV4>) -> StatusCode {
    if state.backend.readiness().is_ok() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn intent(
    State(state): State<HttpStateV4>,
    Path(operation): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(plan) = state.plan.intents.get(&operation) else {
        return problem(
            StatusCode::NOT_FOUND,
            "unknown_operation",
            "unknown generated operation",
        );
    };
    let (context, facts) =
        match authorize_parts(&state.identity, &state.audience, &headers, &plan.scope).await {
            Ok(value) => value,
            Err(response) => return *response,
        };
    let recording =
        match request_recording(&state.plan.service, &operation, &context, &headers, &body) {
            Ok(recording) => recording,
            Err(response) => return *response,
        };
    match state
        .backend
        .intent(&context, facts, &operation, &body, recording)
    {
        Ok(MutationResultV4::Refused(refusal)) => (
            StatusCode::CONFLICT,
            Json(Problem {
                code: refusal.error,
                status: StatusCode::CONFLICT.as_u16(),
                detail: "Entity Runtime refused the operation".into(),
            }),
        )
            .into_response(),
        Ok(MutationResultV4::Committed {
            decision,
            receipt,
            replayed,
        }) => Json(MutationReceiptV4 {
            outcome: decision.record.outcome,
            response: decision.record.response,
            events: decision.events,
            through_version: decision.instance.revision,
            commit: receipt,
            replayed,
        })
        .into_response(),
        Err(ExecutionErrorV4::CommittedAftercare {
            receipt,
            repair_token,
            ..
        }) => (
            StatusCode::ACCEPTED,
            Json(CommittedAftercareV4 {
                commit: *receipt,
                repair_token,
                code: "committed_aftercare_pending".into(),
            }),
        )
            .into_response(),
        Err(error) => execution_problem_v4(&error),
    }
}

async fn query(
    State(state): State<HttpStateV4>,
    Path(operation): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(plan) = state.plan.queries.get(&operation) else {
        return problem(
            StatusCode::NOT_FOUND,
            "unknown_operation",
            "unknown generated operation",
        );
    };
    let (context, facts) =
        match authorize_parts(&state.identity, &state.audience, &headers, &plan.scope).await {
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
    let page = match object.remove("$page") {
        Some(value) => serde_json::from_value::<WirePageV4>(value)
            .ok()
            .and_then(|page| PageRequest::new(page.cursor, page.limit).ok()),
        None => PageRequest::new(None, 100).ok(),
    };
    let Some(page) = page else {
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
        .backend
        .query(&context, facts, &operation, &application_body, page)
    {
        Ok(result) => Json(result).into_response(),
        Err(_) => problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "projection_unavailable",
            "the authoritative projection is unavailable",
        ),
    }
}

async fn authorize_parts(
    identity: &IdentityClient,
    audience: &str,
    headers: &HeaderMap,
    scope: &str,
) -> Result<(VerifiedAuthContext, AuthorityFacts), Box<Response>> {
    authorize_identity(identity, audience, headers, scope).await
}

fn request_recording(
    service: &str,
    operation: &str,
    context: &VerifiedAuthContext,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<Recording, Box<Response>> {
    let record_id = headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
        .map_or_else(
            || {
                let mut digest = Sha256::new();
                for part in [
                    service.as_bytes(),
                    operation.as_bytes(),
                    context.tenant().as_str().as_bytes(),
                    context.authority().as_str().as_bytes(),
                    body,
                ] {
                    digest.update((part.len() as u64).to_be_bytes());
                    digest.update(part);
                }
                format!("sdk-http-v4-{}", hex::encode(digest.finalize()))
            },
            str::to_owned,
        );
    let recorded_at = OffsetDateTime::now_utc().format(&Rfc3339).map_err(|_| {
        Box::new(problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "clock",
            "host clock is invalid",
        ))
    })?;
    Ok(Recording {
        record_id,
        recorded_at,
        correlation: headers
            .get("traceparent")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        causation: None,
        actor: Some(context.user().as_str().to_owned()),
    })
}
