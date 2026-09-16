# Entity Runtime delegation for generated services

Status: binding design for the opt-in `service-*/4` path.

This design adds one versioned execution path beside the existing generated service path. The
`service-definition/3`, `service-runtime-ir/3`, and `service-realization-plan/3` readers, values,
generated entrypoints, and canonical bytes remain unchanged. A `/3` service continues to use the
SDK event reducer. A `/4` service delegates every domain decision and replay to Entity Runtime
while the SDK retains authentication, authorization, hosting, content, queries, projection
delivery, and declared external effects.

## Versioned documents

Each `/4` document has a separate closed Rust type and a separate strict reader. A `/3` reader
checks its format discriminator before validation and refuses `/4`; a `/4` reader likewise refuses
`/3`. No untagged compatibility enum broadens either reader.

`service-definition/4` retains the SDK-owned delivery, realm, input, authorization, query,
projection, content, effect, idempotency, and expected-version annotations. It adds:

- the selected ESS component;
- a nonzero Entity Runtime definition version for every entity in that component's closure;
- exact scale declarations;
- one closed binding for every ordinary host slot emitted by the ESS lowerer;
- one operation-field policy for every lowerer `command/outcome/field` coordinate.

An ordinary slot binding is one of verified context, normalized command input, trusted clock,
UUIDv7, existing SDK obligation, or an exact literal of the lowerer's type. An operation-field
policy is `Preserve`, `Remove`, `CommandField`, or one named SDK obligation. These are closed enums,
not JSON paths or provider-defined property maps. Compile rejects missing, duplicate, extra,
wrong-type, identity-targeting, unsupported-action, and required-field `Remove` bindings. Optional
values stay absent; they are not converted to JSON null. One resolved slot is evaluated once and
its value or absence is reused by every target carrying that slot. Independent slots are evaluated
independently.

`service-runtime-ir/4` contains the exact existing ESS and synthesis bindings plus:

- selected component;
- exact accepted Entity Runtime source revision;
- complete validated Entity Runtime definitions;
- complete typed lowerer binding plan, including source capabilities and obligations;
- the resolved SDK slot and operation-field policies;
- all retained SDK admission, query, projection, content, effect, and authorization plans.

The persisted binding plan is a closed serialization mirror of the lowerer's public typed output.
It exists because the lowerer deliberately has no persisted envelope. It does not interpret ESS.
On initial compilation it is constructed only from `LoweredService`. On reload the SDK selects the
same component, calls `ess-service-contract::extract`, invokes `ess-entity-runtime::lower`, converts
the new typed result, and compares the full document. Source digest, synthesis digest, component,
target revision, every definition byte, every coordinate, every slot value source and presence,
every operation-field requirement, every capability, and every SDK policy must match. The reader
never trusts a persisted definition or binding independently.

`service-realization-plan/4` is generated from a validated `/4` runtime IR. It carries the validated
ER registry material and the same resolved binding coordinates needed at execution, plus SDK host
plans. It has no SDK outcome selector and no `ReducerEffect`. Generated application crates contain
only this definition data and use the shared SDK `/4` engine.

The accepted revisions are exact Cargo Git revisions. The plan also records them as data so a
document produced against one semantic target cannot be opened by another. This candidate uses ESS
lowerer `b5e980fe0aff382bca91a08ebd3827ee8ff4b73e`, Entity Runtime and its `entity-eventlog`
adapter at `7fd93ef43d4a91c460c7305f9e3be90d7b0a4c11`, and Eventlog
`43ceaa09ceec610e25891815e33e03e8df92ee28`. Every SDK dependency on one of those repositories
uses that repository's exact Git URL and revision, so values crossing an API boundary have one Rust
crate identity. There is no path override or equivalent-looking cross-revision conversion.

The lowerer and adapter now share the same `entity-core` identity. Compilation registers and
validates every lowerer-produced `EntityDefinition` directly. Persisted `/4` reload still selects
the same ESS component, reruns extraction and lowering, validates the complete registry, and
compares the freshly compiled closed document including all definitions and binding coordinates.
Strict deserialization rejects unknown fields, and canonical reserialization must equal the
persisted bytes before effects. Removing the obsolete dual-crate bridge does not relax source,
synthesis, component, revision, definition, policy, or binding-plan drift checks. These explicit
candidate pins support source preparation and acceptance testing only; durable acceptance remains
blocked on the adapter's stated administration restriction and native qualification stages.

