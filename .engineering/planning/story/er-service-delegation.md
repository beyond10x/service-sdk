---
format: aep.planning-md/1
id: story:er-service-delegation
kind: story
status: active
title: Delegate SDK service semantics to ER and verify generated billing and gatepass services
owner: service-sdk
refs:
- provider: local
  reference: ess-evolution-20260915-M6
relations:
- decomposes: epic:builder-runtime
- serves: vision:composable-services
scope:
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
  path: crates/service-builder/
- confidence: cited
  path: crates/service-conformance/
- confidence: cited
  path: crates/service-connectors/
- confidence: cited
  path: crates/service-definition/
- confidence: cited
  path: crates/service-engine/
- confidence: cited
  path: crates/service-eventlog/
- confidence: cited
  path: crates/service-host/
- confidence: inferred
  path: crates/service-host/tests/fixtures/er-billing/
- confidence: inferred
  path: crates/service-host/tests/fixtures/er-gatepass/
- confidence: cited
  path: crates/service-http/
- confidence: cited
  path: crates/service-obligations/
- confidence: cited
  path: crates/service-runtime-ir/
- confidence: cited
  path: crates/service-runtime/
- confidence: inferred
  path: docs/design/ess-evolution-er-delegation.md
revision: 21
---
# Complete ER delegation and generated service acceptance

## Approved outcome and existing evidence gap

This is the full original M6 / section6 outcome in approved ESS evolution revision1,
SHA2567579145c3de5a1c6f8088fd7fb804d29dac8903ec505048f3ce595c45023b787, under Atlas ADR0050.
It is one SDK adoption story, not a new prerequisite or a collection of follow-up patches.

At source b5a1c1562984b11b5b945c5f95710497b2a69d80, service-builder/src/realization.rs:195–210
selects exactly one Otherwise outcome; service-engine/src/lib.rs:972 onward loads/folds SDK events,
produces events and appends them, and its ReducerEffect path owns state reduction. These facts do
not prove ER selection or record/replay delegation. Existing service-runtime-ir/3 recompiles its
strict persisted document against exact ESS/synthesis inputs; that binding remains required.

Accepted pure ER target250f699 provides pre-load refusal, selected-outcome fulfillment, service/3
and record/request4. The complete ESS lowerer is being implemented independently. Its closed
semantic design, exact Rust binding/obligation interface and diagnostic model live at ESS
 docs/design/ess-evolution/entity-runtime-lowering.md and
 docs/design/models/entity-runtime-lowering/. They are the typed home of the imported service
semantics; do not create a parallel SDK domain/compiler or turn the diagnostic model into wire data.

## Complete acceptance

- Add opt-in service-definition/4, service-runtime-ir/4 and service-realization-plan/4 through
  distinct strict readers and closed Rust-owned representations. Preserve all /3 constants,
  canonical fixtures, generated paths, reader refusals and behavior. Old readers must reject /4
  before effects. Persist exact ESS source/synthesis identity, selected component, accepted ER
  revision, complete validated definitions and complete binding plan; reload recompiles and
  compares every bound value rather than trusting a persisted definition independently.
- Compile by selecting the authored ESS component, extracting its ServiceIr and invoking the
  official ess-entity-runtime lowerer. Keep complete source capability/obligation coverage.
  Resolve every typed command/outcome/field fulfillment policy explicitly. Missing, duplicate,
  extra, wrong-type, identity-targeting and required-Remove bindings refuse. Do not infer Preserve,
  turn absent optional values into null or invoke one shared value-producing obligation twice.
- Delegate /4 creation, update, transition, ordered conditional outcome/refusal, predicate,
  invariant, identity, relation, exact-value and event order/multiplicity semantics to ER.
  The SDK owns authenticated context, authorization, hosting, realm, content staging, queries,
  projection delivery and declared external effects. No SDK outcome selector, synthesized
  placeholder fields, post-record state patch or SDK ReducerEffect execution on the /4 path.
