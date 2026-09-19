# Engine workflow and dependency graph

The machine-readable source is [workflow.json](workflow.json). Nodes are acceptance gates, not promises that features are already implemented. Parent orchestration owns publication and cross-project acceptance; the engine agent owns this repository. A blocked gate does not mark the entire project complete.

```mermaid
flowchart TD
  E00["E00 Source recovery + public MIT baseline"] --> E01["E01 Shared contract + golden fixtures"]
  E01 --> E02["E02 Durable temporal graph + metadata"]
  E02 --> E03["E03 Plans + joins + provenance"]
  E02 --> E04["E04 Authorization + restriction propagation"]
  E02 --> E05["E05 Atomic event outbox + delivery"]
  E04 --> E06["E06 Adapter lifecycle + effect broker"]
  E05 --> E06
  E03 --> E07["E07 Incremental living views"]
  E04 --> E07
  E05 --> E07
  E02 --> E08["E08 Capsules + attach/detach"]
  E04 --> E08
  E08 --> E09["E09 Offline branches + peer sync"]
  E06 --> E09
  E03 --> E10["E10 Browser + mobile parity"]
  E09 --> E10
  E04 --> E11["E11 Optional governance + acceptance"]
  E09 --> E11
  E03 --> E12["E12 Spaces + geometry + embeddings"]
  E04 --> E12
  E07 --> E13["E13 Clustering + semantic zoom"]
  E12 --> E13
  E06 --> E14["E14 Recovery + fuzzing + measurements"]
  E10 --> E14
  E11 --> E14
  E13 --> E14
  E14 --> E15["E15 Integrated acceptance + public release"]
  L["Language: compatible compiler + fixtures"] --> E03
  L --> E15
```

## Work item state machine

```mermaid
stateDiagram-v2
  [*] --> proposed
  proposed --> ready: sources and dependencies accepted
  ready --> in_progress: owner claims item
  in_progress --> review: implementation and evidence ready
  review --> passed: acceptance and cross-project checks pass
  review --> in_progress: findings need correction
  in_progress --> blocked: named dependency or decision missing
  blocked --> ready: blocker resolved
  passed --> in_progress: regression or contract change invalidates evidence
```

Work proceeds from the earliest ready gate. Parallel work is allowed only when dependencies and file ownership are clear. Contract changes need both project owners, regenerated fixtures and compatibility evidence. Add implementation issues under gates; preserve requirement IDs in issue titles/descriptions and test evidence. Security or correctness failures reopen affected downstream gates.

## Orchestrator cadence

At each gate, report completed behavior, evidence, failures, next ready work and cross-project dependencies. Parent orchestration coordinates the language output format before engine query implementation and checks publication live. Full completion requires E15, not E02's useful local demo. Early releases must identify their supported subset and remaining requirements without claiming the white-paper scope is finished.
