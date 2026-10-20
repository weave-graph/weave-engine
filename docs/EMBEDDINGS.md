# Embeddings and hybrid retrieval

Protocol 0.24.0 (A4, [#13](https://github.com/weave-graph/weave-engine/issues/13))
makes vectors temporal, permissioned facts in typed spaces
([ADR-0006](architecture/ADR-0006-embedding-facts-and-spaces.md)) and adds two
graph expressions: exact k-nearest-neighbor search (`nearest`) and
similarity-derived candidate relations (`similar_candidates`). Both compose
with graph traversal in one plan. The normative rules are in
[core contract v1](contract/core-v1.md) §7 and §8; the version note is
[0.24](contract/v0.24/README.md). Store 31 holds the new fact kinds.

Weave does not generate embeddings. The caller owns the encoder and writes
its vectors, as with any other fact. From Python, see the
[hybrid retrieval tutorial](PYTHON_SCIENCE.md#hybrid-retrieval-with-caller-supplied-embeddings).

## Spaces and embedding facts

A space is an immutable descriptor named by `(id, revision)`:

```json
{"kind": "embedding_space", "id": "hw-notes", "revision": "1",
 "encoder": "text-embedding-3-small", "encoder_version": "2024-01-25",
 "preprocessing": "weave-chunk/512-tokens/v1", "dimensions": 1536,
 "metric": "cosine_distance"}
```

Two vectors are comparable only when their spaces are equal in every field.
Equal dimensions never suffice: a query or query fact from a space that
differs in `encoder`, `encoder_version`, `preprocessing` or `metric` fails
with `E_SPACE`. A new encoder version is a new space revision, so
re-embedding a corpus writes new facts and the old vectors stay queryable.

An embedding is a fact with its own ID (in the graph's record namespace,
shared with edges, assertions and attachments), a subject, a vector, a valid
time, readers, conservative gates and claimed provenance:

```json
{"kind": "embedding", "id": "emb:note:maint:WO-000123",
 "space": {"id": "hw-notes", "revision": "1"},
 "subject": {"kind": "node", "node_id": "maint:WO-000123"},
 "vector": [0.012, -0.031, "…"],
 "valid_time": {"start": 1717200000000, "end": null},
 "readers": ["fleet-admin"],
 "provenance": {"method": {"kind": "model", "model": "text-embedding-3-small",
                           "version": "2024-01-25"},
                "evidence": [{"kind": "assertion", "graph_id": "fleet",
                              "revision": "fact:sha256:…",
                              "assertion_id": "note:maint:WO-000123"}]}}
```

- **Subjects.** `entity`, `node` (document chunks included), `edge` (legacy or
  structural) or `assertion`. The subject must be live when the embedding is
  written (`E_ENDPOINT`), and it cannot be retracted while the embedding is
  live (`E_DEPENDENT_FACT`).
- **Vectors.** JSON numbers are rounded to the nearest binary32 value on
  decode and stored rounded. Components must be finite and of the space's
  dimension, and a cosine space rejects the zero vector (`E_SPACE`).
- **Readers and gates.** An embedding is visible only when its own readers,
  its subject and its `derived_from`, `derived_nodes` and `derived_snapshots`
  gates admit the reader. An embedding of restricted text should cite that
  text in a gate, or copy its readers.

Write spaces and embeddings with the `write` command, like any fact
(assert, correct, retract), with `expected_head` and per-fact `expected`
concurrency. A space is asserted once: correcting it to a different
descriptor or retracting it fails with `E_CONFLICT`, and its fact key is
`id@revision` (a space revision never contains `@`). Snapshot commits cannot
carry embeddings (`E_EMBEDDING`).

Ordinary reads (`query`, `expand`, joins, `as_of`) never return embeddings or
spaces, so a graph with 100,000 vectors costs ordinary reads nothing. Change
sets (`changes`) list them like every other fact.

## Exact k-nearest neighbors

```json
{"kind": "nearest", "graph_id": "fleet",
 "space": {"id": "hw-notes", "revision": "1"},
 "query": {"kind": "vector", "vector": ["…1536 numbers…"]},
 "k": 10, "valid_at": 1717200000000, "as_of": 1717300000000}
```

- `query` is a caller-supplied vector, or `{kind: "embedding", embedding_id}`
  of the same revision. A query fact must be visible (`E_UNAVAILABLE`
  otherwise, the same as a missing one), and its own subject is excluded.
- `revision`, `as_of` (recorded time) or neither (the head) select the state,
  as for `expand`. `valid_at` keeps embeddings whose valid time contains the
  instant. The subject's own node valid time is not re-applied: an embedding
  of a note about past work stays valid after the work. Restrict subjects by
  valid time with a candidate set.
- `candidates` restricts the ranking to the subjects of another value: its
  nodes, their entities and its edges, by origin in the same graph.
- `k` is 1 to 1,000 (`E_NEAREST` for 0, `E_BUDGET` above).
- The ranking streams: it reads the selected embeddings through the fact
  index in chunks, authorizes each chunk with its subjects and keeps only the
  best `k`, so memory is O(k) plus one chunk whatever the space size. The
  vectors it streams are counted against a scan budget of 268,435,456
  components (`MAX_SCAN_COMPONENTS`, about 174,000 vectors of 1,536
  dimensions) instead of the 128 MiB read budget; more fails with `E_BUDGET`.

**Selection comes before ranking.** Space, valid time, candidate set and the
authorization of every embedding and its subject are applied first. A hidden
embedding never changes the rank, score or count of a visible one, nor a tie
between them. The score is the metric's distance (lower is closer), computed
in binary64 in component order (core v1 `embedding.space`), and ties are broken
by subject (kind, then ID) and then embedding ID, so results are
deterministic.

The result is a graph value holding the hits' subject nodes (with their
hosted attachments, such as the note text) and the hit embeddings and space,
plus:

- `neighbors`: `[{rank, embedding_id, subject, score}]` in rank order;
- `exactness: "exact"` (`approximate` is reserved for C5);
- `fact_provenance` for every hit embedding and returned node.

## Composition

**Graph to vector.** Restrict a ranking to a neighborhood:

```json
{"kind": "nearest", "graph_id": "fleet", "space": {"id": "hw-notes", "revision": "1"},
 "query": {"kind": "vector", "vector": ["…"]}, "k": 10, "valid_at": 1717200000000,
 "candidates": {"kind": "expand", "graph_id": "fleet", "seeds": ["unit:SN-000123"],
                "hops": 2, "direction": "both"}}
```

**Vector to graph.** `expand` takes seeds from another value with
`seeds_from` (0.24):

```json
{"kind": "expand", "graph_id": "fleet", "hops": 2, "direction": "out",
 "predicates": ["hw:performed_on", "hw:located_at"],
 "seeds_from": {"kind": "nearest", "graph_id": "fleet",
                "space": {"id": "hw-notes", "revision": "1"},
                "query": {"kind": "vector", "vector": ["…"]}, "k": 10}}
```

This is "nearest 10 notes, then the units they were performed on, then
their sites". A partial input value makes the result partial. When another
operator takes a `nearest` or `similar_candidates` value as input, it takes
only the graph records; the ranking, embeddings and candidates stay with the
expression that produced them.

## Candidate relations

```json
{"kind": "similar_candidates", "graph_id": "fleet",
 "space": {"id": "hw-units", "revision": "1"}, "k": 1,
 "valid_at": 1717200000000, "exclude_linked": true, "max_score": 0.2}
```

For every visible source embedding (all of the space, or the subjects of
`sources`), its `k` nearest visible targets (all, or `targets`) of another
subject and with an overlapping valid time become candidates when at most
`max_score` apart. A pair is reported once, from the lower subject to the
higher. `exclude_linked` drops node pairs already joined by a visible edge
in either direction (valid at `valid_at` when set), so only relations absent
from the graph remain; a hidden edge never drops a candidate.

Each `CandidateRelation` has predicate `weave:similar_to`, its endpoints,
space, score, `exactness`, the intersection of both valid times, and
`origins`: the two embedding fact references, which `fact_provenance`
explains. Its ID is stable across revisions for the same two embeddings. At
most `limit` candidates (default 1,000, at most 100,000; `E_BUDGET` beyond)
and 50,000,000 comparisons.

Candidates are result values. They are never written, never an identity,
and never followed by `expand` or any other traversal. Persisting one takes an
explicit `write` of an ordinary edge (with a predicate of your own, not
`weave:similar_to`) whose provenance cites the two embeddings as evidence
(`{kind: "embedding", graph_id, revision, embedding_id}`); readers who cannot
see an embedding do not see it as evidence.

## Document chunks

A chunk is an ordinary node, by convention of type `weave.DocumentChunk`
with properties `artifact_uri`, `artifact_digest`, `start` and `end` (UTF-8
byte offsets of the chunk in the artifact, half-open), linked to the nodes of
the entities it mentions by `weave:mentions` edges, and embedded as a `node`
subject. The engine gives the convention no special meaning; graphs with a
schema declare the type like any other. The constants are in
`weave_contract::embedding` (`CHUNK_TYPE`, `MENTIONS_PREDICATE`, …).

## From the untyped `vector` property

The science `vector` node property and `nearest_vectors` keep their binary64
semantics and are never reinterpreted. `Engine::vector_property_changes`
plans an explicit conversion into a declared space: one embedding fact per
visible node with the property, rounded to binary32 (it reports how many
components changed), with the node's readers, a valid time you choose and
provenance `method: import` citing the node. Nothing is written until you
apply the returned changes with `write` and the returned `expected_head`.
A malformed value fails the whole conversion with `E_SPACE`, and a repeated
conversion plans nothing.

## Limits

- Ranking is exhaustive (`exact`). An approximate index is C5; it will sit
  behind the same ranking boundary and fall back to the exact scan.
- **Measured** ([report](benchmarks/embeddings-2026-10-20.json), one AMD
  Ryzen 9 7950X Linux host, optimized build): 100,000 embeddings of 1,536
  dimensions load in 100 writes of 1,000 at about 330 ms each, flat from the
  first write to the last; one more embedding is a 1.2 ms write on the full
  store; an exact top-10 over the whole space takes about 7.1 s (3.6 s when
  `valid_at` selects half), with peak memory unchanged by the ranking. The
  store is about 2.1 GB, because vectors are stored as JSON.
- The scan budget of `nearest` counts every stored embedding it streams,
  including ones the reader cannot see, as the read budget counts every read
  record; `similar_candidates` keeps its vectors in memory and is bounded by
  the read budget and its comparison limit. Revision IDs change with hidden
  writes.
- Every read of a fact revision recomputes its ID from its own operations,
  so write large embedding loads in batches of a few thousand.

- Results include the hit vectors; `k` times the dimension is bounded by the
  32 MiB result budget.
- Views and handler recipes reject `nearest` and `similar_candidates`.
