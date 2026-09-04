---
format: aep.planning-md/1
id: story:align-current-connectors-service-identity
kind: story
status: implemented
title: Align generated services with the current Connectors service identity
relations:
- derived_from: epic:builder-runtime
- serves: vision:composable-services
scope:
- confidence: cited
  path: CHANGELOG.md
- confidence: cited
  path: Cargo.lock
- confidence: cited
  path: crates/service-catalog/Cargo.toml
- confidence: cited
  path: crates/service-conformance/Cargo.toml
- confidence: cited
  path: crates/service-connectors/Cargo.toml
revision: 7
---
## Outcome

Generated service factories and a composing Connector runtime share the exact current Connectors `service` crate identity, so an independently promoted runtime remains type-coherent.

## Acceptance

- Service SDK pins its Connector protocol/service dependencies to the exact current Connectors default-branch commit.
- The root lock contains no prior Connector source revision.
- `task check` passes and downstream generated-service consumers can advance by exact Service SDK commit without a protocol migration.
