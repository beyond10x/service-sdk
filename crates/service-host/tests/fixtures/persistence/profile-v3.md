# SDK laboratory profile v3, declared before measurement

This version selects a paced SDK laboratory envelope. It is not production capacity admission.
Profiles v1/v2 and every original observation remain immutable. The complete common
resource, semantic and latency requirements below are retained from v2.

Provider: merged Eventlog 081815cdfcbf1c751e7ee91abd81af2ff7d460cb. Root-owned isolated
PostgreSQL 17.6 Alpine 3.22 container, image digest
`sha256:ef257d85f76e48da1c64832459b59fcaba1a4dac97bf5d7450c77753542eee94`;
limits 2 CPUs, 1 GiB RAM, 256 PIDs. Verified loopback TLS uses the root-owned fixture CA.
Exact storage allocation/usage and role/server connection limits must be measured and
recorded before the run; an absent required value refuses measurement/admission.
Root supplies control after explicit handoff; there is no simultaneous root workload.

Use two independent SDK processes with 2 connections each, at most 4 pool waiters each,
and reserve 4 database connections for migration, observation and root-controlled restart.
The SDK workload budget is therefore 8 connections. The application role has a finite
connection limit of 4; root must reconcile a different current value before opening.
Pool acquisition timeout 250 ms; connect timeout 2 s; statement timeout 2 s; lock timeout
500 ms; transaction timeout 3 s; pool shutdown timeout 2 s; HTTP drain timeout 2 s.
No queue or connection bound is increased after observing results.

Fixture cardinalities: 2 declared SDK service plans with 2 inline tables each;
8 tenants; exact realms None and Some("default"); 256 aggregate streams per process;
at least 1024 retained events before steady measurement. Caller content is fixed
4096-byte UTF-8 text; a domain event is at most 2048 encoded bytes and carries only
opaque content references. A query page requests at most 32 rows; an event page at most
32 events. Record actual persisted/event/response byte counts.

Measure concurrency 1, 8 and 32 across the two processes under both uniform aggregate
selection and a 50% hot-aggregate distribution. Seed/work allocation is fixed before
each run. Workload operation mix is 20% Create, 20% transition/update, 20% same-key retry,
20% projection query, and 20% event page. At least 200 warm-up requests and 2000 steady
requests run per configuration, with steady measurement lasting at least 10 seconds;
both count and duration requirements must be met. Record conflict, deduplication,
overload/deadline refusal and unresolved-outcome counts separately.

Correctness has zero tolerance: no duplicate accepted aggregate/event/effect identity,
cross-partition disclosure, changed retained content, missing feed event, partial inline
state, escaped pool bound or falsely successful unresolved commit. Deliberate stale-key
conflicts and bounded overload refusals are expected outcomes, not missing scenarios.
Within admitted concurrency 1, p99 successful request latency must be at most 2 seconds;
at overload concurrency 8/32, each refusal must complete within 3.5 seconds. Successful
inline reads have zero logical projection lag. Feed catch-up after transaction quiescence
must finish within 3 seconds. A database restart/reconnect interval requires eventual
readiness and authoritative retry/replay within 15 seconds after the server is ready.
HTTP draining plus pool shutdown must complete or return explicit timeout within
4.5 seconds. Record actual p50/p95/p99, throughput, arrival rate, queue time, pool peaks,
RSS/CPU/IO and restart/replay duration; budgets alone are not measurements.

Run file SQLite restart/replay with the same semantic cases. Keep old SDK stream, feed,
cursor, projection, effect and content vectors and actual original-builder service/2
refusal evidence. Comparative speed claims require an identically configured old SDK
comparison; absent that comparison, report candidate-only observations and no speed gain.
Provider comparative results are not reclassified as SDK measurements.

Required proof fails on missing PostgreSQL, zero selected cases, required skips,
missing measurements, any correctness breach, any bound breach or any budget failure.
This file is immutable once measurement starts. Revisions require a new filename and
an explicit reason recorded before another measurement, retaining the original outcomes.

## Retained workload boundaries

The v2 resource and correctness thresholds remain required for this distinct profile.
The 256 streams per process describe the seed population, with two retained events per
seed stream. Successful Create workload requests add streams; they are never counted as
same-key retries. Each configuration is capped at 20000 steady requests, fails if that
cap cannot also satisfy the 10-second minimum, and uses exactly 200 warm-up requests.
All six configurations retain their earlier evidence. Maximum growth is therefore
bounded by six seed populations plus at most 20% of 121200 workload requests.

