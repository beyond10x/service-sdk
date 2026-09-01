---
format: aep.planning-md/1
id: epic:builder-runtime
kind: epic
status: active
title: Builder and runtime foundations
summary: Compile official ESS into strict runtime IR and deterministic standalone service artifacts.
relations:
- derived_from: initiative:service-sdk
- serves: vision:composable-services
revision: 3
---
# Builder and runtime foundations

## Scope

Define the author-facing service model, compile it through official ESS IR into strict runtime IR, synthesize contracts and clients, and provide minimal authentication, content, event-log, projection, and Connector factory ports.

## Done When

Every accepted annotation survives into versioned IR, every unsupported semantic is refused, generated code compiles independently, and deterministic regeneration plus conformance gates pass.
