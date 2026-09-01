# service-sdk

`service-sdk` turns a service definition into a production-service realization around the semantics ESS already owns. It is a standalone foundation repository; it is not part of the abandoned Platform repository.

The pipeline is deliberately one-way:

```text
ESS source
  -> compiler-minted EssIr
  -> ESS SynthesisPlan and structural Rust workspace
  -> validated service-runtime-ir/1
  -> auth/Eventlog/content/projection realization
  -> client plan and inert Connector contribution
```

The SDK never guesses business logic or re-emits ESS-owned types, commands, events, or views. Every ESS synthesis obligation remains explicit until a handwritten realization closes it.

## Workspace

- `service-definition`: author-facing runtime annotations referencing ESS semantic names.
- `service-runtime-ir`: closed, digest-bound, validated runtime realization contract.
- `service-runtime`: transport-independent authenticated intent and guarded Eventlog execution ports.
- `service-connectors`: inert service contributions used by generated `ConnectorServiceFactory` implementations; products only register factories and supply deployment policy.
- `service-builder`: compiles ESS, consumes ESS synthesis, validates runtime IR, and emits deterministic artifacts.
- `service-conformance`: proves runtime IR, client plans, and inert Connector descriptors remain one exact operation contract.

## Authority rule

Authentication chooses tenant, authority, user, optional executor, and optional realm before application decoding. Realm never appears in routes or operation arguments. Optional realm absence is represented as `None`; it is not rewritten to `"default"`.

## Development

```bash
task check
```

Apache-2.0.
