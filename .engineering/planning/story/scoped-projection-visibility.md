---
format: aep.planning-md/1
id: story:scoped-projection-visibility
kind: story
status: implemented
title: Separate projection visibility from aggregate ownership
summary: Let generated service reads use verified scopes without conferring write ownership.
relations:
- derived_from: epic:builder-runtime
- serves: vision:composable-services
scope:
- confidence: cited
  path: .github/workflows/ci.yml
- confidence: cited
  path: CHANGELOG.md
- confidence: cited
  path: Cargo.lock
- confidence: cited
  path: Cargo.toml
- confidence: cited
  path: README.md
- confidence: cited
  path: crates/service-definition
- confidence: cited
  path: crates/service-engine
- confidence: cited
  path: crates/service-eventlog
- confidence: cited
  path: crates/service-obligations
- confidence: cited
  path: crates/service-runtime-ir
revision: 6
---
## Outcome

Generated services can expose tenant/realm-partitioned rows to principals admitted by conjunctive scopes without granting those readers aggregate ownership or mutation authority.

## Context

The existing `sdk.projection.auth-partitioned-visibility/v1` provider evaluates owner and scopes together. Workflow therefore cannot publish a reusable tenant library: a deployment publisher owns the aggregate, so every engineer is filtered from reads. Mutation authorization must remain owner-and-scopes while read visibility becomes scopes-only.

## Acceptance

- Add one versioned scopes-only projection/query obligation provider; do not change the semantics of the existing owner-and-scopes provider.
- The engine evaluates its row `scopes` through receiver-verified authority facts and remains tenant/realm partitioned by the projection request context.
- The obligation catalog, service-definition/runtime validation, generated plan consumption, and tests admit exactly the new provider.
- Tests prove a non-owner with matching scopes can read, a non-matching scope is hidden, and owner-only mutation checks are unchanged.
- Public SDK documentation and release notes explain the separation.

## Out of Scope

Workflow data seeding, deployment-specific identities, weakening tenant/realm partitioning, or allowing non-owners to mutate aggregates.

## Scope

- cited: service-obligations, service-engine, service-eventlog, service-definition/runtime validation, README, changelog, and workspace version metadata.