- Retain admission before application-input decoding and preserve None versus Some(default) realm.
  Derive the exact ER subject address from logical identity. Existing-instance commands call
  decide_before_load; a pre-load refusal performs no subject lookup or fulfillment. Load exactly
  PreparedSubject, continue with ER's opaque PreparedOperation, then invoke policies only for the
  accepting outcome selected by ER and complete it once through PreparedOutcome.
- Preserve PayInvoice: zero/negative amounts refuse before lookup even for an unknown identity;
  positive amounts load only the exact subject, and missing identity remains the host's unknown
  subject result. Cover IssueInvoice.issued_at, AdmitVisitor.badge and every other omitted field.
  Required fields admit Set/Preserve; optional fields additionally admit Remove. Shared semantic
  event/response values use the same completed action/result, with independent obligations separate.
- Persist complete ER decision and observation envelopes through the recorded Eventlog adapter,
  including zero-event accepted decisions, original global IDs, atomic batches, expected-version
  and idempotency semantics. Retry the same identity against the same intent, preserve uncertainty
  after dispatch, refuse changed intent, and replay through ER. Restart must reconstruct the same
  complete state and receipts without running business commands or fulfillment providers again.
- Preserve query/guard/projection delivery, authentication/authorization, event publication and
  content custody behavior. Committed projection failure carries the original durable receipt and
  repairs from authority. Billing email stays its declared SDK/provider effect, not a new stateless
  entity or fabricated ER command. Keep all existing public operation names and generated entrypoints.
- Generate synthetic billing and gatepass packages from the existing selected ESS examples using
  service-builder only. Build and start the generated services, exercise actual HTTP with disposable
  credentials/providers, persist, restart, and verify complete authorization/query/effect behavior.
  Generated application packages remain definition-only; reusable behavior belongs in typed SDK
  providers. These fixtures do not reinstate The excluded adopter or fake-backend scope.
- Use exact qualified ESS/ER/Eventlog pins for final acceptance, and preserve required existing
  dependency minima. Current SDK workspace is Rust1.91; pure upstream ER/ESS minima remain separately
  enforced. No AEP production dependency, platform lineage, release, deployment or credential migration.

## Concrete integration seams to settle in the /4 design

These are parts of the approved preservation/delegation requirement, not additional prerequisite
stories. Existing source demonstrates why simply replacing the old append call is insufficient:

- service-engine/src/lib.rs:522–602 makes AppendRequest a nonempty event batch and RecordedIntent
  carries only events, stream and through_version. At :956–970, replay_intent returns the single
  statically compiled plan.outcome rather than the recorded selected outcome. The /4 resource/result
  contract must carry the complete ER record and original selected result, including zero-event
  decisions, typed actions/removals and exact receipts. Never reconstruct the result from event count
  or the old single-outcome plan. The /3 types/readers stay unchanged.
- service-engine/src/lib.rs:880–949 checks an original-input IntentClaimRequest before generating a
  creation identity and recovers that claim after uncertain writes. Its digest binds plan, operation,
  admitted input and authenticated identities. ER Recording carries record_id/time/correlation/
  causation/actor only; it has no arbitrary SDK-claim metadata slot. The /4 design must bind original
  intent and its winning ER result atomically, retain same-key/different-input conflict, and recover
  a concurrent creation winner despite independently generated losing IDs. Do not repurpose actor
  or correlation as a hidden claim, add a post-commit best-effort claim write, or widen ER records
  with an arbitrary metadata bag. Existing ER atomic batches and complete record lookup are concrete
  mechanisms to examine before proposing any upstream extension.
- ServiceStream and IntentClaimRequest preserve authenticated tenant, service, exact optional realm,
  category and selected stream identity. Candidate entity-eventlog::Authority has one immutable
  logical-scope/tenant/generation binding and Subject is entity/id. Define and test the injective /4
  partition mapping, including None versus Some(default), before opening a store. Derive logical
  identity through the accepted ER address function; never silently collide two realm/service
  subjects or accept client-controlled partition coordinates.
