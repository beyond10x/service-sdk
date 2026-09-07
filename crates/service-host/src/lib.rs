//! Reusable process host for generated Identity-authenticated services.
//!
//! Generated packages supply only their compiled router. This crate owns the operational shell:
//! environment loading, durable persistence initialization, listener lifecycle, and bounded shutdown.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, IntoFuture};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use eventlog_core::{EventLogError, EventStore as DurableEventStore, ProjectionSpec, Projector};
use eventlog_postgres::{PoolOptions, PoolStatus, PostgresConfig, PostgresEventStore};
use eventlog_sqlite::SqliteEventStore;

/// Verified whole-deployment connection budget, supplied before opening an application pool.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConnectionBudget {
    /// Observed connection capacity allocated to this deployment workload.
    pub database_connections: usize,
    /// Number of concurrently running application replicas.
    pub replicas: usize,
    /// Migration and operational reserve within the allocation.
    pub reserved_connections: usize,
}

#[derive(Clone)]
struct PostgresSettings {
    url: String,
    schema: String,
    ca_pem: String,
    pool: PoolOptions,
    budget: ConnectionBudget,
}

impl std::fmt::Debug for PostgresSettings {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PostgresSettings")
            .field("schema", &self.schema)
            .field("pool", &self.pool)
            .field("budget", &self.budget)
            .finish_non_exhaustive()
    }
}

impl PostgresSettings {
    fn from_lookup(
        prefix: &str,
        url_suffix: &str,
        lookup: &impl Fn(&str) -> Option<String>,
    ) -> Result<Self> {
        let required = |suffix: &str| -> Result<String> {
            lookup(&format!("{prefix}_{suffix}"))
                .filter(|value| !value.trim().is_empty())
                .with_context(|| format!("{prefix}_{suffix} is required for PostgreSQL"))
        };
        let number = |suffix: &str| -> Result<usize> {
            required(suffix)?
                .parse()
                .with_context(|| format!("{prefix}_{suffix} must be a non-negative integer"))
        };
        let timeout = |suffix: &str| -> Result<Duration> {
            let millis = number(suffix)?;
            if millis == 0 || millis > 86_400_000 {
                bail!("{prefix}_{suffix} must be between 1 and 86400000 milliseconds");
            }
            Ok(Duration::from_millis(u64::try_from(millis)?))
        };
        Ok(Self {
            url: required(url_suffix)?,
            schema: required("POSTGRES_SCHEMA")?,
            ca_pem: required("POSTGRES_CA_PEM")?,
            pool: PoolOptions {
                max_connections: number("POOL_MAX")?,
                max_waiters: number("POOL_WAITERS")?,
                acquisition_timeout: timeout("ACQUISITION_MS")?,
                connect_timeout: timeout("CONNECT_MS")?,
                statement_timeout: timeout("STATEMENT_MS")?,
                lock_timeout: timeout("LOCK_MS")?,
                transaction_timeout: timeout("TRANSACTION_MS")?,
                shutdown_timeout: timeout("SHUTDOWN_MS")?,
            },
            budget: ConnectionBudget {
                database_connections: number("DATABASE_CONNECTIONS")?,
                replicas: number("REPLICAS")?,
                reserved_connections: number("RESERVED_CONNECTIONS")?,
            },
        })
    }

    fn verified(&self, prefix: &str) -> Result<PostgresConfig> {
        let mut roots = rustls::RootCertStore::empty();
        let mut pem = self.ca_pem.as_bytes();
        for certificate in rustls_pemfile::certs(&mut pem) {
            roots
                .add(certificate.context("PostgreSQL CA PEM is invalid")?)
                .context("PostgreSQL CA certificate is invalid")?;
        }
        PostgresConfig::verified(&self.url, &self.schema, prefix, roots)
            .context("verified PostgreSQL configuration is unavailable")
    }
}

#[derive(Clone, Debug)]
enum BackendConfig {
    Sqlite,
    Postgres(Box<PostgresSettings>),
}

/// Fully resolved process configuration; debug output omits database credentials and trust material.
#[derive(Clone, Debug)]
pub struct HostConfig {
    listen: SocketAddr,
    identity_origin: String,
    database_path: String,
    backend: BackendConfig,
    drain_timeout: Duration,
}

