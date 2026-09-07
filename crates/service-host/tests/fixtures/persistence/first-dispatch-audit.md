# SDK first-dispatch recovery audit clarification

Recorded before the corrected rerun on 2026-09-07. Profile v2 and its provider revision
addendum remain unchanged. Two retained runs failed the workload assertion that every
first-dispatch Create must return replayed=false. Diagnostic command work-1-true-1790
returned replayed=true after a provider overload; the durable audit found exactly one
claim, one command and one Created event with matching original UUID and 4096-byte
content. The engine's bounded final lookup deliberately recovers this committed result.
A deterministic real-SQLite adapter test injects one post-commit load refusal and verifies
one append, one event, preserved content and an unchanged subsequent retry.

The corrected workload keeps unique offered Create keys, records recovered first-dispatch
results separately from fresh commit responses and scheduled prior-command retries, and
requires a classified bounded provider refusal for a recovered first dispatch. After each
steady interval and its timed feed check, an additional audit checks every accepted Create
against exactly one durable claim, command and original event batch, checks its exact
partition, UUID, original event hash and content bytes, retries its original input, then
proves those retries changed no durable claim, command or event. These audit retries are
verification outside the workload measurement and are never included in the five-way
operation mix, throughput, latency or steady counts. Warmup and steady results are audited.
No production behavior, workload cardinality, resource limit or threshold changes.
