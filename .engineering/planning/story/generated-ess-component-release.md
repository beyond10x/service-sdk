---
format: aep.planning-md/1
id: story:generated-ess-component-release
kind: story
status: active
title: Generate standalone ESS component releases
summary: Reuse ESS deployment and OCI bundles for fully buildable generated services.
relations:
- decomposes: epic:builder-runtime
- serves: vision:composable-services
scope:
- confidence: cited
  path: .github/actions/release-component
- confidence: cited
  path: CHANGELOG.md
- confidence: cited
  path: Cargo.lock
- confidence: cited
  path: Cargo.toml
- confidence: cited
  path: README.md
- confidence: cited
  path: Taskfile.yml
- confidence: cited
  path: crates/service-builder
- confidence: cited
  path: crates/service-host
- confidence: cited
  path: crates/service-runtime
revision: 9
---
# Story: Generate standalone ESS component releases

## Outcome

Every Service SDK package generates a runnable host and the complete ESS component/build/runtime/chart inputs required for standalone build and independent release.

## Acceptance

- Service Builder consumes ESS deployment APIs directly and emits no parallel deployment model.
- Generated services include an executable Identity-authenticated HTTP host with deployment-injected persistence and lifecycle.
- A reusable, exact-revision CI adapter invokes ESS build, release verification, OCI bundling, signatures, SBOM, provenance, and conformance publication for any generated component.
- Workflow and Todo can build, smoke-test, bundle, and publish from generated output without handwritten runtime or Docker files.
- Generated-tree drift checks cover all component release inputs.
