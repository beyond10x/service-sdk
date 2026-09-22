---
format: aep.planning-md/1
id: review-result:sdk-er-delegation-source-pass-1
kind: review-result
status: active
title: Complete SDK ER delegation source examination pass 1
relations:
- reviews: story:er-service-delegation
revision: 1
---
unit: complete Service SDK Entity Runtime delegation at 663be6f772fdf3ee9c0003b08d4f6d17357c3f73 plus this tests-only working tree
verdict: NEEDS-CHANGE
cases: executed 163→168, red 5
origin: introduced 5 / pre-existing 0 / undecided 0
wrote-outside-worktree: 22 paths
needs-coordinator: route the five introduced blockers to the original implementor

`git --no-pager diff --stat`:

```text
 crates/service-builder/tests/v4.rs           |  64 +++++++++-
 crates/service-builder/tests/v4_execution.rs | 170 ++++++++++++++++++++++++++-
 2 files changed, 232 insertions(+), 2 deletions(-)
```

Every changed path is a test file. `git diff --check` exits zero. The two removed lines are the
existing single-line imports expanded by formatting; no existing assertion was deleted, skipped,
weakened, or rewritten. The first compile attempt exposed a reviewer-test API typo
(`ArtifactTree::get`); I changed that test-only lookup to collect `ArtifactTree::iter`, then reran
the exact case. The failed compile is retained as log 01 and is not candidate evidence.

The author's full gate log 111 executed 163 Rust test and doctest cases. All five additions were
selected individually, producing the 163→168 count; a post-addition full suite was outside the
bounded allocation.

## Added cases

All commands used this environment:

```text
RUSTUP_TOOLCHAIN=1.91.0
CARGO_BUILD_JOBS=1
CARGO_NET_OFFLINE=true
CARGO_TARGET_DIR=~/.local/state/worktree/trees/b10x/service-sdk/ess-evolution-sdk-source-review-1-20260916/target
TMPDIR=~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-1/scratch/tmp
RUSTC_WRAPPER=
RUSTC_WORKSPACE_WRAPPER=
CARGO_BUILD_RUSTC_WRAPPER=
CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER=
CARGO_INCREMENTAL=0
CARGO_PROFILE_DEV_DEBUG=0
CARGO_PROFILE_TEST_DEBUG=0
RUSTFLAGS=-C link-arg=-fuse-ld=lld
```

1. `crates/service-builder/tests/v4.rs:150`
   `operation_field_command_sources_must_match_the_lowerer_field_type` asserts that the compiler
   rejects a UUID command field selected to fulfill the lowerer's Timestamp `issued_at` field.
   Command:

   ```text
   cargo test --locked -p service-builder --test v4 operation_field_command_sources_must_match_the_lowerer_field_type -- --exact --test-threads=1
   ```

   First executable result, exit 101, red output verbatim:

   ```text
      Compiling service-builder v0.5.11 (~/.local/state/worktree/trees/b10x/service-sdk/ess-evolution-sdk-source-review-1-20260916/crates/service-builder)
       Finished `test` profile [unoptimized] target(s) in 1.23s
        Running tests/v4.rs (target/debug/deps/v4-f68292df097e397b)

   running 1 test
   test operation_field_command_sources_must_match_the_lowerer_field_type ... FAILED

   failures:

   ---- operation_field_command_sources_must_match_the_lowerer_field_type stdout ----

   thread 'operation_field_command_sources_must_match_the_lowerer_field_type' (47458) panicked at crates/service-builder/tests/v4.rs:165:5:
   a UUID command field cannot fulfill the lowerer's Timestamp operation field
   note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


   failures:
       operation_field_command_sources_must_match_the_lowerer_field_type

   test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 6 filtered out; finished in 0.19s

   error: test failed, to rerun pass `-p service-builder --test v4`
   ```

