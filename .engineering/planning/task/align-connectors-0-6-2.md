---
format: aep.planning-md/3
id: task:align-connectors-0-6-2
kind: task
status: implemented
title: Align generated services with Connectors 0.6.2
summary: Advance the nominal Connector factory identity used by every generated service to the exact Git broker release.
relations:
- derived_from: story:align-current-connectors-service-identity
- serves: vision:composable-services
revision: 5
transitions:
- {from: "draft", to: "proposed", at: "2026-09-04T23:34:55Z", actor: "human:timo", revision: 2, imported: true}
- {from: "proposed", to: "active", at: "2026-09-04T23:34:55Z", actor: "human:timo", revision: 3, imported: true}
- {from: "active", to: "implemented", at: "2026-09-04T23:45:28Z", actor: "human:timo", revision: 5, decided_on: {"recorded":{"test_result":1}}, imported: true}
---
# Task: Align generated services with Connectors 0.6.2

## What

Pin the Service SDK Connector factory, catalogue, and conformance crates to the exact Connectors 0.6.2 commit and publish Service SDK 0.5.9.

## Why

Generated applications must implement the same nominal ConnectorServiceFactory trait as the Devcenter host. Without this bridge, independently released services cannot be composed with the current Connectors runtime.

## Done When

- Every Service SDK Connector dependency resolves to Connectors 0.6.2 at commit 03d7fc146c2e11f56ae6fb12e5da578821e2be15.
- The complete task check passes with locked dependency resolution.
- The bot-authored commit is published on the repository default branch and the 0.5.9 tag peels to it.

## Notes

This is the compatibility bridge for the downstream AgentIDE and Todo regeneration needed by Devcenter; it does not bundle or deploy those applications.
