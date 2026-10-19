# Weave contract 0.23.0: traversal, as-of selection and change sets

Protocol 0.23.0 is the A3 activation of core v1 query operators
([core v1](../core-v1.md) §5 `time.as_of`, §8 `query.expand` and
`query.changes`; [#4](https://github.com/weave-graph/weave-engine/issues/4)).
Every addition is a new expression kind or an optional result field omitted
when absent, so existing serialized bytes, content hashes and revision IDs
are unchanged. The store stays at 30. The user guide is
[TRAVERSAL.md](../../TRAVERSAL.md).

## Additions

- **`expand` expression** (`weave_contract::ExpandPlan`). `{kind: "expand",
  graph_id, branch_id, revision?, as_of?, seeds, hops, direction, predicates?,
  valid_at?, limit}` returns the exact k-hop neighborhood of the visible seed
  nodes: every node within `hops` hops along visible edges that match
  `predicates` and `valid_at`, and every such edge that leaves a node fewer
  than `hops` hops away. `direction` is `out` (default), `in` or `both`.
  `hops` is 1 to 16, `seeds` 1 to 1,000, `limit` (nodes plus edges, default
  10,000) at most 200,000. The value is an ordinary graph value with origins,
  so it composes with every other operator and `fact_provenance` explains it.
- **`as_of` expression** `{kind: "as_of", query: QueryPlan, unix_millis}`,
  and the same selection as the `as_of` field of `expand`: read the branch as
  this replica knew it at a recorded time, selected through the recorded-time
  index. It needs no whole-graph visibility and returns no observation
  witness; the selected revision is authorized record by record, so the result
  equals the same query pinned to that revision. A reader who cannot see every
  record gets the visible subset. `recorded_query` keeps its whole-revision
  rule and witnesses.
- **`changes` expression** (`weave_contract::ChangesPlan`). `{kind: "changes",
  graph_id, branch_id, from, to, entities?, predicates?, limit}` returns the
  facts asserted, retracted or corrected between two authorized states. Each
  bound is `{kind: "as_of", unix_millis}`, `{kind: "cursor", revision}` or
  `{kind: "head"}` (the default `to`). The result is a change set in the new
  optional `QueryResult.changes` field (`ChangeSet { from_revision?,
  to_revision?, entries }`, each `ChangeEntry { change, kind, id, before?,
  after? }`). `to_revision` is the cursor for the next pull. A change set is a
  terminal value: no operator takes one as input (`E_CHANGES_OPERAND`).
- `fact_provenance` lists every `before` and `after` fact of a change set.
- New codes: `E_EXPAND` (no seed or no hop), `E_CHANGES_OPERAND`, and the
  diagnostic `E_SEED_UNAVAILABLE` (a seed is missing or hidden; the two are
  not distinguished). Budgets fail with `E_BUDGET`; `limit` is never a
  truncation.

## Compatibility

- Programs of 0.1.0 through 0.22.0 keep their exact semantics. `expand` and
  `changes` under an older version fail with `E_VERSION`. Results of 0.22.0
  and 0.23.0 programs carry `fact_provenance`.
- The reserved expression kinds `expand` and `changes` are released: they
  are now these operators.
- View and handler artifact protocols advance to 0.23.0 and still accept
  0.22.0 and older artifacts. Views and handler recipes
  reject `expand`, `changes` and `as_of`.

## Limits

- Hidden facts never appear in, and never change, `expand` or `changes`
  output, but revision IDs are content-derived over every fact, so a hidden
  write still gives a new revision ID. The read budget (128 MiB) counts the
  stored records a traversal or change set reads, including ones the reader
  cannot see, as every read does.
- A change set between two revisions of one fact line, or between a line and
  the revision it started from, reads only the logged operations between them
  and the records whose visibility depends on them.
  Any other pair (a snapshot revision, or a branch that moved to another
  line) reads both states whole and is bounded by the read budget.
- Every read of a fact revision recomputes its ID from its own operations
  (ADR-0004), so reading a revision made by one large bulk write costs time in
  proportion to that write.