- The existing append receipt's through_version describes the accepted service event batch, while
  ER records retain subject revisions and separate physical receipt positions. The /4 design must
  explicitly map public expected-version inputs and returned versions; do not substitute one
  counter for another because both are u64. Test zero/one/multiple event outcomes, unchanged replay,
  concurrent commands and projection restart against the intended generated public contract.

Stopping condition for these seams: a concrete closed /4 representation and native acceptance
cases that establish the existing required behavior. They stay within this one full SDK unit and
its original review budget. A demonstrated missing upstream capability requires an exact witness;
no speculative target amendment or generic claim/fencing platform is authorized.

## Bounded deliverable and checks

The deliverable is the complete /4 compilation/execution/persistence/generated-service path above,
with its typed design written before implementation and all source/fixture changes under this one
owner. The existing M5 lowerer design is not reopened. Unit tests alone cannot close service acceptance.

Before changing readers, retain literal /3 fixtures and old-reader /4 refusal checks. Test exact
bound-source recompilation, altered-pin/definition/binding refusal, optional/shared slot behavior,
ER pre-load and post-load ordering, full fixture coordinate inventories and recorded history. Use
counting lookup/fulfillment/append boundaries to detect a bypassed refusal or repeated side effect.
Compile-valid controls that reintroduce the old Otherwise selector, skip exact-subject checks,
change fulfillment actions, drop event occurrences, duplicate append/replay or bypass authorization
must fail their named cases; restore exact source and rerun the required cases.

Run actual Rust1.91 checks and complete task check, including generated fixture regeneration,
strict Clippy/rustdoc, existing tests, web checks and retained aliases where applicable. Retain
command exit statuses, generated-source/source-input hashes and actual HTTP/restart observations.
Native PostgreSQL checks, where required by the selected adapter/provider contract, use a real
assigned disposable server and cannot silently skip. Root owns at most two whole source reviews,
final composed gate and local integration; accepted source is not a published release.

Stop the author assignment when the complete deliverable and required checks/report are satisfied.
Report any precise missing upstream required behavior with a reproducer; do not silently refuse a
required fixture or expand into a general interpreter. Further work requires a separate justified
assignment. No new review unit resets the original two-pass source budget.

## Scope

Cited existing surfaces: Cargo.toml/Cargo.lock; crates/service-definition/;
crates/service-runtime-ir/; crates/service-builder/; crates/service-engine/;
crates/service-eventlog/; crates/service-obligations/; crates/service-conformance/;
crates/service-host/; crates/service-runtime/; crates/service-http/;
crates/service-connectors/; CHANGELOG.md; README.md; Taskfile.yml.
Inferred new surfaces: docs/design/ess-evolution-er-delegation.md and additive synthetic /4
billing/gatepass fixtures under crates/service-host/tests/fixtures/.
Original /3 fixtures and accepted ESS/ER/Eventlog production sources are compatibility authorities,
not surfaces for silently weakening checks. Root alone writes planning state and shared handoff.

## Dispatch and blockers

RESUMED full original M6 implementation in the existing managed SDK tree atb5a1c156, preserving source checkpoint2 and root planning lineage. Original M5 selected-identity correction is closed at lowererb5e980fe0aff382bca91a08ebd3827ee8ff4b73e and ER7fd93ef43d4a91c460c7305f9e3be90d7b0a4c11. Exact freezes/common receipts/local caches verified; both lowerer whole reviews remain closed. See local-evidence:ess-evolution/waves/0009-service-convergence/selected-creation-identity/lowerer/freeze/receipt.md and the current resume section of sdk-delegation-unit.md.

