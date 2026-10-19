# Weave contract 0.22.0: fact writes, node valid time and per-fact provenance

Protocol 0.22.0 is the A2 activation of core v1 fields
([core v1](../core-v1.md) §4 `fact.write`, §5 `time.valid`, §6
`provenance.fact`; [ADR-0004](../../architecture/ADR-0004-fact-log-and-storage-backend.md),
[ADR-0005](../../architecture/ADR-0005-per-fact-provenance.md)). Every
addition is optional and omitted when absent, so existing serialized bytes,
content hashes and revision IDs are unchanged.

## Additions

- **`write` command.** `{op: "write", graph_id, branch_id, expected_head?,
  changes}` asserts, corrects or retracts individual facts (`node`, `edge`,
  `structural_edge`, `assertion`, `attachment`) in one atomic *fact revision*
  `fact:sha256:…`. The result is `committed` or `unchanged`.
- **`Node.valid_time`** (optional `Interval`). Absent means valid at every
  time. `valid_at` removes nodes whose explicit valid time excludes the
  instant, with the edges that need them.
- **`provenance`** (optional `ClaimedProvenance`) on `Node`, `Edge`,
  `StructuralEdge`, `Assertion` and `MetadataAttachment`. It is record
  content. A client cannot supply `writer` or `recorded_at_ms`
  (`E_UNKNOWN_FIELD`).
- **`QueryResult.fact_provenance`** (`ProvenanceEntry[]`), filled for programs
  that declare 0.22.0: one entry per stored source fact the result cites, with
  the engine-recorded writer and recorded time.
- New error codes: `E_FACT_WRITE` (empty write), `E_DEPENDENT_FACT`
  (retraction with live dependents), `E_PROVENANCE_UNAVAILABLE` and
  `E_EVIDENCE_UNAVAILABLE` (result diagnostics), `E_UNSUPPORTED` (capsule
  export of a fact revision).

## Compatibility

- Programs of 0.1.0 through 0.21.0 keep their exact semantics. Using any
  0.22 field or `write` under an older version fails with `E_VERSION`. Older
  programs get no `fact_provenance`; their results differ only in the
  producing engine's `version` field.
- The reserved names `assert`, `retract` and `correct` became change kinds of
  `write`, not command ops. `reader_groups` stays reserved for B1
  ([ADR-0007](../../architecture/ADR-0007-identity-groups-and-tenancy.md)
  section 13).
- Store 30 adds the empty fact store. View and handler artifact protocols
  advance to 0.22.0 and still accept 0.21.0 artifacts.

## Limits

Fact writes inherit the graph header (profile, schema, influence, context
typing) from their parent; changing it takes a snapshot commit. A write holds
at most 100,000 changes. Retention never erases fact revisions, and capsules
cannot carry them. Indexed reads for large fact graphs and the removal of the
per-graph object cap follow in the next A2 part.
