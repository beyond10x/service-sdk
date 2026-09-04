---
format: aep.planning-md/1
id: story:generated-client-host-separation
kind: story
status: implemented
title: Separate generated clients from standalone hosts
summary: Let consumers use an SDK-generated client without linking the service process host or persistence adapter.
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
revision: 5
---
## Outcome

A generated Identity HTTP service package remains directly executable while downstream consumers can compile only its client/library surface.

## Context

Generated packages currently make `service-host` unconditional. A client consumer therefore resolves the service's SQLite adapter and can collide with its own persistence stack even though it never runs the generated binary.

## Acceptance

- Identity HTTP packages expose the standalone host as a default feature and mark host-only dependencies optional.
- The generated binary explicitly requires that host feature.
- Consumers can disable default features and use the generated typed client without resolving the host adapter.
- Generation tests assert the feature and binary contract; generated standalone builds retain their current behavior.

## Out of Scope

Splitting the generated library into separately published crates or changing service HTTP semantics.

## Scope

Service Builder Rust package generation, tests, documentation, and release notes.
