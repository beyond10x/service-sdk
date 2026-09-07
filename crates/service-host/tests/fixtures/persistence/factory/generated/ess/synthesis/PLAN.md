<!--
  generated from persistence_factory v1
  model digest 1f61c237f551ba7dd92f1e22a17ff302e51a365f030c9f9d37a2537432687e82
  contract digest f081c5ffdabfff6b3ae5575185f94353ca8257af37615780abd6a5b7cde7495d
  do not edit: regenerate with `ess synthesize`
-->
# Synthesis plan — persistence_factory v1

Scope: `component-skeletons`, planned by `ess-synth`. Regenerate with `ess synthesize`.

20 capabilities: **16 generated**, **4 obligations**, **0 refused**. An obligation is yours to implement against its contract; a refusal is a fact about this synthesis scope, not about the specification.

## Generated

| capability | source |
| --- | --- |
| domain type | `persistence_factory.document.ContentRef` |
| domain type | `persistence_factory.document.Document.State` |
| domain type | `persistence_factory.document.Id` |
| domain type | `persistence_factory.document.Owner` |
| domain type | `persistence_factory.document.Row` |
| domain type | `persistence_factory.document.Scopes` |
| entity lifecycle | `persistence_factory.document.Document` |
| command contract | `persistence_factory.document.Close` |
| command contract | `persistence_factory.document.Create` |
| command contract | `persistence_factory.document.Revise` |
| event type | `persistence_factory.document.Closed` |
| event type | `persistence_factory.document.Created` |
| event type | `persistence_factory.document.Revised` |
| view type | `persistence_factory.document.ById` |
| component port | `persistence-service` |
| component transport | `persistence-service` |

## Obligations — yours to implement

| capability | source | why not generated | contract |
| --- | --- | --- | --- |
| command behaviour | `persistence_factory.document.Close` | the contract is declared; the algorithm is not | given `persistence_factory.document.Close` input, decide and enact exactly one outcome — `closed` otherwise, takes `close` of `persistence_factory.document.Document`, emits `persistence_factory.document.Closed` |
| command behaviour | `persistence_factory.document.Create` | the contract is declared; the algorithm is not | given `persistence_factory.document.Create` input, decide and enact exactly one outcome — `created` otherwise, creates `persistence_factory.document.Document`, emits `persistence_factory.document.Created` |
| command behaviour | `persistence_factory.document.Revise` | the contract is declared; the algorithm is not | given `persistence_factory.document.Revise` input, decide and enact exactly one outcome — `revised` otherwise, updates `persistence_factory.document.Document`, emits `persistence_factory.document.Revised` |
| view query | `persistence_factory.document.ById` | how the projection is kept current is a storage decision | a query answering `persistence_factory.document.ById` with rows projected from `persistence_factory.document.Document` at `read_your_writes` consistency |

## Refused — not represented by this synthesis

| capability | source | stage | why |
| --- | --- | --- | --- |
