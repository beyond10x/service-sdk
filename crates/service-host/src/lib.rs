//! Reusable process host for generated Identity-authenticated services.
//!
//! Generated packages supply only their compiled router. This crate owns the operational shell:
//! environment loading, durable SQLite initialization, listener lifecycle, and graceful shutdown.

#![forbid(unsafe_code)]

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use eventlog_sqlite::SqliteEventStore;

/// Fully resolved, non-secret process configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostConfig {
    listen: SocketAddr,
    identity_origin: String,
    database_path: String,
}

impl HostConfig {
    /// Reads `<PREFIX>_LISTEN`, `<PREFIX>_IDENTITY_ORIGIN`, and `<PREFIX>_DATABASE_PATH`.
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
        Ok(Self {
            listen,
            identity_origin,
            database_path,
        })
    }
}

/// Runs one generated service with its SDK-owned SQLite persistence adapter.
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
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("generated service Tokio runtime is unavailable")?;
    runtime.block_on(async move {
        let store: Arc<dyn service_connectors::DurableEventStore> = Arc::new(
            SqliteEventStore::open(&config.database_path, store_prefix)
                .await
                .context("generated service Eventlog store is unavailable")?,
        );
        let application = router(store, config.identity_origin)
            .await
            .context("generated Identity HTTP service is unavailable")?;
        let listener = tokio::net::TcpListener::bind(config.listen)
            .await
            .context("generated service listener is unavailable")?;
        axum::serve(listener, application)
            .with_graceful_shutdown(shutdown_signal())
            .await
            .context("generated service failed")
    })
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