impl HostConfig {
    /// Reads listener, Identity, persistence selection and the selected backend's settings.
    ///
    /// `SQLite` is the default. Selecting `postgres` requires explicit verified connection,
    /// pool, workload budget and timeout settings documented in the repository README.
    pub fn from_environment(prefix: &str, default_database_path: &str) -> Result<Self> {
        Self::from_lookup(prefix, default_database_path, |name| {
            std::env::var(name).ok()
        })
    }

    fn from_lookup(
        prefix: &str,
        default_database_path: &str,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Self> {
        if prefix.is_empty()
            || !prefix
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        {
            bail!("service environment prefix must contain only uppercase ASCII, digits, or `_`");
        }
        let listen_name = format!("{prefix}_LISTEN");
        let identity_name = format!("{prefix}_IDENTITY_ORIGIN");
        let database_name = format!("{prefix}_DATABASE_PATH");
        let listen = lookup(&listen_name)
            .unwrap_or_else(|| "0.0.0.0:8080".to_owned())
            .parse()
            .with_context(|| format!("{listen_name} is not a socket address"))?;
        let identity_origin = lookup(&identity_name)
            .filter(|value| !value.trim().is_empty())
            .with_context(|| format!("{identity_name} is required"))?;
        let database_path = lookup(&database_name)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| default_database_path.to_owned());
        let backend = match lookup(&format!("{prefix}_PERSISTENCE"))
            .as_deref()
            .unwrap_or("sqlite")
        {
            "sqlite" => BackendConfig::Sqlite,
            "postgres" => BackendConfig::Postgres(Box::new(PostgresSettings::from_lookup(
                prefix,
                "POSTGRES_URL",
                &lookup,
            )?)),
            _ => bail!("{prefix}_PERSISTENCE must be sqlite or postgres"),
        };
        let drain_millis = match lookup(&format!("{prefix}_DRAIN_MS")) {
            Some(value) => value
                .parse::<u64>()
                .context("HTTP drain timeout must be an integer")?,
            None if matches!(backend, BackendConfig::Postgres(_)) => {
                bail!("{prefix}_DRAIN_MS is required for PostgreSQL")
            }
            None => 5_000,
        };
        if drain_millis == 0 || drain_millis > 86_400_000 {
            bail!("HTTP drain timeout is outside its supported bounds");
        }
        Ok(Self {
            listen,
            identity_origin,
            database_path,
            backend,
            drain_timeout: Duration::from_millis(drain_millis),
        })
    }
}

/// Runs one generated service with its SDK-owned `SQLite` persistence adapter.
pub fn run_sqlite<Factory, RouterFuture>(
    environment_prefix: &str,
    default_database_path: &str,
    store_prefix: &str,
    router: Factory,
) -> Result<()>
where
    Factory: FnOnce(Arc<dyn service_connectors::DurableEventStore>, String) -> RouterFuture,
    RouterFuture: Future<Output = Result<service_http::HttpRouter, service_http::ServerError>>
        + Send
        + 'static,
{
    let config = HostConfig::from_environment(environment_prefix, default_database_path)?;
    if !matches!(config.backend, BackendConfig::Sqlite) {
        bail!("run_sqlite requires the SQLite selection");
    }
    run_configured(config, store_prefix, &[store_prefix], router)
}

/// Runs a production generated service after separately migrating its complete service roster.
pub fn run_postgres<Factory, RouterFuture>(
    environment_prefix: &str,
    store_prefix: &str,
    services: &[&str],
    router: Factory,
) -> Result<()>
where
    Factory: FnOnce(Arc<dyn DurableEventStore>, String) -> RouterFuture,
    RouterFuture: Future<Output = Result<service_http::HttpRouter, service_http::ServerError>>
        + Send
        + 'static,
{
    let config = HostConfig::from_environment(environment_prefix, "unused.sqlite3")?;
    if !matches!(config.backend, BackendConfig::Postgres(_)) {
        bail!("run_postgres requires the PostgreSQL selection");
    }
    run_configured(config, store_prefix, services, router)
}

