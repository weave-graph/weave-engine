# Weave core contract v1

Status: **accepted** on 2026-10-19 (A1,
[#2](https://github.com/weave-graph/weave-engine/issues/2)). Baseline protocol:
**0.21.0**. Current protocol: **1.2.0** (B2); 1.0.0 (A6,
[#16](https://github.com/weave-graph/weave-engine/issues/16)), which freezes
core v1 as published in 0.24.0. Protocol 0.22.0 activated per-fact
provenance, node valid time and fact-level writes (A2,
[#3](https://github.com/weave-graph/weave-engine/issues/3)); 0.23.0 adds
k-hop `expand`, `as_of` selection and `changes` change sets (A3,
[#4](https://github.com/weave-graph/weave-engine/issues/4)); 0.24.0 activates
embedding spaces and facts and adds exact `nearest` and `similar_candidates`
(A4, [#13](https://github.com/weave-graph/weave-engine/issues/13)); 1.1.0
activates `reader_groups` and `FactProvenance.on_behalf_of` (B1,
[#38](https://github.com/weave-graph/weave-engine/issues/38), ADR-0007); 1.2.0
adds write-time propagation rules and `widen_readers` on corrections (B2,
[#5](https://github.com/weave-graph/weave-engine/issues/5), ADR-0008).
Profile name: `core-v1` (`weave_contract::wire::CORE_PROFILE`).

This document is the consolidated, normative specification of the core of the
Weave contract that Phases A–C build on. It replaces reading the 21 additive
version notes ([0.1](v0.1/README.md) to [0.21](v0.21/README.md)) for everything
classified *core v1* in [section 12](#12-classification-of-contract-features-01021).
The serializable types in [`crates/weave-contract`](../../crates/weave-contract/src/lib.rs)
remain the exact field reference. Where this document and a version note
disagree on core semantics, this document governs, and the disagreement is a
bug to report.

Every concept below has an identifier in backticks, such as `time.valid`, and
at least one golden fixture under
[`crates/weave-contract/fixtures/core-v1/`](../../crates/weave-contract/fixtures/core-v1/).
[Section 13](#13-golden-fixtures) describes the fixtures and the harnesses that
check them.

The key words MUST, MUST NOT, SHOULD and MAY are normative.

## 1. Scope and conformance

Core v1 covers identities, facts and polarity, valid and recorded time,
per-fact provenance, embedding facts and spaces, query and selection
operators, the result envelope, error codes and the compatibility policy.

An implementation conforms to core v1 when it:

- decodes every positive fixture and re-encodes it to exactly the fixture's
  `input` value;
- rejects every negative fixture with the stated error code;
- produces every fixture's `engine` outcome when it executes programs (the
  weave-engine runtime does; a compiler that only emits programs does not).

Provenance ([section 6](#6-provenance)) is active on the wire from protocol
0.22.0, and embeddings ([section 7](#7-embeddings)) from protocol 0.24.0.

## 2. Notation

- All wire values are JSON (RFC 8259). Decoders MUST reject duplicate object
  keys and non-finite numbers (`E_DECODE`).
- Times are signed 64-bit integers in Unix-epoch **milliseconds**.
- Identifiers are nonempty UTF-8 strings of at most 512 bytes unless stated
  otherwise. Identifiers are compared byte for byte. They carry no meaning
  beyond identity. Equal labels never imply equal identity.
- Request objects deny unknown fields (`E_UNKNOWN_FIELD`). Tagged unions use
  `op` (commands) or `kind` (expressions, selections and references) as the
  tag. An unknown tag is `E_UNKNOWN_VARIANT`.

## 3. Identities

Weave keeps seven identities distinct. None is derived from another by
convention, and two of them are never merged because of a similarity.

- **`identity.entity`.** `entity_id` names a real-world or conceptual thing,
  such as a unit, a part, a site or a person. An entity has no record of its
  own. It is the shared `entity_id` of its manifestations. Two nodes are the
  same entity only when their `entity_id`s are equal. Similar properties,
  similar vectors or equal names never establish it.
  Fixture: `positive/identity-graph-nodes-edges.json`.
- **`identity.node`.** A node is one *manifestation* of an entity in one
  space. Its fields are `id` (graph-local), `entity_id` and `space_id`. One
  entity can have several nodes, for example `unit-7/physical` and
  `unit-7/digital`. The pair (`entity_id`, `space_id`) is the identity key
  that joins use (0.2). The local `id` is not.
  Fixture: `positive/identity-graph-nodes-edges.json`.
- **`identity.edge`.** An edge is a directed, referenceable relation with its
  own `id`, a `predicate`, `from` and `to` node IDs, a `valid_time` and a
  `polarity`. Edges may form cycles, because their endpoints are stable node
  IDs. In the default (legacy) graph profile, an edge is also its own single
  assertion. Fixture: `positive/identity-graph-nodes-edges.json`.
- **`identity.assertion`.** In the explicit profile (`profile: "explicit"`,
  0.6), a structural edge (`structural_edges`) states that a relationship can
  exist, and separate assertions (`assertions`) claim it. Each assertion has
  its own `id`, `edge_id`, claimed `source`, `valid_time`, `polarity`, and
  optionally `context` and `readers`. A structural edge with no assertion
  makes no claim. Several sources can assert or refute the same structural
  edge independently. Edge, assertion, attachment and embedding IDs share one
  disjoint namespace per revision. Fixture:
  `positive/identity-assertions-polarity.json`.
- **`identity.graph`.** `graph_id` names a graph, the unit of commit,
  branching and authorization. A branch (`branch_id`, default `main`) is a
  movable head within a graph. Graphs are referenced by `GraphRef {
  graph_id, revision }`. Fixture: `positive/identity-graph-nodes-edges.json`.
- **`identity.revision`.** A revision is an immutable snapshot of a graph
  branch. Its ID (`sha256:…`) is derived from the canonical serialized graph
  content plus its graph, branch and parent context (ADR-0001). The same
  content in the same context always has the same revision ID. A no-op write
  returns `unchanged` and creates no revision. Fixtures:
  `positive/correction-new-revision.json` (golden revision IDs).
  A *fact revision* (0.22, [ADR-0004](../architecture/ADR-0004-fact-log-and-storage-backend.md))
  has an ID `fact:sha256:…` derived from the graph, branch, parent revision
  and the write's effective operations sorted by fact key. Writer, recorded
  time, change order and concurrency expectations are not content. Fixture:
  `positive/fact-write-assert-correct-retract.json` (golden IDs).
- Replica identity (the `observer` in recorded selections) is the seventh.
  It is covered by `time.recorded`.

## 4. Facts

A **fact** is a node, an edge, an explicit assertion, a metadata attachment or
(from 0.24.0) an embedding space or an embedding.

- **`fact.polarity`.** Every edge or assertion has `polarity`, either
  `positive` (the default) or `negative`. A negative claim is explicit evidence
  that the relation does not hold during its valid time. It is not the absence
  of evidence. There is no closed-world negation: a missing fact is unknown,
  not false. Positive and negative claims on one structural edge coexist and
  are both returned. Operators that adjudicate them (for example `support`)
  report `conflicted` rather than choosing. Fixture:
  `positive/identity-assertions-polarity.json`.
- **`fact.immutability`.** Facts are immutable. A correction is a new
  revision, so the old revision remains readable by its ID with byte-identical
  content. Writes use optimistic concurrency. `expected_head: null` requires a
  new branch, and a string MUST equal the current head revision, otherwise the
  write fails with `E_CONFLICT` and nothing is written. Fixtures:
  `positive/correction-new-revision.json`,
  `negative/stale-expected-head.json`,
`negative/expand-version-gated.json`,
`negative/expand-unknown-field.json`,
`negative/changes-unknown-bound.json`.
- **`fact.write`.** The `write` command (0.22) changes individual facts of one
  graph branch in one atomic fact revision: `{op: "write", graph_id,
  branch_id, expected_head?, changes}`. Each change is
  `{change: "assert", fact}` (the fact must not be live; re-asserting the
  identical live version is a no-op), `{change: "correct", fact, expected?}`
  (replace the live version of the same kind and ID) or
  `{change: "retract", target: {kind, id}, expected?}`. `fact` is a record
  tagged by `kind`: `node`, `edge`, `structural_edge`, `assertion` or
  `attachment`, and from 0.24.0 `embedding_space` or `embedding`
  ([section 7](#7-embeddings)). A key occurs at most once per write. A correction cannot
  change structural identity (node `entity_id`/`space_id`, edge
  `from`/`to`/`predicate`, assertion `edge_id`, attachment `host`/`key`,
  embedding `space`/`subject`; a space is immutable and cannot be retracted). A
  retraction fails with `E_DEPENDENT_FACT` while live facts still need the
  target (edges of a node, attachments on a host). `expected_head` absent
  means no head check, `null` requires a new branch, a string must equal the
  head. `expected` is `{kind: "absent"}` or `{kind: "unchanged_since",
  revision}`. Any mismatch fails the whole write with `E_CONFLICT`. Writes to
  `weave:` graphs fail with `E_RESERVED_NAMESPACE`. At most 100,000 changes
  per write (`E_BUDGET`). The result is `committed` or `unchanged`. Snapshot
  commits cannot carry embedding spaces or embeddings (`E_EMBEDDING`). A live
  version the writer cannot read (ADR-0008 section 4) is absent for
  `correct`, `retract` and `expected`, which fail exactly as for a missing
  fact; an `assert` of its ID fails with `E_CONFLICT` "fact ID is not
  available" whatever its kind or content. In a store with a principal
  registry, a reference to a record the writer cannot read (an edge
  endpoint, an attachment host, an embedding subject) fails exactly as a
  reference to a missing record. Retracting a record that a live record
  depends on fails with `E_DEPENDENT_FACT` whether or not the writer can read
  the dependent. These rules apply to programs of every version. Fixtures:
  `positive/fact-write-assert-correct-retract.json`,
  `negative/write-forged-writer.json`,
  `negative/embedding-snapshot-commit.json`,
  `negative/hidden-fact-id-unavailable.json`.
- **`fact.readers`.** Every fact has `readers`. An empty list means public.
  A nonempty list is a disjunction of principal IDs. An edge is visible only
  when its own readers and both endpoints' readers admit the principal. The
  principal comes from the trusted host, never from the program. Restrictions
  carry through derivation. `derived_from` (assertions), `derived_nodes`,
  `derived_snapshots` and `GraphData.influence` are conservative AND gates that
  every evaluation re-checks under current authority. OR alternatives
  (`derivations`) keep a derived value when at least one complete alternative
  is visible. Hidden input MUST NOT change visible output: not results, counts,
  ranks, explanations or error messages. Fixtures:
  `positive/readers-hide-restricted.json`,
  `positive/readers-allow-listed.json`.
- **`fact.reader_groups`.** From 1.1.0, every fact record except an
  embedding space has `reader_groups` (ADR-0007 section 2), a list of group
  IDs omitted from encodings when empty, so records without groups keep
  their bytes, content hashes and revision IDs. It is a disjunction
  alongside `readers`: a record is public only when **both** lists are
  empty, and otherwise visible when some `readers` entry is the request's
  principal or some `reader_groups` entry is a group the principal is in,
  transitively, under the store's principal registry **as it is when the
  request runs**. Membership is never resolved at write time, so removing a
  member hides the group's records on the member's next request without
  rewriting anything, and history (pinned revisions, `as_of`, `changes`) is
  read under current membership. A group ID inside `readers` is a literal
  that matches no one. A write that names a group must name an active group
  the writer can discover (a member, a group administrator or a tenant
  administrator); any other value, including every group of a store without
  a registry, fails with `E_UNAVAILABLE`, the code of a nonexistent group.
  A record derived from several records keeps an AND of its inputs' reader
  clauses and never flattens groups into literal readers. Under programs
  older than 1.1.0 the field fails with `E_VERSION`. Fixtures:
  `positive/reader-groups-omitted-when-empty.json`,
  `negative/reader-groups-unknown-group.json`,
  `negative/reader-groups-version-gated.json`.
- **`fact.propagation`.** From 1.2.0 (ADR-0008 section 3), writes keep
  restrictions from widening:
  - A `correct` whose `readers` and `reader_groups` admit anyone the live
    version's do not (it is public where the old version was not, or it
    names a principal or group the old version does not) fails with
    `E_READERS_WIDENED`, unless the change sets `widen_readers: true`. The
    field is omitted when false. Narrowing needs no flag.
  - A fact whose `provenance.evidence` cites a restricted record the writer
    can see must either carry that record as an influence carrier
    (`derived_from`, `derived_nodes` or `derived_snapshots`) or have its own
    clause within the record's clause (it is restricted, and its `readers`
    and `reader_groups` are subsets of the record's). Otherwise the write
    fails with `E_DERIVED_READERS`. Evidence the writer cannot see is not
    consulted.
  Both checks compare lists and never resolve group membership. Programs
  older than 1.2.0 keep their published meaning: neither check applies, and
  `widen_readers` fails with `E_VERSION`. Fixtures:
  `negative/readers-widened.json`,
  `positive/readers-widened-explicitly.json`,
  `negative/widen-readers-version-gated.json`,
  `negative/derived-readers-broader.json`.

## 5. Time

Valid time and recorded time are separate axes. Neither is inferred from the
other.

- **`time.valid`.** `valid_time: Interval { start, end }` states when a fact is
  true in the world. It is half-open, `[start, end)`. `end: null` is unbounded,
  and `start < end` is required (`E_INTERVAL` otherwise). `valid_at = t`
  selects facts with `start <= t < end`. Edges, assertions, attachments and
  embeddings have valid time. Nodes have an optional `valid_time` (0.22);
  absent means valid at every time. With `valid_at = t`, a node whose
  `valid_time` excludes `t` is removed with the edges that need it, and with
  `valid_at` as the only filter a node whose explicit `valid_time` contains
  `t` is kept even without an edge. Fixture: `positive/node-valid-time.json`.
  Fixtures: `positive/valid-time-half-open.json`,
  `negative/empty-valid-interval.json`, `negative/wrong-json-type.json`.
- **`time.recorded`.** Recorded time is when this replica learned a fact. The
  engine's trusted clock assigns it, inside the write transaction. Clients
  never supply authoritative recorded time. `recorded_query { query,
  selection }` reads a graph *as known* at a recorded time.
  `selection: {kind: "local_time", unix_millis}` selects the last head this
  replica recorded at or before that millisecond. A time in the future is
  rejected. `{kind: "checkpoint", observer, checkpoint}` selects an exact
  retained observation. Results carry `recorded_observations` witnesses. The
  witnesses are descriptive and never grant authority (ADR-0002).
  `recorded_range` returns every observation in a half-open recorded-time
  interval. Fixture: `positive/recorded-time-as-known.json`.
- **`time.as_of`.** The expression `{kind: "as_of", query: QueryPlan,
  unix_millis}` (0.23), the field `as_of: unix_millis` on `expand` and the
  change bound `{kind: "as_of", unix_millis}` on `changes` read a branch as this
  replica knew it at a recorded time: the last head recorded at or before that
  millisecond, selected through the `(graph, branch, recorded_at_ms)` index in
  O(log n) of the history length, never by walking history. Before the
  branch's first recorded head the state is empty; before a migration
  baseline it is unknown (`E_HISTORY_UNAVAILABLE`); a time in the future is
  rejected (`E_HISTORY_TIME`). The branch must exist and its head must be
  readable (`E_UNAVAILABLE`). Unlike `recorded_query`, `as_of` returns no
  observation witness and does not require the selected revision to be
  wholly visible: it is authorized record by record, exactly as a read pinned
  to that revision. The `as_of` expression's result is therefore exactly the
  result of the same `query` pinned to the selected revision: a reader who
  cannot see every record gets the visible subset, with the coverage a pinned
  read has, and nothing that reveals the hidden records. `recorded_query` and
  `recorded_range` keep the whole-revision rule because they return
  observation witnesses ([ADR-0002](../architecture/ADR-0002-recorded-head-observations.md)).
  Fixtures: `positive/as-of-partially-visible.json`,
  `positive/expand-valid-time-as-of.json`,
  `positive/changes-between-times.json`.

## 6. Provenance

Defined by [ADR-0005](../architecture/ADR-0005-per-fact-provenance.md)
(accepted). Records: `weave_contract::provenance`. Active on the wire from
protocol 0.22.0.

- **`provenance.fact`.** Every stored fact has a `FactProvenance`:
  - `writer` and `recorded_at_ms`, which the engine records from host
    authority and its trusted clock. They are not part of fact content.
  - `source` (optional claimed label), `method` (`human`, `import`, `rule`,
    `model` or `unspecified`) and `evidence` (at most 64 tagged references:
    `assertion`, `node`, `graph` or `artifact`). The writer claims these, as
    `ClaimedProvenance`, inline in the fact's `provenance` field (on `Node`,
  `Edge`, `StructuralEdge`, `Assertion` and `MetadataAttachment`).

  A client cannot send `writer` or `recorded_at_ms`: `ClaimedProvenance`
  rejects them with `E_UNKNOWN_FIELD`. An omitted claim is recorded as
  `method: unspecified`; without a claim, a record's legacy `source` or
  `assertion_source` label is reported as `source`. A claimed `source` must
  equal that label when both are present (`E_PROVENANCE`). Results of
  0.22.0 programs list provenance in `QueryResult.fact_provenance` as
  `ProvenanceEntry { fact: FactRef, provenance }`, one entry per stored
  source fact the result cites, keyed by its origin references. Facts of
  snapshot revisions report the revision's commit event actor and recorded
  time. A fact whose provenance is unavailable is omitted with
  `E_PROVENANCE_UNAVAILABLE` and partial coverage. Programs of earlier
  versions get no list. Evidence a reader cannot see is removed, and coverage becomes
  `partial`. Invalid provenance labels fail with `E_PROVENANCE`, and too much
  evidence fails with `E_BUDGET`. Fixtures:
  `positive/fact-provenance-model.json`,
  `positive/claimed-provenance-import.json`,
  `positive/provenance-entry-unspecified.json`,
  `negative/claimed-provenance-forged-writer.json`,
  `positive/fact-provenance-in-results.json`,
  `negative/node-provenance-version-gated.json`,
  `negative/write-forged-writer.json`.

  From 1.1.0, `FactProvenance` also has `on_behalf_of` (ADR-0007 section 8):
  for a fact written by an agent principal, the ownership chain from the
  agent's owner to the root user at write time, at most 8 principal IDs.
  Like `writer`, it is engine-recorded, outside content, never part of a
  revision ID, and disclosed to the fact's readers. It is omitted when the
  writer acted for no one, and results of programs older than 1.1.0 never
  carry it. Fixture: `positive/fact-provenance-on-behalf-of.json`.

## 7. Embeddings

Defined by [ADR-0006](../architecture/ADR-0006-embedding-facts-and-spaces.md)
(accepted). Records: `weave_contract::embedding`. Active on the wire from
protocol 0.24.0 and store 31. User guide: [EMBEDDINGS.md](../EMBEDDINGS.md).

- **`embedding.space`.** `EmbeddingSpace { id, revision, encoder,
  encoder_version, preprocessing, dimensions, metric }` is an immutable
  descriptor. `dimensions` is between 1 and 4096. `metric` is
  `cosine_distance` or `euclidean`. `revision` never contains `@`: within a
  graph a space is the fact of kind `embedding_space` with key `id@revision`
  (`SpaceRef::key`), asserted once and never corrected to another descriptor
  or retracted (`E_CONFLICT`). Two spaces are compatible only when they
  are equal in every field. A mismatch fails with `E_SPACE`, even when the
  dimensions are equal. The distance is normative: components are widened to
  binary64 exactly and summed in component order; `euclidean` is
  `sqrt(sum((a_i - b_i)^2))` and `cosine_distance` is
  `1 - dot(a, b) / (sqrt(dot(a, a)) * sqrt(dot(b, b)))` clamped to `[0, 2]`
  (`EmbeddingMetric::distance`). A cosine space rejects the zero vector.
  Fixtures: `positive/embedding-space.json`,
  `negative/embedding-space-encoder-version.json`,
  `negative/nearest-space-mismatch.json`.
- **`embedding.fact`.** `EmbeddingFact { id, space, subject, vector,
  valid_time, readers, derived_from, derived_nodes, derived_snapshots,
  provenance }` is a temporal, permissioned fact of kind `embedding`, written
  only with `write` (`fact.write`) and in the record namespace of the graph.
  `subject` is `entity`, `node` (document chunks included), `edge` or
  `assertion`, and must be live (`E_ENDPOINT`); a subject cannot be retracted
  while an embedding of it is live (`E_DEPENDENT_FACT`). `vector` holds
  finite binary32 components of exactly the space's dimension, and the space
  must be declared in the graph (`E_SPACE` otherwise). An embedding is visible
  when its readers, its subject and its gates admit the reader. Ordinary
  reads (`query`, `expand`, `as_of`, algebra) never return embeddings or
  spaces; `nearest`, `similar_candidates` and `changes` do. Selection by
  valid time, recorded time and permission comes before ranking. Fixtures:
  `positive/embedding-fact.json`,
  `positive/embedding-write-and-nearest.json`,
  `negative/embedding-dimension-mismatch.json`,
  `negative/embedding-write-version-gated.json`.
- **`embedding.candidate`.** `CandidateRelation { id, predicate:
  "weave:similar_to", from, to, space, score, exactness, valid_time, origins }`
  is a similarity-derived *result value* (`QueryResult.candidates`). It is
  never an asserted fact or an identity, and no traversal follows it.
  `score` is a distance (lower is closer). `exactness` is `exact`;
  `approximate` is reserved for C5. `valid_time` is the intersection of both
  embeddings' valid times and `origins` their two `FactRef`s. Its ID is
  derived from the graph, space and both embedding IDs. Persisting one takes
  an explicit write of an ordinary edge whose provenance cites the origins as
  `{kind: "embedding", graph_id, revision, embedding_id}` evidence (0.24).
  Fixtures: `positive/candidate-relation.json`,
  `positive/similar-candidates.json`.

A document chunk is an ordinary node, by convention of type
`weave.DocumentChunk` with properties `artifact_uri`, `artifact_digest`,
`start` and `end` (UTF-8 byte offsets, half-open), linked by `weave:mentions`
edges to the nodes it mentions (`weave_contract::embedding::CHUNK_TYPE` and
related constants). The engine gives it no special meaning.

The untyped science `vector` property keeps its current semantics and is
never reinterpreted as an embedding fact. `Engine::vector_property_changes`
plans an explicit conversion into a declared space (method `import`).

## 8. Queries and selection

- **`query.plan`.** `QueryPlan { graph_id, revision, branch_id, predicate,
  from, to, valid_at, include_metadata, max_depth }` reads one graph.
  `revision: null` resolves the branch head once, at query start. A string
  pins an exact revision. `predicate`, `from`, `to` and `valid_at` are
  conjunctive edge filters. With no edge filter, the result holds every
  visible node. With one, it holds the visible endpoints of the selected
  edges. `include_metadata` expands pinned metadata graphs to `max_depth`
  (default 8), bounded and cycle-safe. Unresolved metadata gives partial
  coverage and never distinguishes "missing" from "denied". Commands are
  `commit`, `commit_batch`, `write` (0.22), `query`, `join`, `bind`,
  `evaluate`, `recorded_range` and `accepted_range`. Fixtures:
  `positive/identity-graph-nodes-edges.json`,
  `positive/valid-time-half-open.json`, `positive/defaults-pinned.json` (pins
  every default: `branch_id: "main"`, `max_depth: 8`, `polarity: "positive"`,
  empty `readers`, null `expected_head`).
- **`query.expression`.** `GraphExpression` values are pure, composable and
  bounded. Evaluation depth is at most 32, a command visits at most 1,000
  expression nodes, and a program has a 32 MiB serialized-result budget
  (`E_BUDGET`). The core operators:

  | `kind` | Meaning |
  |---|---|
  | `query` | Read a graph by `QueryPlan` |
  | `recorded_query` | Read a graph as known at a recorded time (`time.recorded`) |
  | `reference` | Reuse a value bound earlier with `bind` in the same program |
  | `filter` | Keep edges matching `predicate` and/or `valid_at`, with their endpoints |
  | `project` | Keep the listed node and edge IDs, plus the endpoints of kept edges |
  | `union` | Combine two values. The same origin is deduplicated; similar names are never merged |
  | `diff` | Membership added or removed between two values, preserving polarity |
  | `join` | Identity-key path join: `left.to` and `right.from` match on equal (`entity_id`, `space_id`) |
  | `window` | Clip valid intervals to a window |
  | `sequence` | Temporal relations `before`, `meets`, `overlaps` and `within` between matched occurrences |
  | `support` | `supported`, `refuted`, `conflicted` or `unknown` at an instant, from visible evidence |
  | `metadata` | Follow a named metadata attachment |
  | `explain` | Derivation explanation of a value's records |
  | `as_of` | Read a graph as known at a recorded time, record by record (0.23, `time.as_of`) |
  | `expand` | k-hop neighborhood of seed nodes (0.23, `query.expand`) |
  | `changes` | Facts changed between two states of a branch (0.23, `query.changes`) |
  | `nearest` | Exact k nearest embeddings of one space (0.24, `query.nearest`) |
  | `similar_candidates` | Candidate similarity relations (0.24, `query.similar_candidates`) |

  Derived values carry their exact source origins and are readable only by
  the evaluating principal until a release policy exists (B2). Fixture:
  `positive/selection-operators.json`.

- **`query.expand`.** `ExpandPlan { graph_id, branch_id, revision?, as_of?,
  seeds, hops, direction, predicates?, valid_at?, limit }` (0.23) reads one
  revision (pinned, as of a recorded time, or the head) and returns every node
  within `hops` hops of a visible seed along visible edges whose predicate is
  in `predicates` (empty: any) and, with `valid_at`, that are valid at that
  instant between endpoints valid then, plus every such edge that leaves a
  node fewer than `hops` hops away. `direction` is `out` (edges from the node,
  the default), `in` or `both`. Explicit-profile assertions are traversed as
  their materialized edges. The value is an ordinary graph value with exact
  origins, so it composes with every operator, and its attachments are those
  valid at `valid_at` on returned records. Bounds: 1 to 1,000 `seeds` and 1 to
  16 `hops` (`E_EXPAND` for none, `E_BUDGET` beyond), and `limit` (default
  10,000, at most 200,000) on returned nodes plus edges, which fails with
  `E_BUDGET` rather than truncating. A seed that is missing or hidden makes
  the result partial (`E_SEED_UNAVAILABLE`); the two are not distinguished. A
  visible seed with no matching edge is a complete, empty neighborhood.
  Fixtures: `positive/expand-k-hop.json`,
  `positive/expand-valid-time-as-of.json`,
  `negative/expand-version-gated.json`, `negative/expand-unknown-field.json`.
- **`query.changes`.** `ChangesPlan { graph_id, branch_id, from, to,
  entities?, predicates?, limit }` (0.23) compares two authorized states of a
  branch fact by fact. Each bound is `{kind: "as_of", unix_millis}`
  (`time.as_of`), `{kind: "cursor", revision}` (an exact revision of the
  graph, typically an earlier `to_revision`) or `{kind: "head"}` (the default
  `to`). A fact visible only in `to` is `asserted`, one visible only in
  `from` is `retracted`, and one visible in both with different content is
  `corrected`; intermediate versions are not listed. `entities` keeps changes
  about those entities (nodes, edges and assertions touching them, and
  attachments on them) and `predicates` keeps edges with those predicates and
  attachments with those keys. The result is `QueryResult.changes`
  (`ChangeSet { from_revision?, to_revision?, entries }`, each entry
  `ChangeEntry { change, kind, id, before?, after? }` in `(kind, id)` order,
  with the authorized value on each side), and `fact_provenance` lists every
  `before` and `after` fact. `to_revision` continues the pull feed as the next
  `cursor`. At most `limit` entries (default 10,000, at most 200,000;
  `E_BUDGET` beyond). A change set is terminal: no operator takes one as
  input (`E_CHANGES_OPERAND`). Fixtures:
  `positive/changes-between-times.json`,
  `positive/changes-result-envelope.json`,
  `negative/changes-unknown-bound.json`.

Both A3 operators authorize every record through the engine's one
authorization path, together with the records it depends on, which gives the
same decision as authorizing the whole revision. Traversal continues only
from visible nodes along visible edges, `limit` counts visible output, and
coverage becomes partial only for denials on records in the output's own
region. Hidden facts therefore never appear in and never change their output.
Revision identifiers are the exception: they are content-derived over every
fact (ADR-0001, ADR-0004), so a hidden write gives a new revision ID.

- **`query.nearest`.** `NearestPlan { graph_id, branch_id, revision?,
  as_of?, space, query, k, valid_at?, candidates? }` (0.24, `kind:
  "nearest"`) ranks the embeddings of one space by distance to `query`: a
  caller-supplied `{kind: "vector", vector}` in that space, or `{kind:
  "embedding", embedding_id}` of the same revision, which must be visible
  (`E_UNAVAILABLE` otherwise, as when missing) and whose subject is excluded.
  It reads one revision (pinned, `as_of` or the head). Only embeddings of the
  space, valid at `valid_at` when set (the subject's node valid time is not
  re-applied), whose subject is a record of the `candidates` value when set
  (its nodes, their entities and its edges, by origin in `graph_id`), and
  that the reader may see with their subject are ranked, so hidden
  embeddings never change the rank, score or count of visible ones. The
  order is ascending score, then subject (kind, then ID), then embedding ID.
  `k` is 1 to 1,000 (`E_NEAREST` for 0, `E_BUDGET` above). The ranking
  streams through the selected embeddings with O(k) memory and counts them
  against its own scan budget, `MAX_SCAN_COMPONENTS` (2^28 vector components,
  about 174,000 vectors of 1,536 dimensions; `E_BUDGET` beyond), instead of
  the read budget. The value holds
  the hits' subject nodes with their attachments and origins, the hit
  embeddings and their space, `QueryResult.neighbors` (`Neighbor { rank,
  embedding_id, subject, score }` in rank order) and `exactness: exact`. A
  partial candidate value or a denied embedding makes it partial. Another
  operator that takes the value as input takes only its graph records.
  `expand` takes seeds from another value's nodes with `seeds_from` (0.24),
  which composes vector search into traversal. Fixtures:
  `positive/embedding-write-and-nearest.json`,
  `positive/nearest-composition.json`, `negative/nearest-zero-k.json`,
  `negative/nearest-version-gated.json`.
- **`query.similar_candidates`.** `CandidatesPlan { graph_id, branch_id,
  revision?, as_of?, space, k, max_score?, valid_at?, sources?, targets?,
  exclude_linked, limit }` (0.24) proposes candidate relations
  (`embedding.candidate`): for every visible source embedding (of the
  `sources` value, or all), its `k` nearest visible target embeddings (of
  `targets`, or all) of another subject with an overlapping valid time, at
  most `max_score` apart, each pair once from the lower subject to the
  higher. `exclude_linked` drops node pairs joined by a visible edge in
  either direction (valid at `valid_at` when set). The result is
  `QueryResult.candidates` in ascending score, `exactness: exact`, the
  endpoint nodes and `fact_provenance` for every origin. At most `limit`
  candidates (default 1,000, at most 100,000) and 50,000,000 comparisons
  (`E_BUDGET`). Fixture: `positive/similar-candidates.json`.

The A4 operators select and authorize embeddings through the same path as
the A3 operators. A denied embedding (its readers admit the reader but a
gate does not) makes the result partial; one whose readers exclude the
reader, or whose subject is hidden, is simply absent.

## 9. Result envelope

- **`result.envelope`.** A `queried` command result holds a `QueryResult`:
  - `version`: the protocol version of the producing engine;
  - `graph`: the authorized value;
  - `snapshots` (legacy map) and `input_snapshots`: the authoritative list of
    every distinct (`graph_id`, `revision`) read;
  - `coverage`: `complete` or `partial`;
  - `diagnostics`: `{code, message}` entries;
  - `provenance`: source assertion references;
  - `edge_origins`, `node_origins` and `attachment_origins`: exact source
    references for each returned record;
  - `metadata_graphs`;
  - optional `recorded_observations`, `accepted_observations`,
    `selected_context`, `source_revisions`, `fact_provenance` (0.22.0),
    `changes` (0.23, `query.changes`), and `neighbors`, `candidates` and
    `exactness` (0.24, `query.nearest`, `query.similar_candidates`), each
    omitted when empty;
  - `graph.embedding_spaces` and `graph.embeddings` (0.24), present only in
    `nearest` values.

  Unknown, partial, denied and empty stay distinguishable. When any selected
  input is missing, hidden or unresolved, the result has `coverage: partial`
  with a generic diagnostic. A result MUST NOT be an empty `complete` result
  for unavailable input. `complete` describes the authorized input, not the
  world. Fixtures: `positive/result-envelope.json`,
  `positive/nearest-result-envelope.json`.

Results are produced by the engine. Their consumers MUST tolerate new
optional fields, which are added only under the additive rules of
[section 11](#11-compatibility-policy). Consumers MUST reject unknown enum
values (such as an unknown `coverage`) instead of guessing their meaning.

## 10. Error codes

Errors are `{code, message}`. **Codes are stable; messages are not.**
Clients match on `code`. A code's meaning never changes. New codes may be
added in any minor version.

### 10.1 Decoding codes (`weave_contract::wire`)

`decode_program` and `decode` check these, in this order:

| Code | Concept | When |
|---|---|---|
| `E_BUDGET` | | Encoded input exceeds 16 MiB |
| `E_DECODE` | `error.decode` | Malformed JSON, a duplicate key, a non-finite number, a wrong JSON type, a missing required field, or a bounded-decoder limit |
| `E_VERSION` | `error.version` | `version` is missing, not a string, or not in `SUPPORTED_VERSIONS`. It is checked **before** the structure, so a newer program is never partially decoded |
| `E_UNKNOWN_FIELD` | `error.unknown_field` | An object has a field its declared type does not define (reserved fields included) |
| `E_UNKNOWN_VARIANT` | `error.unknown_variant` | A tag (`op`, `kind` or an enum string) names no defined variant |

Fixtures: `negative/unknown-version.json`, `negative/missing-version.json`,
`negative/unknown-version-checked-first.json`,
`negative/unknown-field-program.json`, `negative/unknown-command.json`,
`negative/malformed-json.json`, `negative/duplicate-key.json`,
`negative/wrong-json-type.json`.

The `weave-engine run` CLI reports these codes. The science request envelope
(`weave-science`, API 0.1.0) keeps its own `E_INPUT` code for all request
decoding failures, and the C ABI keeps `E_NATIVE_INPUT`.

### 10.2 Core runtime codes

| Code | Meaning |
|---|---|
| `E_VERSION` | A feature is newer than the program's declared version, for example `join` in 0.1.0 (fixture: `negative/legacy-version-feature-gate.json`) |
| `E_CONFLICT` | `expected_head` differs from the branch head, a descriptor conflicts with one already registered, or a write asserts an ID a live fact holds. When the writer cannot read that fact, the message is always "fact ID is not available" (fixture: `negative/hidden-fact-id-unavailable.json`) |
| `E_READERS_WIDENED` | A 1.2.0 correction makes a fact readable by more principals without `widen_readers` (fixture: `negative/readers-widened.json`) |
| `E_DERIVED_READERS` | A 1.2.0 fact cites restricted evidence and is readable by principals that evidence is not (fixture: `negative/derived-readers-broader.json`) |
| `E_DEPENDENT_FACT` | A retraction leaves a live dependent fact, visible or not |
| `E_INTERVAL` | A valid-time interval is empty or inverted |
| `E_ID` | An identifier is empty, too long or otherwise invalid |
| `E_FORBIDDEN` | The host has not granted the required write authority. In a store with a principal registry, also: the principal can discover the named graph but lacks the permission the operation needs |
| `E_UNAVAILABLE` | A requested object is unavailable to this principal. "Missing" and "denied" are not distinguished. In a store with a principal registry, a graph the principal cannot discover gives exactly this error, as a nonexistent graph does |
| `E_BUDGET` | A size, depth, work or read budget is exhausted. Never a truncated complete result |
| `E_INTEGRITY` | Stored data failed an integrity check |
| `E_HISTORY_SELECTOR`, `E_HISTORY_UNAVAILABLE` | An invalid recorded or accepted selector, or a selected observation that is unavailable |
| `E_CONTEXT_REQUIRED` | A contextual claim was consumed without matching context selection |
| `E_RESERVED_NAMESPACE` | A write targets an engine-reserved graph or descriptor (for example identity or governance storage) |
| `E_PROVENANCE` | Invalid provenance, or a missing origin (ADR-0005) |
| `E_SPACE` | An invalid or mismatched embedding space or vector, or a space not declared in the graph (ADR-0006) |
| `E_NEAREST` | A `nearest` or `similar_candidates` with `k` or `limit` 0, or an invalid `max_score` (0.24) |
| `E_EMBEDDING` | A snapshot commit that carries embedding spaces or embeddings (0.24) |
| `E_EXPAND` | An `expand` with no seed or no hop (0.23) |
| `E_CHANGES_OPERAND` | A change set used as the input of another operator (0.23) |
| `E_STORAGE`, `E_FORMAT` | Internal storage or serialization failure |

Extension features define further codes in their version notes. Those codes
are stable under the same rule.

## 11. Compatibility policy

### 11.1 Version identifiers

These versions identify different boundaries. None advances another
automatically:

| Identifier | Current | Where |
|---|---|---|
| Protocol (`Program.version`, `QueryResult.version`) | 1.2.0 | `weave_contract::VERSION`; every accepted version in `weave_contract::versions::SUPPORTED_VERSIONS` |
| Core profile | `core-v1` | This document |
| Contract crate | 1.2.0, always equal to the protocol | `crates/weave-contract/Cargo.toml` |
| Store schema | 32 | `STORAGE_VERSION` |
| Capsule format | 0.4 | [CAPSULES.md](../CAPSULES.md) |
| Science API | 0.1.0 | [SCIENCE_INTERFACE.md](../SCIENCE_INTERFACE.md) |
| Artifact formats | `weave-view-registration/1`, `weave-handler-registration/1` | 0.16 and 0.18 notes |

### 11.2 Semantic versioning of the protocol

- **`compat.versions`.** A program declares exactly one protocol version.
  There is no range negotiation. The engine accepts the closed set
  `SUPPORTED_VERSIONS` (0.1.0 through 0.24.0, then 1.0.0, 1.1.0 and 1.2.0) and executes each
  with the exact semantics it was published with. A feature used under an
  older version fails with `E_VERSION` before any write. Fixtures:
  `positive/legacy-version-accepted.json`,
  `negative/legacy-version-feature-gate.json`,
  `positive/protocol-1-0-0.json`, `negative/unknown-version.json` (1.3.0)
  and `negative/unknown-version-checked-first.json` (2.0.0).
- **Feature gates.** `weave_contract::versions` holds the one ordered
  version table. Every gate names the version that introduced its feature and
  asks `version_at_least(program.version, introduced)`, so a new version
  gets every earlier feature and changes no gate. A gate that must match one
  exact version (a stored program pinned for its digest) compares that
  version explicitly and documents why.
- **0.x (until 0.24.0).** Every wire change bumped MINOR (0.22.0, 0.23.0
  and so on), and each change was additive (section 11.3). PATCH is not used
  on the wire: wire versions are always `X.Y.0`. Clarifications that change
  no bytes need no bump.
- **1.0.0 (A6, Phase A release).** 1.0.0 freezes core v1 with its activated
  provenance, traversal and embedding fields. It adds no wire feature: a
  1.0.0 program has exactly the feature set of 0.24.0, and 0.24.0 programs
  stay accepted with unchanged meaning. Results report the producing
  engine's version, 1.0.0. From 1.0.0 on, MINOR (1.1.0) means additive under
  section 11.3, and MAJOR (2.0.0) means breaking under section 11.4.
- **1.1.0 (B1).** The first additive minor version: `reader_groups` on fact
  records and `FactProvenance.on_behalf_of`, both omitted when empty
  ([note](v1.1/README.md)). Stored bytes, content hashes and revision IDs of
  records without groups are unchanged, and 1.0.0 programs keep their exact
  meaning.
- **1.2.0 (B2).** `widen_readers` on corrections, omitted when false, and
  the write rules of `fact.propagation` for 1.2.0 programs
  ([note](v1.2/README.md)). Nothing stored changes. The write-path rule for
  live versions the writer cannot read (`fact.write`) is authorization and
  applies to every version (ADR-0008 section 4).
- **Core v1 activation.** A2 and A4 activate the reserved core-v1 fields
  through additive minor bumps. Each implementing PR records its exact version
  number. A1 itself changes no wire bytes, so the protocol stays 0.21.0.
  A2 activated provenance, `Node.valid_time` and `write` as **0.22.0**.
  A3 activated `expand`, `as_of`, `changes` and `QueryResult.changes` as
  **0.23.0**, with no store change. A4 activated the embedding fields,
  fact kinds, `nearest`, `similar_candidates`, `ExpandPlan.seeds_from`,
  embedding evidence and the result fields `neighbors`, `candidates` and
  `exactness` as **0.24.0**, with store 31.

### 11.3 What a minor version may add

- A new **optional request field** whose absence means exactly the previous
  behaviour and which is omitted from encodings when absent or empty. Existing
  serialized bytes, content hashes and revision IDs therefore do not change.
- A new command `op`, expression `kind`, selection or enum variant in a
  request type, rejected under older versions with `E_VERSION`.
- A new optional **result** field, omitted when empty, or a new result enum
  value. The result's `version` signals it.
- A new error code or diagnostic code.

### 11.4 What requires a major version, an ADR and a migration

Removing or renaming a field or variant. Changing a default, the meaning of a
field, the meaning of an error code, or the canonical encoding of an existing
value. Rejecting input that a published version accepted, unless a security
fix is documented in an ADR and gated on the new version. These changes MUST
NOT be made silently. Saved programs and stored data keep their original
meaning.

### 11.5 Store and contract versions

- A store schema change bumps `STORAGE_VERSION` with a crash-tested migration
  and an old-binary refusal test. It does not change the protocol.
- A protocol change does not by itself change the store. When it needs new
  stored state, both bump in the same PR.
- **Saved plans keep their version.** Stored programs, view and handler
  artifacts, and cached results keep their declared protocol version and are
  evaluated under that version's semantics. A migration MUST NOT rewrite a
  saved plan's version or reinterpret its fields. If a version is ever
  removed, affected saved plans fail with `E_VERSION`; they are never run
  under a different meaning.

### 11.6 Reserved names

- **`compat.reserved`.** The following names are reserved for core-v1
  activation. The current protocol rejects them (`E_UNKNOWN_FIELD` or
  `E_UNKNOWN_VARIANT`), and no extension may use them for anything else:

  | Name | Kind | Owner |
  |---|---|---|
  | `weave:` | predicate and ID namespace (`weave:similar_to`, `weave:mentions`, `weave:candidate:`) | engine |

  An epic MAY choose different names, in which case its PR releases the
  reservation here. A2 activated `provenance`, `QueryResult.fact_provenance`
  and `Node.valid_time` in 0.22.0, and released `assert`, `retract` and
  `correct` as command ops: they are change kinds of the `write` command.
  A3 activated the reserved expression kinds `expand` and `changes` in
  0.23.0. A4 activated `GraphData.embedding_spaces`, `GraphData.embeddings`,
  `QueryResult.candidates`, `QueryResult.exactness`, `nearest` and
  `similar_candidates` in 0.24.0; under older versions they fail with
  `E_VERSION` (`negative/embedding-write-version-gated.json`,
  `negative/nearest-version-gated.json`). B1 activated `reader_groups` in
  1.1.0 under the reserved name; under older versions it fails with
  `E_VERSION` (`negative/reader-groups-version-gated.json`).
  Fixture: `negative/unknown-command.json`.

### 11.7 Deprecation process

1. An ADR proposes the deprecation, with its reason and replacement.
2. The feature is marked *deprecated since X.Y.0* in the table in section 12,
   in its version note and in `docs/STATUS.md`. It keeps working, with its
   fixtures and tests, for at least one minor release **and** until the next
   phase release.
3. Removal happens only in a major version, with a store migration that
   refuses (rather than reinterprets) any saved plan or data that uses the
   feature.
4. weave-language is told through its vendor update, and its emitter stops
   producing the feature before removal.

### 11.8 Changing fixtures

Adding fixtures is always allowed. Changing a positive fixture's `input` or a
negative fixture's expected code is a wire change, and section 11.4 applies to
it. Every fixture change updates `fixtures/core-v1/manifest.json` and reaches
weave-language through a vendor update.

## 12. Classification of contract features 0.1–0.21

- *Core v1* is required, specified above and built on by Phases A–C.
- *Extension* is supported, tested and covered by the compatibility policy,
  but optional. Phase A does not build on it, and its own version note
  governs it.
- *Deferred* belongs to an epic deferred by the [roadmap](../ROADMAP.md). It is
  maintained as is and keeps its tests, but receives no new development.

No feature changes semantics because of this classification.

| Version | Feature | Class | Notes |
|---|---|---|---|
| 0.1 | Nodes (entity × space manifestations), directed edges, `GraphData`, `GraphRef`, `AssertionRef`, `Interval` | core v1 | §3, §5 |
| 0.1 | `commit` with `expected_head` CAS, immutable revisions, events | core v1 | §4 `fact.immutability` |
| 0.1 | `query` / `QueryPlan`, readers, metadata references, coverage, diagnostics, provenance | core v1 | §8, §9, `fact.readers` |
| 0.2 | `join` command (identity-key path join), `input_snapshots` | core v1 | §8 |
| 0.3 | `bind`, `evaluate`, `GraphExpression` `query`/`reference`/`join`/`filter`, `edge_origins` | core v1 | §8 |
| 0.4 | Graph schemas, node/edge `type_id` | core v1 | Typed validation of integers, strings and booleans |
| 0.4 | Named metadata attachments, `metadata` expression | core v1 | |
| 0.4 | `commit_batch` and snapshot manifests | core v1 | Atomic multi-graph write |
| 0.5 | `union`, `diff`, `project`, `support` | core v1 | §8 |
| 0.5 | Edge `derivations` (OR of AND premises) | core v1 | §4 `fact.readers` |
| 0.6 | Explicit profile: `structural_edges`, `assertions`, claimed `source` | core v1 | §3 `identity.assertion` |
| 0.7 | `reason` (finite rules) | extension | |
| 0.8 | `context` selection, `context_scope`, attachment `context` | extension | Assertion `context` decodes in core; consuming it needs this extension |
| 0.9 | `explain` | core v1 | §8 |
| 0.9 | `Float` schema properties, node `derived_from` | core v1 | |
| 0.9 | `geometry` (distance, transform, axis projection) | extension | Frozen. Embedding retrieval moves to ADR-0006 |
| 0.10 | `counterparts` (declared identity bridges) | extension | |
| 0.11 | Node `derived_nodes` (node influence references) | core v1 | Restriction carrier |
| 0.12 | `decimal` and `quantity` scalar schemas | extension | |
| 0.13 | `resolve_identity` | extension | Phase B building block |
| 0.13 | `cluster` | deferred | E13 clustering |
| 0.14 | `typed_context`, `GraphData.context_typing` | extension | |
| 0.15 | `GraphData.influence`, `derived_nodes` on edges and assertions, `Derivation.node_premises` | core v1 | Restriction carriers |
| 0.16 | `accepted_graph`, governed reads | extension | Governance; B5 reuses it |
| 0.16 | `current_view`, `CompiledViewTemplate` | extension | Views |
| 0.17 | `derived_snapshots`, `GraphInfluence.snapshots` | core v1 | Restriction carriers |
| 0.18 | Sealed handler artifacts (`CompiledHandlerTemplate`) | deferred | Handlers and actors are peripheral |
| 0.19 | Alternative carriers on `GraphInfluence`, `Node` and attachments; `Derivation.snapshot_premises` | core v1 | |
| 0.19 | `window`, `sequence` | core v1 | §8 |
| 0.20 | `recorded_query`, `RecordedSelection`, `recorded_observations` | core v1 | §5 `time.recorded` |
| 0.21 | `recorded_range`, `HistoryRanged` | core v1 | §5 |
| 0.21 | `accepted_history`, `accepted_range`, `accepted_observations` | extension | Governance |
| 0.22 | `write` (assert, correct, retract), fact revisions, `Node.valid_time`, claimed `provenance`, `QueryResult.fact_provenance` | core v1 | §4 `fact.write`, §5, §6 |
| 0.23 | `expand`, `as_of` (expression and field), `changes`, `QueryResult.changes` | core v1 | §5 `time.as_of`, §8 `query.expand`, `query.changes` |
| 0.24 | Embedding spaces and facts (`write` kinds, `GraphData` fields), `nearest`, `similar_candidates`, `ExpandPlan.seeds_from`, embedding evidence, `QueryResult.neighbors`, `candidates`, `exactness` | core v1 | §7, §8 `query.nearest`, `query.similar_candidates` |
| 1.0 | No wire feature: freezes core v1 with the 0.24 feature set ([note](v1.0/README.md)) | core v1 | §11 `compat.versions` |

Capsules, mounts, dispatch, actors, governed effects and the browser host are
native APIs, not contract versions. Their deferral is recorded in the
[roadmap](../ROADMAP.md#deferred-kept-but-not-scheduled).

## 13. Golden fixtures

### 13.1 Layout and format

Fixtures live in `crates/weave-contract/fixtures/core-v1/` so that they ship
inside the vendored crate. The directory holds only JSON:

- `manifest.json`: `{format: "weave-core-fixtures/1", profile, protocol,
  fixtures: {path: sha256}}`. It pins the exact file set and each file's bytes.
- `positive/*.json` and `negative/*.json`, where each file is one fixture:

```json
{
  "fixture": "file stem",
  "concepts": ["time.valid"],
  "type": "program | graph_data | query_result | fact_provenance | claimed_provenance | provenance_entry | embedding_space | embedding_space_pair | embedding_fact | candidate_relation",
  "expect": {"accept": true} | {"reject": "E_CODE"},
  "space": "embedding_fact only: the space to validate against",
"  "input": "the canonical JSON value, or input_text: raw bytes as a string",
  "canonical": "optional: the exact re-encoding of a minimal input, pinning defaults",
  "engine": {"principal": "...", "write_graphs": [], "clock_ms": 0,
             "setup": [{"principal": "...", "write_graphs": [], "clock_ms": 0, "program": {}}],
             "results": [{"kind": "queried", "coverage": "complete", "nodes": [], "edges": [], "edge_properties": {}, "recorded_observations": 1, "neighbors": [], "candidates": [], "exactness": "exact", "revision": "sha256:…"}],
             "error": "E_CODE"}
}
```

A positive fixture's `input` is canonical: decoding it and re-encoding it MUST
give the same JSON value. That catches renamed fields, changed defaults and
changed encodings. A fixture with `canonical` instead gives a minimal `input`
and the exact value it MUST re-encode to, which pins default values.
`engine` is optional. It describes the observable outcome of
executing the program on a fresh store with a manual clock. Revision IDs in
`engine` results are golden, because they are content-derived.

### 13.2 Harnesses

| Harness | Checks | Runs in |
|---|---|---|
| `crates/weave-contract/tests/core_v1_fixtures.rs` | Manifest file set and hashes; fixture shape; every concept covered; positive canonical round trip; negative codes; dedicated unknown-version, unknown-field and unknown-variant tests | Engine CI, and language CI through the vendored crate |
| `crates/weave-engine/tests/core_v1_fixtures.rs` | Every `engine` expectation (including `nearest` neighbors in rank order and candidate pairs); this document cites every fixture and concept; `weave-engine run` codes; the engine accepts exactly `SUPPORTED_VERSIONS` | Engine CI |

### 13.3 Fixture index

Positive: `positive/identity-graph-nodes-edges.json`,
`positive/defaults-pinned.json`,
`positive/identity-assertions-polarity.json`,
`positive/valid-time-half-open.json`,
`positive/correction-new-revision.json`,
`positive/readers-hide-restricted.json`,
`positive/readers-allow-listed.json`,
`positive/recorded-time-as-known.json`,
`positive/selection-operators.json`,
`positive/legacy-version-accepted.json`,
`positive/result-envelope.json`,
`positive/fact-provenance-model.json`,
`positive/claimed-provenance-import.json`,
`positive/provenance-entry-unspecified.json`,
`positive/fact-write-assert-correct-retract.json`,
`positive/node-valid-time.json`,
`positive/fact-provenance-in-results.json`,
`positive/embedding-space.json`, `positive/embedding-fact.json`,
`positive/candidate-relation.json`, `positive/as-of-partially-visible.json`,
`positive/expand-k-hop.json`,
`positive/expand-valid-time-as-of.json`,
`positive/changes-between-times.json`,
`positive/changes-result-envelope.json`,
`positive/embedding-write-and-nearest.json`,
`positive/nearest-composition.json`,
`positive/similar-candidates.json`,
`positive/nearest-result-envelope.json`,
`positive/reader-groups-omitted-when-empty.json`,
`positive/fact-provenance-on-behalf-of.json`,
`positive/readers-widened-explicitly.json`.

Negative: `negative/unknown-version.json`, `negative/missing-version.json`,
`negative/unknown-version-checked-first.json`,
`negative/unknown-field-program.json`,
`negative/reader-groups-version-gated.json`,
`negative/reader-groups-unknown-group.json`,
`negative/readers-widened.json`,
`negative/widen-readers-version-gated.json`,
`negative/derived-readers-broader.json`,
`negative/hidden-fact-id-unavailable.json`,
`negative/write-forged-writer.json`,
`negative/node-provenance-version-gated.json`,
`negative/embedding-write-version-gated.json`,
`negative/claimed-provenance-forged-writer.json`,
`negative/unknown-command.json`,
`negative/nearest-version-gated.json`,
`negative/nearest-space-mismatch.json`,
`negative/nearest-zero-k.json`,
`negative/embedding-snapshot-commit.json`,
`negative/malformed-json.json`, `negative/duplicate-key.json`,
`negative/wrong-json-type.json`,
`negative/embedding-dimension-mismatch.json`,
`negative/embedding-space-encoder-version.json`,
`negative/legacy-version-feature-gate.json`,
`negative/empty-valid-interval.json`,
`negative/stale-expected-head.json`.

### 13.4 Vendoring into weave-language

The fixtures are self-contained and need nothing outside the crate directory.
To vendor them (LA0, weave-language#1):

1. Copy `crates/weave-contract/` (including `fixtures/` and
   `tests/core_v1_fixtures.rs`) to `vendor/weave-contract/`, as today.
2. Add every new file with its SHA-256 to `vendor/manifest.json`.
   `verify_contract.py` already rejects any file set that differs.
3. Run the harness in language CI. The vendored crate is excluded from the
   language workspace, so run it as its own package:
   `cargo test --manifest-path vendor/weave-contract/Cargo.toml --test core_v1_fixtures`.
   That crate has no lockfile of its own, so this command cannot use
   `--locked`. Alternatively, a language-side test can read
   `vendor/weave-contract/fixtures/core-v1/` and decode it through the
   `weave_contract` dependency.
4. Compiler golden tests SHOULD check that every program the compiler emits
   decodes with `weave_contract::decode_program`.

## 14. Limits and non-guarantees

- The JSON shape of programs and results is specified. Byte-level canonical
  JSON for producing revision IDs outside Rust is **not** specified. Revision
  IDs are computed by the engine (ADR-0001).
- The `E_UNKNOWN_FIELD` and `E_UNKNOWN_VARIANT` codes are derived from serde's
  derive error messages. The harness pins this behaviour, and a serde upgrade
  that changes it fails the harness instead of silently changing codes.
- Decoding is structural only. Version feature gates, identifier bounds and
  authorization are checked at execution by the engine.
- `nearest` and `similar_candidates` are exhaustive (`exactness: exact`).
  The scan budget of `nearest` and the read budget of `similar_candidates`
  count the stored embeddings they read, including hidden ones, as every
  read does.


- Error messages are diagnostic text. They are not stable and not part of the
  contract.
