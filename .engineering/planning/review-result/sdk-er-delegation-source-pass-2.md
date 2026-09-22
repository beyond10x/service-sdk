---
format: aep.planning-md/1
id: review-result:sdk-er-delegation-source-pass-2
kind: review-result
status: active
title: Complete SDK ER delegation final source examination pass 2
relations:
- reviews: story:er-service-delegation
revision: 1
---
unit: original final whole-source SDK /4 review 2 of 2, candidate 8ede45c15d6d3df79cc9c48b155889351409a79b plus this tests-only working tree
verdict: NEEDS-CHANGE
cases: executed 7→9 in affected v4_execution target, red 2
origin: introduced 2 / pre-existing 0 / undecided 0
wrote-outside-worktree: 16 paths
needs-coordinator: route two introduced SDK intent-obligation blockers to the original implementor; retain this final original review result without opening a third source review

`git --no-pager diff --stat`:

```text
 crates/service-builder/tests/v4_execution.rs | 236 ++++++++++++++++++++++++++-
 1 file changed, 234 insertions(+), 2 deletions(-)
```

The only changed path is a test file. The two removed lines replace an import and extract the existing test-context helper; no existing assertion was removed, skipped, weakened, or rewritten. `git diff --check` and `rustfmt +1.91.0 --edition 2024 --check crates/service-builder/tests/v4_execution.rs` exited 0. Source HEAD stayed at the candidate. The review compiled under Rust 1.91.0, offline and locked, with one Cargo job, lld, debug 0, incremental 0, four empty wrappers, one test thread, and the assigned cache. The final test file SHA-256 is `d0c79e1f0fe4920f48ce196eb6e562e79832358ba334232333e09f113a702ce9`.

## Added cases

1. `crates/service-builder/tests/v4_execution.rs:270`, `owner_obligation_checks_the_loaded_subject_not_an_unrelated_partition_instance`. A builder-admitted `sdk.auth.owner-and-conjunctive-scopes/v1` protects public payment. The selected invoice is owned by A; a valid other invoice in the same partition is owned by B. B's verified authority must be refused without an append. The first exact execution, before an affected-suite run, failed for this assertion (log 01, exit 101). The final case uses two consistent ER storage/logical identities and the public `execute_public_json` entrypoint. Its exact command was `cargo test --locked -p service-builder --test v4_execution owner_obligation_checks_the_loaded_subject_not_an_unrelated_partition_instance -- --exact --test-threads=1`. Final exact result, exit 101, red output verbatim from `logs/10-owner-public-final-compact.log`:

```text
   Compiling service-builder v0.5.11 (~/.local/state/worktree/trees/b10x/service-sdk/ess-evolution-sdk-source-review-2-20260917/crates/service-builder)
    Finished `test` profile [unoptimized] target(s) in 1.30s
     Running tests/v4_execution.rs (~/.local/state/worktree/trees/b10x/service-sdk/ess-evolution-sdk-source-review-1-20260916/target/debug/deps/v4_execution-5e9b0d24f8285983)

running 1 test
test owner_obligation_checks_the_loaded_subject_not_an_unrelated_partition_instance ... FAILED

failures:

---- owner_obligation_checks_the_loaded_subject_not_an_unrelated_partition_instance stdout ----

thread 'owner_obligation_checks_the_loaded_subject_not_an_unrelated_partition_instance' (1870033) panicked at crates/service-builder/tests/v4_execution.rs:371:5:
the owner check must use the selected invoice, not an unrelated invoice in the same partition; observed committed
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    owner_obligation_checks_the_loaded_subject_not_an_unrelated_partition_instance

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 8 filtered out; finished in 0.24s

error: test failed, to rerun pass `-p service-builder --test v4_execution`
```

The retained log 09 prints the complete `Ok(Committed { ... })` value, including `Paid`, revision 3 and the two-member durable batch; log 10 uses the compact equivalent outcome to keep this report legible. The fixture's other invoice was corrected after log 01 to set both its logical `invoice_id` and corresponding `s:` storage address; logs 04, 06, 09 and 10 all stay red. This is a simulated complete snapshot, not a claim of a newly run PostgreSQL fixture.

2. `crates/service-builder/tests/v4_execution.rs:378`, `lifecycle_obligation_accepts_the_selected_logical_identity_in_an_allowed_state`. A builder-admitted `sdk.lifecycle.require-state/v1` allows `Issued`. The test creates and issues an invoice, asserts the exact logical/public ID and ER `s:` address relation, then pays through public `execute_public_json`. The first exact execution after the case was written failed for the stated assertion (log 03, exit 101). Its final exact command was `cargo test --locked -p service-builder --test v4_execution lifecycle_obligation_accepts_the_selected_logical_identity_in_an_allowed_state -- --exact --test-threads=1`. Final exact result, exit 101, red output verbatim from `logs/07-lifecycle-public-exact.log`:

