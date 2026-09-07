//! Required, serial SDK persistence proof. Missing laboratory inputs are failures.
#![allow(clippy::too_many_lines, clippy::too_many_arguments)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::IntoFuture;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail, ensure};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
};
use connectors_protocol::operation::{
    DescribeRequest, InvokeRequest, OperationRequest, OperationResult,
};
use connectors_service::{
    ConnectorBackend, ConnectorServiceFactory, DeploymentApproval, DeploymentRisk,
    OperationDeployment, PrincipalContext, ProviderIdentity, ServiceDeployment,
};
use eventlog_core::{EventStore, TenantId};
use eventlog_postgres::{PoolOptions, PostgresConfig};
use serde_json::{Value, json};
use service_connectors::{AuthorityFacts, AuthorityFactsError, AuthorityFactsResolver};
use service_engine::RequestMetadata;
use service_eventlog::{EventFeedAuthorizer, EventlogService};
use service_host::{ConnectionBudget, Persistence};
use service_runtime::{AuthorityId, UserId, VerifiedAuthContext, VerifiedIdentity};
use service_runtime::{
    ClaimDisposition, EffectClaim, EffectJournal, EffectOutcome, EffectPlan, EffectRisk,
    EffectState,
};
use uuid::Uuid;

#[path = "fixtures/persistence/workload.rs"]
mod workload;

const HTTP_SERVICE: &str = "persistence_http";
const FACTORY_SERVICE: &str = "persistence_factory";
const ROSTER: &[&str] = &[HTTP_SERVICE, FACTORY_SERVICE];

fn main() -> Result<()> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?
        .block_on(async {
            if arguments == ["--workload-child"] {
                workload::child().await
            } else if arguments == ["--pressure-scheduler-probe"] {
                let fixture = Fixture::required()?;
                fixture.prepare().await?;
                println!("running 1 isolated pressure scheduler case");
                match pressure_case(&fixture, true).await {
                    Ok(()) => {
                        println!("test pressure_cancellation_tracks_admission_under_reordered_dispatch ... ok");
                        println!("test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out");
                        Ok(())
                    }
                    Err(error) => {
                        println!("test pressure_cancellation_tracks_admission_under_reordered_dispatch ... FAILED: {error:#}");
                        println!("test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out");
                        Err(error)
                    }
                }
            } else if arguments == ["--readiness-probe"] {
                let mut fixture = Fixture::required()?;
                fixture.prepare().await?;
                println!("running 1 isolated readiness case");
                match factory_readiness_case(&mut fixture).await {
                    Ok(()) => {
                        println!("test generated_factory_readiness_tracks_database_loss_and_drain ... ok");
                        println!("test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out");
                        Ok(())
                    }
                    Err(error) => {
                        println!("test generated_factory_readiness_tracks_database_loss_and_drain ... FAILED: {error:#}");
                        println!("test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out");
                        Err(error)
                    }
                }
            } else {
                ensure!(
                    arguments.is_empty() || arguments == ["--nocapture"],
                    "unsupported proof selection; required cases cannot be filtered"
                );
                run().await
            }
        })
}

async fn run() -> Result<()> {
    let mut fixture = Fixture::required()?;
    println!("SDK persistence required inputs: {}", fixture.schema);
    let mut passed = 0;
    macro_rules! case {
        ($name:literal, $future:expr) => {{
            let started = Instant::now();
            match $future.await {
                Ok(()) => { passed += 1; println!("test {} ... ok ({:?})", $name, started.elapsed()); }
                Err(error) => {
                    println!("test {} ... FAILED: {error:#}", $name);
                    println!("test result: FAILED. {passed} passed; 1 failed; 0 ignored; 0 measured; 0 filtered out");
                    return Err(error);
                }
            }
        }};
    }
    println!("running 10 required persistence cases");
    case!(
        "steady_dispatch_gate_prevents_catch_up_bursts",
        workload::verify_dispatch_gate()
    );
    case!(
        "file_sqlite_factory_restart_replay_and_exact_partitions",
        factory_case(&fixture, false)
    );
    fixture.prepare().await?;
    case!(
        "generated_factory_readiness_tracks_database_loss_and_drain",
        factory_readiness_case(&mut fixture)
    );
    case!(
        "postgres_factory_restart_replay_and_exact_roster",
        factory_case(&fixture, true)
    );
    case!(
        "file_sqlite_standalone_process_restart_and_authentication",
        standalone_case(&mut fixture, false)
    );
    case!(
        "postgres_standalone_process_database_restart_and_readiness",
        standalone_case(&mut fixture, true)
    );
    case!(
        "file_sqlite_event_pages_content_and_effect_recovery",
        reader_effect_case(&fixture, false)
    );
    case!(
        "postgres_event_pages_content_and_effect_recovery",
        reader_effect_case(&fixture, true)
    );
    case!(
        "postgres_saturation_cancellation_reconnect_and_bounded_drain",
        // Keep the controlled ordering in the required roster, so CI exercises the regression.
        pressure_case(&fixture, true)
    );
    case!(
        "postgres_two_process_sdk_workload_six_configurations",
        workload::measure(&fixture)
    );
    ensure!(
        passed == 10,
        "required persistence case selection was incomplete"
    );
    println!("test result: ok. {passed} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out");
    Ok(())
}

#[derive(Clone)]
struct Fixture {
    app_url: String,
    migration_url: String,
    ca: String,
    schema: String,
    container: String,
    directory: PathBuf,
}

impl Fixture {
    fn required() -> Result<Self> {
        let required = |name| {
            std::env::var(name)
                .with_context(|| format!("{name} is required; the PostgreSQL lane never skips"))
        };
        let app_url = required("EVENTLOG_TEST_HOSTED_POSTGRES_URL")?;
        let migration_url = required("EVENTLOG_TEST_POSTGRES_MIGRATION_URL")?;
        let ca = std::fs::read_to_string(required("EVENTLOG_TEST_POSTGRES_CA")?)?;
        let container = required("EVENTLOG_TEST_POSTGRES_CONTAINER")?;
        ensure!(
            !container.is_empty()
                && container
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "invalid disposable fixture name"
        );
        let schema = std::env::var("SDK_PROOF_SCHEMA")
            .unwrap_or_else(|_| format!("sdk_{}", Uuid::now_v7().simple()));
        ensure!(
            schema.starts_with("sdk_")
                && schema.len() < 64
                && schema
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_'),
            "invalid proof schema"
        );
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/sdk-persistence-proof")
            .join(&schema);
        std::fs::create_dir_all(&directory)?;
        Ok(Self {
            app_url,
            migration_url,
            ca,
            schema,
            container,
            directory,
        })
    }

