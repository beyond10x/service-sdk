---
format: aep.planning-md/3
id: initiative:service-sdk
kind: initiative
status: active
title: Service SDK 0.1
summary: Build the minimal ESS-backed service builder, runtime ports, generators, and conformance seams.
relations:
- serves: vision:composable-services
revision: 3
transitions:
- {from: "draft", to: "proposed", at: "2026-09-01T21:37:45Z", actor: "human:timo", revision: 2, imported: true}
- {from: "proposed", to: "active", at: "2026-09-01T21:37:46Z", actor: "human:timo", revision: 3, imported: true}
---
# Service SDK 0.1

## Outcome

Provide the smallest safe builder and runtime seams needed to turn validated ESS into standalone Rust services.

## Constraints

AEP governs development only. ESS owns semantic IR. The SDK owns lossless service-runtime IR, deterministic synthesis, runtime ports, and optional adapters.