```text
    Finished `test` profile [unoptimized] target(s) in 0.15s
     Running tests/v4_execution.rs (~/.local/state/worktree/trees/b10x/service-sdk/ess-evolution-sdk-source-review-1-20260916/target/debug/deps/v4_execution-5e9b0d24f8285983)

running 1 test
test lifecycle_obligation_accepts_the_selected_logical_identity_in_an_allowed_state ... FAILED

failures:

---- lifecycle_obligation_accepts_the_selected_logical_identity_in_an_allowed_state stdout ----

thread 'lifecycle_obligation_accepts_the_selected_logical_identity_in_an_allowed_state' (1862026) panicked at crates/service-builder/tests/v4_execution.rs:468:5:
the lifecycle obligation must resolve the selected logical identity to its storage address: Err(ObligationRefused("not_found"))
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    lifecycle_obligation_accepts_the_selected_logical_identity_in_an_allowed_state

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 8 filtered out; finished in 0.24s

error: test failed, to rerun pass `-p service-builder --test v4_execution`
```

## Affected-suite run

After both cases existed, the final command `cargo test --locked -p service-builder --test v4_execution -- --test-threads=1` exited 101. Verbatim output from `logs/11-v4-execution-final.log`:

```text
    Finished `test` profile [unoptimized] target(s) in 0.14s
     Running tests/v4_execution.rs (~/.local/state/worktree/trees/b10x/service-sdk/ess-evolution-sdk-source-review-1-20260916/target/debug/deps/v4_execution-5e9b0d24f8285983)

running 9 tests
test admission_precedes_decode_and_committed_projection_failure_keeps_receipt ... ok
test billing_delegates_refusal_fulfillment_atomic_retry_uncertainty_and_replay ... ok
test billing_projection_uses_source_filter_selectors_visibility_sort_and_page ... ok
test committed_content_acceptance_repairs_from_recorded_reference ... ok
test gatepass_preserves_badge_fulfillment_and_expected_view_transition ... ok
test lifecycle_obligation_accepts_the_selected_logical_identity_in_an_allowed_state ... FAILED
test owner_obligation_checks_the_loaded_subject_not_an_unrelated_partition_instance ... FAILED
test same_idempotency_key_with_a_changed_expected_version_is_not_an_exact_retry ... ok
test v4_executes_selected_sdk_intent_obligations ... ok

failures:

---- lifecycle_obligation_accepts_the_selected_logical_identity_in_an_allowed_state stdout ----

thread 'lifecycle_obligation_accepts_the_selected_logical_identity_in_an_allowed_state' (1871019) panicked at crates/service-builder/tests/v4_execution.rs:472:5:
the lifecycle obligation must resolve the selected logical identity to its storage address: Err(ObligationRefused("not_found"))
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

---- owner_obligation_checks_the_loaded_subject_not_an_unrelated_partition_instance stdout ----

thread 'owner_obligation_checks_the_loaded_subject_not_an_unrelated_partition_instance' (1871076) panicked at crates/service-builder/tests/v4_execution.rs:371:5:
the owner check must use the selected invoice, not an unrelated invoice in the same partition; observed committed


failures:
    lifecycle_obligation_accepts_the_selected_logical_identity_in_an_allowed_state
    owner_obligation_checks_the_loaded_subject_not_an_unrelated_partition_instance

test result: FAILED. 7 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.01s

error: test failed, to rerun pass `-p service-builder --test v4_execution`
```

Runs 01–11 are all terminal exit 101 for the intended red cases/affected suites; no reviewer process remains running. Runs 02, 05 and 08 were intermediate affected-suite observations; 03 and 07 are the lifecycle exact case; 04, 06, 09 and 10 are progressively tightened owner exact cases. There was no compiler-error finding. No blanket SDK gate or new actual-PG fixture was run, because the author's exact-candidate full gate, generated child HTTP/provider/restart/repair and drift evidence already exists in `source-correction-1/final-author-report.md` and `source-correction-1/coordinator-final-acceptance.json`. The minimum observed free space at the final check was 73,285,906,432 bytes and available RAM 47,408,873,472 bytes, above the 20 GiB / 16 GiB floors.

## Findings on candidate 8ede45c

