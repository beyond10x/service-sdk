//! Real `SQLite` recorded-adapter restart acceptance for the generated billing service.

use std::{
    collections::BTreeMap,
    num::NonZeroU16,
    path::{Path, PathBuf},
    sync::Arc,
};

use axum::{
    Json, Router,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
};
use entity_core::{Decision, EntityInstance, Registry};
use entity_eventlog::{
    AsyncBindingProvisioner, Authority, ErRecordedProjector, EventlogBackend,
    EventlogBindingProvisioner, EventlogOperationContext,
    sync::{
        BridgeConfig, CallWait, EventlogRecordedStoreOwner, RecordedEventlogBridge, ShutdownMode,
        ShutdownOutcome,
    },
};
use entity_store::{Recording, asynchronous::CommitReceipt};
use eventlog_core::{CaptureLimits, EventStore, InlineProjectionAdmin, TenantId};
use serde_json::{Map, Value, json};
use service_builder::ess::EssSources;
use service_definition::v4::ServiceDefinitionV4;
use service_engine::{
    ObligationUse,
    v4::{
        AuthenticatedPartition, EngineV4, ExecutionErrorV4, IntentPlanV4, MutationResultV4,
        ServicePlanV4, StagedContentV4,
    },
};
use service_eventlog::{
    AuthorityFacts,
    v4::{EventlogResourcesV4, HostResourcesV4},
};
use service_http::v4::{IdentityHttpServiceV4, RecordedServiceBackendV4};
use service_runtime::{
    AuthorityId, TenantId as RuntimeTenantId, UserId, VerifiedAuthContext, VerifiedIdentity,
};
use time::OffsetDateTime;

const LIMITS: CaptureLimits = CaptureLimits {
    max_events: 256,
    max_blobs: 1024,
    max_projection_rows: 1024,
    max_payload_bytes: 8 * 1024 * 1024,
};

#[derive(Default)]
struct Host {
    uuid: usize,
    slot: usize,
    projections: usize,
    effects: usize,
    fail_projection: bool,
}

impl HostResourcesV4 for Host {
    fn authorize(&mut self, _: &VerifiedAuthContext, _: &str) -> Result<(), String> {
        Ok(())
    }

    fn trusted_clock(&mut self) -> Result<Value, String> {
        Ok(json!("2026-09-16T12:00:00Z"))
    }

    fn uuid_v7(&mut self) -> Result<Value, String> {
        self.uuid += 1;
        Ok(json!("018f7f4c-9b68-7abc-8def-0123456789ab"))
    }

    fn slot_obligation(
        &mut self,
        name: &str,
        input: &Map<String, Value>,
        _: &VerifiedAuthContext,
    ) -> Result<Option<Value>, String> {
        self.slot += 1;
        match name {
            "payee_from_email" => Ok(Some(json!({
                "kind": "person",
                "value": input.get("customer_email").cloned().ok_or("email")?,
            }))),
            other => Err(format!("unknown slot obligation {other}")),
        }
    }

    fn operation_field_obligation(
        &mut self,
        _: &str,
        _: &str,
        _: &Map<String, Value>,
        _: &EntityInstance,
        _: &VerifiedAuthContext,
    ) -> Result<Value, String> {
        Ok(json!("2026-09-16T12:00:00Z"))
    }

    fn stage_content(
        &mut self,
        _: &VerifiedAuthContext,
        _: &str,
        _: &str,
        _: &str,
        _: &[u8],
    ) -> Result<StagedContentV4, String> {
        Ok(StagedContentV4 {
            reference: "content-reference".into(),
            token: "content-token".into(),
        })
    }

    fn accept_content(&mut self, _: &VerifiedAuthContext, _: String) -> Result<(), String> {
        Ok(())
    }

