---
format: aep.planning-md/1
id: story:operational-connector-runtime
kind: story
status: implemented
title: Generate an operational Connector runtime
summary: Make SDK obligations, Eventlog persistence, and Connector dispatch deployable from generated service artifacts.
relations:
- derived_from: epic:builder-runtime
- serves: vision:composable-services
revision: 4
---
## Acceptance

- The SDK supplies Eventlog SQLite/PostgreSQL-compatible event, inline projection, and erasable content adapters behind generated resource ports.
- Generated Connector factories bind the SDK engine directly; application repositories inject adapters but implement no obligations or dispatch.
- Authenticated tenant/user/authority/executor and exact optional realm are mapped before body decoding.
- Read and write operations are exercised through the generated Connector backend with restart-safe state and idempotency.
- Deterministic regeneration and all SDK gates pass.
