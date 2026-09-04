---
format: aep.planning-md/1
id: story:conditional-generated-host
kind: story
status: implemented
title: Emit generated hosts only for HTTP services
summary: Keep Connector-only generated services library-only while preserving standalone hosts for Identity HTTP delivery.
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
  path: crates/service-builder/src/realization.rs
- confidence: cited
  path: crates/service-builder/tests/build.rs
revision: 6
---
# Story: Emit generated hosts only for HTTP services

## Outcome

Service authors can regenerate Connector-only services without receiving a broken standalone HTTP executable.

## Context

Service SDK 0.5.5 emitted `rust/src/main.rs` and `service-host` for every package even though `http_router` is intentionally generated only for Identity HTTP delivery. AgentIDE's Connector-only gate exposed the mismatch.

## Acceptance

- Identity HTTP packages retain their generated standalone host.
- Composed Connector packages omit `service-host`, `anyhow`, and `rust/src/main.rs`.
- A regression test generates a Connector-only package and verifies the host is absent.
- `task check` passes.

## Out of Scope

Changing AgentIDE's declared delivery or adding an HTTP API to a Connector-only service.

## Open Questions

None.
