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

<!-- b10x-docs-operations:start -->
## Public documentation operations

This repository owns the public source and presentation allowlist in `b10x.docs.yaml`. The generated credential-free `.github/workflows/b10x-docs-bundle.yml` passively packages only those declared files for the exact successful `main` commit; it must never run repository code. Atlas selects the latest successful bundle with every other catalog source, and Website plus Docs System own rendering, shared components, search, and feeds. Do not add a standalone docs deployer or put App credentials in this public repository. If Atlas catalogs a former Pages workflow, that file remains repository-owned validation: preserve its bespoke checks while keeping exact read-only permissions, an unconditional pull-request trigger, and no deployment primitives. Project Pages at `/service-sdk/` is only the generated stable redirect façade in `.github/workflows/b10x-docs-pages.yml`; content-only publication never rebuilds it.

From the complete organization workspace, verify the contract with a clean Atlas checkout at the current remote `main`. Set `B10X_ATLAS_CHECKOUT` to a managed Atlas worktree when the primary checkout is dirty or stale; never infer command availability from the primary alone.

```bash
atlas_checkout="${B10X_ATLAS_CHECKOUT:-atlas}"
atlas_head="$(git -C "$atlas_checkout" rev-parse HEAD)"
atlas_main="$(git -C "$atlas_checkout" ls-remote origin refs/heads/main | awk '{print $1}')"
test -z "$(git -C "$atlas_checkout" status --porcelain)"
test "$atlas_head" = "$atlas_main"
cargo run --manifest-path "$atlas_checkout/Cargo.toml" --locked -q -- \
  --store "$atlas_checkout/catalog/store" docs reconcile --workspace . --check
```

Keep internal plans, stories, ADRs, decisions, worklogs, security material, and research out of the public allowlist unless a repository authority explicitly declares them public.
<!-- b10x-docs-operations:end -->