    fn accept_recorded_content(
        &mut self,
        _: &VerifiedAuthContext,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    fn abandon_content(&mut self, _: &VerifiedAuthContext, _: String) -> Result<(), String> {
        Ok(())
    }

    fn project(
        &mut self,
        _: &VerifiedAuthContext,
        _: &IntentPlanV4,
        _: &Decision,
        _: &CommitReceipt,
    ) -> Result<(), String> {
        self.projections += 1;
        if self.fail_projection {
            Err("projection unavailable".to_owned())
        } else {
            Ok(())
        }
    }

    fn effects(
        &mut self,
        _: &VerifiedAuthContext,
        _: &IntentPlanV4,
        _: &Decision,
        _: &CommitReceipt,
    ) -> Result<(), String> {
        self.effects += 1;
        Ok(())
    }

    fn visible(
        &mut self,
        _: &EntityInstance,
        _: &[ObligationUse],
        _: &VerifiedAuthContext,
        _: &AuthorityFacts,
    ) -> Result<bool, String> {
        Ok(true)
    }
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/er-v4/billing")
}

fn sources() -> EssSources {
    let base = fixture().join("ess");
    let mut pending = vec![base.clone()];
    let mut paths = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "yaml")
            {
                paths.push(path);
            }
        }
    }
    paths.sort();
    EssSources::new(
        paths
            .into_iter()
            .map(|path| {
                let label = path.strip_prefix(&base).unwrap().display().to_string();
                let text = std::fs::read_to_string(path).unwrap();
                (label, text)
            })
            .collect::<BTreeMap<_, _>>(),
    )
    .unwrap()
}

fn plan() -> ServicePlanV4 {
    let definition = ServiceDefinitionV4::from_yaml(
        &std::fs::read_to_string(fixture().join("runtime.yaml")).unwrap(),
    )
    .unwrap();
    service_builder::build_service_v4(&sources(), &definition)
        .unwrap()
        .realization_plan
}

fn registry(plan: &ServicePlanV4) -> Registry {
    let mut registry = Registry::new();
    for definition in plan.er.definitions.values() {
        registry.register(definition.clone()).unwrap();
    }
    registry.validate_all().unwrap();
    registry
}

fn context() -> VerifiedAuthContext {
    VerifiedAuthContext::from_verified(VerifiedIdentity::after_verification(
        RuntimeTenantId::new("tenant-a").unwrap(),
        AuthorityId::new("account-a").unwrap(),
        UserId::new("user-a").unwrap(),
        None,
        None,
    ))
}

fn operation(label: &str) -> EventlogOperationContext {
    EventlogOperationContext {
        subject: "user-a".into(),
        actor: "billing-service".into(),
        request_id: format!("request-{label}"),
        trace_id: "trace-recorded-restart".into(),
        causation_id: None,
        causation_depth: 0,
        occurred_at: OffsetDateTime::from_unix_timestamp(1_789_531_200).unwrap(),
    }
}

fn recording(label: &str) -> Recording {
    Recording {
        record_id: format!("record-{label}"),
        recorded_at: "2026-09-16T12:00:00Z".into(),
        correlation: Some("recorded-restart".into()),
        causation: None,
        actor: Some("user-a".into()),
    }
}

fn owner(path: &str, prefix: &str, authority: Authority) -> EventlogRecordedStoreOwner {
    EventlogRecordedStoreOwner::Sqlite {
        path: path.to_owned(),
        prefix: prefix.to_owned(),
        authority,
        limits: LIMITS,
    }
}

async fn identity(headers: HeaderMap) -> impl IntoResponse {
    let token = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let Some(tenant) = token.strip_prefix("Bearer tenant-") else {
        return (
            StatusCode::UNAUTHORIZED,
            [("cache-control", "no-store"), ("pragma", "no-cache")],
            Json(json!({})),
        )
            .into_response();
    };
    let audience = headers
        .get("x-b10x-audience")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    (
        StatusCode::OK,
        [("cache-control", "no-store"), ("pragma", "no-cache")],
        Json(json!({
            "iss": "sdk-v4-proof",
            "sub": "account-a",
            "aud": audience,
            "iat": 1,
            "nbf": 1,
            "exp": 4_102_444_800_i64,
            "jti": "fixture-v4",
            "act": {"sub": "user-a"},
            "scope": "invoices.manage invoices.read",
            "principal_kind": "human",
            "tenant_id": tenant,
            "email": null,
            "groups": []
        })),
    )
        .into_response()
}