## Addressing and authenticated partition

The authenticated partition is encoded by one total length-delimited function:

```text
sdk-er-partition/1
  | <service-byte-length>:<service>
  | <tenant-byte-length>:<tenant>
  | 0

or

sdk-er-partition/1
  | <service-byte-length>:<service>
  | <tenant-byte-length>:<tenant>
  | 1 | <realm-byte-length>:<realm>
```

Lengths are UTF-8 byte lengths. Presence has its own tag, so an absent realm differs from every
present realm, including `default`. Length prefixes keep separators inside identities as data.
The service is generated configuration and tenant and realm come only from `VerifiedAuthContext`;
request bodies and routes cannot select them. This complete encoding is the immutable ER
`logical_scope`. It is not placed verbatim into the bounded Eventlog tenant field: an individually
admitted tenant can already consume that field's full 512-byte limit, so adding service/realm
framing must not reject it merely because the composite is longer.

The physical Eventlog tenant is `sdk-er-v4-` followed by lowercase hexadecimal SHA-256 of the exact
UTF-8 partition encoding. The full encoding is retained in the immutable adapter authority binding
and compared byte-for-byte on provision, open and recovery. The digest is an address, never proof
of identity: a physical-address collision with another full logical scope returns the existing
binding conflict before any service read or write, without adopting or rewriting that binding.
Tests retain separate absent/default realms, delimiter and Unicode identities, maximum-length
previously admitted tenants, and a substituted full binding at one physical address. Ordinary SDK
and provider validation of the individual identities still applies.

One adapter authority is opened for exactly this full partition, with the bounded physical tenant
above and the exact expected provider generation supplied by its admitted provisioning record.
Neither a physical digest nor a freshly observed generation silently replaces the caller's full
expected authority. Two services, tenants, or exact optional realms therefore cannot share an ER
store binding accidentally, even if their entity names and logical identities are equal.

Within the partition, the lowerer identifies the target definition and logical identity value.
The SDK gets the declared identity field kind from the validated ER definition and calls
`entity_core::identity::address`. That result is the only ER subject id. The SDK never prefixes or
normalizes it. `Subject { entity, id }` therefore remains the kernel's exact typed address inside
an already isolated authenticated partition.

## Decision order

Admission and authorization that do not inspect application input run first. Only then does the
SDK decode the operation body and normalize the declared input. The SDK resolves the command
binding and ordinary bound slots once.

For `InstanceBinding::Created`, creation resolves the one shared logical identity, calls the ER
address function, and calls `Runtime::decide_create`. For
`InstanceBinding::SelectedOutcome`, the closed binding carries every accepting outcome's exact
identity source and observation coordinate; the SDK supplies all declared inputs and bound slots
and calls `Runtime::decide_create_derived`. Only ER selects the branch and derives its address from
the selected validated fields. The SDK never evaluates the outcome predicates or chooses one of
the per-outcome identities. Existing-instance commands derive the exact subject first and call
`Runtime::decide_before_load` with the bound entity, definition version, subject id, operation, and
normalized `{ input, bound }` arguments.

`PreloadDecision::Refused` maps the ER branch to the declared public refusal. It performs no
subject lookup, fulfillment, append, projection, content acceptance, or external effect. This is
the PayInvoice zero/negative amount rule even for an unknown invoice.

`PreloadDecision::Load` exposes one `PreparedSubject`. The store loads only that subject. An absent
subject returns the existing unknown-subject result. The SDK gives the loaded instance to the
original opaque `PreparedOperation::select_with`; it never rebuilds or substitutes the
continuation. `LoadedDecision::Complete` is used directly. For
`LoadedDecision::NeedsFulfillment`, the selected outcome and exact advertised requirements choose
the already compiled policies. Each policy runs once after selection, producing exactly one typed
`OperationFieldAction`. The SDK checks the complete key set and action admission, then calls
`PreparedOutcome::complete` once. Required fields accept `Set` and `Preserve`; optional fields also
accept `Remove`. The SDK never invents `Preserve`.