2. `crates/service-builder/tests/v4.rs:172`
   `realization_reader_refuses_a_self_consistent_incompatible_er_target` changes the persisted ER
   target, recomputes the plan digest, and asserts that the strict reader rejects it. Command:

   ```text
   cargo test --locked -p service-builder --test v4 realization_reader_refuses_a_self_consistent_incompatible_er_target -- --exact --test-threads=1
   ```

   Exit 101, red output verbatim:

   ```text
       Finished `test` profile [unoptimized] target(s) in 0.14s
        Running tests/v4.rs (target/debug/deps/v4-f68292df097e397b)

   running 1 test
   test realization_reader_refuses_a_self_consistent_incompatible_er_target ... FAILED

   failures:

   ---- realization_reader_refuses_a_self_consistent_incompatible_er_target stdout ----

   thread 'realization_reader_refuses_a_self_consistent_incompatible_er_target' (48235) panicked at crates/service-builder/tests/v4.rs:180:5:
   the executable reader cannot admit a plan for another ER semantic target
   note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


   failures:
       realization_reader_refuses_a_self_consistent_incompatible_er_target

   test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 6 filtered out; finished in 0.19s

   error: test failed, to rerun pass `-p service-builder --test v4`
   ```

3. `crates/service-builder/tests/v4.rs:205`
   `generated_v4_openapi_describes_the_v4_mutation_contract` asserts that generated OpenAPI
   describes the actual `/4` receipt fields and committed-aftercare response. Command:

   ```text
   cargo test --locked -p service-builder --test v4 generated_v4_openapi_describes_the_v4_mutation_contract -- --exact --test-threads=1
   ```

   Exit 101, red output verbatim:

   ```text
       Finished `test` profile [unoptimized] target(s) in 0.14s
        Running tests/v4.rs (target/debug/deps/v4-f68292df097e397b)

   running 1 test
   test generated_v4_openapi_describes_the_v4_mutation_contract ... FAILED

   failures:

   ---- generated_v4_openapi_describes_the_v4_mutation_contract stdout ----

   thread 'generated_v4_openapi_describes_the_v4_mutation_contract' (48845) panicked at crates/service-builder/tests/v4.rs:218:5:
   assertion failed: properties.contains_key("response")
   note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


   failures:
       generated_v4_openapi_describes_the_v4_mutation_contract

   test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 6 filtered out; finished in 0.20s

   error: test failed, to rerun pass `-p service-builder --test v4`
   ```

4. `crates/service-builder/tests/v4_execution.rs:656`
   `same_idempotency_key_with_a_changed_expected_version_is_not_an_exact_retry` asserts that an
   already-used key with a changed public expected version conflicts instead of replaying. Command:

   ```text
   cargo test --locked -p service-builder --test v4_execution same_idempotency_key_with_a_changed_expected_version_is_not_an_exact_retry -- --exact --test-threads=1
   ```

   Exit 101, red output verbatim:

   ```text
      Compiling service-builder v0.5.11 (~/.local/state/worktree/trees/b10x/service-sdk/ess-evolution-sdk-source-review-1-20260916/crates/service-builder)
       Finished `test` profile [unoptimized] target(s) in 0.94s
        Running tests/v4_execution.rs (target/debug/deps/v4_execution-5e9b0d24f8285983)

   running 1 test
   test same_idempotency_key_with_a_changed_expected_version_is_not_an_exact_retry ... FAILED

   failures:

   ---- same_idempotency_key_with_a_changed_expected_version_is_not_an_exact_retry stdout ----

   thread 'same_idempotency_key_with_a_changed_expected_version_is_not_an_exact_retry' (49780) panicked at crates/service-builder/tests/v4_execution.rs:703:5:
   assertion failed: matches!(engine.execute_public_json(&context, "issue_invoice",
       &changed_precondition, recording("issue-changed-expected-version"), &mut
       resources,), Err(ExecutionErrorV4::IdempotencyConflict))
   note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


   failures:
       same_idempotency_key_with_a_changed_expected_version_is_not_an_exact_retry

   test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 6 filtered out; finished in 0.27s

   error: test failed, to rerun pass `-p service-builder --test v4_execution`
   ```