/// Runs only the explicit migration phase, reading separate migration credentials.
pub fn run_migrations(
    environment_prefix: &str,
    store_prefix: &str,
    services: &[&str],
) -> Result<()> {
    let settings =
        PostgresSettings::from_lookup(environment_prefix, "POSTGRES_MIGRATION_URL", &|name| {
            std::env::var(name).ok()
        })?;
    let config = settings.verified(store_prefix)?;
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(Persistence::migrate_postgres(
            config,
            settings.pool,
            services,
        ))
}

fn run_configured<Factory, RouterFuture>(
    config: HostConfig,
    store_prefix: &str,
    services: &[&str],
    router: Factory,
) -> Result<()>
where
    Factory: FnOnce(Arc<dyn DurableEventStore>, String) -> RouterFuture,
    RouterFuture: Future<Output = Result<service_http::HttpRouter, service_http::ServerError>>
        + Send
        + 'static,
{
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("generated service Tokio runtime is unavailable")?;
    runtime.block_on(async move {
        let persistence = match &config.backend {
            BackendConfig::Sqlite => {
                Persistence::open_sqlite(&config.database_path, store_prefix, services).await?
            }
            BackendConfig::Postgres(settings) => {
                Persistence::open_postgres(
                    settings.verified(store_prefix)?,
                    settings.pool.clone(),
                    settings.budget,
                    services,
                )
                .await?
            }
        };
        let result = async {
            let application = router(persistence.store(), config.identity_origin)
                .await
                .context("generated Identity HTTP service is unavailable")?;
            persistence.seal().await?;
            let listener = tokio::net::TcpListener::bind(config.listen)
                .await
                .context("generated service listener is unavailable")?;
            serve(
                listener,
                application,
                &persistence,
                config.drain_timeout,
                shutdown_signal(),
            )
            .await
        }
        .await;
        let shutdown = persistence.shutdown().await;
        result.and(shutdown)
    })
}

/// Serves one already-composed router with a bounded graceful drain.
pub async fn serve(
    listener: tokio::net::TcpListener,
    application: service_http::HttpRouter,
    persistence: &Persistence,
    drain_timeout: Duration,
    shutdown: impl Future<Output = ()> + Send,
) -> Result<()> {
    persistence.readiness().await?;
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = axum::serve(listener, application)
        .with_graceful_shutdown(async {
            let _ = stopped.await;
        })
        .into_future();
    tokio::pin!(server);
    tokio::pin!(shutdown);
    tokio::select! {
        result = &mut server => result.context("generated service failed"),
        () = &mut shutdown => {
            persistence.begin_drain();
            let _ = stop.send(());
            tokio::time::timeout(drain_timeout, &mut server).await
                .context("HTTP drain timed out; interrupted command outcomes require original-identity reconciliation")?
                .context("generated service failed while draining")
        }
    }
}

/// One host-owned adapter plus its exact startup roster and lifecycle.
pub struct Persistence {
    store: Arc<ManagedStore>,
    postgres: Option<Arc<PostgresEventStore>>,
}

impl Persistence {
    /// Migrates all declared service tables using a separately supplied migration-role config.
    pub async fn migrate_postgres(
        config: PostgresConfig,
        pool: PoolOptions,
        services: &[&str],
    ) -> Result<()> {
        let roster = Roster::new(services)?;
        PostgresEventStore::migrate(config, pool, &roster.projections())
            .await
            .context("generated service schema migration was refused")
    }

    /// Opens the same file `SQLite` adapter used by the default generated host.
    pub async fn open_sqlite(path: &str, prefix: &str, services: &[&str]) -> Result<Self> {
        let roster = Roster::new(services)?;
        let inner: Arc<dyn DurableEventStore> =
            Arc::new(SqliteEventStore::open(path, prefix).await?);
        Ok(Self {
            store: Arc::new(ManagedStore::new(inner, roster, Duration::from_secs(5))),
            postgres: None,
        })
    }