ER owns branch order, predicates, refusals, transitions and updates, invariants, identity checks,
relations, exact value semantics, and ordered event multiplicity. Event and response fields read
the same completed ER action/result. An obligation that supplies a shared semantic value is called
once. Independent obligations stay separate. The SDK does not patch the resulting instance, run
the `/3` reducer, or synthesize a self-transition.

Content remains staged before a decision only where the existing custody contract requires an
opaque reference in arguments. A refusal or proved-not-committed write abandons staged content. A
committed or uncertain write retains it for retry recovery. Projection and declared external
effects start only after the complete decision is durably known. Billing email remains its declared
SDK effect driven from the committed selected result; it is not an ER entity or command.

## Atomic original intent and selected result

A `/4` mutation has a canonical `service-intent/4` document containing the realization-plan
digest, public operation, admitted normalized input, verified service/tenant/exact-realm/authority/
user/executor identities, and idempotency key. Its SHA-256 digest is an integrity value. The claim
key is a canonical length-delimited value over authenticated partition, the existing compiled
category, the exact existing stream selector, and idempotency key. The selector is a closed choice:
`CommandField` carries the exact admitted selected stream key, while `GeneratedUuidV7` is a stable
selector tag carrying no generated UUID. This preserves the namespace currently encoded by
`service-eventlog::intent_claim`: equal keys in different categories or explicitly selected streams
remain independent; two creation contenders address the same generated-selector claim before
minting competing identities. The claim key contains neither input nor a newly generated subject
identity. Within the same existing namespace, changed intent addresses the same claim and is
refused after comparison. Category and selector are also retained in the strict original-intent
observation and checked against its compiled plan and selected result during recovery.

A successful mutation is one Entity Runtime named batch with two ordered members:

1. the complete ER `RecordedCommit`, using the caller-stable decision record id and exact
   predecessor expectation;
2. a `RecordedObservation` on the resulting subject revision, whose closed
   `service-intent-observation/4` value holds the canonical original intent, its digest, public
   operation, selected outcome, and the public result fields that are owned by SDK annotations.

The observation has its own deterministic record id derived from the claim key. It is evidence at
the newly produced revision and does not change state. Both members are stored with the adapter's
canonical original request bytes. The named batch key is the claim key. Eventlog commits the batch,
its blob material, and indexes atomically. There is no post-commit claim write and no overloading of
actor, correlation, or causation.

The first member exists for every accepted ER decision, including decisions with zero domain
events. Its `DecisionRecord` retains normalized input, exact fulfillment actions and removals,
selected outcome, effect, response, complete result, ordered events, and definition. The second
member retains SDK-only original-intent and result facts. `StoredBatch.receipt` retains immutable ER
record receipts and physical Eventlog positions. The public response is reconstructed only after
the store has verified the complete recorded history through ER replay.

Before generating a UUID, staging content, loading a subject, deciding, or invoking a fulfillment
provider, the SDK looks up the named batch. It requires exactly the two expected members, validates
their kinds, ids, adjacency, subject and revision agreement, parses the strict observation,
byte-compares the canonical original intent, verifies the recorded subject history through Entity
Runtime replay, and returns the recorded selected result with `replayed = true`. A changed intent is
an idempotency conflict. Corrupt or partial evidence is an integrity refusal.

For a fresh creation, contenders may mint different UUIDs after both observe no batch. The winner's
atomic named batch owns the claim. A losing append or uncertain response performs the same batch
lookup and validation and returns that winner, ignoring its losing generated id. This recovery is
therefore independent of the proposed subject. A retry after restart finds the batch before any
business decision or fulfillment provider is called. For an existing subject, the same mechanism
also separates an exact retry from a new command that now sees a later revision.

Append uncertainty is preserved unless lookup proves the complete matching batch committed or
proves a conflict. The SDK never loops through a new decision. An ordinary idempotent retry returns
the original result and receipt without invoking projection or effects again, including when later
decisions have advanced the subject: recovery verifies the complete current history, then replays
the exact prefix through the recorded decision revision.

