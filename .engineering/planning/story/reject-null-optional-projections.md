---
format: aep.planning-md/3
id: story:reject-null-optional-projections
kind: story
status: implemented
title: Reject null optional projection values
summary: ESS Optional object fields admit absence, not explicit JSON null.
relations:
- derived_from: story:optional-projection-fields
- serves: vision:composable-services
scope:
- confidence: cited
  path: crates/service-engine/src/lib.rs
revision: 5
transitions:
- {from: "draft", to: "proposed", at: "2026-09-04T11:54:21Z", actor: "human:timo", revision: 3, imported: true}
- {from: "proposed", to: "active", at: "2026-09-04T11:54:21Z", actor: "human:timo", revision: 4, imported: true}
- {from: "active", to: "implemented", at: "2026-09-04T12:07:43Z", actor: "human:timo", revision: 5, decided_on: {"recorded":{"test_result":1}}, imported: true}
---
## Context

ESS object-field `Optional<T>` is represented by field absence; explicit JSON null is not a member of the type. Projection validation currently accepts a present optional field whose value is null, allowing a row that contradicts the generated wire contract.

## Acceptance

A declared optional projection field may be omitted, but a present null value is rejected. Valid present values continue to pass, and required/extra-field validation is unchanged.
