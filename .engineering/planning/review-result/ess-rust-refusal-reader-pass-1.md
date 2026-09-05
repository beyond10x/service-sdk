---
format: aep.planning-md/1
id: review-result:ess-rust-refusal-reader-pass-1
kind: review-result
status: active
title: SDK ESS target refusal admission adversary pass 1
owner: impl_containment
relations:
- reviews: story:reject-ess-rust-target-refusals
revision: 1
---
unit: story:reject-ess-rust-target-refusals at c6bd6e7e88f76196a228a76e9ad5fdbb3f937d7d
verdict: nothing found
cases: executed 24→30, red 0
origin: introduced 0 / pre-existing 0 / undecided 0
wrote-outside-worktree: none
needs-coordinator: frozen corrected ESS candidate execution, full SDK gate and lifecycle remain coordinator-owned

`git --no-pager diff --stat`

```text
 .../tests/support/rust_target_adversary.rs         | 195 +++++++++++++++++++++
 1 file changed, 195 insertions(+)
```

## 1. Subject and test-only boundary

The only new tracked diff is crates/service-builder/tests/support/rust_target_adversary.rs,
195 inserted lines. The subject's production guard, existing cases, shared fixture, test-only
module hook and coordinator planning changes were not edited. Base was
15c2bf4306d77f85397b3b1df5879e829497513e; exact subject was
c6bd6e7e88f76196a228a76e9ad5fdbb3f937d7d. Read their diff, last commit, complete story revision 7,
AGENTS, assigned brief, charter, implementation report and callers before testing.

The authored test patch is adversary-1-tests.patch. git diff --check exited 0 with no output;
adversary-1-status.log lists only the assigned test file. No implementation, dependency, planning,
Git index or lifecycle mutation was made. rustfmt --edition 2024 was applied only to that test file
before its first execution; the subsequent package formatting check passed.

Acceptance attacked: checked failures must remain errors with actionable original details; the
report policy accepts historical None or empty Rust notes, refuses any mismatched target/refusal/
weakening before downstream work, and preserves successful synthesis. The later frozen producer
lane is explicitly separate and was not substituted with synthetic tests.

## 2. Isolated cases, authored before any suite

Baseline 24 is carried from the implementor's reported final package runner: 18 library cases,
6 integration cases, no binary/doc cases. No baseline suite was rerun before the new cases existed.
All six new cases were written and formatted together, then each ran alone with --exact and a
runner-confirmed selection of one. Every initial run passed. There is no behavioral red, compiler
red, zero-selection run, mutation run or setup retry to report for this pass.

All case locations below are in crates/service-builder/tests/support/rust_target_adversary.rs.

| Case and line | Assertion | Final state |
|---|---|---|
| checked_failure_retains_nested_typed_source_chain:76 | Original concrete outer error, typed root cause, both chain messages and actionable source location survive Result conversion. | Green |
| successful_result_cannot_erase_rejected_report_before_admission:100 | Ok wrapping preserves a wrong-target, refused or weakened report so the following admission still refuses it. | Green |
| repeated_report_notes_and_source_provenance_are_not_deduplicated:125 | Repeated refusal/weakening occurrences, multiline Unicode details, source/contract provenance and immutable report contents survive diagnostics. | Green |
| an_empty_weakening_record_is_not_an_empty_report:149 | A weakening with empty text and no affected kinds still triggers the bound any-weakening policy. | Green |
| both_success_forms_preserve_empty_and_non_ascii_artifact_bytes:163 | Direct and Ok forms preserve complete plan/report/artifact values, including empty, NUL, CRLF and Unicode text, with None and empty Rust reports. | Green |
| zero_capability_model_keeps_the_historical_build_ess_success_path:182 | An actual current-pin compiler/synthesizer/projector control with zero capabilities remains successful and byte/value equal through build_ess. | Green |

Cases 1–5 exercise private helpers with typed synthetic values. They do not establish that the
current Rust producer emits a target failure. Case 6 executes the real old-pin success facade,
not a new-producer rejection. These distinctions bound the result.