#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::too_many_lines)]
async fn complete_decision_and_observation_reopen_without_provider_reexecution() {
    let plan = plan();
    let context = context();
    let partition = AuthenticatedPartition::derive("billing", &context);
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("billing.sqlite");
    let database = database.to_str().unwrap().to_owned();
    let prefix = "sdk_er_billing_vfour";
    let tenant = TenantId::new(&partition.physical_tenant).unwrap();
    let backend = Arc::new(
        eventlog_sqlite::SqliteEventStore::open(&database, prefix)
            .await
            .unwrap(),
    );
    let stream_identity = backend.stream_identity(&tenant).await.unwrap();
    let projector = Arc::new(ErRecordedProjector::new());
    backend.create_projections(projector.clone()).await.unwrap();
    backend.attach_inline_existing(projector).await.unwrap();
    let authority = Authority {
        logical_scope: partition.logical_scope.clone(),
        tenant: partition.physical_tenant.clone(),
        stream_identity,
    };
    let erased: Arc<dyn EventlogBackend> = backend;
    EventlogBindingProvisioner::new(erased, LIMITS)
        .provision_binding(authority.clone(), operation("provision"))
        .await
        .unwrap();

    let mut bridge = RecordedEventlogBridge::start(
        registry(&plan),
        owner(&database, prefix, authority.clone()),
        BridgeConfig {
            queue_capacity: NonZeroU16::new(8).unwrap(),
        },
    )
    .unwrap();
    let engine = EngineV4::new(&plan).unwrap();
    let input = serde_json::to_vec(&json!({
        "account_id": "018f7f4c-9b68-7abc-8def-111111111111",
        "customer_email": "person@example.test",
        "amount": {"amount": 12.5, "currency": "EUR"}
    }))
    .unwrap();
    let mut first_host = Host {
        fail_projection: true,
        ..Host::default()
    };
    let mut first_resources = EventlogResourcesV4::new(
        &bridge,
        &authority,
        operation("first"),
        CallWait::Forever,
        &mut first_host,
    );
    let aftercare = engine
        .execute_json(
            &context,
            "create_invoice",
            &input,
            0,
            "create-1",
            recording("first"),
            &mut first_resources,
        )
        .unwrap_err();
    let ExecutionErrorV4::CommittedAftercare {
        receipt: aftercare_receipt,
        repair_token,
        ..
    } = aftercare
    else {
        panic!("projection failure did not retain committed evidence");
    };
    let first = engine
        .execute_json(
            &context,
            "create_invoice",
            &input,
            0,
            "create-1",
            recording("first-retry"),
            &mut first_resources,
        )
        .unwrap();
    let MutationResultV4::Committed {
        decision: first_decision,
        receipt: first_receipt,
        replayed,
    } = first
    else {
        panic!("creation unexpectedly refused");
    };
    assert!(replayed, "ordinary retry recovers the committed decision");
    assert_eq!(first_receipt, *aftercare_receipt);
    assert_eq!(
        (
            first_host.uuid,
            first_host.slot,
            first_host.projections,
            first_host.effects
        ),
        (1, 1, 1, 0)
    );
    assert_eq!(
        bridge.shutdown(ShutdownMode::Drain, CallWait::Forever),
        ShutdownOutcome::Joined { provider: Ok(()) },
    );

    let reopened = RecordedEventlogBridge::start(
        registry(&plan),
        owner(&database, prefix, authority.clone()),
        BridgeConfig {
            queue_capacity: NonZeroU16::new(8).unwrap(),
        },
    )
    .unwrap();
    let backend = Arc::new(
        RecordedServiceBackendV4::new(
            plan.clone(),
            reopened,
            authority,
            CallWait::Forever,
            Host::default(),
        )
        .unwrap(),
    );
    let identity_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let identity_origin = format!("http://{}", identity_listener.local_addr().unwrap());
    let identity_task = tokio::spawn(async move {
        axum::serve(
            identity_listener,
            Router::new().route("/v1/access-authority", get(identity)),
        )
        .await
        .unwrap();
    });
    let service = IdentityHttpServiceV4::initialize(
        plan,
        backend.clone(),
        &identity_origin,
        "urn:b10x:billing",
    )
    .unwrap();
    let service_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let service_origin = format!("http://{}", service_listener.local_addr().unwrap());
    let service_task = tokio::spawn(async move {
        axum::serve(service_listener, service.router())
            .await
            .unwrap();
    });
    let client = reqwest::Client::new();
    let unauthenticated = client
        .post(format!("{service_origin}/v1/intents/create_invoice"))
        .body("invalid JSON")
        .send()
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
    let public_input = json!({
        "account_id": "018f7f4c-9b68-7abc-8def-111111111111",
        "customer_email": "person@example.test",
        "amount": {"amount": 12.5, "currency": "EUR"},
        "idempotency_key": "create-1"
    });
    let retried = client
        .post(format!("{service_origin}/v1/intents/create_invoice"))
        .bearer_auth("tenant-tenant-a")
        .header("x-request-id", "record-retry")
        .json(&public_input)
        .send()
        .await
        .unwrap();
    let retried_status = retried.status();
    let retried_body = retried.bytes().await.unwrap();
    assert_eq!(
        retried_status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&retried_body)
    );
    let retried: Value = serde_json::from_slice(&retried_body).unwrap();
    assert_eq!(retried["replayed"], true);
    assert_eq!(retried["through_version"], first_decision.instance.revision);
    assert_eq!(
        retried["commit"],
        serde_json::to_value(&first_receipt).unwrap()
    );
    let unauthenticated_repair = client
        .post(format!("{service_origin}/v1/repairs/{repair_token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthenticated_repair.status(), StatusCode::UNAUTHORIZED);
    {
        let before_repair = backend.host().unwrap();
        assert_eq!(
            (
                before_repair.uuid,
                before_repair.slot,
                before_repair.projections,
                before_repair.effects
            ),
            (0, 0, 0, 0),
            "ordinary HTTP retry cannot redeliver committed aftercare"
        );
    }
    let repaired = client
        .post(format!("{service_origin}/v1/repairs/{repair_token}"))
        .bearer_auth("tenant-tenant-a")
        .header("x-request-id", "record-repair")
        .send()
        .await
        .unwrap();
    let repaired_status = repaired.status();
    let repaired_body = repaired.bytes().await.unwrap();
    assert_eq!(
        repaired_status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&repaired_body)
    );
    let repaired: Value = serde_json::from_slice(&repaired_body).unwrap();
    assert_eq!(repaired["replayed"], true);
    assert_eq!(
        repaired["commit"],
        serde_json::to_value(&first_receipt).unwrap()
    );
    let invoice_id = first_decision.instance.fields["invoice_id"].clone();
    let queried = client
        .post(format!("{service_origin}/v1/queries/get_invoice"))
        .bearer_auth("tenant-tenant-a")
        .json(&json!({"invoice_id": invoice_id, "$page": {"limit": 100}}))
        .send()
        .await
        .unwrap();
    assert_eq!(queried.status(), StatusCode::OK);
    let queried: Value = queried.json().await.unwrap();
    assert_eq!(queried["items"].as_array().unwrap().len(), 1);
    let restarted_host = backend.host().unwrap();
    assert_eq!(
        (
            restarted_host.uuid,
            restarted_host.slot,
            restarted_host.projections,
            restarted_host.effects
        ),
        (0, 0, 1, 1)
    );
    drop(restarted_host);
    service_task.abort();
    identity_task.abort();
}