Amend unpublished /4 concrete design and closed binding mirror before new readers. Shared mappings retain Created; unequal SelectedOutcome mappings keep each authored identity/coordinate and call ER decide_create_derived. Align all ESS dependencies to b5e980fe and direct ER dependencies to7fd93ef4. The obsolete250/9769 type split can disappear, while strict persisted source recompilation, complete binding/definition comparison and /3 compatibility remain mandatory. Establish coherent direct Eventlog dependency identity where adapter types cross; candidate adapter uses43ceaa09. No path override, host selector, denied administration examination or inferred qualification.

Worker operation_fulfillment_correction owns source/own lease and one bounded max1 Rust1.91 compile/unit lane with task-owned output/resource floors. Nested worker owns heavy acceptance; SDK full service/browser/provider gate waits for explicit handoff. Complete original delegation, persistence, generated billing/gatepass HTTP/restart/auth/query/effects and repository acceptance remain the stopping condition. Root alone owns AEP, at-most-two complete SDK source reviews, source freeze, final qualified composition and integration. Provider/native qualification and full composed ESS gate remain final acceptance restrictions; B-WRITER concerns real planning-store cutovers, not synthetic source implementation. No release/deployment/publication or new prerequisite.

## Whole implementation dispatch after lowerer author closure

The complete pure lowerer author assignment is closed at candidatea8ec43feb2364819941ce0cdc1fdb1290983c041, based on accepted ESSbe604d87 and ER250f699. Exact report local-evidence:ess-evolution/waves/0009-service-convergence/lowerer-implementation/report.md SHAa2d55950e1b9f56e1433812a2296f5387ccdb1fbac3ca22ada4991eb62081d24; source manifest8fcf0c5f verifies all14paths. Dedicated Rust1.85 tests/docs, strict scoped Clippy, formatting and three compiled fault controls pass. Independent lowerer source reviews and composed consumer-enabled ESS gate/site build remain outstanding; this is a source-preparation pin, not a qualified dependency.

Root dispatches this entire existing M6 contract to the now-closed lowerer worker, without a fourth worker or new prerequisite story. Author the concrete /4 typed design before code, including atomic original-intent/selected-result retention, injective authenticated partition mapping and explicit version meaning. Then implement complete compilation/delegation/persistence/generated billing+gatepass acceptance within the existing scope. No substituted static outcome, fabricated field policy, event-only record, hidden claim metadata or weaker fixture is admitted.

The complete fixed stopping contract remains local-evidence:ess-evolution/waves/0009-service-convergence/sdk-delegation-unit.md. Source work may use explicit candidate interfaces while qualified dependency pins and B-ADMIN-REVIEW still gate final durable acceptance/integration. The candidate adapter is being composed with accepted250f699; do not invent a published/accepted adapter pin. Report a precise required upstream gap with a reproducer. Root owns all planning and at most two complete source reviews; author closes once the entire deliverable and required checks are satisfied. No publication, real planning-store cutover or deployment.

## Compilation native delegation and SQLite restart evidence

Actual source-path milestones now pass on the candidate-facing implementation: billing/gatepass /4 compilation and bound strict reload; native billing ER selection/refusal/fulfillment, original-intent retry/conflict, uncertain committed recovery/replay and committed-projection receipt; and a real disposable SQLite recorded-adapter shutdown/reopen that recovers complete decisions/observations without UUID, fulfillment, append or effect reexecution. Root inspected the SQLite fixture: it uses eventlog_sqlite::SqliteEventStore and fresh RecordedEventlogBridge owners on the same database, not a memory-only stand-in.

15-builder-v4-fixtures exit0, log SHA256 b613962229083f817bb3d5bc6e3bda66c49b6196419fad3acb966ee3c0626123
20-builder-v4-native exit0, log SHA256 c49cc47ca1ecbefdce0c904f1a7242c4ac838ffa2559766fdcb99713dd67b038
23-recorded-sqlite-restart exit0, log SHA256 29cb92b3a9ad4e84fa835519397aa30b503844617764682d0b78a46ed3daf1b0

