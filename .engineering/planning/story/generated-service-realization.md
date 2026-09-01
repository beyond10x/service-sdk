---
format: aep.planning-md/1
id: story:generated-service-realization
kind: story
status: implemented
title: Generate complete service realizations
summary: Provide versioned SDK obligations and synthesize definition-only services without handwritten runtime code.
relations:
- derived_from: epic:builder-runtime
- serves: vision:composable-services
revision: 4
---
# Generate complete service realizations

## Acceptance

- A modular service package compiles transactionally through official ESS IR and closed service-runtime IR.
- Every runtime obligation resolves to a versioned SDK catalog entry; unknown, incomplete, or uncovered obligations are refused.
- Synthesis emits a complete authenticated event-sourced Rust service, actual Connector factory, clients, projections, and conformance fixtures with no handwritten realization.
- Deployment injects persistence, content, authentication, clock, and identifier resources; application definitions never select or implement adapters.
- The Todo reference repository contains declarative source and manifest-owned generated output only.
