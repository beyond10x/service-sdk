---
format: aep.planning-md/1
id: story:generated-identity-http-delivery
kind: story
status: implemented
title: Generate Identity-authenticated HTTP delivery
summary: Generate a supported HTTP server and typed client from one service package.
relations:
- derived_from: epic:builder-runtime
- serves: vision:composable-services
scope:
- confidence: cited
  path: CHANGELOG.md
- confidence: cited
  path: Cargo.lock
- confidence: cited
  path: Cargo.toml
- confidence: cited
  path: README.md
- confidence: cited
  path: crates/service-builder
- confidence: cited
  path: crates/service-conformance
- confidence: cited
  path: crates/service-definition
- confidence: cited
  path: crates/service-engine
- confidence: cited
  path: crates/service-eventlog
- confidence: cited
  path: crates/service-obligations
- confidence: cited
  path: crates/service-runtime
- confidence: cited
  path: crates/service-runtime-ir
revision: 5
---
# Generated Identity-authenticated HTTP delivery

## Outcome

Service authors can generate a supported Identity-authenticated HTTP server and typed client from one `service/1` package, without handwritten delivery adapters.

## Context

`epic:builder-runtime` already owns deterministic service generation. Workflow authoring needs a stable audience, operation scopes, HTTP transport, problem responses, probes, and client contract that are all generated from the same validated runtime IR.

## Acceptance

- A new breaking runtime format replaces `service-definition/2`, `service-runtime-ir/2`, and `service-client-plan/1`; old formats are rejected and no compatibility reader is retained.
- A service declares either Identity-authenticated HTTP delivery or composed Connector delivery.
- The builder emits a compiling Rust HTTP server and typed client, deterministic DTOs, OpenAPI, problem responses, and health/readiness probes.
- Identity verification and exact operation-scope authorization occur before application input decoding.
- Generated mutation paths carry idempotency keys and expected aggregate versions through guarded Eventlog append.
- Reusable obligation providers cover owned nested draft mutation, graph validation, canonical publish, and owned revision activation.
- Generated standalone services can inject SQLite or PostgreSQL Eventlog adapters.
- The complete workspace uses the exact Connectors 0.5.6 revision and `task check` passes.

## Out of Scope

Workflow execution, triggers, schedules, leases, cancellation, arbitrary proxying, caller-supplied tenant or realm, and compatibility with the superseded generated formats.

## Scope

- cited: `crates/service-definition`, `crates/service-runtime-ir`, `crates/service-builder`, `crates/service-engine`, `crates/service-runtime`, `crates/service-eventlog`, `crates/service-obligations`, `crates/service-conformance`, generated examples, workspace manifests, README and changelog.
- inferred: no consumer-owned runtime adapters are introduced; AgentIDE, Todo, and Workflow consume only generated output.