    /// Opens an already-migrated production adapter with the application role and finite budget.
    pub async fn open_postgres(
        config: PostgresConfig,
        pool: PoolOptions,
        budget: ConnectionBudget,
        services: &[&str],
    ) -> Result<Self> {
        let roster = Roster::new(services)?;
        let timeout = pool.transaction_timeout;
        let postgres = Arc::new(
            PostgresEventStore::open(
                config,
                pool,
                budget.database_connections,
                budget.replicas,
                budget.reserved_connections,
            )
            .await?,
        );
        let inner: Arc<dyn DurableEventStore> = postgres.clone();
        Ok(Self {
            store: Arc::new(ManagedStore::new(inner, roster, timeout)),
            postgres: Some(postgres),
        })
    }

    /// Injects this exact adapter into every generated factory before sealing once.
    pub fn store(&self) -> Arc<dyn DurableEventStore> {
        self.store.clone()
    }

    /// Refuses an incomplete roster, then freezes registration before listening.
    pub async fn seal(&self) -> Result<()> {
        let _registration = self.store.registration.lock().await;
        if self.store.phase.load(Ordering::Acquire) != 0 {
            bail!("service persistence startup is already sealed or stopped");
        }
        {
            let registered = self
                .store
                .registered
                .lock()
                .map_err(|_| anyhow::anyhow!("service registry is unavailable"))?;
            if registered.len() != self.store.roster.0.len()
                || !self
                    .store
                    .roster
                    .0
                    .keys()
                    .all(|name| registered.contains(name))
            {
                bail!("not every declared generated service has registered its projector");
            }
        }
        if let Some(postgres) = &self.postgres {
            postgres.seal().await;
        }
        self.store.phase.store(1, Ordering::Release);
        Ok(())
    }

    /// Reads the actual database under the configured deadline; pool counters are insufficient.
    pub async fn readiness(&self) -> Result<()> {
        let probe = eventlog_core::StreamId::new(
            eventlog_core::TenantId::new("sdk-host-probe")?,
            "sdk-host-probe",
            "readiness",
        )?;
        self.store
            .stream_version(&probe)
            .await
            .map(|_| ())
            .map_err(Into::into)
    }

    /// Rejects new storage operations while the host drains accepted responses.
    pub fn begin_drain(&self) {
        self.store.phase.store(2, Ordering::Release);
    }

    /// Closes admissions and waits within the adapter's explicit shutdown deadline.
    pub async fn shutdown(&self) -> Result<()> {
        self.begin_drain();
        if let Some(postgres) = &self.postgres {
            postgres.shutdown().await?;
        }
        Ok(())
    }

    /// Non-sensitive `PostgreSQL` pool measurements, when this host selected `PostgreSQL`.
    pub fn pool_status(&self) -> Option<PoolStatus> {
        self.postgres
            .as_ref()
            .map(|postgres| postgres.pool_status())
    }
}

struct Roster(BTreeMap<&'static str, &'static [ProjectionSpec]>);

impl Roster {
    fn new(services: &[&str]) -> Result<Self> {
        if services.is_empty() || services.len() > 64 {
            bail!("a host must declare between 1 and 64 generated services");
        }
        let mut roster = BTreeMap::new();
        for service in services {
            if service.trim().is_empty() {
                bail!("a generated service identity must not be empty");
            }
            let (name, specs) = service_eventlog::EventlogService::projection_declaration(service);
            if roster.insert(name, specs).is_some() {
                bail!("duplicate or colliding generated service identity in the host roster");
            }
        }
        Ok(Self(roster))
    }

    fn projections(&self) -> Vec<ProjectionSpec> {
        self.0
            .values()
            .flat_map(|specs| specs.iter().copied())
            .collect()
    }

    fn admits(&self, projector: &dyn Projector) -> bool {
        self.0.get(projector.name()).is_some_and(|expected| {
            let actual = projector.projections();
            expected.len() == actual.len()
                && expected
                    .iter()
                    .zip(actual)
                    .all(|(left, right)| left.name == right.name && left.indexed == right.indexed)
        })
    }
}

struct ManagedStore {
    inner: Arc<dyn DurableEventStore>,
    roster: Roster,
    registered: Mutex<BTreeSet<&'static str>>,
    registration: tokio::sync::Mutex<()>,
    phase: AtomicU8,
    timeout: Duration,
}