| file:line | verdict / origin | what was measured | what reaches it |
|---|---|---|---|
| `crates/service-engine/src/v4.rs:1994` | NEEDS-CHANGE / introduced | The final owner case exits 101 because `pay_invoice` commits `Paid` at revision 3 for owner A when verified authority B matches only a different invoice. `bound_instance_field` picks the first same-entity snapshot item for both owner and scopes, ignoring the loaded command subject. | `build_service_v4` admits the selected catalog obligation; generated `IdentityHttpServiceV4::intent` passes verified context/facts to `RecordedServiceBackendV4::intent`, which calls `EngineV4::execute_public_json`. `EventlogResourcesV4::obligation_instances` supplies every terminal instance in the authenticated partition, and the actual authority evaluator admits B when the unrelated object's owner/scopes match. The two-instance snapshot is therefore reachable; this test substitutes that snapshot without claiming a live adapter run. The base has no /4 route. |
| `crates/service-engine/src/v4.rs:1980` | NEEDS-CHANGE / introduced | The final lifecycle case exits 101: public payment on an `Issued` invoice returns `ObligationRefused("not_found")`. The test proves `EntityInstance.id == "s:" + invoice_id`; `bound_instance` compares it directly with raw public `invoice_id`. | A normal generated UUID billing invoice and a selected, catalog-valid lifecycle obligation reach `EngineV4::execute_public_json` from the same generated HTTP route. The original pass-1 lifecycle test asserted only `is_err` for a disallowed state, so this always-refused allowed state escaped it. The base has no /4 route. |

These are separate failures in the fifth original correction class, selected SDK obligation execution. The first requires subject-bound authority facts; the second requires converting a typed logical identity to the exact ER address before locating the instance. Similar raw-ID comparisons appear in aggregate and graph helpers; this review did not promote those to additional findings without separate executable cases.

## Original findings disposition and examined boundaries

- Original pass-1 wrong-type command binding: the correction validates source/target field types at compile time, including present values for optional Set; the imported regression and valid controls are green in the author's exact final gate. No further new counterexample found in this pass.
- Original pass-1 self-consistent wrong ER target: the strict reader now checks the exact accepted and adapter revision. Generated `service_plan()` uses that reader. No further new counterexample found.
- Original pass-1 /4 client drift: source emits /4 receipt, commit and 202 schemas through both OpenAPI and catalog, while /3 uses its old path. The author's generated child HTTP and drift checks cover billing and gatepass at the fixed candidate. No further new counterexample found.
- Original pass-1 changed expected-version retry: new observations bind `Some(expected_version)`; candidate-era absent fields retain prior comparison meaning. Claim lookup and recovery occur before new fulfillment, and historical prefix replay uses the original decision. The imported regression remains green. No further new counterexample found.
- Original pass-1 selected intent obligations: the reported disallowed-state case is green, but the two new public-input cases above show that selected owner and allowed-state behavior remains incomplete.
- Source tracing additionally covered authenticated service/tenant/optional-realm partition derivation, authorization before mutation decoding and repair, exact batch lookup, aftercare repair from recorded content, ER-only decision/replay, generated package readers, query source filter/selector/page/visibility, and /3 preservation boundaries. No third independently measured break was found. Eventlog provider administration and denied review material remained outside this assignment.

## Paths written outside the review worktree

- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-2/report.md`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-2/logs/`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-2/logs/01-owner-subject-exact.log`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-2/logs/02-v4-execution-affected.log`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-2/logs/03-lifecycle-identity-exact.log`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-2/logs/04-owner-valid-instance-exact.log`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-2/logs/05-v4-execution-final-affected.log`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-2/logs/06-owner-public-exact.log`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-2/logs/07-lifecycle-public-exact.log`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-2/logs/08-v4-execution-public-affected.log`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-2/logs/09-owner-public-final-exact.log`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-2/logs/10-owner-public-final-compact.log`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-2/logs/11-v4-execution-final.log`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-2/scratch/`
- `~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/source-review-2/scratch/tmp/`
- `~/.local/state/worktree/trees/b10x/service-sdk/ess-evolution-sdk-source-review-1-20260916/target/` (assigned compiler cache)

The source and cache trees remain managed for the coordinator; no cleanup, commit, publication, AEP write or source fix was performed. Review assignment CLOSE after writing this report and releasing both own leases.

```findings
- file: crates/service-engine/src/v4.rs
  line: 1994
  category: acceptance
  severity: blocker
  verdict: NEEDS-CHANGE
  origin: introduced
  message: /4 owner-and-scopes checks an unrelated same-entity partition instance, allowing a different authority to commit a payment on the selected invoice
- file: crates/service-engine/src/v4.rs
  line: 1980
  category: acceptance
  severity: blocker
  verdict: NEEDS-CHANGE
  origin: introduced
  message: /4 identity-bound lifecycle checks compare raw public logical IDs with encoded ER storage addresses and refuse valid allowed-state payments as not_found
```