Projection or effect failure after a commit returns the immutable batch receipt and a closed
`sdk-er-repair-v1-{operation-hex}-{claim}` token. `operation-hex` is the lowercase hexadecimal
UTF-8 public operation name and `claim` is the existing lowercase claim digest. The generated
`POST /v1/repairs/{token}` route extracts the operation without reading application input,
authenticates and authorizes its exact declared scope, binds the authenticated partition, and
looks up that named batch. Repair requires the recorded original intent to match the current plan,
operation, tenant, exact optional realm, authority, user, executor, and recomputed claim. It
verifies the complete history and recorded decision prefix before rerunning only idempotent
projection and durable effect delivery from the recorded decision and immutable receipt. It does
not execute the business command or fulfillment again. For each content input present in the
recorded original intent, repair extracts only the committed record-safe reference from the
recorded decision arguments and asks the content provider to accept by authenticated partition,
declared policy, original idempotency key, and exact reference. The transient staging token and
plaintext are never persisted or reconstructed; a content provider must make this recorded
acceptance coordinate idempotent.

## Version meanings

`service-realization-plan/4` changes the public mutation counter to one precise meaning:

- `expected_version = 0` means the ER subject must be absent for creation;
- `expected_version = n > 0` means the exact ER predecessor revision `n` for an existing command;
- `through_version` is the resulting ER subject revision from the recorded decision.

Every accepted decision advances the revision once, including unchanged and zero-event decisions.
One or several domain events from a decision share that decision revision and do not independently
advance the public counter. A replay returns the original `through_version` unchanged. This avoids
deriving a public version from event count.

`RecordReceipt.position.subject`, `RecordReceipt.position.store`, Eventlog global sequence, and
physical stream version are persistence coordinates. They remain in the durable receipt used for
diagnostics and repair and are never accepted as public expected versions. `/3` retains its existing
service-event-batch counter semantics and bytes.

Concurrent commands with the same predecessor and different idempotency keys race on the ER
expectation; one can commit and the other receives an exact revision conflict. The same claim key
and same intent replays the winner. The same claim key and different intent is an idempotency
conflict regardless of current subject revision.

## Projection, queries, effects, and restart

After a new commit, the SDK uses the ER result instance and ordered recorded domain events to feed
the existing generated projections and publication boundary. Projection rows retain hidden
authenticated partition data and exact source subject. Queries enforce authentication,
authorization, selectors, shape, and partition before exposing rows. A projection write records
the ER `through_version`; restart rebuilds from the verified complete ER history and reaches the
same rows even for removals and zero-event decisions.

Declared external effects are prepared from the recorded selected outcome/response and use the
existing effect journal. The effect boundary receives the same verified authentication context,
declared intent obligations, selected decision, and immutable commit receipt, so it can prepare
the existing authenticated effect plan without reconstructing authority from application data.
Effects are never inferred from event count. A retry or repair reuses the
recorded effect claim. Content acceptance likewise follows proven commit and can resume from the
atomic claim. These host activities cannot modify the recorded ER result.

Generated billing and gatepass fixtures are built only by `service-builder`. Billing proves issue
time fulfillment from the trusted clock, payment pre-load refusal ordering, issue and payment
events, query authorization, invoice projection, and the declared email effect. Gatepass proves
badge reuse from normalized command input, lifecycle moves, optional removal/preservation,
authorization, and projection. Each package is started through its actual HTTP entrypoint against
disposable credentials and provider state, stopped, restarted on the same store, and queried and
retried again.

## Refusals and controls

Compilation accumulates diagnostics in stable coordinate order. It refuses source or synthesis
identity drift, component drift, target revision drift, definition drift, binding drift, unknown
capabilities, incomplete obligations, missing or extra versions/scales, and every invalid policy
shape. Strict readers reject unknown fields and noncanonical or mismatched bound documents before
effects.

Native acceptance counts admission, lookup, fulfillment, append, projection, and effect calls. It
covers zero/one/multiple events; unchanged decisions; `Set`, `Preserve`, and `Remove`; exact retry;
changed-intent conflict; concurrent creation with different proposed ids; concurrent predecessor
conflict; uncertain append recovery; projection failure and repair; and restart. It verifies all
old `/1` through `/3` vectors unchanged and old readers refusing `/4`.

Mutation controls independently demonstrate that the acceptance suite fails if an ordinary slot is
miswired, a duplicate event occurrence is dropped, an update becomes a self-move, a fulfillment
coordinate/action is changed, a shared provider is called twice, load happens before pre-load
selection, the prepared subject is substituted, the ER result is patched, a refusal is appended,
an action or removal is dropped from the record, `/4` is framed as `/3`, authorization is bypassed,
or physical positions are substituted for public revision.
