---
format: aep.planning-md/1
id: story:optional-projection-fields
kind: story
status: implemented
title: Materialize absent optional projection fields
summary: Generated services must not reject valid rows merely because an optional view field is absent.
relations:
- derived_from: epic:builder-runtime
- serves: vision:composable-services
scope:
- confidence: cited
  path: crates/service-engine
- confidence: cited
  path: crates/service-eventlog/src/lib.rs
- confidence: cited
  path: crates/service-eventlog/tests
revision: 5
---
## Context

Service SDK projection materialization omits an absent optional field, while row validation requires every declared view field to be present. A valid aggregate whose optional projection value is absent therefore becomes an invalid projection row and the generated HTTP service returns `500 service_contract`. Workflow demonstrates the failure after creating a definition without `active_revision_id`, but the defect applies to every generated service with optional view fields.

## Acceptance

Projection rows represent absent optional view fields canonically without weakening required-field validation. A focused regression builds and queries a row with an absent optional field through the service projection path and receives a valid row whose field is null or otherwise matches the released wire contract. Missing required fields remain rejected, existing projection visibility and pagination behavior remain unchanged, and the full Service SDK gate passes.

## Out of scope

Workflow-specific seeding, Devcenter error masking, database rewrites, and treating every missing field as optional.
