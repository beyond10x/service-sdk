---
format: aep.planning-md/1
id: story:typescript-catalog-bindings
kind: story
status: draft
title: Emit native TypeScript catalog bindings
summary: Derive the SDK-owned browser contract from Rust with ts-rs under service-builder ownership.
relations:
- derived_from: epic:builder-runtime
- serves: vision:composable-services
revision: 1
---
## Context

The Rust `service-catalog` contract defines `ServiceCatalog`, `CatalogAuthentication`,
`CatalogOperation`, and their enums in `crates/service-catalog/src/lib.rs:30`, while
`web/service-console/src/types.ts:1` independently hand-declares the corresponding TypeScript
surface. `service-builder` already exclusively owns generated output and emits the console scaffold
from `crates/service-builder/src/realization.rs:832`; the duplicate type declarations can drift
from Serde rename and optionality semantics.

## Scope

- cited: `crates/service-catalog`, `crates/service-builder`, `web/service-console/src/types.ts`, `web/service-console/src/index.ts`, console tests, workspace manifests, and builder drift tests.
- inferred: feature-gated `ts-rs` derives for SDK-owned wire DTOs, deterministic builder-owned TypeScript output, and Rust-JSON/TypeScript conformance vectors.

## Boundaries

- Emit only SDK-owned transport, catalog, authentication-metadata, and binding DTOs; ESS remains the owner of semantic compilation, structural domain types, and general-purpose target emitters.
- Preserve the exact Serde wire spelling, tagged-enum representation, optional/null behavior, and opaque JSON Schema or ESS semantic payloads.
- Keep runtime `unknown` validation at the browser boundary; generated compile-time declarations do not replace `assertServiceCatalog` or authority-coordinate refusals.
- Rewrite generated TypeScript only through `service-builder`, include it in the output ownership manifest, and byte-check it for drift.

## Acceptance

`service-builder generate` deterministically emits `ts-rs`-derived TypeScript declarations for every SDK-owned service-catalog wire type, the console consumes those declarations without handwritten duplicates, TypeScript compilation and cross-language JSON vectors pass, and no ESS-owned semantic type or authentication coordinate becomes caller-controlled.