Both measurement processes exercise the SDK EventlogService boundary from the actual
generated plans, through the same host Persistence store. Their loopback control router
is a test driver with fixed receiver-verified identities, not production HTTP Identity
or Connector transport capacity evidence. The actual generated standalone executable
and generated Connector factory have separate restart/authentication/pressure cases.

Observe provider errors at the injected durable port, retaining exact conflict,
idempotency-mismatch, overload, deadline and unknown-commit categories. Report unresolved
SDK responses separately; never count a refusal as an accepted mutation. Queue time is
sampled waiter occupancy integrated at 1-ms resolution (waiter-milliseconds), with actual
pool maxima and wall-clock success/refusal tails. It is not an invented per-request
acquisition trace. Report sampling resolution and effective observed sample interval.

Process resource evidence is Linux /proc RSS high-water, user/system CPU ticks and IO
bytes, plus before/after fixture Docker statistics and actual database/table size/counts.
Record container limits and role capacity again before opening the two workload pools.
Parent measurement concurrency counts all in-flight child HTTP control requests across
both processes; workload operation selection remains the declared five-way 20% mix.

## Version 3 scheduling decision

The coordinator selected this profile before version 3 implementation or measurement.
The unpaced v2 full gate remained red. Its unchanged diagnostic reached the 20000
steady-request guard at 3595434 microseconds, with 19915 completed responses at that
instant, in uniform concurrency32. The guard prevented dispatch of the next batch.
Partial per-worker overload/refusal snapshots are not a full final outcome tally.
The diagnostic's host roster executed 8 passing cases and 1 failing case. Its raw stdout
SHA256 is 86dfd3306d14c5eb379aa841dec04122700c9da7b5f580574d26fbed4e937064.
A profile can legitimately reject an offered load; version 2 is not reclassified as green.

Version 3 changes only steady driver arrival scheduling and the evidence needed to
measure that schedule. It retains exactly 200 unpaced warm-up requests, at least 2000
steady requests over at least 10 seconds, the 20000 steady-request cap, all six
configurations, original five-way operation totals, concurrency maxima 1/8/32 and all
other limits above. The independent unpaced saturation, cancellation, timeout and
shutdown cases remain mandatory. No production implementation is changed to satisfy
the schedule, and no old/new speed comparison is asserted.

All steady workers share a single asynchronous dispatch gate containing next_dispatch.
A worker prepares its next request before entering the gate. While holding it, wait
until max(current monotonic time, next_dispatch); record the actual grant time and
set next_dispatch to that time plus one millisecond. Release the gate and enter the
HTTP send immediately, with no intervening await. Granting the first request may be
immediate. Subsequent grants are separated by at least one millisecond: at most one
initial grant plus floor(interval_milliseconds) grants in an interval. This is a
1000-per-second driver admission ceiling with one initial grant, not a guarantee of
socket arrival spacing or achieved throughput.

A late worker schedules the next grant from the actual current time. It does not
accumulate or consume expired permits in a burst. The selected at-most-32 workers are
the only pending callers; there is no separate unbounded producer queue. Existing
five-operation batch allocation still determines unique keys and exact operation mix.
Consecutive grants need not follow allocation-index order. Slow configurations remain
slower. The driver continues offering work until both count and duration requirements
hold, subject to the unchanged request cap; it does not stop early and sleep out the
duration. Finish already allocated batches before the normal outcome audit.

Record actual dispatch offsets/gaps/count, achieved admission rate, pacing wait and
scheduled-to-actual lateness separately from actual HTTP-call success/refusal latency.
Validate the admission-spacing invariant using recorded monotonic grant times.
Do not count pacing delay as provider acquisition time or count a rejected request as
a successful mutation. Retain pool waiter occupancy and actual observation intervals
as separate measurements. All other required evidence and failure conditions above
remain in force.

Audit all offered Create keys after the timed workload and feed checks. Successful
responses require exactly one original durable claim, command and Created event with
the exact partition, UUID, hash, content and subsequent authoritative retry result.
First-dispatch receipt recovery is counted separately from first-dispatch fresh success
and explicit business retries. A failed response stays a failed/unresolved response:
classify its quiescent durable disposition as absent, one committed original batch, or
a correctness failure for malformed/extra identities. Reject unoffered durable keys.
Do not silently turn a later audit into an earlier successful response.
