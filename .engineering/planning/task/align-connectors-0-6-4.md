---
format: aep.planning-md/1
id: task:align-connectors-0-6-4
kind: task
status: implemented
title: Align generated services with Connectors 0.6.4
summary: Use one exact Connector factory identity for the faster Smart Git runtime and generated consumers.
relations:
- derived_from: story:align-current-connectors-service-identity
- serves: vision:composable-services
revision: 5
---
# Align generated services with Connectors 0.6.4

## Outcome

Advance the SDK Connector factory, catalogue, and conformance dependencies to the published Connectors 0.6.4 commit dbdd285c629d8b93bb685cc5a89a270316978ce5 and prepare Service SDK 0.5.10. Generated consumers and the Devcenter host must share one nominal Connector service identity when composing the faster Smart Git runtime.

## Scope

Change only the exact Connector dependency coordinates, workspace patch version, Cargo lockfile, changelog, and this governed planning record. Existing generated-service APIs, authenticated authority, and runtime obligations remain the released contracts.

## Acceptance

- Every SDK Connector dependency resolves to the published Connectors 0.6.4 commit.
- The complete task check passes with locked dependency resolution.
- The candidate is handed to the coordinating agent for bot publication before downstream generators consume its exact commit.

## Evidence

Source inspection found no smaller released compatibility seam because generated factories expose the provider's nominal Rust service trait. The six direct Connector coordinates and all nine resolved provider packages now point to dbdd285c629d8b93bb685cc5a89a270316978ce5; the lockfile otherwise changes only the twelve SDK workspace versions to 0.5.10.

On 2026-09-05, task check exited 0: Rust formatting, all-target warning-denying Clippy, workspace tests and doc tests, warning-denying Rustdoc, AEP validation, release-action checks, and the pinned pnpm 10.15.0 console typecheck plus four browser unit tests. git diff --check also passed. The candidate is ready for the coordinating agent's bot publication; downstream consumers will use that exact published SDK commit.
