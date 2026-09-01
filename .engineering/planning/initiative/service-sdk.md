---
format: aep.planning-md/1
id: initiative:service-sdk
kind: initiative
status: active
title: Service SDK 0.1
summary: Build the minimal ESS-backed service builder, runtime ports, generators, and conformance seams.
relations:
- serves: vision:composable-services
revision: 3
---
# Service SDK 0.1

## Outcome

Provide the smallest safe builder and runtime seams needed to turn validated ESS into standalone Rust services.

## Constraints

AEP governs development only. ESS owns semantic IR. The SDK owns lossless service-runtime IR, deterministic synthesis, runtime ports, and optional adapters.