Each preserved .command includes the exact argv and environment. Every Cargo command used this
tree's target as TMPDIR, /usr/bin/sccache with the assigned coordinator w4 socket,
CARGO_INCREMENTAL=0, CARGO_PROFILE_DEV_DEBUG=0, CARGO_PROFILE_TEST_DEBUG=0,
CARGO_CACHE_RUSTC_INFO=0 and CARGO_NET_OFFLINE=true. No CARGO_TARGET_DIR override was set.

### Isolated 1

```bash
export TMPDIR="$PWD/target"
export RUSTC_WRAPPER=/usr/bin/sccache
export SCCACHE_SERVER_UDS=/home/timo/.local/state/worktree/trees/b10x/ess/wt-752828a285ba/target/w4-cache.sock
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_CACHE_RUSTC_INFO=0
export CARGO_NET_OFFLINE=true
cargo test --locked --offline -p service-builder --lib rust_target_adversary::checked_failure_retains_nested_typed_source_chain -- --exact > target/review-boundaries-5/adversary-1-isolated-1.log 2>&1
adversary_status=$?
printf '%s\n' "$adversary_status" > target/review-boundaries-5/adversary-1-isolated-1.exit
cat target/review-boundaries-5/adversary-1-isolated-1.log
exit "$adversary_status"
```

```text
   Compiling service-builder v0.5.11 (/home/timo/.local/state/worktree/trees/b10x/service-sdk/ess-rust-refusal-reader/crates/service-builder)
    Finished `test` profile [unoptimized] target(s) in 1.06s
     Running unittests src/lib.rs (target/debug/deps/service_builder-0616f57529aa5443)

running 1 test
test rust_target_adversary::checked_failure_retains_nested_typed_source_chain ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 23 filtered out; finished in 0.00s

```

Exit: 0.

### Isolated 2

```bash
export TMPDIR="$PWD/target"
export RUSTC_WRAPPER=/usr/bin/sccache
export SCCACHE_SERVER_UDS=/home/timo/.local/state/worktree/trees/b10x/ess/wt-752828a285ba/target/w4-cache.sock
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_CACHE_RUSTC_INFO=0
export CARGO_NET_OFFLINE=true
cargo test --locked --offline -p service-builder --lib rust_target_adversary::successful_result_cannot_erase_rejected_report_before_admission -- --exact > target/review-boundaries-5/adversary-1-isolated-2.log 2>&1
adversary_status=$?
printf '%s\n' "$adversary_status" > target/review-boundaries-5/adversary-1-isolated-2.exit
cat target/review-boundaries-5/adversary-1-isolated-2.log
exit "$adversary_status"
```

```text
    Finished `test` profile [unoptimized] target(s) in 0.11s
     Running unittests src/lib.rs (target/debug/deps/service_builder-0616f57529aa5443)

running 1 test
test rust_target_adversary::successful_result_cannot_erase_rejected_report_before_admission ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 23 filtered out; finished in 0.00s

```

Exit: 0.

### Isolated 3

```bash
export TMPDIR="$PWD/target"
export RUSTC_WRAPPER=/usr/bin/sccache
export SCCACHE_SERVER_UDS=/home/timo/.local/state/worktree/trees/b10x/ess/wt-752828a285ba/target/w4-cache.sock
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_CACHE_RUSTC_INFO=0
export CARGO_NET_OFFLINE=true
cargo test --locked --offline -p service-builder --lib rust_target_adversary::repeated_report_notes_and_source_provenance_are_not_deduplicated -- --exact > target/review-boundaries-5/adversary-1-isolated-3.log 2>&1
adversary_status=$?
printf '%s\n' "$adversary_status" > target/review-boundaries-5/adversary-1-isolated-3.exit
cat target/review-boundaries-5/adversary-1-isolated-3.log
exit "$adversary_status"
```

```text
    Finished `test` profile [unoptimized] target(s) in 0.11s
     Running unittests src/lib.rs (target/debug/deps/service_builder-0616f57529aa5443)

running 1 test
test rust_target_adversary::repeated_report_notes_and_source_provenance_are_not_deduplicated ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 23 filtered out; finished in 0.00s

```

