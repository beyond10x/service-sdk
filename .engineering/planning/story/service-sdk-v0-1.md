---
format: aep.planning-md/3
id: story:service-sdk-v0-1
kind: story
status: implemented
title: Build service-sdk 0.1
summary: Implement lossless compilation, runtime ports, deterministic generators, Connector contribution, and conformance.
relations:
- derived_from: epic:builder-runtime
- serves: vision:composable-services
revision: 4
transitions:
- {from: "draft", to: "proposed", at: "2026-09-01T21:37:46Z", actor: "human:timo", revision: 2, imported: true}
- {from: "proposed", to: "active", at: "2026-09-01T21:37:46Z", actor: "human:timo", revision: 3, imported: true}
- {from: "active", to: "implemented", at: "2026-09-01T22:27:38Z", actor: "human:timo", revision: 4, decided_on: {"recorded":{"test_result":1}}, imported: true}
---
# Build service-sdk 0.1

## Acceptance

- The definition compiler consumes only official validated ESS IR and binds its exact source digest.
- Admission verifies authentication, authority, and Required/Optional/Forbidden realm policy before application decoding.
- Mutations rehydrate, validate intent, decide commands, guarded-append events with an expected version, reduce state, and update projections.
- Content staging is idempotent and typed references cannot be forged from arbitrary strings.
- Synthesis emits deterministic Rust contracts, service seams, OpenAPI, clients, inert Connector contributions, and conformance fixtures.
- Runtime core does not depend on AEP or ESS compiler internals.