impl ManagedStore {
    fn new(inner: Arc<dyn DurableEventStore>, roster: Roster, timeout: Duration) -> Self {
        Self {
            inner,
            roster,
            registered: Mutex::new(BTreeSet::new()),
            registration: tokio::sync::Mutex::new(()),
            phase: AtomicU8::new(0),
            timeout,
        }
    }

    fn execute<'a, T: Send + 'a>(
        &'a self,
        future: eventlog_core::BoxFuture<'a, Result<T, EventLogError>>,
    ) -> eventlog_core::BoxFuture<'a, Result<T, EventLogError>> {
        Box::pin(async move {
            if self.phase.load(Ordering::Acquire) != 1 {
                return Err(EventLogError::Invalid(
                    "host persistence is not accepting operations".to_owned(),
                ));
            }
            tokio::time::timeout(self.timeout, future)
                .await
                .map_err(|_| EventLogError::UnknownCommit)?
        })
    }
}

macro_rules! forward_store {
    ($($name:ident($($argument:ident: $ty:ty),*) -> $result:ty;)+) => {
        $(fn $name<'a>(&'a self, $($argument: $ty),*) -> eventlog_core::BoxFuture<'a, Result<$result, EventLogError>> {
            self.execute(self.inner.$name($($argument),*))
        })+
    };
}