Exit: 0.

### Isolated 4

```bash
export TMPDIR="$PWD/target"
export RUSTC_WRAPPER=/usr/bin/sccache
export SCCACHE_SERVER_UDS=/home/timo/.local/state/worktree/trees/b10x/ess/wt-752828a285ba/target/w4-cache.sock
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_CACHE_RUSTC_INFO=0
export CARGO_NET_OFFLINE=true
cargo test --locked --offline -p service-builder --lib rust_target_adversary::an_empty_weakening_record_is_not_an_empty_report -- --exact > target/review-boundaries-5/adversary-1-isolated-4.log 2>&1
adversary_status=$?
printf '%s\n' "$adversary_status" > target/review-boundaries-5/adversary-1-isolated-4.exit
cat target/review-boundaries-5/adversary-1-isolated-4.log
exit "$adversary_status"
```

```text
    Finished `test` profile [unoptimized] target(s) in 0.11s
     Running unittests src/lib.rs (target/debug/deps/service_builder-0616f57529aa5443)

running 1 test
test rust_target_adversary::an_empty_weakening_record_is_not_an_empty_report ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 23 filtered out; finished in 0.00s

```

Exit: 0.

### Isolated 5

```bash
export TMPDIR="$PWD/target"
export RUSTC_WRAPPER=/usr/bin/sccache
export SCCACHE_SERVER_UDS=/home/timo/.local/state/worktree/trees/b10x/ess/wt-752828a285ba/target/w4-cache.sock
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_CACHE_RUSTC_INFO=0
export CARGO_NET_OFFLINE=true
cargo test --locked --offline -p service-builder --lib rust_target_adversary::both_success_forms_preserve_empty_and_non_ascii_artifact_bytes -- --exact > target/review-boundaries-5/adversary-1-isolated-5.log 2>&1
adversary_status=$?
printf '%s\n' "$adversary_status" > target/review-boundaries-5/adversary-1-isolated-5.exit
cat target/review-boundaries-5/adversary-1-isolated-5.log
exit "$adversary_status"
```

```text
    Finished `test` profile [unoptimized] target(s) in 0.11s
     Running unittests src/lib.rs (target/debug/deps/service_builder-0616f57529aa5443)

running 1 test
test rust_target_adversary::both_success_forms_preserve_empty_and_non_ascii_artifact_bytes ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 23 filtered out; finished in 0.00s

```

Exit: 0.

### Isolated 6

```bash
export TMPDIR="$PWD/target"
export RUSTC_WRAPPER=/usr/bin/sccache
export SCCACHE_SERVER_UDS=/home/timo/.local/state/worktree/trees/b10x/ess/wt-752828a285ba/target/w4-cache.sock
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_CACHE_RUSTC_INFO=0
export CARGO_NET_OFFLINE=true
cargo test --locked --offline -p service-builder --lib rust_target_adversary::zero_capability_model_keeps_the_historical_build_ess_success_path -- --exact > target/review-boundaries-5/adversary-1-isolated-6.log 2>&1
adversary_status=$?
printf '%s\n' "$adversary_status" > target/review-boundaries-5/adversary-1-isolated-6.exit
cat target/review-boundaries-5/adversary-1-isolated-6.log
exit "$adversary_status"
```

```text
    Finished `test` profile [unoptimized] target(s) in 0.12s
     Running unittests src/lib.rs (target/debug/deps/service_builder-0616f57529aa5443)

running 1 test
test rust_target_adversary::zero_capability_model_keeps_the_historical_build_ess_success_path ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 23 filtered out; finished in 0.01s

```

Exit: 0.

## 3. Complete package suite and checks

Only after all six isolated executions did the package suite run. Actual runner totals:
**executed 24 → 30, exit 0** (library 18 → 24; integration 6 → 6; binary/doc 0 → 0),
with 0 failed, 0 ignored and no filtering in the package lane. Formatting and strict Clippy also
exited 0. No full SDK workspace gate, evolving ESS source, candidate dependency override or generated
product compilation was run. The implementation's prior 63-file control is carried context, not an
execution newly attributed to this reviewer.