5. `crates/service-builder/tests/v4_execution.rs:716`
   `v4_executes_selected_sdk_intent_obligations` adds a catalog-valid
   `sdk.lifecycle.require-state/v1` obligation that permits payment only in Draft, compiles it into
   the `/4` intent plan, issues the invoice, and asserts that payment is refused. Command:

   ```text
   cargo test --locked -p service-builder --test v4_execution v4_executes_selected_sdk_intent_obligations -- --exact --test-threads=1
   ```

   Exit 101, red output verbatim:

   ```text
       Finished `test` profile [unoptimized] target(s) in 0.14s
        Running tests/v4_execution.rs (target/debug/deps/v4_execution-5e9b0d24f8285983)

   running 1 test
   test v4_executes_selected_sdk_intent_obligations ... FAILED

   failures:

   ---- v4_executes_selected_sdk_intent_obligations stdout ----

   thread 'v4_executes_selected_sdk_intent_obligations' (51420) panicked at crates/service-builder/tests/v4_execution.rs:800:5:
   sdk.lifecycle.require-state/v1 must refuse paying the issued invoice because only Draft was allowed
   note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


   failures:
       v4_executes_selected_sdk_intent_obligations

   test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 6 filtered out; finished in 0.24s

   error: test failed, to rerun pass `-p service-builder --test v4_execution`
   ```

## Suite run

No suite or gate was run. The coordinator authorized only the five exact cases above alongside the
M9 heavy lane. The cold target reached 991 MiB; minimum observed free disk was 25,487,716,352 bytes
and minimum observed available RAM was 45,823,119,360 bytes, both above the required floors.

## Judgement findings

All findings cover candidate `663be6f772fdf3ee9c0003b08d4f6d17357c3f73`. The base has no `/4`
implementation, so all five are introduced.

| file:line | verdict / origin | what was measured | what reaches it |
|---|---|---|---|
| `crates/service-runtime-ir/src/v4.rs:1015` | NEEDS-CHANGE / introduced | The exact compiler case exits 101 because `resolve_policies` accepts `IssueInvoice.invoice_id` (UUID) for the lowerer's `issued_at` (Timestamp); this loop validates only whether `Remove` targets an optional field. | `build_service_v4` calls `compile_v4` for every authored `/4` package, and `OperationFieldPolicy::CommandField` is a public accepted policy. ER may later reject the bad value, but acceptance requires wrong-type bindings to refuse during compilation. |
| `crates/service-engine/src/v4.rs:497` | NEEDS-CHANGE / introduced | The exact reload case exits 101 because a plan with an all-zero ER target and a correctly recomputed semantic digest passes `ServicePlanV4::from_json`. | Every generated crate's public `service_plan()` reads its packaged persisted plan with this reader and immediately passes it to `IdentityHttpServiceV4`; no source-bound recompile occurs at that boundary. |
| `crates/service-builder/src/lib.rs:243` | NEEDS-CHANGE / introduced | The exact generation case exits 101 on the missing `response` field. Source comparison also shows the actual `/4` receipt contains `response` and `commit`, and aftercare returns 202, while the emitted schema omits both fields and declares no 202 response. The service catalog is built through the same legacy host plan and publishes the same legacy receipt. | `build_service_v4` unconditionally emits `http::openapi(&client_plan)` for Identity HTTP definitions; both retained billing and gatepass generated packages contain these bytes for public clients and consoles. |
| `crates/service-engine/src/v4.rs:1134` | NEEDS-CHANGE / introduced | The exact public-envelope case exits 101 because changing expected version from 1 to 2 with the same key and command input replays instead of returning `IdempotencyConflict`. `execute_public_json` removes expected version before constructing `OriginalIntentV4`, whose equality is the recovery conflict check. | Every non-create billing and gatepass HTTP intent uses `ExpectedVersionPlan::OperationField` and `execute_public_json`, so an ordinary retry with a changed concurrency precondition reaches this path. |
| `crates/service-engine/src/v4.rs:1108` | NEEDS-CHANGE / introduced | The exact execution case exits 101 after a valid selected `sdk.lifecycle.require-state/v1` obligation is retained in `IntentPlanV4` but payment commits. `execute_admitted` never reads `intent.obligations`. | The obligation catalog accepts lifecycle, authorization, graph, nested-entity, and owned-revision intent providers for normal `/4` definitions. Existing `/3` executes them in `run_intent_obligations`; `/4` exposes no equivalent resource seam. The generated package only accepts an arbitrary backend, and the retained external harness manually implements SDK-named slot, field, visibility, and effect behavior, so it does not close this reusable-provider gap. |

