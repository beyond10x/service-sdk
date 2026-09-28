---
format: aep.planning-md/3
id: story:align-current-connectors-service-identity
kind: story
status: implemented
title: Align generated services with the current Connectors service identity
relations:
- derived_from: epic:builder-runtime
- serves: vision:composable-services
scope:
- confidence: cited
  path: .engineering/planning
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
revision: 8
transitions:
- {from: "draft", to: "proposed", at: "2026-09-04T14:27:00Z", actor: "human:timo", revision: 4, imported: true}
- {from: "proposed", to: "active", at: "2026-09-04T14:27:00Z", actor: "human:timo", revision: 5, imported: true}
- {from: "active", to: "implemented", at: "2026-09-04T14:28:46Z", actor: "human:timo", revision: 7, decided_on: {"recorded":{"test_result":1}}, imported: true}
---
## Outcome

Generated service factories and a composing Connector runtime share the exact current Connectors `service` crate identity, so an independently promoted runtime remains type-coherent.

## Acceptance

- Service SDK pins its Connector protocol/service dependencies to the exact current Connectors default-branch commit.
- The root lock contains no prior Connector source revision.
- `task check` passes and downstream generated-service consumers can advance by exact Service SDK commit without a protocol migration.