### package

```bash
export TMPDIR="$PWD/target"
export RUSTC_WRAPPER=/usr/bin/sccache
export SCCACHE_SERVER_UDS=/home/timo/.local/state/worktree/trees/b10x/ess/wt-752828a285ba/target/w4-cache.sock
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_CACHE_RUSTC_INFO=0
export CARGO_NET_OFFLINE=true
cargo test --locked --offline -p service-builder > target/review-boundaries-5/adversary-1-package.log 2>&1
adversary_status=$?
printf '%s\n' "$adversary_status" > target/review-boundaries-5/adversary-1-package.exit
cat target/review-boundaries-5/adversary-1-package.log
exit "$adversary_status"
```

```text
   Compiling service-builder v0.5.11 (/home/timo/.local/state/worktree/trees/b10x/service-sdk/ess-rust-refusal-reader/crates/service-builder)
    Finished `test` profile [unoptimized] target(s) in 1.65s
     Running unittests src/lib.rs (target/debug/deps/service_builder-0616f57529aa5443)

running 24 tests
test client::tests::optionality_comes_from_the_resolved_type ... ok
test client::tests::the_complete_authentication_coordinate_set_is_reserved ... ok
test ess::tests::source_paths_cannot_escape_the_definition_root ... ok
test client::tests::superseded_client_plan_format_is_refused ... ok
test ess::tests::a_source_set_requires_the_ess_header ... ok
test rust_target_admission::every_refusal_keeps_its_capability_source_and_original_reason ... ok
test rust_target_admission::empty_rust_report_is_admitted ... ok
test rust_target_admission::historical_absent_report_is_admitted ... ok
test rust_target_admission::weakening_without_refusals_keeps_guarantee_replacement_and_affected_capabilities ... ok
test rust_target_admission::mismatched_target_is_rejected_even_without_notes ... ok
test rust_target_adversary::repeated_report_notes_and_source_provenance_are_not_deduplicated ... ok
test synthesis_outcome_admission::fallible_failure_preserves_original_typed_cause_without_a_capability ... ok
test rust_target_admission::mismatched_target_preserves_both_refusals_and_weakenings ... ok
test rust_target_adversary::checked_failure_retains_nested_typed_source_chain ... ok
test rust_target_adversary::an_empty_weakening_record_is_not_an_empty_report ... ok
test tree::tests::paths_cannot_escape_or_claim_the_manifest ... ok
test tree::tests::stale_owned_files_are_removed_but_unowned_files_are_refused ... ok
test tree::tests::writing_then_checking_is_byte_identical ... ok
test synthesis_outcome_admission::direct_synthesis_keeps_plan_artifacts_and_target ... ok
test rust_target_adversary::successful_result_cannot_erase_rejected_report_before_admission ... ok
test synthesis_outcome_admission::fallible_success_keeps_plan_artifacts_and_target ... ok
test rust_target_adversary::both_success_forms_preserve_empty_and_non_ascii_artifact_bytes ... ok
test rust_target_adversary::zero_capability_model_keeps_the_historical_build_ess_success_path ... ok
test synthesis_outcome_admission::historical_rust_success_preserves_every_ess_artifact ... ok

test result: ok. 24 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s

     Running unittests src/main.rs (target/debug/deps/service_builder-cb36718f7d4302de)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/build.rs (target/debug/deps/build-b7f1cfec0890e3df)

running 6 tests
test realization_plan_carries_optional_projection_fields_from_ess ... ok
test realization_plan_only_marks_outer_optional_projection_fields_as_absentable ... ok
test one_runtime_ir_derives_identical_client_and_connector_surfaces ... ok
test composed_connector_package_does_not_emit_a_standalone_http_host ... ok
test unified_package_emits_compilable_service_and_connector_factory_sources ... ok
test cli_generate_then_check_detects_byte_drift ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.38s

   Doc-tests service_builder

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

```

