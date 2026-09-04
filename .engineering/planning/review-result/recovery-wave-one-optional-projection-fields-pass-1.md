---
format: aep.planning-md/1
id: review-result:recovery-wave-one-optional-projection-fields-pass-1
kind: review-result
status: active
title: 'Adversarial review: optional projection fields pass 1'
summary: Optional omission holds, but explicit null remains incorrectly admissible.
owner: wave-adversary
relations:
- reviews: story:optional-projection-fields
revision: 1
---
## Report

unit: story:optional-projection-fields
verdict: red
cases: executed 0→1, red 1
origin: introduced 0, pre-existing 1, undecided 0
wrote-outside-worktree: none
needs-coordinator: yes

Projection omission, required-field enforcement, unknown-field rejection, absent-selector behavior, legacy-plan deserialization, canonical qualified types, and outer-vs-nested `Optional` detection passed.

Blocker: projection validation accepts explicit top-level `null`, although ESS defines optional object fields through absence. The adversarial regression fails at `crates/service-engine/src/lib.rs:2805`. This behavior existed at the base commit, but violates the supplied contract and needs a production correction.

Commands:

- `cargo test -p service-builder --test build --locked` — 6 passed.
- `cargo test -p service-engine --locked` — 11 passed, 1 adversarial failure.
- `cargo clippy -p service-engine -p service-builder --all-targets --locked -- -D warnings` — passed.
- `cargo fmt --all -- --check` — passed.
- `git diff --check` — passed.

```findings
- file: crates/service-engine/src/lib.rs
  line: 1889
  category: contract
  severity: blocker
  verdict: CONFIRMED
  origin: pre-existing
  message: projection-row validation accepts Value::Null for an Optional field even though ESS object-field optionality requires omission and explicit null must be rejected
```
