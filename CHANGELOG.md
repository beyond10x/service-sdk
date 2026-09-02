# Changelog

## 0.2.1 - 2026-09-02

- Align generated Connector factories, catalogs, and conformance execution with Connectors 0.4.4 so downstream runtimes share one exact service trait while consuming the hosted catalog and provider-profile update.

## 0.2.0 - 2026-09-02

- Generate exact `service-catalog/1` documents from ESS's `ess-browser-catalog/1` and the same runtime schemas published by each domain Connector factory.
- Provide a separate, read-only `ServiceCatalogFactory` through the existing external Connector factory and deployment-overlay seam, with no changes to Connectors.
- Provide the reusable `@b10x/service-console-vue` widget, explicit live and demo bindings, write confirmation, read-your-writes query refresh, lifecycle/view presentation, and activity results.
- Generate a standalone documentation app for every synthesized service while keeping authentication coordinates, including realm, out of routes and operation inputs.

## 0.1.1 - 2026-09-02

- Align generated Connector factories and conformance execution with Connectors 0.4.3 so composed
  services share one exact factory trait with hosted Integration administration.

## 0.1.0 - 2026-09-02

- Compile `service/1` packages from ESS semantics, runtime obligation bindings, exact SDK locks, and declarative scenarios.
- Generate deterministic Eventlog-backed Rust services, operation client plans, and composable Connector factories without handwritten application runtime.
- Keep tenant, authority, user, executor, and optional realm in verified authentication context and out of operation inputs.
- Supply versioned obligations for optimistic concurrency, authorization, nested authority, lifetime, content custody, projections, and read-your-writes behavior.
- Execute generated service scenarios through the Connector factory with deployment-injected authority facts and exact absent-versus-`default` realm isolation.
