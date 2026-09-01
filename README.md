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
  -> generated Rust service, client plan, and ConnectorServiceFactory
```

The SDK never guesses business logic or redefines ESS-owned meaning. Every runtime gap must select a
closed, versioned SDK obligation provider; unknown, incomplete, wrong-surface, unused, and uncovered
selections fail generation. Application repositories contain definitions and generated output, not
handwritten runtime hooks.

## Workspace

- `service-definition`: author-facing runtime annotations referencing ESS semantic names.
- `service-obligations`: closed, versioned SDK implementations and complete-coverage checks.
- `service-runtime-ir`: closed, digest-bound, validated runtime realization contract.
- `service-runtime`: transport-independent authenticated intent and guarded Eventlog execution ports.
- `service-engine`: executes generated realization plans and owns obligation ordering and behavior; deployment injects resource adapters only.
- `service-connectors`: inert service contributions used by generated `ConnectorServiceFactory` implementations; products only register factories and supply deployment policy.
- `service-builder`: transactionally loads `service/1`, compiles ESS/runtime IR, and emits deterministic plans, Rust services, and Connector factories.
- `service-conformance`: proves runtime IR, client plans, and inert Connector descriptors remain one exact operation contract.

## Authority rule

Authentication chooses tenant, authority, user, optional executor, and optional realm before application decoding. Realm never appears in routes or operation arguments. Optional realm absence is represented as `None`; it is not rewritten to `"default"`.

## Development

```bash
task check
```

Apache-2.0.
