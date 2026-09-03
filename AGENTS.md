# Service SDK contributor instructions

## Serves

O1, O2, O4, O5, O6

- This repository is a standalone service-construction foundation. It does not belong to the abandoned `platform` repository and must not acquire Platform lineage.
- AEP governs planning and evidence under `.engineering/`; production crates must not depend on AEP.
- ESS owns semantic compilation, `EssIr`, `SynthesisPlan`, structural types, commands, events, views, and target emitters. Consume those contracts; never duplicate their planner or general-purpose emitters here.
- `service-definition` owns service-runtime annotations, `service-runtime-ir` owns their closed validated persisted form, `service-obligations` owns the versioned executable catalog, and `service-engine` owns generated-plan execution over injected resources.
- Authentication supplies tenant, authority, user, optional executor, and optional realm before application input is decoded. Realm is never a URL, query, body, caller-set header, generated-client argument, or Connector operation coordinate. `None` and `Some("default")` are distinct.
- Generated files are rewritten only through `service-builder`. Application repositories are definition-only and must not add handwritten realizations; new reusable behavior belongs in a reviewed, versioned SDK obligation provider.
- Every accepted mutation must flow through authenticated intent, authorization and validation, semantic command decision, guarded Eventlog append, reduction, and the declared projection delivery guarantee.
- Write `README.md` for public SDK adopters: lead with the outcome, the supported source-consumption path, and the smallest verified example. Keep repository internals below the onboarding path.
- This repository is a satellite leaf. It owns its source, gate, tag, and GitHub release, then hands the published commit and release coordinates to an Atlas-based coordinator. Do not mutate Atlas, Website source locks, documentation snapshots, or façade delivery from this leaf.
- Run `task check` before publishing a commit.
