# Contributing

Service SDK welcomes focused issues and pull requests that preserve the ESS and runtime ownership
boundaries described in [README.md](README.md).

## Before opening a change

- Discuss new public contracts or obligation semantics in an issue before implementing them.
- Keep business semantics in ESS. Service SDK consumes compiler-minted `EssIr` and
  `SynthesisPlan`; it does not duplicate their planner or emitters.
- Change generated files only through `service-builder` and commit the definition and regenerated
  output together in consuming repositories.
- Do not add credentials, private source, production data, or transcripts to fixtures or issues.

## Verify the change

Install the prerequisites listed in the README, then run the complete gate:

```bash
task check
```

Pull requests should explain the user-visible outcome, name affected contracts, and include tests
for accepted behavior and refusals. Breaking persisted or generated contracts require a new
version; do not rewrite an already released contract in place.

Contributions are licensed under Apache-2.0 as described in [LICENSE](LICENSE).
