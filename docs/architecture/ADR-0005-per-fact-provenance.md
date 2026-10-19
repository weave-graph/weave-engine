# ADR 0005: per-fact provenance

Status: **accepted** on 2026-10-19 (A1, [#2](https://github.com/weave-graph/weave-engine/issues/2)).
The records are defined in [core contract v1](../contract/core-v1.md#6-provenance)
and in `crates/weave-contract/src/provenance.rs`. They are activated on the
wire by the A2 protocol bump ([#3](https://github.com/weave-graph/weave-engine/issues/3)).
Until then, no protocol 0.21.0 `Program` can carry them.

## Context

Weave records the writer per commit (`events.actor`), not per fact.
`Assertion.source` and `Edge.assertion_source` are labels the client claims.
There is no per-fact method, model version or evidence link
([audit](../AUDIT_2026-10-17.md), gap 6). Goal G2 needs every fact to say who
or what asserted it, from which evidence, when and how. G3 needs retrieval
results that cite their sources.

Two facts make this harder than adding a field:

- Revision IDs are content-derived (ADR-0001). If the recording principal or
  recording time were part of a fact's content, the same immutable content
  recorded on two replicas, or recorded again after a no-op, would get
  different identities.
- Clients must never be able to forge the writer or the recorded time
  (VISION principle; A2 acceptance criterion).

## Decision

Provenance has two parts with different trust.

1. **Engine-recorded part.** It holds `writer`, the principal from the trusted
   host context, and `recorded_at_ms`, the trusted operation clock in Unix
   milliseconds. The engine sets both inside the write transaction. They are
   not part of fact content and do not enter content hashes. They belong to
   this replica's recording of the fact, like the recorded head observations
   in ADR-0002.
2. **Claimed part (`ClaimedProvenance`).** It holds an optional `source`
   label, a required `method` and an optional list of `evidence` references
   (at most 64). The client sends it inline with the fact as the reserved
   field `provenance`. It is part of the fact's content, so a different
   claim is a different fact.

`ClaimedProvenance` has no `writer` or `recorded_at_ms` field and denies
unknown fields. A client-supplied writer therefore fails to decode with
`E_UNKNOWN_FIELD`. Fixture: `negative/claimed-provenance-forged-writer.json`.

`Method` is a tagged value:

| `kind` | Fields | Meaning |
|---|---|---|
| `human` | none | Entered or confirmed by a person |
| `import` | `importer`, `version` | Loaded by a named importer |
| `rule` | `rule_id`, `revision` | Derived by a revisioned rule set |
| `model` | `model`, `version` | Produced by a model, for example extraction or embedding |
| `unspecified` | none | Not stated |

If a write omits `provenance`, the engine records `method: unspecified`. It
never guesses `human`. Unknown is not false.

`EvidenceRef` is a tagged reference: `assertion` (graph, revision,
assertion ID), `node` (graph, revision, node ID), `graph` (graph, revision)
or `artifact` (URI and digest). Evidence is descriptive. It neither grants
nor restricts authority. Restriction-carrying dependencies remain
`derived_from`, `derived_nodes` and `derived_snapshots`. B2 (ADR-0008) may
decide that cited evidence also implies restriction propagation.

**Reads.** A result returns `FactProvenance`: the engine-recorded `writer`
and `recorded_at_ms` merged with the claimed `source`, `method` and
`evidence`. Results carry it in the reserved list
`QueryResult.fact_provenance`, made of `ProvenanceEntry { fact: FactRef,
provenance }`. There is one entry for each stored source fact that
contributes to the result. `FactRef` names an assertion (which also covers
edges and attachments, since they share one ID namespace per revision), a
node or an embedding (ADR-0006). Derived values have no provenance of their
own. They reach their source facts through the existing `edge_origins`,
`node_origins` and `attachment_origins` indexes, whose references key into
`fact_provenance`. A3 `Explain` follows the same path.

**Permissions.** Provenance is visible exactly when its fact is visible. An
evidence reference to a fact the reader cannot see is removed, and the
result's coverage becomes `partial`. The engine never shows a redaction
placeholder that would reveal that hidden evidence exists. A writer
principal is disclosed to every reader of the fact, so a deployment that
must hide writer identity has to restrict the fact.

**Migration.** Facts that exist before A2 receive provenance derived from data
the store already holds. No new data is invented:

- `writer` is the actor of the commit event that first recorded the revision
  containing the fact;
- `recorded_at_ms` is that revision's trusted recording time;
- `source` is the existing `Assertion.source` or `Edge.assertion_source`, if
  any;
- `method` is `unspecified`;
- `evidence` is empty.

Where the store has no recorded actor or time for a revision, the fact's
provenance is reported as unavailable, with partial coverage. The migration
does not fabricate a writer. Existing revision IDs and content bytes are not
changed.

## Consequences

- The A2 write API must take `writer` and `recorded_at_ms` only from host
  authority. `FactProvenance::record` is the single constructor.
- Content hashes and ADR-0001 identities are unaffected. The same content
  recorded twice is one fact with possibly two recordings. A2 (ADR-0004)
  decides whether the first or every recording is listed, and must state its
  choice.
- `fact_provenance` grows results in proportion to their source facts. It is
  bounded by the existing result byte budgets (`E_BUDGET`).
- No signature or attestation is implied. `source` and `method` are claims by
  an authenticated writer, not verified facts about the world.

## Alternatives rejected

- **Writer inside fact content.** This breaks content identity across replicas
  and makes the writer forgeable whenever content is copied.
- **Provenance only per commit (status quo).** It cannot express one batch
  holding facts from several agents or methods, as B4 needs.
- **A free-form `provenance` JSON object.** Untyped claims cannot be validated,
  filtered or explained, and they invite reinterpretation later.
