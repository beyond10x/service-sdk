# Changelog

## 0.5.7 - 2026-09-04

- Separate scope-based projection visibility from aggregate ownership so tenant/realm-partitioned
  service libraries can be read by admitted principals without granting them mutation authority.

## 0.5.6 - 2026-09-04

- Emit the generated executable host only for services that declare Identity HTTP delivery, while
  keeping composed Connector services library-only.

## 0.5.5 - 2026-09-04

- Align generated Connector factories and conformance with Connectors 0.5.11 so OAuth refresh
  responses that omit an unchanged scope remain usable without splitting the composed runtime's
  factory type graph.

## 0.5.4 - 2026-09-04

- Add a reusable, configuration-neutral GitHub Action that drives generated service conformance,
  ESS BuildKit execution, Sigstore signatures, SBOM and provenance evidence, release verification,
  and canonical OCI component-bundle publication from each component repository.

## 0.5.3 - 2026-09-04

- Execute generated Helm packaging from the repository root copied into the ESS build graph, so
  independently released components can publish their generated charts.

## 0.5.2 - 2026-09-04

- Allow each service package to pin a separate minimal OCI runtime base, copying only the generated
  executable out of the Rust build stage instead of shipping the compiler and source tree.

## 0.5.1 - 2026-09-04

- Upgrade to ESS 0.13.1 so generated configuration-neutral charts pass Helm lint before private
  environment bindings are supplied.

## 0.5.0 - 2026-09-04

- Generate an executable service entry point backed by the reusable `service-host` lifecycle and
  SQLite Eventlog adapter, removing handwritten process hosts from generated-service repositories.
- Compile optional `service/1` release coordinates through ESS 0.13 into canonical component,
  realization, build, and runtime IR plus executable BuildKit inputs and a configuration-neutral
  Helm chart.
- Include every release input in the generated-tree ownership manifest so CI refuses drift before
  publishing a component bundle.

## 0.4.2 - 2026-09-03

- Canonically wrap every generated HTTP client call chain so formatting is independent of service
  and operation name length.

## 0.4.1 - 2026-09-03

- Emit rustfmt-clean generated Identity HTTP client code for empty request DTOs and services with a
  larger operation inventory.

## 0.4.0 - 2026-09-03

- Add first-class `identity_http` delivery with an exact Identity audience, per-operation scopes,
  generated typed Rust clients, an Axum server boundary, and deterministic OpenAPI 3.1 output.
- Require `service-definition/3`, `service-runtime-ir/3`, `service-client-plan/2`, and
  `service-realization-plan/2`; superseded generated contracts are rejected with no compatibility
  path.
- Add closed directed-graph obligations for node and edge mutation, acyclicity, referenced-node
  protection, immutable publish snapshots, and deterministic graph digests.
- Align generated Connector factories, catalogs, and conformance with Connectors 0.5.6 so composed
  runtimes use one exact factory type.

## 0.3.4 - 2026-09-03

- Publish the complete Service SDK source and history under Apache 2.0 so generated-service
  consumers can build from exact public Git revisions.
- Add a public developer quickstart, contribution and security guidance, repository CI, and the
  documentation surface metadata needed for organization-owned delivery.

## 0.3.3 - 2026-09-03

- Return the exact authorized aggregate revision with bounded single-stream projection pages,
  while leaving mixed, empty, and source-less pages explicitly unknown.
- Recover aggregate source metadata for projections written before source tracking was introduced,
  without exposing that hidden metadata as application data.

## 0.3.2 - 2026-09-03

- Align generated-service Connector factories, catalogs, and conformance contracts with
  Connectors 0.5.3 so composed runtimes share one exact factory trait.

## 0.3.1 - 2026-09-03

- Align generated-service Connector factories and conformance contracts with Connectors 0.5.2,
  retaining delegated execution provenance across the released composition boundary.

## Unreleased

- Align generated Connector factories and conformance with Connectors 0.5.0, preserving
  receiver-verified agent, attempt, delegation, and grant provenance through service execution.
- Carry receiver-verified agent, attempt, delegation, and grant provenance alongside the existing
  generated-service authentication context.
- Add bounded projection pages and authorized aggregate event cursors on top of the organization
  Eventlog implementation.
- Add exact external-effect plans, an Eventlog-backed durable prepare/claim/complete journal, and
  a recovery-first adapter contract that records uncertain outcomes instead of blind retries.

## 0.3.0 - 2026-09-02

- Add a documented semantic CSS token contract so composed products can theme the generated Vue
  service console without product-specific props or storage coupling.
- Preserve accessible neutral fallbacks, inherited native control color scheme, and explicit focus,
  status, code, and elevation styling for standalone generated documentation.

## 0.2.4 - 2026-09-02

- Align generated factories, external service catalogs, and conformance with the generic Connector
  embedding seam that lets SIP-disabled compositions omit the unrelated voice dependency graph.
- Preserve the existing external factory and deployment-overlay architecture; this release adds no
  generated-service concept to Connectors.

## 0.2.3 - 2026-09-02

- Keep service-console activity records reactive so completed live and demo calls repaint from
  `running` to their exact result or failure state.
- Cover the asynchronous activity result in the SDK widget test instead of observing only the
  outbound binding call.

## 0.2.2 - 2026-09-02

- Pin generated service documentation to the supported TypeScript 5 line and explicitly allow only
  esbuild's install step, keeping standalone docs reproducible under current pnpm defaults.

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