Exit: 0.

### format

```bash
export TMPDIR="$PWD/target"
export RUSTC_WRAPPER=/usr/bin/sccache
export SCCACHE_SERVER_UDS=/home/timo/.local/state/worktree/trees/b10x/ess/wt-752828a285ba/target/w4-cache.sock
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_CACHE_RUSTC_INFO=0
export CARGO_NET_OFFLINE=true
cargo fmt -p service-builder -- --check > target/review-boundaries-5/adversary-1-format.log 2>&1
adversary_status=$?
printf '%s\n' "$adversary_status" > target/review-boundaries-5/adversary-1-format.exit
cat target/review-boundaries-5/adversary-1-format.log
exit "$adversary_status"
```

```text

```

Exit: 0.

### clippy

```bash
export TMPDIR="$PWD/target"
export RUSTC_WRAPPER=/usr/bin/sccache
export SCCACHE_SERVER_UDS=/home/timo/.local/state/worktree/trees/b10x/ess/wt-752828a285ba/target/w4-cache.sock
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_CACHE_RUSTC_INFO=0
export CARGO_NET_OFFLINE=true
cargo clippy --locked --offline -p service-builder --all-targets -- -D warnings > target/review-boundaries-5/adversary-1-clippy.log 2>&1
adversary_status=$?
printf '%s\n' "$adversary_status" > target/review-boundaries-5/adversary-1-clippy.exit
cat target/review-boundaries-5/adversary-1-clippy.log
exit "$adversary_status"
```

```text
    Checking service-builder v0.5.11 (/home/timo/.local/state/worktree/trees/b10x/service-sdk/ess-rust-refusal-reader/crates/service-builder)
    Finished `dev` profile [unoptimized] target(s) in 0.88s
```

Exit: 0.

## 4. Findings and reachability

Nothing found in this bounded pass. There are no judgement findings or unmeasured defects promoted
into findings. This report does not approve the unit or satisfy a human-review requirement.

The actual caller order is source-grounded at crates/service-builder/src/lib.rs:87–96:
compile → synthesize/IntoSynthesisResult → admit_rust_target → generate_all → EssBuild.
build_service at :136–176 propagates build_ess before runtime/client/realization/catalog work;
build_package at :185–213 propagates build_service before generated package/release artifacts.
The CLI's src/main.rs:45–73 calls compile before either ArtifactTree::write or check. These are
source-order observations, not an actual future-producer refusal execution. Current ESS pin
is d1a66772a91b5411d942d7a45bbf08dfc5de4651, whose Rust synthesize returns direct Synthesis.
The coordinator must still prove actual frozen Result<TargetFailure> compatibility and CLI
refusal/no-output-mutation before adopting that producer pin, as the story requires.

## 5. Attacks that did not break the unit

- Nested error chain and typed-cause loss: original outer and root types/messages survived.
- Successful Result laundering a rejected report: all three rejected-report classes remained visible.
- Diagnostic deduplication, text loss and report mutation: repeated occurrences and original contents survived.
- Empty weakening metadata: a nonempty notes vector was still refused.
- Successful-value normalization or content loss: direct/checked values remained exact.
- Zero-capability success regression: the real current-pin build_ess control remained successful.

## 6. Writes and handoff limits

Outside-worktree writes: none. The only authored tracked path is the assigned tests/support file;
all reports, raw command/log/exit files and patch remain under this tree's
 target/review-boundaries-5/. The prescribed shared compiler-cache server was used without lifecycle
or configuration changes. The pre-run disk observation was 77 GiB available, above the 8 GiB reserve.
No directory/cache cleanup, AEP command, source mutation, publication, release, pin or worktree action
was performed. Source/test writes are relinquished with this immutable report for coordinator routing.

The coordinator owns preservation of this report, final producer proof, remaining required SDK
gates, change publication and worktree cleanup. No claim of complete future producer compatibility,
SDK/AgentIDE upgrade or external deployment follows from these tests.

```findings
[]
```

