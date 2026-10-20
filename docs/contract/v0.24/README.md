# Weave contract 0.24.0: embedding facts and hybrid retrieval

Protocol 0.24.0 is the A4 activation of the core v1 embedding records
([core v1](../core-v1.md) §7 and §8 `query.nearest`,
`query.similar_candidates`; [ADR-0006](../../architecture/ADR-0006-embedding-facts-and-spaces.md);
[#13](https://github.com/weave-graph/weave-engine/issues/13)). Every addition
is a new fact kind, expression kind, enum variant or field that is omitted
when absent or empty, so existing serialized bytes, content hashes and
revision IDs are unchanged. The store moves to 31. The user guide is
[EMBEDDINGS.md](../../EMBEDDINGS.md).

## Additions

- **Fact kinds** `embedding_space` (`weave_contract::embedding::EmbeddingSpace`)
  and `embedding` (`EmbeddingFact`) in `write` changes and change sets. A
  space's fact key is `id@revision` (`SpaceRef::key`, `Fact::key_id`); a space
  revision never contains `@`. A space is asserted once and never corrected to
  another descriptor or retracted (`E_CONFLICT`). An embedding names a space
  declared in the graph and a live subject (`E_SPACE`, `E_ENDPOINT`); a
  subject with a live embedding cannot be retracted (`E_DEPENDENT_FACT`).
  A cosine space rejects the zero vector.
- **`GraphData.embedding_spaces` and `GraphData.embeddings`**, reserved since
  A1. They appear in `nearest` values. Snapshot commits that carry them fail
  with `E_EMBEDDING`: embeddings are written incrementally with `write`.
- **`nearest` expression** (`NearestPlan`): exact k nearest embeddings of one
  space to a vector or a visible query fact, at a pinned, `as_of` or head
  revision, with `valid_at` and an optional `candidates` value. Selection and
  authorization come before ranking. `k` is 1 to 1,000.
- **`similar_candidates` expression** (`CandidatesPlan`): candidate relations
  `weave:similar_to` between visible embeddings, with `k`, `max_score`,
  `valid_at`, `sources`, `targets`, `exclude_linked` and `limit` (default
  1,000, at most 100,000).
- **`ExpandPlan.seeds_from`**: an expression whose nodes (by origin in the
  graph) are added to the seeds. `seeds` may then be empty.
- **`EvidenceRef::Embedding`** `{kind: "embedding", graph_id, revision,
  embedding_id}`, for example to cite the origins of a persisted candidate.
- **Result fields** `QueryResult.neighbors` (`Neighbor { rank, embedding_id,
  subject, score }`), `QueryResult.candidates` (`CandidateRelation`) and
  `QueryResult.exactness` (`exact`; `approximate` is reserved for C5).
- `EmbeddingMetric::distance` is the normative distance: binary64 sums in
  component order.
- New codes: `E_NEAREST` (`k` or `limit` 0, invalid `max_score`) and
  `E_EMBEDDING` (embeddings in a snapshot commit).

## Compatibility

- Programs of 0.1.0 through 0.23.0 keep their exact semantics. The new fact
  kinds, evidence kind, expressions and `seeds_from` under an older version
  fail with `E_VERSION`.
- The reserved names `GraphData.embedding_spaces`, `GraphData.embeddings`,
  `QueryResult.candidates`, `QueryResult.exactness`, `nearest` and
  `similar_candidates` are released: they are now these features. The former
  negative fixtures `reserved-graph-embeddings` and
  `reserved-expression-nearest` became `embedding-write-version-gated` and
  `nearest-version-gated`.
- Change sets of programs older than 0.24.0 omit embedding and space
  entries, which those versions cannot express; their `limit` still counts
  them.
- `ExpandPlan` no longer implements `Eq` (it may hold an expression).
- `EmbeddingSpace::validate` rejects a revision containing `@`, and
  `EmbeddingFact::validate` rejects the zero vector in a cosine space. No
  published version could carry these records, so no accepted program changes.
- Store 30 upgrades to 31 by moving the marker only; no row is rewritten,
  and store-30 binaries refuse store 31.
- View and handler artifact protocols advance to 0.24.0 and still accept
  0.23.0 and older artifacts. Views and handler recipes reject `nearest` and
  `similar_candidates`.

## Limits

- Ranking is exhaustive. The read budget (128 MiB) counts the stored
  embeddings a ranking reads, including hidden ones, as every read does.
- Ordinary reads never return embeddings; another operator that takes a
  `nearest` or `similar_candidates` value as input takes only its graph
  records.
- Candidate relations are never written by the engine.
