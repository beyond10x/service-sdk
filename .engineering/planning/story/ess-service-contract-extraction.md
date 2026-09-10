---
format: aep.planning-md/1
id: story:ess-service-contract-extraction
kind: story
status: active
title: Consume reusable ESS service contract resolution
relations:
- serves: vision:composable-services
scope:
- confidence: cited
  path: Cargo.lock
- confidence: cited
  path: Cargo.toml
- confidence: cited
  path: README.md
- confidence: cited
  path: crates/service-runtime-ir
revision: 5
---
## Outcome

Consume ess-service-contract from the published ESS extraction candidate. Remove the duplicated required-capability resolver and command/view field summarizers from service-runtime-ir; preserve its public RequiredDisposition name through re-export and map errors into existing RuntimeCode/path/message values. Keep definition validation, ESS/synthesis provenance binding, obligation admission and every auth/content/execution decision in the SDK. Existing service-runtime-ir/3 bytes must remain identical.

## Ownership

This is the Service SDK side of ESS story:service-contract-extraction and docs/design/service-contract.md. It serves O2 in AGENTS.md and the operator-authorized ESS evolution semantic convergence. No release or production deployment is requested. Pin all ESS packages consistently to the exact published provider revision, updating the lock without unrelated dependency upgrades.

## Verification

Capture the existing runtime compiler fixture's canonical bytes against source c70c954da43c063143b34601ef8af7a1c511e5aa before changing implementation. Reuse those bytes as an exact compatibility fixture after extraction. Existing runtime-IR compilation, tampering, provenance, obligation and annotation refusals must pass. Run affected package tests and strict Clippy. No full or remote persistence gate is authorized; ER execution migration and application adoption remain outstanding.

## Implementation and observed compatibility

ESS provider d453c50769f08d1c348daad3c2e8b721be571078 is published on feat/service-contract-extraction. All SDK workspace ESS dependencies now use that exact revision; Cargo.lock changes only those ESS packages and the added ess-service-contract edge. service-runtime-ir re-exports RequiredDisposition at its original public path, consumes ESS field summaries, and maps extracted cardinality/refusal errors back into its existing RuntimeCode/path/message surface. The former local type and resolver/summary implementations were removed. All runtime policy remains in the SDK.

Captured the existing composed-annotation compiler fixture against unmodified runtime implementation at c70c954da43c063143b34601ef8af7a1c511e5aa, before extraction. Retained its 8,951 canonical bytes as tests/fixtures/runtime-before-service-contract.json. The extracted compiler emits exactly those bytes. Both already-committed service-host persistence profiles, factory and standalone, also compile to their original generated/runtime/ir.json bytes and remain readable without regeneration. This is compatibility evidence, not proof that the hosts were started.

All five original runtime compiler tests pass after extraction, covering canonical binding, persisted input/tampering, unsupported annotations, wrong-model provenance and authentication coordinates. The persisted-reader case now also rejects unknown fields inside a generated disposition, closing the old serde unit-variant gap without changing valid bytes. The additional existing-host fixture test passed for both profiles. Total: six targeted tests, no ignored cases. Strict service-runtime-ir all-target Clippy passed. cargo check --locked --workspace --all-targets passed in 39.86 seconds, including both generated host crates. No full suite, remote persistence proof or live host test ran.

Evidence: local-evidence:ess-evolution-20260910/sdk-runtime-before-extraction.log; sdk-runtime-before-extraction.json; sdk-service-contract-tests.log; sdk-service-contract-host-fixtures.log; sdk-service-contract-clippy.log; sdk-service-contract-workspace-build.log. Main integration remains pending. Next: converge service-engine decisions/replay with the governed ER lowering while preserving these SDK binding and runtime format boundaries; this extraction does not claim that migration or application adoption is complete.
