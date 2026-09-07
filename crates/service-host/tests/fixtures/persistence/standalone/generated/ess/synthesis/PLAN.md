<!--
  generated from persistence_http v1
  model digest 6e8c2aa44b4c1cecbd05e6c66fefb0a44d64ece7d9c87bfbc727e9a80d42fe46
  contract digest 3d0faab36c842042317c0500936ab21cc2bf15e3f727183b029fa1f6dfbd2185
  do not edit: regenerate with `ess synthesize`
-->
# Synthesis plan — persistence_http v1

Scope: `component-skeletons`, planned by `ess-synth`. Regenerate with `ess synthesize`.

20 capabilities: **16 generated**, **4 obligations**, **0 refused**. An obligation is yours to implement against its contract; a refusal is a fact about this synthesis scope, not about the specification.

## Generated

| capability | source |
| --- | --- |
| domain type | `persistence_http.document.ContentRef` |
| domain type | `persistence_http.document.Document.State` |
| domain type | `persistence_http.document.Id` |
| domain type | `persistence_http.document.Owner` |
| domain type | `persistence_http.document.Row` |
| domain type | `persistence_http.document.Scopes` |
| entity lifecycle | `persistence_http.document.Document` |
| command contract | `persistence_http.document.Close` |
| command contract | `persistence_http.document.Create` |
| command contract | `persistence_http.document.Revise` |
| event type | `persistence_http.document.Closed` |
| event type | `persistence_http.document.Created` |
| event type | `persistence_http.document.Revised` |
| view type | `persistence_http.document.ById` |
| component port | `persistence-service` |
| component transport | `persistence-service` |

## Obligations — yours to implement

| capability | source | why not generated | contract |
| --- | --- | --- | --- |
| command behaviour | `persistence_http.document.Close` | the contract is declared; the algorithm is not | given `persistence_http.document.Close` input, decide and enact exactly one outcome — `closed` otherwise, takes `close` of `persistence_http.document.Document`, emits `persistence_http.document.Closed` |
| command behaviour | `persistence_http.document.Create` | the contract is declared; the algorithm is not | given `persistence_http.document.Create` input, decide and enact exactly one outcome — `created` otherwise, creates `persistence_http.document.Document`, emits `persistence_http.document.Created` |
| command behaviour | `persistence_http.document.Revise` | the contract is declared; the algorithm is not | given `persistence_http.document.Revise` input, decide and enact exactly one outcome — `revised` otherwise, updates `persistence_http.document.Document`, emits `persistence_http.document.Revised` |
| view query | `persistence_http.document.ById` | how the projection is kept current is a storage decision | a query answering `persistence_http.document.ById` with rows projected from `persistence_http.document.Document` at `read_your_writes` consistency |

## Refused — not represented by this synthesis

| capability | source | stage | why |
| --- | --- | --- | --- |