Evidence directory: local-evidence:ess-evolution/waves/0009-service-convergence/sdk-delegation/resume/logs/. The SQLite test uses counting host effect/projection ports; it does not establish actual generated HTTP, external email or query/provider acceptance. Those full billing/gatepass runtime behaviors, complete package/MSRV/strict/full task gates, both whole source reviews and qualified integration remain required. Source is still changing and is not frozen. Original complete author contract and stopping condition remain unchanged; owner stays active, no publication or provider qualification claimed.

## Recorded HTTP restart and package emission evidence

The recorded billing acceptance now starts actual localhost TCP Identity and service HTTP routers in-process over a real reopened SQLite RecordedEventlogBridge. Log39 proves unauthenticated malformed input receives401 before decoding, authenticated exact retry returns the original persisted commit with replayed=true and no repeated UUID/slot/projection/effect host calls, and authenticated get_invoice reads one authoritative row. Root inspected the fixture's provider, shutdown/reopen, TCP listeners and assertions. The first native commit calls the fixture projection/effect ports once; these are counting host ports, not actual email-provider acceptance.

Log40 passes native gatepass register/admit semantics, badge fulfillment retention and Expected view transition. Log42 passes billing/gatepass strict /4 package emission, complete compilation and changed-binding/definition/target rejection. These generated crates have not yet been built and launched as standalone service processes; package emission is not that acceptance.

The complete original assignment still requires remaining /4 context/content implementation, a root-frozen local SDK candidate for generated self-consumer build/start/HTTP/restart acceptance, actual required provider/effect behavior and all minimum/strict/full workspace/web/browser/provider gates. Root will freeze an unqualified local candidate when source is ready; needing its Git pin is not an external blocker. Final independent reviews and qualified integration follow complete checks. No full M6 acceptance is claimed and /3 behavior remains protected.

Evidence local-evidence:ess-evolution/waves/0009-service-convergence/sdk-delegation/resume/logs/:
39-recorded-http-restart-test log SHA256 5d4a9695c1443a84ee3b803aef00b69035f69e1a0de8fa99f4955e124e029edd
40-gatepass-native-test log SHA256 c00328b7524f921bb7ab235c5c88b45264b3151ca13c596cb2e35923cc6f4120
42-package-v4-test log SHA256 86e84444a2e47db8952fa2a9de772e4e7fee3d49eefee0fbe887c5a92fe0d9ea

## Local candidate for generated service acceptance

Local unqualified SDK candidate090c21c6bb1692081fb54f9b2f8ef482516d2d7b, tree68ec1c05a064fa485603a90c03a73ea809da6d91, parentb5a1c1562984b11b5b945c5f95710497b2a69d80 is frozen from exactly42 source paths. Root independently checked path/content/check/report manifests, committed with the organization bot, verified signed common evidence and rechecked every committed source digest. Both planning files remained byte-identical and outside the source commit. Existing local Cargo Git databases were seeded with the exact object; no publication or remote fetch.

Source-ready evidence: content/context compile43; content policy/privacy45; scoped strict Clippy58; unfiltered /4/native/real SQLite HTTP restart60; rustdoc61 and fmt/diff all0. Exact reports/manifests and receipt are retained under local-evidence:ess-evolution/waves/0009-service-convergence/sdk-delegation/resume/candidate-freeze/receipt.md.

This local source identity unblocks the original actual generated billing/gatepass package build/start/HTTP/restart/effect acceptance. Emission, native tests and the real in-process TCP fixture do not replace standalone generated process acceptance. Full SDK workspace/browser/provider/minimum gates, final source reports and original independent review/integration remain. No full M6 acceptance or qualified provider pin is claimed.

## Generated service process acceptance

# Generated service acceptance: coordinator scope review

