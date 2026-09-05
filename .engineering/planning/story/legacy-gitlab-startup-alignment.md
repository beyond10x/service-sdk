---
format: aep.planning-md/1
id: story:legacy-gitlab-startup-alignment
kind: story
status: implemented
title: Align composition with safe legacy GitLab startup
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
  path: crates/service-catalog/Cargo.toml
- confidence: cited
  path: crates/service-conformance/Cargo.toml
- confidence: cited
  path: crates/service-connectors/Cargo.toml
revision: 7
---
## Outcome

Consume the exact published compatible provider revision that leaves legacy GitLab connections unusable while keeping the host available for verified reconnection. Align the nominal Connector service factory types across Service SDK, generated AgentIDE and Todo, and the composed Devcenter service. This is a source-pin and release-identity update; the semantic service contracts remain unchanged.

## Acceptance

Regenerate owned outputs through their declared generator, refresh locks, and run the complete repository gate. Publish exact source coordinates for the provider-first deployment. Legacy credentials must not acquire a current grant from configuration; recovery requires the existing verified connect flow.

## Completed validation

The complete repository gate passed for this source candidate. The affected GitLab transport and upgrade regression suite also passed all 40 tests, and all-target Clippy passed with warnings denied. Exact provider source pins retain current-authority checks and the existing service contracts. This source is ready for the approved provider-first rollout; hosted success remains dependent on verifying the published runtime artifacts and normal reconnection of legacy GitLab accounts.
