# ADR 0006: embedding facts and spaces

Status: **accepted** on 2026-10-19 (A1, [#2](https://github.com/weave-graph/weave-engine/issues/2)).
The records are defined in [core contract v1](../contract/core-v1.md#7-embeddings)
and in `crates/weave-contract/src/embedding.rs`. They are activated on the wire
by the A4 protocol bump ([#13](https://github.com/weave-graph/weave-engine/issues/13)).
Until then, no protocol 0.21.0 `Program` can carry them.

## Context

Weave has two disconnected vector mechanisms
([audit](../AUDIT_2026-10-17.md), G3 gaps):

- Science kNN reads an untyped node property `vector`. It has no encoder
  descriptor and no valid time, and it is not part of the plan algebra.
- The typed `weave-spaces` geometry extension describes an embedding space by
  encoder, preprocessing, dimensions and metric, with no encoder version. It
  supports only pairwise distance between graph assertions.

G3 needs one model in which vectors are temporal, permissioned and
provenance-carrying facts. Vector search must be composable with traversal,
and similarity must never become identity.

## Decision

**Spaces.** An `EmbeddingSpace` is an immutable descriptor named by
`(id, revision)` with these fields: `encoder`, `encoder_version`,
`preprocessing`, `dimensions` (1–4096) and `metric` (`cosine_distance` or
`euclidean`). Two vectors are comparable only when their spaces are equal in
every field. Equal dimensions never suffice. A comparison across spaces
fails with `E_SPACE`. Registering a different descriptor under an existing
`(id, revision)` is a conflict (`E_CONFLICT`), as with schema labels since
0.4. A new encoder version is a new space revision.

**Embedding facts.** An `EmbeddingFact` is a fact. It is immutable, and a
correction is a new fact. It has:

- `id`, in the same per-revision ID namespace as edges, assertions and
  attachments;
- `space`, a `SpaceRef { id, revision }`;
- `subject`, a tagged `entity`, `node`, `edge` or `assertion`. Document chunks
  are nodes, so a chunk is embedded as a `node` subject. A4 defines the
  chunk node's type and its text-artifact reference;
- `vector`, as IEEE-754 binary32 components. JSON numbers are rounded to the
  nearest binary32 value on decode, and the stored value is the rounded one.
  Non-finite values and a wrong dimension are rejected (`E_SPACE`);
- `valid_time`, a half-open interval with the same semantics as edges;
- `readers`, with the same semantics as every other record (empty means
  public);
- `derived_from`, `derived_nodes` and `derived_snapshots`, the conservative
  AND gates. An embedding of restricted text must cite that text, so it is
  restricted to the readers of all of its inputs;
- `provenance`, a `ClaimedProvenance` (ADR-0005). Its `method` is normally
  `model` with the encoder and version.

They live in the reserved `GraphData.embedding_spaces` and
`GraphData.embeddings` fields and are written incrementally through A2. A
subject may have many embeddings: different spaces, or different valid
intervals in one space.

**Ranking.** A4 adds a `nearest` expression. Its exact mode respects these
rules:

1. Selection comes before ranking. Valid-at, as-of and the authorization of
   every embedding and its subject are applied first. Hidden vectors never
   affect the rank or score of visible ones.
2. The score is the metric's distance, and lower is closer. Ties are broken
   by ascending subject reference and then by embedding ID, so results are
   deterministic.
3. Results report `exactness: exact`. `approximate` is reserved for C5 and is
   never produced in Phase A.

**Candidate relations.** A `CandidateRelation` is a result value with
predicate `weave:similar_to`, endpoints, space, score, exactness, a valid time
(the intersection of both embeddings' valid times) and `origins`, the two
embedding `FactRef`s. A candidate is never an asserted fact or an identity,
and it never appears in ordinary traversal unless explicitly requested. Its
readers are restricted to the evaluating principal, the conservative rule
that joins already use. Persisting a candidate takes an explicit write by a
principal. That write creates an ordinary assertion (not identity) with method
`rule` or `model` and evidence pointing at the origins. The `weave:` predicate
namespace is engine-owned.

**Compatibility path.** The untyped `vector` node property and science
`nearest_vectors` keep their current semantics and binary64 values. They are
never reinterpreted as embedding facts. An explicit importer may convert them
into a declared space. That conversion is a new write with method `import`,
and the old property stays as it was. The `weave-spaces` geometry extension is
frozen. Its `Embedding` geometry has no encoder version, so it is not
comparable with an `EmbeddingSpace`.

## Consequences

- binary32 storage halves vector size compared with binary64. That matters
  for the A4 target of 100k vectors at 1536 dimensions. Callers that need
  bit-exact binary64 vectors must keep them elsewhere.
- Because each encoder version is its own space, re-embedding a corpus
  produces new facts in a new space revision. The old vectors stay queryable
  as of their recorded time.
- Permission-before-ranking rules out precomputed global rankings that ignore
  readers. C5's approximate index must filter by permission, or over-fetch and
  re-check.

## Alternatives rejected

- **Vectors as node properties (status quo).** They have no space, no valid
  time of their own and no provenance, and one property per node cannot hold
  several encoders.
- **Space descriptor without encoder version.** Two checkpoints of one model
  are different spaces, and comparing them silently gives wrong neighbors.
- **Similarity edges as ordinary asserted edges.** That would turn similarity
  into a fact or an identity, against the VISION principle.
