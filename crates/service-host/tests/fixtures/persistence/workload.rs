//! Bounded two-process SDK workload; the loopback driver is only a laboratory receiver.
use super::*;
use axum::routing::post;
use eventlog_core::{BoxFuture, EventLogError, ProjectionSpec, Projector};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, AtomicUsize};

tokio::task_local! { static ERRORS: RefCell<Vec<&'static str>>; }

struct DispatchGate {
    next: tokio::sync::Mutex<Instant>,
}
impl DispatchGate {
    fn new() -> Self {
        Self {
            next: tokio::sync::Mutex::new(Instant::now()),
        }
    }
    async fn grant(&self) -> (Instant, Duration, Duration) {
        let requested = Instant::now();
        let mut next = self.next.lock().await;
        let due = Instant::now().max(*next);
        tokio::time::sleep_until(due.into()).await;
        let actual = Instant::now();
        *next = actual + Duration::from_millis(1);
        (
            actual,
            actual.duration_since(requested),
            actual.saturating_duration_since(due),
        )
    }
}

pub(super) async fn verify_dispatch_gate() -> Result<()> {
    let gate = Arc::new(DispatchGate::new());
    let held = gate.next.lock().await;
    let mut workers = Vec::new();
    for _ in 0..32 {
        let gate = gate.clone();
        workers.push(tokio::spawn(async move { gate.grant().await }));
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
    drop(held);
    let mut grants = Vec::new();
    for worker in workers {
        grants.push(worker.await?.0);
    }
    grants.sort_unstable();
    ensure!(grants.len() == 32, "dispatch gate lost a pending worker");
    for pair in grants.windows(2) {
        ensure!(
            pair[1].duration_since(pair[0]) >= Duration::from_millis(1),
            "dispatch gate emitted a catch-up burst: gap={:?}",
            pair[1].duration_since(pair[0])
        );
    }
    println!(
        "dispatch gate: 32 queued workers, 20ms forced stall, minimum gap {:?}",
        grants
            .windows(2)
            .map(|pair| pair[1].duration_since(pair[0]))
            .min()
            .unwrap()
    );
    Ok(())
}

#[derive(Default)]
struct Observations {
    calls: AtomicU64,
    active_peak: AtomicUsize,
    waiting_peak: AtomicUsize,
    waiter_micros: AtomicU64,
    samples: AtomicU64,
    sample_micros: AtomicU64,
}
struct ObservedStore {
    inner: Arc<dyn EventStore>,
    observations: Arc<Observations>,
}
impl ObservedStore {
    fn observe<'a, T: Send + 'a>(
        &'a self,
        future: BoxFuture<'a, Result<T, EventLogError>>,
    ) -> BoxFuture<'a, Result<T, EventLogError>> {
        Box::pin(async move {
            self.observations.calls.fetch_add(1, Ordering::Relaxed);
            let result = future.await;
            if let Err(error) = &result {
                let category = match error {
                    EventLogError::Conflict { .. } => "conflict",
                    EventLogError::IdempotencyMismatch { .. } => "idempotency_mismatch",
                    EventLogError::Overloaded => "overload",
                    EventLogError::Deadline { .. } => "deadline",
                    EventLogError::UnknownCommit => "unknown_commit",
                    EventLogError::Closed => "closed",
                    _ => "backend_or_invalid",
                };
                let _ = ERRORS.try_with(|errors| errors.borrow_mut().push(category));
            }
            result
        })
    }
}
macro_rules! observed {
    ($($name:ident($($argument:ident: $ty:ty),*) -> $result:ty;)+) => {
        $(fn $name<'a>(&'a self, $($argument: $ty),*) -> BoxFuture<'a, Result<$result, EventLogError>> {
            self.observe(self.inner.$name($($argument),*))
        })+
    };
}
impl EventStore for ObservedStore {
    observed! {
        append(stream: &'a eventlog_core::StreamId, expected: eventlog_core::Expected, events: &'a [eventlog_core::NewEvent], meta: &'a eventlog_core::CommandMeta) -> eventlog_core::AppendResult;
        recorded_claim(tenant: &'a TenantId, claim: &'a eventlog_core::Claim) -> Option<eventlog_core::ClaimedCommand>;
        recorded_command(stream: &'a eventlog_core::StreamId, idempotency_key: &'a str, request_hash: &'a str) -> Option<eventlog_core::AppendResult>;
        read_stream(stream: &'a eventlog_core::StreamId, after_version: u64, limit: usize) -> eventlog_core::StreamSlice;
        stream_version(stream: &'a eventlog_core::StreamId) -> Option<u64>;
        read_feed(tenant: &'a TenantId, after_position: u64, limit: usize) -> eventlog_core::FeedPage;
        redact(stream: &'a eventlog_core::StreamId, version: u64, reason: &'a str) -> eventlog_core::RecordedEvent;
        save_snapshot(stream: &'a eventlog_core::StreamId, snapshot: &'a eventlog_core::Snapshot) -> ();
        load_snapshot(stream: &'a eventlog_core::StreamId) -> Option<eventlog_core::Snapshot>;
        forget_tenant(tenant: &'a TenantId) -> ();
        append_guarded(stream: &'a eventlog_core::StreamId, expected: eventlog_core::Expected, events: &'a [eventlog_core::NewEvent], meta: &'a eventlog_core::CommandMeta, guard: Arc<dyn eventlog_core::Guard>) -> eventlog_core::AppendResult;
        run_catch_up(projector: Arc<dyn Projector>, tenant: &'a TenantId, batch: usize) -> eventlog_core::CatchUpProgress;
        rebuild_projection(projector: Arc<dyn Projector>, tenant: &'a TenantId) -> u64;
        projection_get(projection: &'a ProjectionSpec, tenant: &'a TenantId, key: &'a str) -> Option<Value>;
        projection_find(projection: &'a ProjectionSpec, tenant: &'a TenantId, field: &'a str, value: &'a str, limit: usize) -> Vec<Value>;
        projection_list(projection: &'a ProjectionSpec, tenant: &'a TenantId, after_key: Option<&'a str>, limit: usize) -> Vec<(String, Value)>;
        projection_page(projection: &'a ProjectionSpec, tenant: &'a TenantId, prefix: Option<&'a str>, cursor: Option<&'a str>, limit: usize) -> eventlog_core::ProjectionPage;
        stream_identity(tenant: &'a TenantId) -> String;
        put_blob(tenant: &'a TenantId, digest: &'a str, bytes: &'a [u8]) -> ();
        get_blob(tenant: &'a TenantId, digest: &'a str) -> Option<Vec<u8>>;
        delete_blob(tenant: &'a TenantId, digest: &'a str) -> ();
    }
    fn create_projections(
        &self,
        projector: Arc<dyn Projector>,
    ) -> BoxFuture<'_, Result<(), EventLogError>> {
        self.observe(self.inner.create_projections(projector))
    }
    fn register_inline(
        &self,
        projector: Arc<dyn Projector>,
    ) -> BoxFuture<'_, Result<(), EventLogError>> {
        self.observe(self.inner.register_inline(projector))
    }
    fn is_inline<'a>(&'a self, name: &'a str) -> BoxFuture<'a, bool> {
        self.inner.is_inline(name)
    }
}

#[derive(Clone)]
struct ChildState {
    persistence: Arc<Persistence>,
    services: [EventlogService; 2],
    observations: Arc<Observations>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Call {
    service: usize,
    tenant: usize,
    realm: bool,
    operation: String,
    input: Value,
}
async fn call(State(state): State<ChildState>, Json(call): Json<Call>) -> Json<Value> {
    if call.service >= 2 || call.tenant >= 8 {
        return Json(json!({"error": "invalid fixture coordinate", "provider_errors": []}));
    }
    ERRORS
        .scope(RefCell::new(Vec::new()), async {
            let caller = context(
                &format!("workload-{}", call.tenant),
                call.realm.then_some("default"),
            );
            let service = &state.services[call.service];
            let result: Result<Value> = async {
                if call.operation == "feed" {
                    let key = call.input["id"].as_str().context("feed key")?;
                    let category = service.plan().intents["create"]
                        .obligations
                        .iter()
                        .find_map(|o| o.bindings.get("category"))
                        .map_or("aggregate", String::as_str);
                    let authority = FeedAuthority {
                        admitted: Arc::new(AtomicBool::new(true)),
                        tenant: caller.tenant().as_str().into(),
                        realm: caller.realm().map(|r| r.as_str().into()),
                        category: category.to_owned(),
                        key: key.into(),
                    };
                    Ok(serde_json::to_value(
                        service
                            .events_page(
                                &caller,
                                &authority,
                                category,
                                key,
                                call.input.get("cursor").and_then(Value::as_str),
                                32,
                            )
                            .await?,
                    )?)
                } else if call.operation == "query" {
                    let mut cursor = None;
                    let body = serde_json::to_vec(&call.input)?;
                    for _ in 0..626 {
                        let page = service
                            .query_page(
                                &caller,
                                facts(),
                                "get",
                                &body,
                                service_eventlog::PageRequest::new(cursor, 32)?,
                            )
                            .await?;
                        if !page.items.is_empty() || page.next_cursor.is_none() {
                            return Ok(serde_json::to_value(page)?);
                        }
                        cursor = page.next_cursor;
                    }
                    bail!("query continuation exceeded bounded workload population")
                } else {
                    let operation = if call.operation == "retry" {
                        "create"
                    } else {
                        &call.operation
                    };
                    let receipt = service
                        .intent(
                            &caller,
                            facts(),
                            RequestMetadata::default(),
                            operation,
                            &serde_json::to_vec(&call.input)?,
                        )
                        .await?;
                    for event in &receipt.events {
                        ensure!(
                            serde_json::to_vec(event)?.len() <= 2048,
                            "event exceeded declared size"
                        );
                    }
                    Ok(serde_json::to_value(receipt)?)
                }
            }
            .await;
            let errors = ERRORS.with(|errors| errors.borrow().clone());
            Json(match result {
                Ok(value) => json!({"result": value, "provider_errors": errors}),
                Err(error) => json!({"error": error.to_string(), "provider_errors": errors}),
            })
        })
        .await
}
async fn metrics(State(state): State<ChildState>) -> Json<Value> {
    let pool = state.persistence.pool_status().unwrap();
    Json(json!({
        "calls": state.observations.calls.load(Ordering::Relaxed),
        "active_peak": state.observations.active_peak.load(Ordering::Relaxed),
        "waiting_peak": state.observations.waiting_peak.load(Ordering::Relaxed),
        "waiter_micros": state.observations.waiter_micros.load(Ordering::Relaxed),
        "samples": state.observations.samples.load(Ordering::Relaxed),
        "sample_micros": state.observations.sample_micros.load(Ordering::Relaxed),
        "checked_out": pool.checked_out, "waiting": pool.waiting,
        "proc_status": std::fs::read_to_string("/proc/self/status").unwrap(),
        "proc_stat": std::fs::read_to_string("/proc/self/stat").unwrap(),
        "proc_io": std::fs::read_to_string("/proc/self/io").unwrap(),
    }))
}
async fn child_ready(State(state): State<ChildState>) -> StatusCode {
    if state.persistence.readiness().await.is_ok() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}
pub(super) async fn child() -> Result<()> {
    let fixture = Fixture::required()?;
    let persistence = Arc::new(fixture.open(true, ROSTER).await?);
    let observations = Arc::new(Observations::default());
    let store: Arc<dyn EventStore> = Arc::new(ObservedStore {
        inner: persistence.store(),
        observations: observations.clone(),
    });
    let factory = EventlogService::initialize(
        store.clone(),
        persistence_factory_generated_service::service()?,
    )
    .await?;
    let http =
        EventlogService::initialize(store, persistence_http_generated_service::service()?).await?;
    persistence.seal().await?;
    let state = ChildState {
        persistence: persistence.clone(),
        services: [factory, http],
        observations: observations.clone(),
    };
    let sampling = persistence.clone();
    let sampler = tokio::spawn(async move {
        let mut prior = Instant::now();
        let mut reported = false;
        loop {
            tokio::time::sleep(Duration::from_millis(1)).await;
            let now = Instant::now();
            let elapsed = u64::try_from(now.duration_since(prior).as_micros()).unwrap();
            prior = now;
            let status = sampling.pool_status().unwrap();
            if !reported && (status.checked_out > 2 || status.waiting > 4) {
                println!(
                    "first public pool counter violation: checked_out={}, waiting={}, idle={}, max_connections={}, max_waiters={}",
                    status.checked_out,
                    status.waiting,
                    status.idle,
                    status.max_connections,
                    status.max_waiters
                );
                reported = true;
            }
            observations
                .active_peak
                .fetch_max(status.checked_out, Ordering::Relaxed);
            observations
                .waiting_peak
                .fetch_max(status.waiting, Ordering::Relaxed);
            observations.waiter_micros.fetch_add(
                elapsed * u64::try_from(status.waiting).unwrap(),
                Ordering::Relaxed,
            );
            observations.samples.fetch_add(1, Ordering::Relaxed);
            observations
                .sample_micros
                .fetch_add(elapsed, Ordering::Relaxed);
        }
    });
    let listener = tokio::net::TcpListener::bind(std::env::var("SDK_PROOF_LISTEN")?).await?;
    let router = Router::new()
        .route("/call", post(call))
        .route("/metrics", get(metrics))
        .route("/readyz", get(child_ready))
        .with_state(state);
    let mut signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let result = service_host::serve(
        listener,
        router,
        &persistence,
        Duration::from_secs(2),
        async {
            signal.recv().await;
        },
    )
    .await;
    let shutdown = persistence.shutdown().await;
    sampler.abort();
    result.and(shutdown)
}

fn spawn_child(fixture: &Fixture, index: usize) -> Result<(Process, String)> {
    let socket = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = socket.local_addr()?.to_string();
    drop(socket);
    let log = std::fs::File::create(
        fixture
            .directory
            .join(format!("workload-child-{index}.log")),
    )?;
    let process = Command::new(std::env::current_exe()?)
        .arg("--workload-child")
        .env("SDK_PROOF_SCHEMA", &fixture.schema)
        .env("SDK_PROOF_LISTEN", &address)
        .env("EVENTLOG_TEST_HOSTED_POSTGRES_URL", &fixture.app_url)
        .env(
            "EVENTLOG_TEST_POSTGRES_MIGRATION_URL",
            &fixture.migration_url,
        )
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .spawn()?;
    Ok((Process(process), format!("http://{address}")))
}
async fn send(client: &reqwest::Client, origin: &str, call: &Call) -> Result<(Value, usize)> {
    let response = client
        .post(format!("{origin}/call"))
        .json(call)
        .send()
        .await?;
    ensure!(
        response.status().is_success(),
        "workload driver transport failed: {}",
        response.status()
    );
    let body = response.bytes().await?;
    Ok((serde_json::from_slice(&body)?, body.len()))
}
#[derive(Clone)]
struct Slot {
    coordinate: Call,
    id: String,
    version: u64,
    original: Value,
    receipt: Value,
}
async fn seed(
    client: &reqwest::Client,
    origins: &[String; 2],
    configuration: usize,
) -> Result<Vec<Slot>> {
    let mut slots = Vec::new();
    for (process, origin) in origins.iter().enumerate() {
        for index in 0..256 {
            let input = create(&format!("seed-{configuration}-{process}-{index}"));
            let call = Call {
                service: index % 2,
                tenant: (index / 2) % 8,
                realm: (index / 16) % 2 == 1,
                operation: "create".into(),
                input: input.clone(),
            };
            let (response, _) = send(client, origin, &call).await?;
            let receipt = response
                .get("result")
                .context("seed Create was refused")?
                .clone();
            let id = id(&receipt)?.to_owned();
            let mut revision = call.clone();
            revision.operation = "revise".into();
            revision.input = json!({"id": id, "key": format!("seed-revise-{configuration}-{process}-{index}"), "version": 1});
            let (revised, _) = send(client, origin, &revision).await?;
            ensure!(
                revised["result"]["through_version"] == 2,
                "seed revision was refused: {revised}"
            );
            slots.push(Slot {
                coordinate: call,
                id,
                version: 2,
                original: input,
                receipt,
            });
        }
    }
    ensure!(slots.len() == 512, "seed population missing");
    Ok(slots)
}
#[derive(Serialize)]
struct DispatchObservation {
    index: usize,
    grant_offset_nanos: u64,
    pacing_wait_nanos: u64,
    scheduled_lateness_nanos: u64,
    http_start_offset_nanos: u64,
}

#[derive(Default)]
struct Results {
    dispatch: Vec<DispatchObservation>,
    elapsed_micros: Vec<u64>,
    success_micros: Vec<u64>,
    refusal_micros: Vec<u64>,
    accepted_creates: u64,
    recovered_first_dispatches: u64,
    offered_create_keys: BTreeMap<String, Call>,
    created: Vec<(Call, Value)>,
    accepted_revisions: u64,
    replays: u64,
    errors: BTreeMap<String, u64>,
    responses_with_error: u64,
    response_bytes: u64,
    kinds: [u64; 5],
}
impl Results {
    fn merge(&mut self, other: Self) {
        self.dispatch.extend(other.dispatch);
        self.elapsed_micros.extend(other.elapsed_micros);
        self.success_micros.extend(other.success_micros);
        self.refusal_micros.extend(other.refusal_micros);
        self.accepted_creates += other.accepted_creates;
        self.recovered_first_dispatches += other.recovered_first_dispatches;
        self.offered_create_keys.extend(other.offered_create_keys);
        self.created.extend(other.created);
        self.accepted_revisions += other.accepted_revisions;
        self.replays += other.replays;
        self.responses_with_error += other.responses_with_error;
        self.response_bytes += other.response_bytes;
        for (key, count) in other.errors {
            *self.errors.entry(key).or_default() += count;
        }
        for (left, right) in self.kinds.iter_mut().zip(other.kinds) {
            *left += right;
        }
    }
}
async fn phase(
    client: &reqwest::Client,
    origins: &[String; 2],
    slots: Arc<tokio::sync::Mutex<Vec<Slot>>>,
    configuration: usize,
    concurrency: usize,
    hot: bool,
    steady: bool,
) -> Result<(Results, Duration)> {
    let counter = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(AtomicUsize::new(0));
    let start = Instant::now();
    let dispatch = Arc::new(DispatchGate::new());
    let mut workers = Vec::new();
    for worker in 0..concurrency {
        let client = client.clone();
        let origins = origins.clone();
        let slots = slots.clone();
        let counter = counter.clone();
        let completed = completed.clone();
        let dispatch = dispatch.clone();
        workers.push(tokio::spawn(async move {
            let mut results = Results::default();
            loop {
                let batch = counter.fetch_add(5, Ordering::Relaxed);
                if (!steady && batch >= 200) || (steady && batch >= 2000 && start.elapsed() >= Duration::from_secs(10)) { break; }
                if steady && batch >= 20_000 {
                    println!("steady cap diagnostic: configuration={configuration}, concurrency={concurrency}, hot={hot}, elapsed_micros={}, allocated_batch_start={batch}, completed_responses={}, worker={worker}, worker_provider_errors={:?}, worker_refused_responses={}", start.elapsed().as_micros(), completed.load(Ordering::Relaxed), results.errors, results.responses_with_error);
                }
                ensure!(!steady || batch < 20_000, "steady request cap reached before duration requirement");
                for index in batch..batch + 5 {
                let process = index % 2;
                let selected = if hot && index % 4 < 2 { 0 } else { (index * 73 + worker * 17) % 256 };
                let position = process * 256 + selected;
                let slot = slots.lock().await[position].clone();
                let mut call = slot.coordinate.clone();
                let kind = index % 5;
                match kind {
                    0 => { call.operation = "create".into(); call.input = create(&format!("work-{configuration}-{steady}-{index}")); }
                    1 => { call.operation = "revise".into(); call.input = json!({"id": slot.id, "version": slot.version, "key": format!("revision-{configuration}-{steady}-{index}")}); }
                    2 => { call.operation = "retry".into(); call.input = slot.original.clone(); }
                    3 => { call.operation = "query".into(); call.input = json!({"id": slot.id}); }
                    _ => { call.operation = "feed".into(); call.input = json!({"id": slot.id}); }
                }
                if kind == 0 {
                    ensure!(results.offered_create_keys.insert(call.input["key"].as_str().context("Create key")?.to_owned(), call.clone()).is_none(), "Create key was offered twice");
                }
                if steady {
                    let (actual, pacing_wait, lateness) = dispatch.grant().await;
                    results.dispatch.push(DispatchObservation {
                        index,
                        grant_offset_nanos: u64::try_from(actual.duration_since(start).as_nanos())?,
                        pacing_wait_nanos: u64::try_from(pacing_wait.as_nanos())?,
                        scheduled_lateness_nanos: u64::try_from(lateness.as_nanos())?,
                        http_start_offset_nanos: u64::try_from(start.elapsed().as_nanos())?,
                    });
                }
                let began = Instant::now();
                let (response, bytes) = send(&client, &origins[process], &call).await?;
                completed.fetch_add(1, Ordering::Relaxed);
                let elapsed = u64::try_from(began.elapsed().as_micros())?;
                results.elapsed_micros.push(elapsed);
                results.kinds[kind] += 1;
                results.response_bytes += u64::try_from(bytes)?;
                for error in response["provider_errors"].as_array().context("provider classifications missing")? {
                    *results.errors.entry(error.as_str().context("invalid provider category")?.into()).or_default() += 1;
                }
                if response.get("error").is_some() {
                    ensure!(response["provider_errors"].as_array().is_some_and(|errors| !errors.is_empty()), "unclassified semantic refusal: {response}");
                    ensure!(elapsed <= 3_500_000, "refusal exceeded SDK budget");
                    results.responses_with_error += 1;
                    results.refusal_micros.push(elapsed);
                    continue;
                }
                results.success_micros.push(elapsed);
                let result = &response["result"];
                match kind {
                    0 => {
                        if result["replayed"] == true {
                            ensure!(response["provider_errors"].as_array().is_some_and(|errors| errors.iter().any(|error| matches!(error.as_str(), Some("overload" | "deadline" | "unknown_commit")))), "first-dispatch recovery had no bounded provider refusal: {response}");
                            results.recovered_first_dispatches += 1;
                        } else {
                            ensure!(result["replayed"] == false, "new Create had no explicit disposition: {response}");
                        }
                        results.created.push((call.clone(), result.clone()));
                        let id = id(result)?.to_owned();
                        let mut slots = slots.lock().await;
                        slots[position] = Slot { coordinate: slot.coordinate, id, version: 1, original: call.input, receipt: result.clone() };
                        results.accepted_creates += 1;
                    }
                    1 => {
                        let version = result["through_version"].as_u64().context("revision receipt")?;
                        ensure!(version == slot.version + 1, "accepted revision advanced wrong version");
                        let mut slots = slots.lock().await;
                        if slots[position].id == slot.id { slots[position].version = slots[position].version.max(version); }
                        results.accepted_revisions += 1;
                    }
                    2 => { same_receipt(&slot.receipt, result)?; results.replays += 1; }
                    3 => {
                        let items = result["items"].as_array().context("query page")?;
                        ensure!(items.len() == 1 && items[0]["id"] == slot.id, "query lost exact partition or inline row");
                        ensure!(result["through_version"].as_u64().context("authorized revision")? >= slot.version, "inline projection lag");
                        ensure!(items[0]["content_ref"] == slot.receipt["events"][0]["fields"]["content_ref"], "query changed retained content reference");
                    }
                    _ => {
                        let events = result["events"].as_array().context("event page")?;
                        ensure!(!events.is_empty() && events.len() <= 32, "event page escaped bound or lost events");
                        ensure!(events[0]["data"]["id"] == slot.id, "feed disclosed another aggregate");
                    }
                }
                }
            }
            Ok::<_, anyhow::Error>(results)
        }));
    }
    let mut results = Results::default();
    for worker in workers {
        results.merge(worker.await??);
    }
    ensure!(
        u64::try_from(results.offered_create_keys.len())? == results.kinds[0],
        "parallel workers reused an offered Create key"
    );
    if steady {
        results
            .dispatch
            .sort_unstable_by_key(|entry| entry.grant_offset_nanos);
        ensure!(
            results.dispatch.len() == results.elapsed_micros.len(),
            "dispatch count differs from response count"
        );
        ensure!(
            results
                .dispatch
                .windows(2)
                .all(|pair| pair[1].grant_offset_nanos - pair[0].grant_offset_nanos >= 1_000_000),
            "steady driver admission spacing breached"
        );
    }
    Ok((results, start.elapsed()))
}
fn percentile(values: &mut [u64], percent: usize) -> u64 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    values[((values.len() * percent).div_ceil(100))
        .saturating_sub(1)
        .min(values.len() - 1)]
}

async fn audit_created(
    fixture: &Fixture,
    client: &reqwest::Client,
    origins: &[String; 2],
    configuration: usize,
    steady: bool,
    results: &Results,
    known_creates: &BTreeSet<String>,
) -> Result<Value> {
    let prefix = format!("work-{configuration}-{steady}-%");
    let tables = format!("{}.{}", fixture.schema, HTTP_SERVICE);
    let query = format!(
        "SELECT jsonb_build_object(\
        'claims', COALESCE((SELECT jsonb_agg(to_jsonb(c) ORDER BY claim_key, scope) FROM {tables}_claims c WHERE claim_key LIKE '{prefix}'), '[]'::jsonb), \
        'commands', COALESCE((SELECT jsonb_agg(to_jsonb(c) ORDER BY idempotency_key, stream_id) FROM {tables}_commands c WHERE idempotency_key LIKE '{prefix}'), '[]'::jsonb), \
        'events', COALESCE((SELECT jsonb_agg(to_jsonb(e) || jsonb_build_object('content_retained', b.bytes=convert_to(repeat('x',4096),'UTF8')) ORDER BY global_seq) FROM {tables}_events e LEFT JOIN {tables}_blobs b ON b.tenant_id=e.tenant_id AND 'content:' || b.digest=e.data->>'content_ref' WHERE request_id LIKE '{prefix}'), '[]'::jsonb), \
        'all_create_commands', COALESCE((SELECT jsonb_agg(jsonb_build_object('key',idempotency_key,'stream_id',stream_id) ORDER BY idempotency_key,stream_id) FROM {tables}_commands WHERE tenant_id LIKE 'workload-%' AND first_version=1), '[]'::jsonb), \
        'all_create_claims', COALESCE((SELECT jsonb_agg(jsonb_build_object('key',claim_key,'stream_id',stream_id) ORDER BY claim_key,stream_id) FROM {tables}_claims WHERE tenant_id LIKE 'workload-%' AND first_version=1), '[]'::jsonb), \
        'all_create_events', COALESCE((SELECT jsonb_agg(jsonb_build_object('key',request_id,'stream_id',stream_id) ORDER BY request_id,stream_id) FROM {tables}_events WHERE tenant_id LIKE 'workload-%' AND version=1), '[]'::jsonb))"
    );
    let before: Value = serde_json::from_str(fixture.sql(&query)?.trim())?;
    for table in [
        "all_create_commands",
        "all_create_claims",
        "all_create_events",
    ] {
        let mut keys = BTreeSet::new();
        let mut uuids = BTreeSet::new();
        for row in before[table]
            .as_array()
            .context("all-workload Create audit")?
        {
            let key = row["key"].as_str().context("durable Create key")?;
            let uuid = row["stream_id"]
                .as_str()
                .and_then(|stream| stream.rsplit_once(':'))
                .map(|(_, uuid)| uuid)
                .context("durable Create UUID")?;
            ensure!(
                known_creates.contains(key) && keys.insert(key),
                "unoffered or duplicate durable Create key: {key}"
            );
            ensure!(
                Uuid::parse_str(uuid).is_ok() && uuids.insert(uuid),
                "extra or reused durable Create UUID: {uuid}"
            );
        }
    }
    ensure!(
        before["all_create_commands"] == before["all_create_claims"]
            && before["all_create_commands"] == before["all_create_events"],
        "workload Create claims, commands and first events differ"
    );
    let mut returned_ids = BTreeSet::new();
    let mut failed_without_commit = 0;
    let mut failed_with_commit = 0;
    let mut verified_retries = 0;
    let mut dispositions = BTreeMap::new();
    for (index, (key, call)) in results.offered_create_keys.iter().enumerate() {
        let successful = results
            .created
            .iter()
            .find(|(accepted, _)| accepted.input["key"] == *key);
        let original_events = before["events"]
            .as_array()
            .context("durable events")?
            .iter()
            .filter(|row| row["request_id"] == *key)
            .collect::<Vec<_>>();
        let claims = before["claims"]
            .as_array()
            .context("durable claims")?
            .iter()
            .filter(|row| row["claim_key"] == *key)
            .count();
        let commands = before["commands"]
            .as_array()
            .context("durable commands")?
            .iter()
            .filter(|row| row["idempotency_key"] == *key)
            .count();
        if successful.is_none() && original_events.is_empty() && claims == 0 && commands == 0 {
            failed_without_commit += 1;
            dispositions.insert(key, "unsuccessful_response_no_durable_batch");
            continue;
        }
        ensure!(
            original_events.len() == 1 && claims == 1 && commands == 1,
            "offered Create has partial or multiple durable records: {key}"
        );
        let receipt = if let Some((_, receipt)) = successful {
            dispositions.insert(key, "successful_response_original_batch");
            receipt.clone()
        } else {
            failed_with_commit += 1;
            dispositions.insert(key, "unsuccessful_response_one_committed_batch");
            let event = original_events[0];
            json!({"events": [{"name": event["event_name"], "fields": event["data"]}], "through_version": 1})
        };
        let stream_key = id(&receipt)?.to_owned();
        ensure!(
            returned_ids.insert(stream_key.clone()),
            "different offered Creates returned the same UUID"
        );
        let tenant = format!("workload-{}", call.tenant);
        let service = if call.service == 0 {
            FACTORY_SERVICE
        } else {
            HTTP_SERVICE
        };
        let stream_type = format!("generated-service:{service}");
        let stream_id = format!(
            "{}|9:aggregate|36:{stream_key}",
            if call.realm { "1:7:default" } else { "0:" }
        );
        for (table, key_field) in [
            ("claims", "claim_key"),
            ("commands", "idempotency_key"),
            ("events", "request_id"),
        ] {
            let rows = before[table]
                .as_array()
                .context("durable Create audit rows")?
                .iter()
                .filter(|row| row[key_field] == *key)
                .collect::<Vec<_>>();
            ensure!(
                rows.len() == 1,
                "offered Create has extra or missing durable {table}: {key}"
            );
            let row = rows[0];
            ensure!(
                row["tenant_id"] == tenant
                    && row["stream_type"] == stream_type
                    && row["stream_id"] == stream_id,
                "durable Create changed partition or original UUID: {key}"
            );
            if table == "events" {
                ensure!(
                    row["version"] == 1
                        && row["event_schema_version"] == 1
                        && row["redacted_at"].is_null(),
                    "original Create event changed: {key}"
                );
                ensure!(
                    receipt["events"]
                        .as_array()
                        .is_some_and(|events| events.len() == 1)
                        && row["data"] == receipt["events"][0]["fields"]
                        && row["event_name"] == receipt["events"][0]["name"],
                    "returned Create differs from its sole durable batch: {key}"
                );
                ensure!(
                    row["content_retained"] == true,
                    "original Create content was lost: {key}"
                );
            } else {
                ensure!(
                    row["first_version"] == 1
                        && row["last_version"] == 1
                        && receipt["through_version"] == 1,
                    "original Create receipt range changed: {key}"
                );
                if table == "commands" {
                    ensure!(
                        row["request_hash"] == eventlog_core::request_hash(&receipt["events"])?,
                        "original Create event hash changed: {key}"
                    );
                }
            }
        }
        let mut retry = call.clone();
        retry.operation = "retry".into();
        let (response, _) = send(client, &origins[index % 2], &retry).await?;
        ensure!(
            response.get("error").is_none(),
            "quiescent original Create retry failed: {response}"
        );
        same_receipt(&receipt, &response["result"])?;
        verified_retries += 1;
    }
    let after: Value = serde_json::from_str(fixture.sql(&query)?.trim())?;
    ensure!(
        before == after,
        "Create retry audit appended or changed a durable claim, command or event"
    );
    ensure!(
        results.created.len() + failed_without_commit + failed_with_commit
            == results.offered_create_keys.len(),
        "offered Create disposition missing"
    );
    let evidence = json!({"offered_unique_keys": results.offered_create_keys.len(), "accepted_creates": results.accepted_creates, "recovered_first_dispatches": results.recovered_first_dispatches, "unsuccessful_response_no_commit": failed_without_commit, "unsuccessful_response_committed": failed_with_commit, "verified_subsequent_retries": verified_retries, "dispositions": dispositions, "durable": before});
    std::fs::write(
        fixture
            .directory
            .join(format!("create-audit-{configuration}-{steady}.json")),
        serde_json::to_vec_pretty(&evidence)?,
    )?;
    Ok(
        json!({"offered_unique_keys": results.offered_create_keys.len(), "accepted_creates": results.accepted_creates, "fresh_commit_responses": results.accepted_creates - results.recovered_first_dispatches, "recovered_first_dispatches": results.recovered_first_dispatches, "unsuccessful_response_no_commit": failed_without_commit, "unsuccessful_response_committed": failed_with_commit, "verified_subsequent_retries": verified_retries}),
    )
}

async fn snapshots(client: &reqwest::Client, origins: &[String; 2]) -> Result<Vec<Value>> {
    let mut values = Vec::new();
    for origin in origins {
        let value: Value = client
            .get(format!("{origin}/metrics"))
            .send()
            .await?
            .json()
            .await?;
        println!("SDK process snapshot: {value}");
        ensure!(
            value["active_peak"].as_u64().context("pool peak")? <= 2
                && value["waiting_peak"].as_u64().context("queue peak")? <= 4,
            "measured pool/queue escaped bounds"
        );
        ensure!(
            value["samples"].as_u64().context("sampling count")? > 0,
            "no resource/queue samples"
        );
        values.push(value);
    }
    Ok(values)
}
fn docker_stats(fixture: &Fixture) -> Result<String> {
    let output = Command::new("docker")
        .args([
            "stats",
            "--no-stream",
            "--format",
            "{{json .}}",
            &fixture.container,
        ])
        .output()?;
    ensure!(
        output.status.success(),
        "required database resource measurement unavailable"
    );
    Ok(String::from_utf8(output.stdout)?)
}

fn freeze_inputs(fixture: &Fixture) -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .context("canonicalizing SDK source root for input freeze")?;
    let listed = Command::new("git")
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            "crates",
            "ess",
        ])
        .current_dir(&root)
        .output()
        .context("executing git ls-files for source input freeze")?;
    ensure!(listed.status.success(), "source inventory failed");
    let mut paths = String::from_utf8(listed.stdout)?
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    paths.extend(["Cargo.toml", "Cargo.lock"].into_iter().map(str::to_owned));
    paths.push(
        std::env::current_exe()?
            .strip_prefix(&root)?
            .display()
            .to_string(),
    );
    paths.push(
        binary()
            .canonicalize()
            .with_context(|| {
                format!(
                    "canonicalizing standalone fixture binary {}",
                    binary().display()
                )
            })?
            .strip_prefix(&root)?
            .display()
            .to_string(),
    );
    paths.sort();
    let output = Command::new("sha256sum")
        .args(&paths)
        .current_dir(&root)
        .output()
        .context("executing sha256sum for source/binary input freeze")?;
    ensure!(
        output.status.success(),
        "source/binary input hashing failed"
    );
    std::fs::write(
        fixture.directory.join("measured-inputs.sha256"),
        &output.stdout,
    )?;
    println!(
        "Exact source and binary inputs before workload:\n{}",
        String::from_utf8(output.stdout)?
    );
    Ok(())
}

pub(super) async fn measure(fixture: &Fixture) -> Result<()> {
    freeze_inputs(fixture)?;
    ensure!(
        fixture
            .sql("SELECT rolconnlimit FROM pg_roles WHERE rolname = 'eventlog_test_application'")?
            .trim()
            == "4",
        "application role capacity differs from immutable workload"
    );
    let limits = Command::new("docker")
        .args([
            "inspect",
            &fixture.container,
            "--format",
            "{{.HostConfig.NanoCpus}} {{.HostConfig.Memory}} {{.HostConfig.PidsLimit}}",
        ])
        .output()?;
    ensure!(
        limits.status.success()
            && String::from_utf8_lossy(&limits.stdout).trim() == "2000000000 1073741824 256",
        "database resource bounds differ from immutable profile"
    );
    println!(
        "SDK profile v3: two processes, pool 2+4 waiters each, role 4, reserve 4, concurrency 1/8/32, uniform/hot50, unpaced warmup 200, steady >=2000 and >=10s, cap 20000, no-catch-up steady grants >=1ms apart"
    );
    let (mut left, left_origin) = spawn_child(fixture, 0)?;
    let (mut right, right_origin) = spawn_child(fixture, 1)?;
    let origins = [left_origin, right_origin];
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(4500))
        .build()?;
    for origin in &origins {
        ready(&client, origin).await?;
    }
    let mut configurations = 0;
    let mut known_creates = BTreeSet::new();
    for hot in [false, true] {
        for concurrency in [1, 8, 32] {
            let configuration = configurations;
            let slots = Arc::new(tokio::sync::Mutex::new(
                seed(&client, &origins, configuration).await?,
            ));
            for slot in slots.lock().await.iter() {
                ensure!(
                    known_creates.insert(
                        slot.original["key"]
                            .as_str()
                            .context("seed Create key")?
                            .to_owned()
                    ),
                    "seed Create key repeated"
                );
            }
            let count = fixture
                .sql(&format!(
                    "SELECT count(*) FROM {}.{}_events WHERE tenant_id LIKE 'workload-%'",
                    fixture.schema, HTTP_SERVICE
                ))?
                .trim()
                .parse::<u64>()?;
            ensure!(count >= 1024, "retained seed event population missing");
            let (warmup, _) = phase(
                &client,
                &origins,
                slots.clone(),
                configuration,
                concurrency,
                hot,
                false,
            )
            .await?;
            ensure!(
                warmup.elapsed_micros.len() == 200,
                "warmup selection differs"
            );
            let before = snapshots(&client, &origins).await?;
            let db_before = docker_stats(fixture)?;
            let (mut result, elapsed) = phase(
                &client,
                &origins,
                slots.clone(),
                configuration,
                concurrency,
                hot,
                true,
            )
            .await?;
            let after = snapshots(&client, &origins).await?;
            let db_after = docker_stats(fixture)?;
            ensure!(
                result.elapsed_micros.len() >= 2000 && elapsed >= Duration::from_secs(10),
                "steady count/duration missing"
            );
            let p50 = percentile(&mut result.success_micros, 50);
            let p95 = percentile(&mut result.success_micros, 95);
            let p99 = percentile(&mut result.success_micros, 99);
            ensure!(
                result.accepted_creates > 0 && result.accepted_revisions > 0 && result.replays > 0,
                "required successful operation classes missing"
            );
            let feed_start = Instant::now();
            // Four bounded readers use two connections per process; each SDK page stays <=32.
            let captured = Arc::new(slots.lock().await.clone());
            let mut catchups = Vec::new();
            for worker in 0..4 {
                let slots = captured.clone();
                let client = client.clone();
                let origins = origins.clone();
                catchups.push(tokio::spawn(async move {
                    for selected in (worker..512).step_by(4) {
                        let position = (selected % 2) * 256 + selected / 2;
                        let slot = &slots[position];
                        let mut call = slot.coordinate.clone();
                        call.operation = "feed".into();
                        call.input = json!({"id": slot.id});
                        let mut seen = 0_u64;
                        loop {
                            let (page, _) = send(&client, &origins[position / 256], &call).await?;
                            ensure!(page.get("error").is_none(), "quiescent feed failed");
                            let events = page["result"]["events"]
                                .as_array()
                                .context("quiescent feed")?;
                            ensure!(
                                !events.is_empty() && events.len() <= 32,
                                "quiescent feed lost its bounded window"
                            );
                            for event in events {
                                seen += 1;
                                ensure!(
                                    event["version"].as_u64() == Some(seen)
                                        && event["data"]["id"] == slot.id,
                                    "quiescent feed has a gap or changed partition"
                                );
                            }
                            if page["result"]["has_more"] == false {
                                break;
                            }
                            ensure!(
                                seen <= 20_002,
                                "quiescent feed exceeded configured workload bound"
                            );
                            call.input["cursor"] = page["result"]["cursor"].clone();
                        }
                        ensure!(
                            seen >= slot.version,
                            "quiescent feed lost an accepted revision"
                        );
                    }
                    Ok::<_, anyhow::Error>(())
                }));
            }
            for catchup in catchups {
                catchup.await??;
            }
            let feed_elapsed = feed_start.elapsed();
            for key in warmup
                .offered_create_keys
                .keys()
                .chain(result.offered_create_keys.keys())
            {
                ensure!(
                    known_creates.insert(key.clone()),
                    "Create key reused between workload phases"
                );
            }
            // Audits run after both the steady interval and timed quiescent feed check.
            // Their retries are independent verification, never workload operations.
            let warmup_create_audit = audit_created(
                fixture,
                &client,
                &origins,
                configuration,
                false,
                &warmup,
                &known_creates,
            )
            .await?;
            let steady_create_audit = audit_created(
                fixture,
                &client,
                &origins,
                configuration,
                true,
                &result,
                &known_creates,
            )
            .await?;
            let persisted = fixture.sql(&format!("SELECT count(*), sum(octet_length(data::text)), max(octet_length(data::text)), pg_database_size(current_database()) FROM {}.{}_events WHERE tenant_id LIKE 'workload-%'", fixture.schema, HTTP_SERVICE))?;
            let output = json!({
                "profile": "v3",
                "configuration": configuration, "concurrency": concurrency, "hot_fraction": if hot {0.5} else {0.0},
                "warmup_executed": warmup.elapsed_micros.len(), "steady_executed": result.elapsed_micros.len(),
                "seconds": elapsed.as_secs_f64(), "arrival_rate": f64::from(u32::try_from(result.elapsed_micros.len())?) / elapsed.as_secs_f64(),
                "success_rate": f64::from(u32::try_from(result.success_micros.len())?) / elapsed.as_secs_f64(),
                "success_p50_micros": p50, "success_p95_micros": p95, "success_p99_micros": p99,
                "refusal_max_micros": result.refusal_micros.iter().max(), "accepted_creates": result.accepted_creates,
                "recovered_first_dispatches": result.recovered_first_dispatches,
                "fresh_commit_responses": result.accepted_creates - result.recovered_first_dispatches,
                "warmup_create_audit": warmup_create_audit, "steady_create_audit": steady_create_audit,
                "accepted_revisions": result.accepted_revisions, "replays": result.replays,
                "provider_error_counts": result.errors, "unresolved_or_refused_responses": result.responses_with_error,
                "operation_counts": result.kinds, "response_bytes": result.response_bytes, "persisted": persisted.trim(),
                "quiescent_feed_seconds": feed_elapsed.as_secs_f64(),
                "process_before": before, "process_after": after, "database_before": db_before, "database_after": db_after,
                "queue_measurement": "sampled waiter-microseconds; requested interval 1 ms, actual sample_micros/samples",
                "driver_dispatch": result.dispatch,
                "driver_grant_count": result.dispatch.len(),
                "driver_min_grant_gap_nanos": result.dispatch.windows(2).map(|pair| pair[1].grant_offset_nanos - pair[0].grant_offset_nanos).min(),
                "driver_max_pacing_wait_nanos": result.dispatch.iter().map(|entry| entry.pacing_wait_nanos).max(),
                "driver_max_scheduled_lateness_nanos": result.dispatch.iter().map(|entry| entry.scheduled_lateness_nanos).max(),
                "driver_grant_rate": f64::from(u32::try_from(result.dispatch.len())?) / elapsed.as_secs_f64(),
            });
            let bytes = serde_json::to_vec_pretty(&output)?;
            std::fs::write(
                fixture
                    .directory
                    .join(format!("workload-{configuration}.json")),
                &bytes,
            )?;
            println!("SDK workload measurement {}", String::from_utf8(bytes)?);
            ensure!(
                concurrency != 1 || p99 <= 2_000_000,
                "admitted p99 exceeded 2 seconds"
            );
            ensure!(
                feed_elapsed <= Duration::from_secs(3),
                "quiescent feed catch-up exceeded 3 seconds"
            );
            configurations += 1;
        }
    }
    ensure!(configurations == 6, "workload matrix selection incomplete");
    left.stop().await?;
    right.stop().await?;
    println!("SDK workload matrix: executed 6 configurations; 0 skips");
    Ok(())
}
