---
format: aep.planning-md/1
id: story:refresh-connectors-runtime
kind: story
status: implemented
title: Refresh the generated Connector runtime
summary: Promote Connectors 0.5.11 through the Service SDK so generated services share one verified authority/runtime contract.
relations:
- decomposes: epic:builder-runtime
- serves: vision:composable-services
scope:
- confidence: cited
  path: CHANGELOG.md
- confidence: cited
  path: Cargo.lock
- confidence: cited
  path: Cargo.toml
- confidence: cited
  path: crates/service-catalog/Cargo.toml
- confidence: cited
  path: crates/service-conformance/Cargo.toml
- confidence: cited
  path: crates/service-connectors/Cargo.toml
revision: 7
---
# Story: Refresh the generated Connector runtime

## Outcome

Generated-service maintainers can compose Connectors 0.5.11 through Service SDK 0.5.5 without duplicate Connector factory types.

## Context

GitLab OAuth refresh responses may omit an unchanged scope. Connectors 0.5.11 accepts that valid response while continuing to verify refreshed authority before committing token rotation. Every Service SDK crate that imports the Connector factory contract must therefore advance together.

## Acceptance

- Service SDK Connector catalog, factory, and conformance crates pin the merged Connectors 0.5.11 commit.
- The workspace lock contains one Connector protocol and service revision for those crates.
- Service SDK reports version 0.5.5 with a release-note entry for the authority-refresh compatibility change.
- `task check` passes.

## Out of Scope

Regenerating or releasing application repositories and deploying a product composition are downstream promotions.

## Open Questions

None.