impl DurableEventStore for ManagedStore {
    forward_store! {
        append(stream: &'a eventlog_core::StreamId, expected: eventlog_core::Expected, events: &'a [eventlog_core::NewEvent], meta: &'a eventlog_core::CommandMeta) -> eventlog_core::AppendResult;
        recorded_claim(tenant: &'a eventlog_core::TenantId, claim: &'a eventlog_core::Claim) -> Option<eventlog_core::ClaimedCommand>;
        recorded_command(stream: &'a eventlog_core::StreamId, idempotency_key: &'a str, request_hash: &'a str) -> Option<eventlog_core::AppendResult>;
        read_stream(stream: &'a eventlog_core::StreamId, after_version: u64, limit: usize) -> eventlog_core::StreamSlice;
        stream_version(stream: &'a eventlog_core::StreamId) -> Option<u64>;
        read_feed(tenant: &'a eventlog_core::TenantId, after_position: u64, limit: usize) -> eventlog_core::FeedPage;
        redact(stream: &'a eventlog_core::StreamId, version: u64, reason: &'a str) -> eventlog_core::RecordedEvent;
        save_snapshot(stream: &'a eventlog_core::StreamId, snapshot: &'a eventlog_core::Snapshot) -> ();
        load_snapshot(stream: &'a eventlog_core::StreamId) -> Option<eventlog_core::Snapshot>;
        forget_tenant(tenant: &'a eventlog_core::TenantId) -> ();
        append_guarded(stream: &'a eventlog_core::StreamId, expected: eventlog_core::Expected, events: &'a [eventlog_core::NewEvent], meta: &'a eventlog_core::CommandMeta, guard: Arc<dyn eventlog_core::Guard>) -> eventlog_core::AppendResult;
        run_catch_up(projector: Arc<dyn Projector>, tenant: &'a eventlog_core::TenantId, batch: usize) -> eventlog_core::CatchUpProgress;
        rebuild_projection(projector: Arc<dyn Projector>, tenant: &'a eventlog_core::TenantId) -> u64;
        projection_get(projection: &'a ProjectionSpec, tenant: &'a eventlog_core::TenantId, key: &'a str) -> Option<serde_json::Value>;
        projection_find(projection: &'a ProjectionSpec, tenant: &'a eventlog_core::TenantId, field: &'a str, value: &'a str, limit: usize) -> Vec<serde_json::Value>;
        projection_list(projection: &'a ProjectionSpec, tenant: &'a eventlog_core::TenantId, after_key: Option<&'a str>, limit: usize) -> Vec<(String, serde_json::Value)>;
        projection_page(projection: &'a ProjectionSpec, tenant: &'a eventlog_core::TenantId, prefix: Option<&'a str>, cursor: Option<&'a str>, limit: usize) -> eventlog_core::ProjectionPage;
        stream_identity(tenant: &'a eventlog_core::TenantId) -> String;
        put_blob(tenant: &'a eventlog_core::TenantId, digest: &'a str, bytes: &'a [u8]) -> ();
        get_blob(tenant: &'a eventlog_core::TenantId, digest: &'a str) -> Option<Vec<u8>>;
        delete_blob(tenant: &'a eventlog_core::TenantId, digest: &'a str) -> ();
    }

    fn create_projections(
        &self,
        projector: Arc<dyn Projector>,
    ) -> eventlog_core::BoxFuture<'_, Result<(), EventLogError>> {
        Box::pin(async move {
            let _registration = self.registration.lock().await;
            if self.phase.load(Ordering::Acquire) != 0 || !self.roster.admits(projector.as_ref()) {
                return Err(EventLogError::Invalid(
                    "projector is outside the exact unsealed host roster".to_owned(),
                ));
            }
            self.inner.create_projections(projector).await
        })
    }

    fn register_inline(
        &self,
        projector: Arc<dyn Projector>,
    ) -> eventlog_core::BoxFuture<'_, Result<(), EventLogError>> {
        Box::pin(async move {
            let _registration = self.registration.lock().await;
            if self.phase.load(Ordering::Acquire) != 0 || !self.roster.admits(projector.as_ref()) {
                return Err(EventLogError::Invalid(
                    "projector is outside the exact unsealed host roster".to_owned(),
                ));
            }
            let name = projector.name();
            self.inner.register_inline(projector).await?;
            self.registered
                .lock()
                .map_err(|_| EventLogError::Backend("host registration is unavailable".to_owned()))?
                .insert(name);
            Ok(())
        })
    }

    fn is_inline<'a>(&'a self, name: &'a str) -> eventlog_core::BoxFuture<'a, bool> {
        self.inner.is_inline(name)
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let terminate = async {
            if let Ok(mut stream) = signal(SignalKind::terminate()) {
                stream.recv().await;
            }
        };
        tokio::select! {
            result = tokio::signal::ctrl_c() => { let _ = result; }
            () = terminate => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn postgres_selection_requires_explicit_production_configuration() {
        let values = BTreeMap::from([
            (
                "DEMO_IDENTITY_ORIGIN".to_owned(),
                "https://identity.example.invalid".to_owned(),
            ),
            ("DEMO_PERSISTENCE".to_owned(), "postgres".to_owned()),
        ]);
        let result =
            HostConfig::from_lookup("DEMO", "demo.sqlite3", |name| values.get(name).cloned());
        assert!(
            result.is_err(),
            "explicit PostgreSQL selection must not silently open the SQLite default"
        );
    }

    #[test]
    fn unknown_persistence_selection_is_refused() {
        let values = BTreeMap::from([
            (
                "DEMO_IDENTITY_ORIGIN".to_owned(),
                "https://identity.example.invalid".to_owned(),
            ),
            ("DEMO_PERSISTENCE".to_owned(), "postgrez".to_owned()),
        ]);
        let result =
            HostConfig::from_lookup("DEMO", "demo.sqlite3", |name| values.get(name).cloned());
        assert!(
            result.is_err(),
            "an unknown configured persistence backend must refuse before serving"
        );
    }

    #[test]
    fn configuration_has_closed_names_and_safe_defaults() {
        let values = BTreeMap::from([(
            "TODO_IDENTITY_ORIGIN".to_owned(),
            "https://identity.example.invalid".to_owned(),
        )]);
        let config = HostConfig::from_lookup("TODO", "/var/lib/todo/todo.sqlite3", |name| {
            values.get(name).cloned()
        })
        .expect("configuration resolves");
        assert_eq!(config.listen, "0.0.0.0:8080".parse().unwrap());
        assert_eq!(config.identity_origin, "https://identity.example.invalid");
        assert_eq!(config.database_path, "/var/lib/todo/todo.sqlite3");
    }

    #[test]
    fn identity_origin_is_required() {
        let error = HostConfig::from_lookup("TODO", "todo.sqlite3", |_| None)
            .expect_err("missing Identity endpoint is refused");
        assert!(
            error
                .to_string()
                .contains("TODO_IDENTITY_ORIGIN is required")
        );
    }
}
