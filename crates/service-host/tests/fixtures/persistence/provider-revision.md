# SDK laboratory provider revision addendum

The immutable laboratory profile v2 remains SHA256
`cada7c39636c66585e372a345c111d70ba22643741c28b478c8a3272afa2748b`.
Its workload, resource limits, thresholds, case selection and reporting requirements
are unchanged. Its original provider revision and all failed observations are retained.

Before the next SDK workload run on 2026-09-07, the coordinator supplied published
Eventlog main `081815cdfcbf1c751e7ee91abd81af2ff7d460cb`, whose tree is
`0bd349819a4fe3dbd8ebcf329f43efcd856a1f32`. This replaces the original
`a9c2fb51c4756b1f640dfe2269e1a3d7a8e7deb2` dependency for the rerun. The bounded
provider correction addresses coherent pool observations and cancellation/retirement
capacity accounting found during the SDK proof. SDK assertions and pool budgets are
not altered. All three Eventlog dependencies use the full published revision.

The coordinator verified provider CI run 34083247459 (73 cases, 12 comparative
configurations and 2 restarts) and the downloaded artifact provenance before this
handoff. Those are provider observations; the SDK must independently execute all
required cases and all six configurations under the original profile. No production
capacity or SDK comparative speed gain follows from the provider result.