## Attacked without an additional break

- Read all 49 changed files and traced the public builder, generated package, HTTP, engine,
  recorded-adapter, catalog, and retained harness callers; Eventlog administration stayed excluded.
- Strict `/4` format discrimination, canonical runtime-IR source/synthesis binding, and `/3` reader
  refusal remained intact in source.
- Authenticated partition derivation distinguishes absent realm from present realm, binds before
  persistence access, and authorization precedes mutation body decoding and repair work.
- ER remains the mutation decision authority for pre-load refusal, exact-subject continuation,
  selected outcomes, event order, and Set/Preserve/Remove execution.
- Atomic decision-plus-observation append, uncertain-write recovery, historical prefix replay,
  aftercare repair from recorded evidence, and content-reference repair retained their intended
  source ordering; no further reachable violation was found.
- Query selector/filter/sort/page and row visibility paths retained their source ordering.
- Direct ESS, ER, and Eventlog pins use one exact Git identity each; the arbitrary-precision YAML
  normalization and owned persistence fixture updates are coherent.

## Paths written outside the worktree

- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-1/report.md`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-1/logs/`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-1/scratch/`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-1/scratch/tmp/`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-1/logs/01-operation-field-type.log`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-1/logs/01-operation-field-type.exit`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-1/logs/01-operation-field-type.capacity`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-1/logs/02-operation-field-type-red.log`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-1/logs/02-operation-field-type-red.exit`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-1/logs/02-operation-field-type-red.capacity`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-1/logs/03-plan-target-red.log`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-1/logs/03-plan-target-red.exit`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-1/logs/03-plan-target-red.capacity`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-1/logs/04-openapi-contract-red.log`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-1/logs/04-openapi-contract-red.exit`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-1/logs/04-openapi-contract-red.capacity`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-1/logs/05-idempotency-version-red.log`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-1/logs/05-idempotency-version-red.exit`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-1/logs/05-idempotency-version-red.capacity`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-1/logs/06-intent-obligation-red.log`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-1/logs/06-intent-obligation-red.exit`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-1/logs/06-intent-obligation-red.capacity`

```findings
- file: crates/service-runtime-ir/src/v4.rs
  line: 1015
  category: acceptance
  severity: blocker
  verdict: NEEDS-CHANGE
  origin: introduced
  message: /4 compilation accepts a UUID command field as fulfillment for the lowerer's Timestamp operation field instead of refusing the wrong-type binding
- file: crates/service-engine/src/v4.rs
  line: 497
  category: contract-drift
  severity: blocker
  verdict: NEEDS-CHANGE
  origin: introduced
  message: the strict realization-plan reader accepts a self-consistent plan naming an incompatible Entity Runtime target revision
- file: crates/service-builder/src/lib.rs
  line: 243
  category: contract-drift
  severity: blocker
  verdict: NEEDS-CHANGE
  origin: introduced
  message: generated /4 OpenAPI and catalog artifacts publish the legacy mutation receipt and omit the committed-aftercare response implemented by the HTTP boundary
- file: crates/service-engine/src/v4.rs
  line: 1134
  category: concurrency
  severity: blocker
  verdict: NEEDS-CHANGE
  origin: introduced
  message: expected version is absent from OriginalIntentV4 so a same-key request with a changed concurrency precondition is replayed as an exact retry
- file: crates/service-engine/src/v4.rs
  line: 1108
  category: acceptance
  severity: blocker
  verdict: NEEDS-CHANGE
  origin: introduced
  message: /4 execution retains selected SDK intent obligations but never runs their authorization, lifecycle, graph, or aggregate provider behavior
```
