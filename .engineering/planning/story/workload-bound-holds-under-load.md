---
format: aep.planning-md/2
id: story:workload-bound-holds-under-load
kind: story
status: implemented
title: The two-process workload bound holds under machine load
relations:
- serves: vision:composable-services
scope:
- confidence: cited
  path: crates/service-host/tests/persistence.rs
revision: 5
---
# The two-process workload bound holds under machine load

## Outcome

`postgres_two_process_sdk_workload_six_configurations` (`crates/service-host/tests/persistence.rs`) meets its
unchanged 3 s quiescent-feed catch-up bound on every run, at the load a shared build machine sees.

## Why

On the SDK 0.6.0 tree (ESS 0.31.0, Entity Runtime 0.23.0, Eventlog 0.4.0) it failed 2 of 10 runs on
2026-09-25: 3.052 s at load 8.81 and 3.267 s at load 7.85, the second one a full `task check`. The 8 green
runs peaked between 0.58 s and 2.77 s at loads 4.75–17.9. The tree from before ER delegation failed the
same way (3.24 s and 3.06 s at load 28–31), so the ER vector did not introduce it.

## Suspected cause (not verified)

The same binary spans 0.36 s to 3.27 s per configuration at similar load, which points at scheduling or
the PostgreSQL container's CPU limit (`--cpus 2`) rather than a code path.

## Reproduce

`cargo test -p service-host --test persistence --no-run`, then
`.ess-evolution/waves/0020-close-c-g/cp3/pair-after.sh <n>` with `PERSISTENCE_EXE` set to that binary.
Each run starts its own loopback TLS PostgreSQL and records load and seconds in `pairs/<n>/summary.txt`.
