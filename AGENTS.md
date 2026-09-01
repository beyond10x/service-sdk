# Service SDK contributor instructions

## Serves

O1, O2, O4, O5, O6

- This repository is a standalone service-construction foundation. It does not belong to the abandoned `platform` repository and must not acquire Platform lineage.
- AEP governs planning and evidence under `.engineering/`; production crates must not depend on AEP.
- ESS owns semantic compilation, `EssIr`, `SynthesisPlan`, structural types, commands, events, views, and target emitters. Consume those contracts; never duplicate their planner or general-purpose emitters here.
- `service-definition` owns service-runtime annotations, `service-runtime-ir` owns their closed validated persisted form, and `service-runtime` owns transport-independent execution ports.
- Authentication supplies tenant, authority, user, optional executor, and optional realm before application input is decoded. Realm is never a URL, query, body, caller-set header, generated-client argument, or Connector operation coordinate. `None` and `Some("default")` are distinct.
- Generated files are rewritten only through `service-builder`. Handwritten realizations live outside generated trees.
- Every accepted mutation must flow through authenticated intent, authorization and validation, semantic command decision, guarded Eventlog append, reduction, and the declared projection delivery guarantee.
- Run `task check` before publishing a commit.
