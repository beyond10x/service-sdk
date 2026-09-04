---
format: aep.planning-md/1
id: review-result:recovery-wave-one-optional-projection-fields-pass-2
kind: review-result
status: active
title: 'Adversarial review: optional projection fields pass 2'
summary: Plan integrity, format compatibility, and nested Optional rendering require final correction.
owner: wave-adversary
relations:
- reviews: story:optional-projection-fields
revision: 1
---
## Report

unit: story:optional-projection-fields
verdict: red
cases: executed 18→22, red 3
origin: introduced 3, pre-existing 1, undecided 0
wrote-outside-worktree: none
needs-coordinator: yes

The null correction works. Legacy plan decoding, deterministic serialization, absent-field selectors, and the full `ServiceEngine::query` path with wire-level field omission all pass.

Findings:

- Plan admission accepts forged optionality on an ESS-required field, allowing row validation to omit it.
- Dangling `optional_fields` metadata is also accepted.
- Generated Rust loses nested optionality: `List<Optional<String>>` becomes `Vec<String>`.
- Compatibility warning: both schemas claim `service-realization-plan/2`, but the previous engine rejects new plans because `ViewPlan` denies unknown fields.

Commands:

- `cargo test -p service-engine --locked` — 14 passed, 2 adversarial failures.
- `cargo test -p service-builder --test build --locked` — 5 passed, 1 adversarial failure.
- Focused null, query-path, deterministic, and legacy compatibility tests — passed.
- `cargo clippy -p service-engine -p service-builder --all-targets --locked -- -D warnings` — passed.
- `cargo fmt --all -- --check` — passed.
- `git diff --check` — passed.

```findings
- file: crates/service-engine/src/lib.rs
  line: 65
  category: integrity
  severity: blocker
  verdict: CONFIRMED
  origin: introduced
  message: ServicePlan admission does not bind optional metadata to ESS-requiredness, so forged optionality weakens required projection-row shape validation
- file: crates/service-engine/src/lib.rs
  line: 65
  category: integrity
  severity: warning
  verdict: CONFIRMED
  origin: introduced
  message: ServicePlan admission accepts optional_fields entries that are absent from the declared public fields
- file: crates/service-builder/src/realization.rs
  line: 871
  category: typing
  severity: warning
  verdict: CONFIRMED
  origin: pre-existing
  message: Generated Rust erases nested Optional values inside lists, producing Vec<String> for List<Optional<String>>
- file: crates/service-engine/src/lib.rs
  line: 20
  category: compatibility
  severity: warning
  verdict: CONFIRMED
  origin: introduced
  message: New and old realization plans share format /2, while the older deny-unknown-fields reader rejects plans carrying optional_fields
```