Updated 2026-09-16T17:29:35.775Z. Root rehashed generated-candidate.sha256 (122 retained artifacts) and generated-candidate-evidence.sha256 successfully, and inspected the entire Rust harness plus terminal log71. Candidate090c21c6 is unchanged. Logs62/63 prove generation and locked/offline builds of both generated crates. Log71 proves actual child-process generated HTTP routers over real SQLite, shutdown/reopen, authenticated queries, billing replay and gatepass register/admit/replay/list transition. Authentication authority is a disposable local fixture.

The harness Host::effects is custom: it serializes declared effect obligations to create_new JSON files, accepting an existing equal file. Thus a single retained artifact does NOT prove the existing SDK email provider executed or that effects was called once. Host::authorize and visible are permissive fixture implementations. This evidence does not close full provider/authorization/query partition acceptance. Root directed the original worker to exercise the existing provider binding and exact receipt/invocation behavior under the original M6 contract. No new task or review budget. Full SDK/browser/provider gates, two whole-source reviews and qualified integration remain.

Manifest hashes: paths fcfbaa32118248f5ddb582612cf616f17e105ee144343a9d04117a0822d9ecbd; artifact manifest 1f357f383a5d6cb543841e38abb4f2bd2e32e03bc8ecf525951230e0b1e2bd2b; terminal log e717b7d45640e7743168fae95e2649330c27bedfaa91587f65217313dcecc15c.

## Actual journal provider acceptance and remaining retry semantics

Candidate5397ba4e37ff87339bec6da7bea8023155553bde generated billing/gatepass packages build through exact Cargo Git inputs. Final log87 exits0 for actual generated child-process HTTP over real SQLite, public EffectPlan/EventlogEffectJournal prepare→claim→resume_effect and a disposable email EffectAdapter receiver. Billing returns exact equal structured commit on restart/retry and receiver dispatch count remains1; gatepass count0. Root inspected the actual harness, not just its success message, and rehashed all122 generated/source artifacts plus48 retained log artifacts successfully from the manifest base.

Evidence local-evidence:ess-evolution/waves/0009-service-convergence/sdk-delegation/resume/candidate2-provider-acceptance.md (SHA3b2a3a08aeaed540ac664f048596a27e790d36e36e2b8888841f073706668211), candidate2-generated.sha256 (5b0684d8d2fb86c70fc56d93b92d6f46570ac4165fca23ed7da302649f7b4e67), candidate2-provider-checks.sha256 (05d6e9c1b8faad591a082bb9999150b2bff7b2bec54e91202e46431a7cb8cb4a), final log87 (7c8a6d84f1c3cbd91cb5915e46b7ea9dde8dc0772bde62648bc54de73b63aa39). Separate explicit effect-journal authority is not atomic with the ER decision batch; no provider registration/projector guard changed. Disposable credentials/receiver are synthetic acceptance, not external email delivery.

M6 remains incomplete. Root identified a concrete residual retry risk at EngineV4::recover: replay of complete current history is compared to the original committed instance, so a later command may break retry of an earlier identity. Author will prove/correct the original historical retry requirement and identify the original aftercare repair-from-authority path before final gate/report. No new story or review budget. Full task check and independent whole-source examinations remain, with qualified dependency integration still externally gated.

## Historical retry and recorded aftercare correction

Local correction candidate ddcc41767acc0ddae0d26108ff3ecbd5127beb58 preserves parent5397ba4e and the exact ESS/ER/Eventlog pins. Historical retries verify complete current history and recover the exact original decision prefix. Explicit authenticated repair reloads the original intent, decision and immutable receipt before projection/effect/content aftercare, without rerunning business decisions or fulfillment. Recorded content repair uses the committed reference and original authenticated coordinates; transient staging tokens/plaintext are absent.

