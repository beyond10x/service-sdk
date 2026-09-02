# service-sdk

`service-sdk` turns a service definition into a production-service realization around the semantics ESS already owns. It is a standalone foundation repository; it is not part of the abandoned Platform repository.

The pipeline is deliberately one-way:

```text
service/1 package
  -> ESS fragments + service-definition/2 + scenarios + exact SDK lock
  -> compiler-minted EssIr
  -> ESS SynthesisPlan
  -> validated service-runtime-ir/2 + versioned obligation catalog
  -> SDK-executable realization plan
  -> generated Rust service, client plan, service-catalog/1, docs, validated scenarios, and external Connector factories
```

The SDK never guesses business logic or redefines ESS-owned meaning. Every runtime gap must select a
closed, versioned SDK obligation provider; unknown, incomplete, wrong-surface, unused, and uncovered
selections fail generation. Application repositories contain definitions and generated output, not
handwritten runtime hooks.

## Workspace

- `service-definition`: author-facing runtime annotations referencing ESS semantic names.
- `service-obligations`: closed, versioned SDK implementations and complete-coverage checks.
- `service-runtime-ir`: closed, digest-bound, validated runtime realization contract.
- `service-catalog`: exact generated catalog plus a read-only external `ServiceCatalogFactory`; it uses the ordinary Connector composition seam and adds nothing to Connectors.
- `service-runtime`: transport-independent authenticated intent, guarded Eventlog execution, verified delegated-execution provenance, and restart-safe external-effect ports.
- `service-engine`: executes generated realization plans and owns obligation ordering and behavior; deployment injects resource adapters only.
- `service-connectors`: inert service contributions used by generated `ConnectorServiceFactory` implementations; products register factories and inject deployment policy plus authority-fact resolution after authentication.
- `service-builder`: transactionally loads `service/1`, validates scenarios against its generated operation surface, compiles ESS/runtime IR, and emits deterministic plans, executable scenario tests, Rust services, catalogs, standalone docs, and Connector factories.
- `service-conformance`: proves runtime IR, client plans, inert Connector descriptors, and declared scenarios remain one exact operation contract through the generated Connector seam.
- `@b10x/service-console-vue`: reusable generic operation, lifecycle, view and activity UI. Standalone generated docs inject its explicit demo binding; products inject a session-authenticated BFF binding.

## Service console theming

The Vue console inherits an optional semantic CSS contract from its host. A product may set these
custom properties on any ancestor of `ServiceConsole`; missing values retain the console's neutral,
system-aware defaults:

| Token | Purpose |
| --- | --- |
| `--b10x-color-canvas` | Widget canvas |
| `--b10x-color-surface` | Cards and controls |
| `--b10x-color-surface-muted` | Operation navigation and quiet regions |
| `--b10x-color-text` | Primary text |
| `--b10x-color-text-muted` | Secondary text |
| `--b10x-color-border` | Dividers and control borders |
| `--b10x-color-accent` | Header, selected state, and primary action |
| `--b10x-color-on-accent` | Content drawn on the accent |
| `--b10x-color-success` | Successful activity |
| `--b10x-color-warning` | Warning and demo surfaces |
| `--b10x-color-danger` | Failed activity |
| `--b10x-color-focus` | Keyboard focus ring |
| `--b10x-color-code-surface` | Structured output background |
| `--b10x-color-code-text` | Structured output foreground |
| `--b10x-color-overlay` | Reserved for console overlays |
| `--b10x-shadow-panel` | Widget and selected-panel elevation |

The contract is deliberately preference-agnostic: the widget accepts no named theme, reads no host
storage, and inherits native `color-scheme`. Generated standalone documentation therefore needs no
product theme dependency, while composed products can apply one palette to the entire surface.

## Authority rule

Authentication chooses tenant, authority, user, optional executor, and optional realm before application decoding. Realm never appears in routes or operation arguments. Optional realm absence is represented as `None`; it is not rewritten to `"default"`.

An authenticated adapter may additionally attach receiver-verified agent, attempt, delegation,
grant, and grant-revision provenance. These values have no deserializable request representation;
generated services can only receive them from the trusted transport context.

Generated factories accept an authority-fact resolver supplied by the authenticated deployment. This keeps project, team, extension, and capability membership out of service inputs while allowing obligation providers to evaluate those facts. The built-in fallback resolves only the authenticated subject and groups already present in the verified context.

## Paging and external effects

Generated projection queries accept an optional `$page` transport envelope containing an opaque
cursor and a limit from 1 through 1000. Existing unpaged calls retain their array result; paged
calls return visible `items`, `next_cursor`, and an explicit `partial` flag. Aggregate event feeds
are separately bound to a deployment-supplied stream authorizer and reject cursors from another
Eventlog incarnation or aggregate.

External side effects use `service-effect-plan/1`: preview seals normalized input, bindings,
aggregate and resource revisions, downstream authority, grant revision, risk, and consequences
into one digest and stable operation identity. The Eventlog effect journal durably prepares that
plan before dispatch, gives workers bounded claims, and records success, refusal, failure, or an
explicit unknown result. Recovery observes the downstream operation identity before dispatch and
turns transport uncertainty into `unknown` instead of repeating an effect blindly.

## Development

```bash
task check
```

Apache-2.0.