    fn config(&self, migration: bool) -> Result<PostgresConfig> {
        let mut roots = rustls::RootCertStore::empty();
        for cert in rustls_pemfile::certs(&mut self.ca.as_bytes()) {
            roots.add(cert?)?;
        }
        Ok(PostgresConfig::verified(
            if migration {
                &self.migration_url
            } else {
                &self.app_url
            },
            &self.schema,
            HTTP_SERVICE,
            roots,
        )?)
    }

    async fn prepare(&self) -> Result<()> {
        let sql = format!(
            "CREATE SCHEMA {schema}; GRANT USAGE ON SCHEMA {schema} TO eventlog_test_application; ALTER DEFAULT PRIVILEGES IN SCHEMA {schema} GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO eventlog_test_application; ALTER DEFAULT PRIVILEGES IN SCHEMA {schema} GRANT USAGE, SELECT ON SEQUENCES TO eventlog_test_application;",
            schema = self.schema
        );
        self.sql(&sql)?;
        Persistence::migrate_postgres(self.config(true)?, pool(), ROSTER).await?;
        Ok(())
    }

    fn sql(&self, sql: &str) -> Result<String> {
        let output = Command::new("psql")
            .args([
                "--no-psqlrc",
                "--set",
                "ON_ERROR_STOP=1",
                "--tuples-only",
                "--no-align",
                "--dbname",
                &self.migration_url,
                "--command",
                sql,
            ])
            .output()?;
        ensure!(
            output.status.success(),
            "fixture SQL failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout)?;
        println!("fixture SQL: {sql}\n{text}");
        Ok(text)
    }

    async fn wait_database(&self) -> Result<()> {
        let start = Instant::now();
        loop {
            let output = Command::new("psql")
                .env("PGCONNECT_TIMEOUT", "2")
                .args([
                    "--no-psqlrc",
                    "--tuples-only",
                    "--no-align",
                    "--dbname",
                    &self.migration_url,
                    "--command",
                    "SELECT 1",
                ])
                .output()?;
            println!(
                "database-ready probe: {} {} {}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            if output.status.success() {
                return Ok(());
            }
            ensure!(
                start.elapsed() <= Duration::from_secs(15),
                "disposable database did not recover"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn open(&self, postgres: bool, services: &[&str]) -> Result<Persistence> {
        if postgres {
            Persistence::open_postgres(self.config(false)?, pool(), budget(), services).await
        } else {
            Persistence::open_sqlite(
                self.directory
                    .join("factory.sqlite3")
                    .to_str()
                    .context("path encoding")?,
                HTTP_SERVICE,
                services,
            )
            .await
        }
    }

    async fn start_database(&mut self) -> Result<()> {
        ensure!(
            Command::new("docker")
                .args(["start", &self.container])
                .status()?
                .success(),
            "fixture start failed"
        );
        let output = Command::new("docker")
            .args(["port", &self.container, "5432/tcp"])
            .output()?;
        ensure!(output.status.success(), "port rediscovery failed");
        let endpoint = String::from_utf8(output.stdout)?.trim().to_owned();
        println!("rediscovered PostgreSQL address: {endpoint}");
        let (host, port) = endpoint.rsplit_once(':').context("loopback endpoint")?;
        ensure!(host == "127.0.0.1", "fixture escaped loopback");
        let port = port.parse()?;
        for value in [&mut self.app_url, &mut self.migration_url] {
            let mut url = reqwest::Url::parse(value)?;
            url.set_port(Some(port))
                .map_err(|()| anyhow::anyhow!("invalid fixture port"))?;
            *value = url.to_string();
        }
        self.wait_database().await
    }

    fn environment(
        &self,
        postgres: bool,
        listen: &str,
        identity: &str,
    ) -> BTreeMap<String, String> {
        let values = [
            ("LISTEN", listen.to_owned()),
            ("IDENTITY_ORIGIN", identity.to_owned()),
            (
                "DATABASE_PATH",
                self.directory
                    .join("standalone.sqlite3")
                    .display()
                    .to_string(),
            ),
            (
                "PERSISTENCE",
                if postgres { "postgres" } else { "sqlite" }.to_owned(),
            ),
            ("POSTGRES_URL", self.app_url.clone()),
            ("POSTGRES_MIGRATION_URL", self.migration_url.clone()),
            ("POSTGRES_SCHEMA", self.schema.clone()),
            ("POSTGRES_CA_PEM", self.ca.clone()),
            ("POOL_MAX", "2".into()),
            ("POOL_WAITERS", "4".into()),
            ("ACQUISITION_MS", "250".into()),
            ("CONNECT_MS", "2000".into()),
            ("STATEMENT_MS", "2000".into()),
            ("LOCK_MS", "500".into()),
            ("TRANSACTION_MS", "3000".into()),
            ("SHUTDOWN_MS", "2000".into()),
            ("DRAIN_MS", "2000".into()),
            ("DATABASE_CONNECTIONS", "8".into()),
            ("REPLICAS", "2".into()),
            ("RESERVED_CONNECTIONS", "4".into()),
        ];
        values
            .into_iter()
            .map(|(name, value)| (format!("PERSISTENCE_HTTP_{name}"), value))
            .collect()
    }
}

fn pool() -> PoolOptions {
    PoolOptions {
        max_connections: 2,
        max_waiters: 4,
        acquisition_timeout: Duration::from_millis(250),
        connect_timeout: Duration::from_secs(2),
        statement_timeout: Duration::from_secs(2),
        lock_timeout: Duration::from_millis(500),
        transaction_timeout: Duration::from_secs(3),
        shutdown_timeout: Duration::from_secs(2),
    }
}
fn budget() -> ConnectionBudget {
    ConnectionBudget {
        database_connections: 8,
        replicas: 2,
        reserved_connections: 4,
    }
}
fn context(tenant: &str, realm: Option<&str>) -> VerifiedAuthContext {
    VerifiedAuthContext::from_verified(VerifiedIdentity::after_verification(
        service_runtime::TenantId::new(tenant).unwrap(),
        AuthorityId::new("owner").unwrap(),
        UserId::new("owner").unwrap(),
        None,
        realm.map(|realm| service_runtime::RealmId::new(realm).unwrap()),
    ))
}
fn principal(tenant: &str, realm: Option<&str>) -> PrincipalContext {
    PrincipalContext::hosted(
        tenant.to_owned(),
        "owner".to_owned(),
        "owner".to_owned(),
        None,
        "proof".to_owned(),
        "a".repeat(64),
    )
    .unwrap()
    .with_verified_realm(realm.map(str::to_owned))
    .unwrap()
}
fn facts() -> AuthorityFacts {
    AuthorityFacts {
        teams: BTreeSet::from(["engineering".into()]),
        ..AuthorityFacts::default()
    }
}
fn create(key: &str) -> Value {
    json!({"key": key, "scopes": {"team": "engineering"}, "content": {"media_type": "text/plain", "text": "x".repeat(4096)}})
}

#[derive(Clone)]
struct Authority(Arc<AtomicBool>);
#[async_trait::async_trait]
impl AuthorityFactsResolver for Authority {
    async fn resolve(
        &self,
        _context: &PrincipalContext,
    ) -> Result<AuthorityFacts, AuthorityFactsError> {
        Ok(if self.0.load(Ordering::Acquire) {
            facts()
        } else {
            AuthorityFacts::default()
        })
    }
}

async fn bind(
    persistence: &Persistence,
    authority: Authority,
    http: bool,
) -> Result<Arc<dyn ConnectorBackend>> {
    let factory = if http {
        persistence_http_generated_service::connector_factory_with_authority(
            persistence.store(),
            Arc::new(authority),
        )?
    } else {
        persistence_factory_generated_service::connector_factory_with_authority(
            persistence.store(),
            Arc::new(authority),
        )?
    };
    let manifest = factory.manifest();
    let deployment = ServiceDeployment {
        service_ref: manifest.service_ref,
        provider: ProviderIdentity {
            provider_ref: "provider:sdk-proof".into(),
            authority: "dev.b10x.sdk-proof".into(),
            connection_ref: "connection:sdk-proof".into(),
        },
        operations: manifest
            .operations
            .into_iter()
            .map(|operation| {
                (
                    operation.operation_ref,
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
    let (backend, _) = factory.bind(&deployment).await?.into_parts();
    Ok(backend)
}

async fn invoke(
    backend: &Arc<dyn ConnectorBackend>,
    context: &PrincipalContext,
    service: &str,
    operation: &str,
    input: Value,
) -> Result<Value> {
    let operation_ref = format!("{service}.{operation}");
    let OperationResult::Describe(description) = backend
        .handle(
            context,
            OperationRequest::Describe(DescribeRequest {
                operation_ref: operation_ref.clone(),
            }),
        )
        .await
        .map_err(|e| anyhow::anyhow!("describe: {:?}", e.code))?
    else {
        bail!("wrong description variant");
    };
    let OperationResult::Invoke(result) = backend
        .handle(
            context,
            OperationRequest::Invoke(InvokeRequest {
                operation_ref,
                connection_ref: "connection:sdk-proof".into(),
                description_ref: description.description_ref,
                input,
                approval_evidence_ref: None,
            }),
        )
        .await
        .map_err(|e| anyhow::anyhow!("invoke: {:?}", e.code))?
    else {
        bail!("wrong invocation variant");
    };
    Ok(result.output)
}

fn same_receipt(first: &Value, replay: &Value) -> Result<()> {
    ensure!(
        replay["replayed"] == true,
        "retry was not a replay: {replay}"
    );
    ensure!(
        first["events"] == replay["events"],
        "replay changed original events"
    );
    ensure!(
        first["through_version"] == replay["through_version"],
        "replay changed accepted version"
    );
    Ok(())
}
fn id(receipt: &Value) -> Result<&str> {
    receipt["events"][0]["fields"]["id"]
        .as_str()
        .context("receipt has no original UUID")
}

async fn factory_readiness_case(fixture: &mut Fixture) -> Result<()> {
    use connectors_service::BackendReadinessError;
    let persistence = fixture.open(true, ROSTER).await?;
    let authority = Authority(Arc::new(AtomicBool::new(true)));
    let factory = bind(&persistence, authority.clone(), false).await?;
    let http_factory = bind(&persistence, authority.clone(), true).await?;
    persistence.seal().await?;
    factory.ready().await?;
    http_factory.ready().await?;
    ensure!(
        Command::new("docker")
            .args(["stop", "--time", "2", &fixture.container])
            .status()?
            .success(),
        "fixture stop failed"
    );
    let started = Instant::now();
    let factory_unavailable = factory.ready().await;
    let factory_duration = started.elapsed();
    let started = Instant::now();
    let http_unavailable = http_factory.ready().await;
    let http_duration = started.elapsed();
    println!(
        "generated factory readiness during database loss: factory={factory_unavailable:?} ({factory_duration:?}), standalone-plan factory={http_unavailable:?} ({http_duration:?})"
    );
    // Restore this disposable fixture before evaluating the regression's captured result.
    fixture.start_database().await?;
    persistence.shutdown().await?;
    let recovered = fixture.open(true, ROSTER).await?;
    let factory = bind(&recovered, authority.clone(), false).await?;
    let http_factory = bind(&recovered, authority, true).await?;
    recovered.seal().await?;
    factory.ready().await?;
    http_factory.ready().await?;
    recovered.begin_drain();
    let factory_draining = factory.ready().await;
    let http_draining = http_factory.ready().await;
    recovered.shutdown().await?;
    ensure!(
        factory_unavailable == Err(BackendReadinessError)
            && http_unavailable == Err(BackendReadinessError),
        "generated Connector backends claimed ready while PostgreSQL was stopped"
    );
    ensure!(
        factory_duration <= Duration::from_millis(3500)
            && http_duration <= Duration::from_millis(3500),
        "factory readiness exceeded bounded refusal time"
    );
    ensure!(
        factory_draining == Err(BackendReadinessError)
            && http_draining == Err(BackendReadinessError),
        "generated Connector backends claimed ready while persistence was draining"
    );
    Ok(())
}

async fn factory_case(fixture: &Fixture, postgres: bool) -> Result<()> {
    let admitted = Arc::new(AtomicBool::new(true));
    let persistence = fixture.open(postgres, ROSTER).await?;
    ensure!(
        persistence.readiness().await.is_err(),
        "unsealed store accepted traffic"
    );
    ensure!(
        persistence.seal().await.is_err(),
        "missing complete roster was accepted"
    );
    let factory = bind(&persistence, Authority(admitted.clone()), false).await?;
    ensure!(
        persistence.seal().await.is_err(),
        "one service sealed a two-service store"
    );
    let http_factory = bind(&persistence, Authority(admitted.clone()), true).await?;
    persistence.seal().await?;
    ensure!(persistence.seal().await.is_err(), "second seal succeeded");
    ensure!(
        bind(&persistence, Authority(admitted.clone()), false)
            .await
            .is_err(),
        "late registration succeeded"
    );
    persistence.readiness().await?;
    let request = create("lost-create");
    let caller = principal("factory-a", None);
    let first = invoke(
        &factory,
        &caller,
        FACTORY_SERVICE,
        "create",
        request.clone(),
    )
    .await?;
    let close = json!({"id": id(&first)?, "key": "lost-close", "version": 1});
    let closed = invoke(&factory, &caller, FACTORY_SERVICE, "close", close.clone()).await?;
    admitted.store(false, Ordering::Release);
    ensure!(
        invoke(
            &factory,
            &caller,
            FACTORY_SERVICE,
            "create",
            request.clone()
        )
        .await
        .is_err(),
        "revoked retry returned historical content"
    );
    ensure!(
        invoke(&factory, &caller, FACTORY_SERVICE, "close", close.clone())
            .await
            .is_err(),
        "revoked transition retry succeeded"
    );
    let invisible = invoke(
        &factory,
        &caller,
        FACTORY_SERVICE,
        "list",
        json!({"$page": {"limit": 32}}),
    )
    .await?;
    ensure!(
        invisible["items"]
            .as_array()
            .context("query page")?
            .is_empty(),
        "revoked page disclosed rows"
    );
    admitted.store(true, Ordering::Release);
    same_receipt(
        &first,
        &invoke(
            &factory,
            &caller,
            FACTORY_SERVICE,
            "create",
            request.clone(),
        )
        .await?,
    )?;
    same_receipt(
        &closed,
        &invoke(&factory, &caller, FACTORY_SERVICE, "close", close.clone()).await?,
    )?;
    let mut changed = request.clone();
    changed["content"]["text"] = json!("changed");
    ensure!(
        invoke(&factory, &caller, FACTORY_SERVICE, "create", changed)
            .await
            .is_err(),
        "changed intent reused a claim"
    );
    for (tenant, realm) in [
        ("factory-a", Some("default")),
        ("factory-b", None),
        ("factory-b", Some("default")),
    ] {
        let other = invoke(
            &factory,
            &principal(tenant, realm),
            FACTORY_SERVICE,
            "create",
            request.clone(),
        )
        .await?;
        ensure!(id(&other)? != id(&first)?, "partitions shared an aggregate");
        let forbidden = invoke(
            &factory,
            &principal(tenant, realm),
            FACTORY_SERVICE,
            "get",
            json!({"id": id(&first)?}),
        )
        .await?;
        ensure!(
            forbidden.as_array().context("query rows")?.is_empty(),
            "cross-partition query disclosed original document"
        );
    }
    let other_service = invoke(
        &http_factory,
        &caller,
        HTTP_SERVICE,
        "create",
        request.clone(),
    )
    .await?;
    ensure!(
        id(&other_service)? != id(&first)?,
        "services shared a claim"
    );
    let tenant = TenantId::new("factory-a")?;
    let blob = first["events"][0]["fields"]["content_ref"]
        .as_str()
        .context("content reference")?
        .strip_prefix("content:")
        .context("content prefix")?;
    ensure!(
        persistence
            .store()
            .get_blob(&tenant, blob)
            .await?
            .context("retained content")?
            == vec![b'x'; 4096],
        "content bytes changed"
    );
    persistence.shutdown().await?;
    drop(factory);
    drop(http_factory);
    drop(persistence);
    let reopened = fixture.open(postgres, ROSTER).await?;
    let factory = bind(&reopened, Authority(admitted.clone()), false).await?;
    let _http = bind(&reopened, Authority(admitted), true).await?;
    reopened.seal().await?;
    same_receipt(
        &first,
        &invoke(&factory, &caller, FACTORY_SERVICE, "create", request).await?,
    )?;
    same_receipt(
        &closed,
        &invoke(&factory, &caller, FACTORY_SERVICE, "close", close).await?,
    )?;
    ensure!(
        reopened
            .store()
            .get_blob(&tenant, blob)
            .await?
            .context("reopened content")?
            == vec![b'x'; 4096],
        "restart changed content"
    );
    reopened.begin_drain();
    ensure!(
        reopened.readiness().await.is_err(),
        "draining store remained ready"
    );
    reopened.shutdown().await?;
    Ok(())
}

#[derive(Clone)]
struct IdentityState {
    admitted: Arc<AtomicBool>,
}
async fn identity(State(state): State<IdentityState>, headers: HeaderMap) -> impl IntoResponse {
    let token = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let Some(tenant) = token.strip_prefix("Bearer tenant-") else {
        return (
            StatusCode::UNAUTHORIZED,
            [("cache-control", "no-store"), ("pragma", "no-cache")],
            Json(json!({})),
        );
    };
    let audience = headers
        .get("x-b10x-audience")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let groups = if state.admitted.load(Ordering::Acquire) {
        vec!["engineering"]
    } else {
        vec![]
    };
    (
        StatusCode::OK,
        [("cache-control", "no-store"), ("pragma", "no-cache")],
        Json(json!({
            "iss": "sdk-proof", "sub": "owner", "aud": audience, "iat": 1, "nbf": 1,
            "exp": 4_102_444_800_i64, "jti": "fixture", "act": {"sub": "owner"}, "scope": "documents.manage documents.read",
            "principal_kind": "human", "tenant_id": tenant, "email": null, "groups": groups
        })),
    )
}

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
impl Process {
    async fn stop(&mut self) -> Result<()> {
        ensure!(
            Command::new("kill")
                .args(["-TERM", &self.0.id().to_string()])
                .status()?
                .success(),
            "SIGTERM failed"
        );
        let start = Instant::now();
        loop {
            if let Some(status) = self.0.try_wait()? {
                ensure!(status.success(), "generated process failed: {status}");
                return Ok(());
            }
            ensure!(
                start.elapsed() <= Duration::from_millis(4500),
                "drain/shutdown exceeded 4.5 seconds"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}
fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/debug/persistence-http-generated-service")
}
fn spawn(fixture: &Fixture, postgres: bool, address: &str, identity: &str) -> Result<Process> {
    ensure!(
        binary().is_file(),
        "generated standalone binary missing; run cargo build -p persistence-http-generated-service first"
    );
    let log = std::fs::File::create(fixture.directory.join(format!(
        "standalone-{}-{}.log",
        if postgres { "pg" } else { "sqlite" },
        Uuid::now_v7()
    )))?;
    let mut command = Command::new(binary());
    command
        .envs(fixture.environment(postgres, address, identity))
        .env_remove("PERSISTENCE_HTTP_POSTGRES_MIGRATION_URL")
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    Ok(Process(command.spawn()?))
}
async fn ready(client: &reqwest::Client, origin: &str) -> Result<()> {
    let start = Instant::now();
    loop {
        if client
            .get(format!("{origin}/readyz"))
            .send()
            .await
            .is_ok_and(|r| r.status() == StatusCode::NO_CONTENT)
        {
            return Ok(());
        }
        ensure!(
            start.elapsed() < Duration::from_secs(15),
            "generated process never became ready"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
async fn http(
    client: &reqwest::Client,
    origin: &str,
    operation: &str,
    input: &Value,
) -> Result<Value> {
    let response = client
        .post(format!("{origin}/v1/intents/{operation}"))
        .bearer_auth("tenant-standalone")
        .json(input)
        .send()
        .await?;
    let status = response.status();
    let body: Value = response.json().await?;
    ensure!(
        status.is_success(),
        "generated HTTP refusal {status}: {body}"
    );
    Ok(body)
}
async fn standalone_case(fixture: &mut Fixture, postgres: bool) -> Result<()> {
    let admitted = Arc::new(AtomicBool::new(true));
    let identity_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let identity_origin = format!("http://{}", identity_listener.local_addr()?);
    let identity_task = tokio::spawn(
        axum::serve(
            identity_listener,
            Router::new()
                .route("/v1/access-authority", get(identity))
                .with_state(IdentityState {
                    admitted: admitted.clone(),
                }),
        )
        .into_future(),
    );
    let port = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = port.local_addr()?.to_string();
    drop(port);
    let origin = format!("http://{address}");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()?;
    let mut process = spawn(fixture, postgres, &address, &identity_origin)?;
    ready(&client, &origin).await?;
    let unauthenticated = client
        .post(format!("{origin}/v1/intents/create"))
        .body("invalid JSON")
        .send()
        .await?;
    ensure!(
        unauthenticated.status() == StatusCode::UNAUTHORIZED,
        "body decoded before authentication"
    );
    let request = create("standalone-lost-create");
    let first = http(&client, &origin, "create", &request).await?;
    let abandoned = create("abandoned-success-response");
    let unread = client
        .post(format!("{origin}/v1/intents/create"))
        .bearer_auth("tenant-standalone")
        .json(&abandoned)
        .send()
        .await?;
    ensure!(
        unread.status().is_success(),
        "abandoned response did not commit"
    );
    drop(unread);
    let recovered = http(&client, &origin, "create", &abandoned).await?;
    ensure!(
        recovered["replayed"] == true,
        "successful unread response was not recoverable"
    );
    same_receipt(
        &recovered,
        &http(&client, &origin, "create", &abandoned).await?,
    )?;
    let close = json!({"id": id(&first)?, "version": 1, "key": "standalone-lost-close"});
    let closed = http(&client, &origin, "close", &close).await?;
    admitted.store(false, Ordering::Release);
    ensure!(
        http(&client, &origin, "create", &request).await.is_err(),
        "HTTP replay ignored current grants"
    );
    admitted.store(true, Ordering::Release);
    process.stop().await?;
    process = spawn(fixture, postgres, &address, &identity_origin)?;
    ready(&client, &origin).await?;
    same_receipt(&first, &http(&client, &origin, "create", &request).await?)?;
    same_receipt(&closed, &http(&client, &origin, "close", &close).await?)?;
    if postgres {
        standalone_pressure(fixture, &client, &origin, &request).await?;
        same_receipt(&first, &http(&client, &origin, "create", &request).await?)?;
        fixture.sql("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename = 'eventlog_test_application' AND pid <> pg_backend_pid()")?;
        ready(&client, &origin).await?;
        same_receipt(&first, &http(&client, &origin, "create", &request).await?)?;
        ensure!(
            Command::new("docker")
                .args(["stop", "--time", "2", &fixture.container])
                .status()?
                .success(),
            "fixture stop failed"
        );
        let unavailable = client.get(format!("{origin}/readyz")).send().await?;
        ensure!(
            unavailable.status() == StatusCode::SERVICE_UNAVAILABLE,
            "readiness ignored actual database loss"
        );
        ensure!(
            Command::new("docker")
                .args(["start", &fixture.container])
                .status()?
                .success(),
            "fixture start failed"
        );
        let port = Command::new("docker")
            .args(["port", &fixture.container, "5432/tcp"])
            .output()?;
        ensure!(port.status.success(), "port rediscovery failed");
        let port = String::from_utf8(port.stdout)?.trim().to_owned();
        println!("rediscovered PostgreSQL address: {port}");
        let endpoint_changed = !fixture.app_url.contains(&port);
        if endpoint_changed {
            process.stop().await?;
            let (host, port) = port
                .rsplit_once(':')
                .context("rediscovered loopback endpoint")?;
            ensure!(host == "127.0.0.1", "fixture escaped loopback");
            let port = port.parse()?;
            for value in [&mut fixture.app_url, &mut fixture.migration_url] {
                let mut url = reqwest::Url::parse(value)?;
                url.set_port(Some(port))
                    .map_err(|()| anyhow::anyhow!("invalid fixture port"))?;
                *value = url.to_string();
            }
            println!(
                "disposable Docker port changed; restarting generated process with rediscovered endpoint"
            );
        }
        fixture.wait_database().await?;
        let start = Instant::now();
        if endpoint_changed {
            process = spawn(fixture, true, &address, &identity_origin)?;
        }
        ready(&client, &origin).await?;
        same_receipt(&first, &http(&client, &origin, "create", &request).await?)?;
        same_receipt(&closed, &http(&client, &origin, "close", &close).await?)?;
        ensure!(
            start.elapsed() <= Duration::from_secs(15),
            "restart replay exceeded budget"
        );
        println!(
            "database restart readiness and replay: {:?}",
            start.elapsed()
        );
    }
    process.stop().await?;
    identity_task.abort();
    Ok(())
}

async fn lock_events(fixture: &Fixture, name: &str) -> Result<Process> {
    let lock_sql = format!(
        "BEGIN; LOCK TABLE {}.{}_events IN ACCESS EXCLUSIVE MODE; SELECT pg_sleep(2); COMMIT;",
        fixture.schema, HTTP_SERVICE
    );
    let lock_output = std::fs::File::create(fixture.directory.join(format!("{name}.stdout")))?;
    let lock_error = std::fs::File::create(fixture.directory.join(format!("{name}.stderr")))?;
    println!("pressure fixture SQL: {lock_sql}");
    let locker = Process(
        Command::new("psql")
            .args([
                "--no-psqlrc",
                "--set",
                "ON_ERROR_STOP=1",
                "--dbname",
                &fixture.migration_url,
                "--command",
                &lock_sql,
            ])
            .stdout(Stdio::from(lock_output))
            .stderr(Stdio::from(lock_error))
            .spawn()?,
    );
    let observed_lock = format!(
        "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE relation = '{}.{}_events'::regclass AND mode = 'AccessExclusiveLock' AND granted)",
        fixture.schema, HTTP_SERVICE
    );
    let begin = Instant::now();
    loop {
        if fixture.sql(&observed_lock)?.trim() == "t" {
            break;
        }
        ensure!(
            begin.elapsed() < Duration::from_secs(1),
            "pressure lock never became visible"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Ok(locker)
}

async fn standalone_pressure(
    fixture: &Fixture,
    client: &reqwest::Client,
    origin: &str,
    input: &Value,
) -> Result<()> {
    let mut locker = lock_events(fixture, "standalone-pressure-lock").await?;
    let mut requests = Vec::new();
    let started = Instant::now();
    for index in 0..32 {
        if index == 2 {
            let observed = Instant::now();
            loop {
                let count = fixture.sql("SELECT count(*) FROM pg_stat_activity WHERE usename = 'eventlog_test_application' AND wait_event_type = 'Lock'")?;
                if count.trim() == "2" {
                    break;
                }
                ensure!(
                    observed.elapsed() < Duration::from_millis(400),
                    "standalone requests did not occupy both database connections"
                );
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
        let client = client.clone();
        let endpoint = format!("{origin}/v1/intents/create");
        let input = input.clone();
        requests.push(tokio::spawn(async move {
            let started = Instant::now();
            let response = client
                .post(endpoint)
                .bearer_auth("tenant-standalone")
                .json(&input)
                .send()
                .await;
            (started.elapsed(), response)
        }));
    }
    let occupied = fixture.sql("SELECT count(*) FROM pg_stat_activity WHERE usename = 'eventlog_test_application' AND wait_event_type = 'Lock'")?;
    ensure!(
        occupied.trim() == "2"
            && started.elapsed() < Duration::from_millis(400)
            && !requests[0].is_finished()
            && !requests[1].is_finished(),
        "standalone cancellation targets are no longer the two observed active clients"
    );
    requests[0].abort();
    requests[1].abort();
    let mut cancelled = 0;
    let mut refused = 0;
    for request in requests {
        match request.await {
            Err(error) if error.is_cancelled() => cancelled += 1,
            Err(error) => return Err(error.into()),
            Ok((elapsed, response)) => {
                ensure!(
                    elapsed <= Duration::from_millis(3500),
                    "standalone pressure exceeded refusal budget"
                );
                ensure!(
                    response?.status() == StatusCode::SERVICE_UNAVAILABLE,
                    "standalone pressure returned an unexpected status"
                );
                refused += 1;
            }
        }
    }
    ensure!(
        refused == 30 && cancelled == 2,
        "standalone pressure did not exercise every request"
    );
    let status = locker.0.wait()?;
    println!("standalone pressure lock exit: {status}");
    ensure!(status.success(), "standalone pressure transaction failed");
    println!(
        "standalone pool pressure: two database connections occupied, refused={refused}, client_cancelled={cancelled}"
    );
    ready(client, origin).await?;
    Ok(())
}

async fn pressure_case(fixture: &Fixture, reorder: bool) -> Result<()> {
    let persistence = Arc::new(fixture.open(true, ROSTER).await?);
    let authority = Authority(Arc::new(AtomicBool::new(true)));
    let factory = bind(&persistence, authority.clone(), false).await?;
    let _http = bind(&persistence, authority, true).await?;
    persistence.seal().await?;
    let mut locker = lock_events(fixture, "pressure-lock").await?;
    let mut requests = Vec::new();
    let delayed = Arc::new(tokio::sync::Semaphore::new(0));
    let first_runnable = usize::from(reorder) * 2;
    let admission_started = Instant::now();
    for index in 0..32 {
        let running = persistence.clone();
        let delayed = delayed.clone();
        requests.push(tokio::spawn(async move {
            if reorder && index < 2 {
                let _permit = delayed.acquire().await.unwrap();
            }
            let start = Instant::now();
            let result = running.readiness().await;
            (start.elapsed(), result)
        }));
        if (first_runnable..first_runnable + 3).contains(&index) {
            let expected = match index - first_runnable {
                0 => (1, 0),
                1 => (2, 0),
                _ => (2, 1),
            };
            let observed = Instant::now();
            loop {
                let status = persistence.pool_status().context("PG pool counters")?;
                if (status.checked_out, status.waiting) == expected {
                    break;
                }
                ensure!(
                    observed.elapsed() < Duration::from_millis(40),
                    "request {index} did not reach its required admission stage {expected:?}"
                );
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            println!(
                "identified readiness request {index}: active={}, waiting={}",
                expected.0, expected.1
            );
        }
    }
    if reorder {
        let observed = Instant::now();
        loop {
            let status = persistence.pool_status().context("PG pool counters")?;
            if status.checked_out == 2 && status.waiting == 4 {
                break;
            }
            ensure!(
                observed.elapsed() < Duration::from_millis(40),
                "controlled scheduler did not fill admissions"
            );
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        delayed.add_permits(2);
        let observed = Instant::now();
        while !requests[0].is_finished() || !requests[1].is_finished() {
            ensure!(
                observed.elapsed() < Duration::from_millis(40),
                "controlled late handles did not finish"
            );
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        println!(
            "controlled scheduler: original cancellation handles 0 and 1 completed after all six admission positions were occupied"
        );
    }
    let mut peak_active = 0;
    let mut peak_waiting = 0;
    for _ in 0..20 {
        let status = persistence.pool_status().context("PG pool counters")?;
        peak_active = peak_active.max(status.checked_out);
        peak_waiting = peak_waiting.max(status.waiting);
        ensure!(
            status.checked_out <= 2 && status.waiting <= 4,
            "configured pool or queue bound escaped"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    // No later request can replace either occupied call before its locked read expires.
    // Identify the queued future while those two calls remain live, then cancel it first.
    let queued = first_runnable + 2;
    let status = persistence.pool_status().context("PG pool counters")?;
    ensure!(
        admission_started.elapsed() < pool().acquisition_timeout
            && status.checked_out == 2
            && status.waiting > 0
            && !requests[first_runnable].is_finished()
            && !requests[first_runnable + 1].is_finished()
            && !requests[queued].is_finished(),
        "identified readiness cancellation targets left their occupied/queued stages"
    );
    println!(
        "cancelling identified queued request {queued} and occupied request {first_runnable}; active={}, waiting={}",
        status.checked_out, status.waiting
    );
    requests[queued].abort();
    match requests.remove(queued).await {
        Err(error) if error.is_cancelled() => {}
        outcome => bail!("identified queued cancellation did not complete: {outcome:?}"),
    }
    let after_queued = persistence.pool_status().context("PG pool counters")?;
    ensure!(
        admission_started.elapsed() < pool().acquisition_timeout
            && after_queued.checked_out == 2
            && after_queued.waiting + 1 == status.waiting
            && !requests[first_runnable].is_finished()
            && !requests[first_runnable + 1].is_finished(),
        "queued cancellation did not retire its waiter before either occupied request"
    );
    println!(
        "queued cancellation completed before occupied cancellation: active={}, waiting={}",
        after_queued.checked_out, after_queued.waiting
    );
    requests[first_runnable].abort();
    let mut cancelled = 1;
    let mut refused = 0;
    for task in requests {
        match task.await {
            Err(error) if error.is_cancelled() => cancelled += 1,
            Err(error) => return Err(error.into()),
            Ok((elapsed, result)) => {
                ensure!(
                    elapsed <= Duration::from_millis(3500),
                    "pressure result exceeded refusal budget"
                );
                ensure!(result.is_err(), "locked database claimed readiness");
                refused += 1;
            }
        }
    }
    ensure!(
        peak_active == 2 && peak_waiting > 0 && refused == 30,
        "pressure did not exercise actual pool saturation"
    );
    ensure!(
        cancelled == 2,
        "the two admitted/queued cancellation cases did not execute"
    );
    let status = locker.0.wait()?;
    println!("pressure lock exit: {status}");
    ensure!(status.success(), "pressure transaction failed");
    println!(
        "SDK pool pressure: active_peak={peak_active}, waiter_peak={peak_waiting}, refused={refused}, cancelled={cancelled}"
    );
    persistence.readiness().await?;
    let input = create("after-cancel");
    let receipt = invoke(
        &factory,
        &principal("pressure", None),
        FACTORY_SERVICE,
        "create",
        input.clone(),
    )
    .await?;
    same_receipt(
        &receipt,
        &invoke(
            &factory,
            &principal("pressure", None),
            FACTORY_SERVICE,
            "create",
            input,
        )
        .await?,
    )?;
    let entered = Arc::new(tokio::sync::Notify::new());
    let notify = entered.clone();
    let router = Router::new().route(
        "/stall",
        get(move || {
            let notify = notify.clone();
            async move {
                notify.notify_one();
                tokio::time::sleep(Duration::from_secs(30)).await;
                StatusCode::NO_CONTENT
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let running = persistence.clone();
    let server = tokio::spawn(async move {
        service_host::serve(listener, router, &running, Duration::from_secs(2), async {
            let _ = stopped.await;
        })
        .await
    });
    let client = tokio::spawn(async move { reqwest::get(format!("{origin}/stall")).await });
    entered.notified().await;
    let started = Instant::now();
    stop.send(())
        .map_err(|()| anyhow::anyhow!("drain signal disappeared"))?;
    ensure!(
        server.await?.is_err(),
        "stalled HTTP request falsely reported a complete drain"
    );
    persistence.shutdown().await?;
    ensure!(
        started.elapsed() <= Duration::from_millis(4500),
        "HTTP drain and pool shutdown exceeded declared bound"
    );
    println!(
        "explicit timed-out drain plus pool shutdown: {:?}",
        started.elapsed()
    );
    client.abort();
    Ok(())
}

struct FeedAuthority {
    admitted: Arc<AtomicBool>,
    tenant: String,
    realm: Option<String>,
    category: String,
    key: String,
}
impl EventFeedAuthorizer for FeedAuthority {
    fn allows<'a>(
        &'a self,
        context: &'a VerifiedAuthContext,
        category: &'a str,
        key: &'a str,
    ) -> eventlog_core::BoxFuture<'a, Result<bool, eventlog_core::EventLogError>> {
        Box::pin(std::future::ready(Ok(self
            .admitted
            .load(Ordering::Acquire)
            && context.tenant().as_str() == self.tenant
            && context.realm().map(service_runtime::RealmId::as_str)
                == self.realm.as_deref()
            && category == self.category
            && key == self.key)))
    }
}
async fn readers(persistence: &Persistence) -> Result<(EventlogService, EventlogService)> {
    let factory = EventlogService::initialize(
        persistence.store(),
        persistence_factory_generated_service::service()?,
    )
    .await?;
    let http = EventlogService::initialize(
        persistence.store(),
        persistence_http_generated_service::service()?,
    )
    .await?;
    persistence.seal().await?;
    Ok((factory, http))
}
async fn reader_effect_case(fixture: &Fixture, postgres: bool) -> Result<()> {
    let persistence = fixture.open(postgres, ROSTER).await?;
    let (service, other) = readers(&persistence).await?;
    let caller = context("reader-tenant", Some("default"));
    let request = serde_json::to_vec(&create("reader-create"))?;
    let first = service
        .intent(
            &caller,
            facts(),
            RequestMetadata::default(),
            "create",
            &request,
        )
        .await?;
    let key = first.events[0].fields["id"]
        .as_str()
        .context("reader stream")?
        .to_owned();
    let close = serde_json::to_vec(&json!({"id": key, "version": 1, "key": "reader-close"}))?;
    let closed = service
        .intent(
            &caller,
            facts(),
            RequestMetadata::default(),
            "close",
            &close,
        )
        .await?;
    let category = service.plan().intents["create"]
        .obligations
        .iter()
        .find_map(|o| o.bindings.get("category"))
        .map_or_else(|| "aggregate".to_owned(), Clone::clone);
    let admitted = Arc::new(AtomicBool::new(true));
    let authority = FeedAuthority {
        admitted: admitted.clone(),
        tenant: "reader-tenant".into(),
        realm: Some("default".into()),
        category: category.clone(),
        key: key.clone(),
    };
    let page = service
        .events_page(&caller, &authority, &category, &key, None, 1)
        .await?;
    ensure!(
        page.events.len() == 1 && page.has_more,
        "raw event window was not bounded"
    );
    ensure!(
        page.events[0].data == serde_json::to_value(&first.events[0].fields)?,
        "event reader changed retained fields"
    );
    admitted.store(false, Ordering::Release);
    ensure!(
        service
            .events_page(&caller, &authority, &category, &key, Some(&page.cursor), 1)
            .await
            .is_err(),
        "reconnected event page ignored revoked authority"
    );
    admitted.store(true, Ordering::Release);
    ensure!(
        other
            .events_page(&caller, &authority, &category, &key, Some(&page.cursor), 1)
            .await
            .is_err(),
        "cross-service cursor was accepted"
    );
    let absent_caller = context("reader-tenant", None);
    let absent_authority = FeedAuthority {
        admitted: admitted.clone(),
        tenant: "reader-tenant".into(),
        realm: None,
        category: category.clone(),
        key: key.clone(),
    };
    ensure!(
        service
            .events_page(
                &absent_caller,
                &absent_authority,
                &category,
                &key,
                Some(&page.cursor),
                1
            )
            .await
            .is_err(),
        "absent realm accepted named realm cursor"
    );
    let mut journal = service.effect_journal();
    let base = EffectPlan {
        format: service_runtime::EFFECT_PLAN_FORMAT.into(),
        service: FACTORY_SERVICE.into(),
        operation: "proof-effect".into(),
        input_digest: "a".repeat(64),
        input_reference: first.events[0].fields["content_ref"]
            .as_str()
            .map(str::to_owned),
        binding_digest: "b".repeat(64),
        aggregate_version: 2,
        resource_revision: "fixture:1".into(),
        authority_reference: "fixture:verified".into(),
        grant_reference: None,
        grant_revision: None,
        risk: EffectRisk::Low,
        consequences: BTreeSet::from(["fixture_observation".into()]),
    };
    let prepared = base.clone().prepare("prepared")?;
    let claimed = base.clone().prepare("claimed")?;
    let completed = base.prepare("completed")?;
    journal.prepare(&caller, prepared.clone()).await?;
    journal.prepare(&caller, claimed.clone()).await?;
    journal.prepare(&caller, completed.clone()).await?;
    let lease = EffectClaim {
        lease_id: "lease-a".into(),
        worker: "proof".into(),
        expires_at: "2030-01-01T00:01:00Z".into(),
    };
    for effect in [&claimed, &completed] {
        ensure!(
            matches!(
                journal
                    .claim(
                        &caller,
                        &effect.operation_id,
                        lease.clone(),
                        "2030-01-01T00:00:00Z"
                    )
                    .await?,
                ClaimDisposition::Acquired(_)
            ),
            "effect was not acquired"
        );
    }
    let outcome = EffectOutcome::Succeeded {
        result_reference: "evidence:fixture".into(),
        result_digest: "sha256:fixture".into(),
    };
    journal
        .complete(
            &caller,
            &completed.operation_id,
            &lease.lease_id,
            outcome.clone(),
        )
        .await?;
    let before = persistence
        .store()
        .read_feed(&TenantId::new("reader-tenant")?, 0, 1000)
        .await?
        .events;
    persistence.shutdown().await?;
    drop(journal);
    drop(service);
    drop(other);
    drop(persistence);
    let persistence = fixture.open(postgres, ROSTER).await?;
    let (service, _other) = readers(&persistence).await?;
    let resumed = service
        .events_page(&caller, &authority, &category, &key, Some(&page.cursor), 32)
        .await?;
    ensure!(
        resumed.events.len() == 1 && !resumed.has_more && resumed.events[0].version == 2,
        "event continuation changed across restart"
    );
    ensure!(
        resumed.events[0].data == serde_json::to_value(&closed.events[0].fields)?,
        "restarted page changed original transition UUID"
    );
    let replay = service
        .intent(
            &caller,
            facts(),
            RequestMetadata::default(),
            "create",
            &request,
        )
        .await?;
    ensure!(
        replay.replayed && replay.events == first.events,
        "reader-side replay changed original content or UUID"
    );
    let mut journal = service.effect_journal();
    ensure!(
        journal.prepare(&caller, prepared).await?.state == EffectState::Prepared,
        "prepared effect changed across restart"
    );
    ensure!(
        journal.prepare(&caller, claimed).await?.state
            == EffectState::Claimed {
                claim: lease.clone()
            },
        "claimed effect changed across restart"
    );
    ensure!(
        journal.prepare(&caller, completed.clone()).await?.state
            == EffectState::Completed {
                outcome: outcome.clone()
            },
        "completed effect changed across restart"
    );
    ensure!(
        matches!(
            journal
                .claim(
                    &caller,
                    &completed.operation_id,
                    lease.clone(),
                    "2030-01-01T00:00:00Z"
                )
                .await?,
            ClaimDisposition::Terminal(_)
        ),
        "terminal effect was redispatched"
    );
    journal
        .complete(&caller, &completed.operation_id, &lease.lease_id, outcome)
        .await?;
    ensure!(
        journal
            .claim(
                &absent_caller,
                &completed.operation_id,
                lease,
                "2030-01-01T00:00:00Z"
            )
            .await
            .is_err(),
        "effect identity crossed optional realms"
    );
    let after = persistence
        .store()
        .read_feed(&TenantId::new("reader-tenant")?, 0, 1000)
        .await?
        .events;
    ensure!(
        before == after,
        "retry or effect recovery appended new identities"
    );
    persistence.shutdown().await?;
    Ok(())
}