Evidence: local-evidence sdk-delegation/resume/retry-aftercare-content-report.md and retry-aftercare-content-freeze/receipt.md. Root verified7 declared source hashes and52 check artifacts; complete scoped suite99 and strict/fmt100 exit0. All33 planning/config files remained unchanged across the source freeze; common check and signed verification exit0. Existing generated actual-journal/provider87 remains qualified only at candidate5397ba4e. Regenerated acceptance against ddcc417, complete task check, original whole-source reviews and qualified integration remain required under this same assignment. No publication or full acceptance claimed.

## Complete actual PostgreSQL gate and compatibility restoration

Complete actual-PostgreSQL task check111 exited zero on the exact source frozen as candidate663be6f772fdf3ee9c0003b08d4f6d17357c3f73, parentddcc41767acc0ddae0d26108ff3ecbd5127beb58. All ten required persistence cases ran; the two-process workload executed all six configurations with zero skips. Workspace tests, strict Clippy, rustdoc, formatting, producer-fixture drift, web, AEP and release-action checks passed. Disposable PostgreSQL authority was provisioned only for this gate and torn down after terminal success.

Fullgate105 exposed legacy generation under unified serde_json arbitrary_precision and producer-owned fixture drift. Exact compatibility correction preserves numeric YAML using a typed JSON-text conversion boundary, and regenerates six fixture digest/manifest paths through service-builder. Focused107/109 pass; fullgate110 correctly refused absent required PostgreSQL authority; fullgate111 is the complete green restoration. Root independently rehashed7 source and27check artifacts, preserved all33 planning/config files across bot commit, verified signed common receipt, and seeded exact local Cargo Git objects without publication.

Evidence: local-evidence sdk-delegation/resume/full-gate-compat-report.md and full-gate-compat-freeze/receipt.md. Generated actual journal/provider repair104 passed at parentddcc417; final candidate663be6f generated acceptance and full original author report are in progress. Original whole-source examinations and qualified composed integration remain open; this is no acceptance waiver or lifecycle completion claim.

## Complete author acceptance and handoff

## Complete author acceptance and handoff

Original full M6 author assignment is CLOSED at local candidate663be6f772fdf3ee9c0003b08d4f6d17357c3f73 (tree d7ada01885e5900a977e8c0a899c7baf4c9e0cd9). Complete actual PostgreSQL task check111 passed, including ten required persistence cases, six workload configurations and zero skips. Final exact-candidate generated billing/gatepass regeneration, locked builds and child-process provider/restart/repair acceptance112–115 all passed. Root independently rehashed49 source paths,122 generated artifacts and309 check artifacts, and read all five terminal exit statuses.

Whole author report: ~/beyond10x/.ess-evolution/waves/0009-service-convergence/sdk-delegation/resume/final-author-report.md, SHA256 fb35eda65f10df54ff0d753078510490fe2ec72587d6c62bc46425616e61a12c. Root verification: coordinator-final-author-verification.json in that directory. Final-source manifest fbf2774e56331817e357920b4314b7b9883a08b03c6119f42d77419713c4c37a; generated b7fa5048fa2c0554368ccfe117e62e1e43aa585a412dacc0dec23de2109a58df; checks50ede27868ea1e92dfc4467560f7ca9c03d1617e3709722deacd42aee754a908.

Required independent whole-source review (original max2 passes) and qualified integration remain. These checks prove author acceptance, not provider qualification or final integrated M6 completion. Disposable PG fixture was torn down; worker lease released. No publication or external mail delivery claimed.

## Original source review correction

Original whole source pass1 remains closed NEEDS-CHANGE at663be6f772fdf3ee9c0003b08d4f6d17357c3f73. Its five introduced contract failures remain recorded verbatim in review-result:sdk-er-delegation-source-pass-1. The original author assignment and correction worker remain closed; root carried the original correction through complete final-source author acceptance at local candidate8ede45c15d6d3df79cc9c48b155889351409a79b.

