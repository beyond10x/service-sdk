---
format: aep.planning-md/3
id: story:preserve-nested-optional-types
kind: story
status: implemented
title: Preserve nested Optional types in Rust bindings
summary: Generated Rust must retain Optional wrappers inside collection shapes.
relations:
- derived_from: story:optional-projection-fields
- serves: vision:composable-services
scope:
- confidence: cited
  path: crates/service-builder/src/realization.rs
- confidence: cited
  path: crates/service-builder/tests/build.rs
revision: 5
transitions:
- {from: "draft", to: "proposed", at: "2026-09-04T12:02:54Z", actor: "human:timo", revision: 3, imported: true}
- {from: "proposed", to: "active", at: "2026-09-04T12:02:54Z", actor: "human:timo", revision: 4, imported: true}
- {from: "active", to: "implemented", at: "2026-09-04T12:07:43Z", actor: "human:timo", revision: 5, decided_on: {"recorded":{"test_result":1}}, imported: true}
---
## Context

Generated Rust type rendering strips `Optional<T>` nested inside a list but does not retain its Option wrapper, so `List<Optional<String>>` becomes `Vec<String>` instead of `Vec<Option<String>>`. The generated binding therefore contradicts the ESS type even though top-level Optional values are correct.

## Acceptance

Generated Rust recursively preserves Optional at every type nesting level, with focused coverage for `List<Optional<String>>` and top-level Optional/List combinations. Existing scalar, list, and top-level optional bindings remain unchanged.
