# Traversal, time travel and change sets

Protocol 0.23.0 adds three query capabilities to the engine
([#4](https://github.com/weave-graph/weave-engine/issues/4), A3):

- **`expand`**: the k-hop neighborhood of seed nodes ("how was it related").
- **`as_of`**: read a branch as this replica knew it at a recorded time
  ("what did we know then").
- **`changes`**: the facts asserted, retracted or corrected between two
  states of a branch, as a pull change feed ("what changed").

Every result element is explained by its source facts and their provenance,
and every result states its coverage. The normative definitions are in
[core contract v1](contract/core-v1.md) §5 `time.as_of` and §8
`query.expand` and `query.changes`; the field reference is
`weave_contract::traversal`.

All three are graph expressions, evaluated with `evaluate` or `bind` in
a 0.23.0 program, through `Engine::execute`, `weave-engine run` or the science
`execute` request. The Python client has `Engine.expand`, `Engine.as_of`,
`Engine.changes` and `Engine.explain` and matching expression builders
([tutorial](PYTHON_SCIENCE.md#traversal-as-of-reads-and-change-sets)); the
science JSON forms are in [SCIENCE_INTERFACE.md](SCIENCE_INTERFACE.md#traversal-as-of-reads-and-change-sets).

## Expand

```json
{"version": "0.23.0", "commands": [{"op": "evaluate", "value": {
  "kind": "expand", "graph_id": "fleet", "seeds": ["unit-7"], "hops": 2,
  "direction": "both", "predicates": ["installed_at", "runs"],
  "valid_at": 1760000000000, "as_of": 1759000000000}}]}
```

| Field | Meaning |
|---|---|
| `graph_id`, `branch_id` | The branch to read (`branch_id` defaults to `main`) |
| `revision` | An exact revision. Absent means the branch head, or `as_of` |
| `as_of` | Read the branch as known at this recorded time (Unix ms). Not with `revision` |
| `seeds` | 1 to 1,000 node IDs to start from |
| `hops` | 1 to 16 |
| `direction` | `out` (edges from a node, the default), `in` or `both` |
| `predicates` | Edge predicates to follow; empty follows all |
| `valid_at` | Follow only edges valid at this instant, between nodes valid then |
| `limit` | Most nodes plus edges to return (default 10,000, at most 200,000) |

The result holds every node within `hops` hops of a visible seed along
visible, matching edges, and every such edge that leaves a node fewer than
`hops` hops away. Attachments on returned records come with them (with
`valid_at`, the ones valid then). It is an ordinary graph value: it can be
bound, filtered, joined, unioned or explained like any other, and
`node_origins` and `edge_origins` name the exact stored fact behind each
element.

On a fact revision, each hop reads only the edges at the frontier through the
fact store's subject and object indexes, plus the records those edges need to
be authorized. A snapshot revision is read once and indexed in memory.

## As-of selection

Any query can be read as of a recorded time:

```json
{"kind": "as_of", "unix_millis": 1759000000000,
 "query": {"graph_id": "fleet", "predicate": "located_at", "valid_at": 1758000000000}}
```

`expand` takes the same selection as its `as_of` field, and `changes` as a
bound. `as_of: T` selects the last head this replica recorded at or before `T`
through the recorded-time index, without walking history (the cost is
logarithmic in the number of recorded heads; see the
[benchmark](benchmarks/README.md)). Before the branch existed, `expand` fails
with `E_UNAVAILABLE`, and `changes` treats the state as empty. A time after
the trusted clock fails with `E_HISTORY_TIME`. Valid time and recorded time
stay separate: `valid_at` asks what was true in the world, `as_of` what the
store knew.

`as_of` differs from `recorded_query` in two ways. It returns no observation
witness, and it does not require the reader to see the whole selected
revision: the revision is authorized record by record, so the result equals
the same query pinned to that revision. A reader who cannot see restricted
units gets the visible subset "as known at K". Its coverage is the coverage
of that pinned read: hidden records are absent and never make the result
partial, because a partial flag for hidden records would itself reveal that
they exist. `recorded_query` keeps the whole-revision rule and is the way to
obtain a replayable checkpoint.

## Changes

```json
{"kind": "changes", "graph_id": "fleet",
 "from": {"kind": "as_of", "unix_millis": 1759000000000},
 "to": {"kind": "head"}, "entities": ["UNIT-7"]}
```

Each bound is one of:

- `{"kind": "as_of", "unix_millis": T}`: the branch as known at `T`;
- `{"kind": "cursor", "revision": R}`: an exact revision of the graph;
- `{"kind": "head"}`: the current head (the default `to`).

The result's `changes` field holds the change set:

```json
{"from_revision": "fact:sha256:…", "to_revision": "fact:sha256:…",
 "entries": [
   {"change": "corrected", "kind": "attachment", "id": "fw-7",
    "before": {"kind": "attachment", "...": "..."}, "after": {"...": "..."}},
   {"change": "retracted", "kind": "edge", "id": "at-7", "before": {"...": "..."}}]}
```

A fact visible only in `to` is `asserted`, one visible only in `from` is
`retracted`, and one visible in both with different content is `corrected`.
The change set is the difference of the two states, so a fact asserted and
retracted again between them is not listed. `entities` keeps changes about
those entities: their nodes, edges and assertions touching them, and
attachments on them. `predicates` keeps edges with those predicates and
attachments with those keys. More than `limit` entries (default 10,000) fails
with `E_BUDGET`; narrow the window or the scope.

**Pull feed.** Pass the previous `to_revision` as the next `from`:

```json
{"kind": "changes", "graph_id": "fleet", "from": {"kind": "cursor", "revision": "<to_revision>"}}
```

An empty `entries` list with complete coverage means nothing visible changed.
A change set is a terminal value: no operator takes it as input
(`E_CHANGES_OPERAND`). Views and handler recipes reject both operators.

## Explanations and provenance

Results of 0.22.0 and 0.23.0 programs carry `fact_provenance`: for every
stored fact the result cites, the engine-recorded writer and recorded time
plus the claimed source, method and evidence. For `expand` these are the
facts in `node_origins`, `edge_origins` and `attachment_origins`; for
`changes`, each entry's `before` fact (at `from_revision`) and `after` fact
(at `to_revision`). Join results list the source edges of every joined edge
the same way. `{"kind": "explain", "input": …}` turns an expansion into an
explicit derivation graph. Evidence the reader cannot see is removed and makes
the result partial.

## Coverage

Unknown, partial, denied and empty stay distinguishable:

| Situation | Outcome |
|---|---|
| A visible seed with no matching edge, or no visible change | `complete`, empty |
| A seed that is missing or hidden (not distinguished) | `partial`, `E_SEED_UNAVAILABLE` |
| A reachable record, or a changed fact, whose derivation premises are hidden | `partial`, `E_DERIVATION_UNAVAILABLE` |
| Hidden provenance evidence | `partial`, `E_EVIDENCE_UNAVAILABLE` |
| A missing or denied graph | `E_UNAVAILABLE` |
| A result larger than `limit`, or a read beyond the read budget | `E_BUDGET` (never truncated) |

## Permissions

Both operators authorize every record through the engine's one authorization
path (`readers`, derivation gates, whole-value influence), together with the
records it depends on, so each decision equals the one a whole-revision read
makes. Traversal continues only from visible nodes along visible edges,
`limit` counts only visible output, and coverage reports only denials in the
result's own region. A record whose `readers` exclude the principal is simply
absent. Hidden facts therefore never appear in, and never change, the output:
not the elements, counts, change kinds, explanations, coverage or error
messages. `tests/traversal.rs` checks this with paired stores that differ only
in hidden facts.

Revision identifiers are the one exception. They are content-derived over
every fact, so a hidden write gives a new revision ID, which reveals that
something changed but not what.

## Limits

- The read budget (128 MiB per operation) counts the stored records an
  operator reads, including records the reader cannot see, as every read does.
- A change set between two revisions of one fact line, or between a line and
  the revision it started from, reads only the logged operations between them
  and the records whose visibility depends on them. Any other pair (a snapshot
  revision, or a branch moved to another line) reads both states whole.
- Each read of a fact revision recomputes its ID from its own operations, so
  reading a revision made by one large bulk write costs time in proportion to
  that write.
- A fact lookup by ID is one index probe however often the fact was
  corrected, but selecting the attachments or edges of a node scans every
  version recorded under that node, so a node corrected very many times makes
  its traversal slower.
- The change feed is pull only. Push subscriptions are C2.