All five imported regressions and valid billing/gatepass counterparts pass. Existing checks additionally exposed overly strict optional Set admission and unintended /3 OpenAPI drift; root fixed both while preserving original expected artifacts. Required strict checks exposed two overlong obligation functions; one bounded source refactor is closed. Two inherited fixture assumptions about checkout-local Cargo targets were corrected while retaining exact source/binary hashes. These are repairs within this existing outcome, not new product scope.

Final task check12 on Rust1.91 passed, with all10 actual PostgreSQL/persistence cases and all6 workload configurations, strict Clippy, fmt, rustdoc, web, AEP, release-action and legacy generated drift. Exact8ede45c generated billing/gatepass compilation, three offline locks/all-target checks, real child HTTP/auth/query/restart/provider/repair acceptance and final producer drift14–25 pass. Generated SDK Git pins all resolve to8ede45c. Signed common checks pass; no publication. Fresh PostgreSQL fixture was removed and absence verified; owned disposable data retired, logs/manifests and next review cache retained.

Evidence: root sdk-delegation/source-correction-1/final-author-report.md, coordinator-final-acceptance.json and final-source/final-generated/final-checks manifests. Prior failed runs remain red; earlier source receipts are historical, not overwritten. This closes correction author acceptance only. Original final whole-source pass2 of2 and qualified integration remain; no third review, gate waiver, cutover or provider-administration review is authorized. Source-review-2 is prepared on exact8ede45c. M6 remains active and provider qualification remains external.

## Original final source review and remaining correction

Final whole-source review2 is CLOSED NEEDS-CHANGE at8ede45c. Its verbatim report is review-result:sdk-er-delegation-source-pass-2. Machine ledger:0carried,2new,5resolved; these two new failures are within the original selected-obligation execution requirement. Public owner-and-scopes selects an unrelated same-entity instance and permits a wrong-owner commit. Allowed-state lifecycle compares raw logical identity with encoded ER storage identity and refuses a valid Issued subject. Both exact cases and the affected suite failed for the intended assertions; seven other affected cases passed. Prior full author acceptance remains exact-source historical evidence, not final acceptance.

A bounded correction of this same contract imports both reviewer cases, fixes selected subject and typed ER identity resolution, preserves original assertions, runs complete affected package tests/strict checks, then closes. Root verifies the final diff and both cases and owns required final-source full/generated acceptance and qualified integration. No third source review, review budget reset, new task, baseline change or acceptance waiver. Existing provider qualification remains blocked separately. Evidence: local-evidence sdk-delegation/source-review-2/report.md and source-correction-2/brief.md/findings-ledger.json.

## Final correction accepted on provisional dependency vector

The original final review remains immutable NEEDS-CHANGE at 8ede45c. Both new findings are corrected at local candidate 9a51ced509009405ea54a98a7d6250747e62d103. Root verified the diff, original assertions, valid controls and the same identity class in aggregate/graph callers. Both bounded workers are closed; no third examination or reset.

Complete Rust 1.91 task check ended zero at 2026-09-17T03:10:49Z with all ten actual PostgreSQL persistence cases and six workload configurations executed. Exact-candidate generated billing/gatepass packages passed offline locked builds, actual child HTTP/auth/query/restart/provider/repair acceptance and strict producer drift; equal billing receipt and one receiver dispatch were observed. Signed common checks pass. Source209/generated122/check47 manifests bind this evidence.

Evidence: local-evidence:ess-evolution/waves/0009-service-convergence/sdk-delegation/source-correction-2/coordinator-final-acceptance.json and coordinator-correction-verification.json. Earlier failed runs and immutable review reports remain. This closes corrected SDK source acceptance; M6 stays active because its exact dependency vector remains provisional and qualified integration is still blocked on original provider acceptance. No publication or cutover.

Checkpoint cleanup: owned PostgreSQL fixture removed and absence verified; its data size is unmeasured, so no byte recovery claimed. Terminal SDK build cache retired after preserving exact executables and receipts, net 4495499264 bytes reclaimed. Managed source and local commit retained; no remote recovery proof means no whole-tree removal.
