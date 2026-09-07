---
format: aep.planning-md/1
id: story:connectors-0-7-contract
kind: story
status: active
title: Align generated service adapters with released Connectors 0.7
relations:
- decomposes: epic:builder-runtime
- informed_by: story:refresh-connectors-runtime
- serves: vision:composable-services
scope:
- confidence: cited
  path: Cargo.lock
- confidence: cited
  path: crates/service-catalog/Cargo.toml
- confidence: cited
  path: crates/service-catalog/src/lib.rs
- confidence: cited
  path: crates/service-conformance/Cargo.toml
- confidence: cited
  path: crates/service-connectors/Cargo.toml
- confidence: cited
  path: crates/service-connectors/src/lib.rs
- confidence: cited
  path: crates/service-host/Cargo.toml
revision: 5
---
## Outcome

Generated Connector factories and conformance tests consume the released Connectors 0.7.0 contract at its immutable release commit e80b7ae1b2151d13aa9786cf67ea05e66717ee35. Devcenter's operator explicitly requested this release. The current factory constructors omit the newly introduced optional rate-advice field and therefore fail to compile when composed with 0.7.

## Acceptance

- Update all four SDK crates that pin Connector protocol/service to the released 0.7.0 commit and refresh the affected locked graph.
- Operation descriptions from generated service and catalog factories explicitly have no rate advice when the SDK declaration has none; do not fabricate limits or retry behavior.
- Existing factory descriptions, invocation, admission and conformance behavior remain covered by the repository gate. Run task check before publishing.
- The correction lands in the owning SDK and is consumed at a published immutable revision by Devcenter. Generated application source is never edited by hand.

## Scope

Cited: crates/service-connectors, crates/service-catalog, crates/service-conformance and crates/service-host pin Connector dependencies. The first two construct OperationDescription in their runtime sources. Cargo.lock holds the resulting graph. No new SDK domain entity or persistence semantics are introduced; this aligns an existing external contract.

## Authorization

The operator requested the release migration. Advance this bounded story from draft through proposed to active before implementation. There is no multi-story decomposition requiring a critique panel.

## Verification

The complete task check passed, including fmt, clippy, all workspace tests, generated fixture freshness, documentation, web checks and the required PostgreSQL persistence/workload proof. The proof exercised actual database loss/recovery, process restart, saturation/cancellation and all six two-process workload configurations. Initial local attempts exposed assumptions about the test binary's repository-local target path and the disposable database's remapped port; the final run used the documented repository target and current endpoint. No gate, test or runtime source was bypassed or weakened.

Devcenter's root client migrated to released 0.7.0 and its composed runtime compiled with these candidate SDK crates and the SDK's corresponding Eventlog revision. Temporary path resolution is only a local pre-publication check; the final composition will pin the published SDK commit.
