# Weave core contract v1

Status: **accepted** on 2026-10-19 (A1,
[#2](https://github.com/weave-graph/weave-engine/issues/2)). Baseline protocol:
**0.21.0**. Profile name: `core-v1` (`weave_contract::wire::CORE_PROFILE`).

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

Two parts of core v1 are **defined but not yet active on the wire**:
provenance ([section 6](#6-provenance), activated by A2) and embeddings
([section 7](#7-embeddings), activated by A4). Their records are normative
and fixture-tested as standalone records. The fields that will carry them in
programs are reserved ([section 11.6](#116-reserved-names)), and protocol
0.21.0 rejects them with `E_UNKNOWN_FIELD`.

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
- Replica identity (the `observer` in recorded selections) is the seventh.
  It is covered by `time.recorded`.

## 4. Facts

A **fact** is a node, an edge, an explicit assertion, a metadata attachment or
(from A4) an embedding.

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
  `negative/stale-expected-head.json`.
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

## 5. Time

Valid time and recorded time are separate axes. Neither is inferred from the
other.

- **`time.valid`.** `valid_time: Interval { start, end }` states when a fact is
  true in the world. It is half-open, `[start, end)`. `end: null` is unbounded,
  and `start < end` is required (`E_INTERVAL` otherwise). `valid_at = t`
  selects facts with `start <= t < end`. Edges, assertions, attachments and
  embeddings have valid time today. Nodes gain it in A2 (field reserved).
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

## 6. Provenance

Defined by [ADR-0005](../architecture/ADR-0005-per-fact-provenance.md)
(accepted). Records: `weave_contract::provenance`. Active on the wire from A2.

- **`provenance.fact`.** Every stored fact has a `FactProvenance`:
  - `writer` and `recorded_at_ms`, which the engine records from host
    authority and its trusted clock. They are not part of fact content.
  - `source` (optional claimed label), `method` (`human`, `import`, `rule`,
    `model` or `unspecified`) and `evidence` (at most 64 tagged references:
    `assertion`, `node`, `graph` or `artifact`). The writer claims these, as
    `ClaimedProvenance`, inline in the fact's reserved `provenance` field.

  A client cannot send `writer` or `recorded_at_ms`: `ClaimedProvenance`
  rejects them with `E_UNKNOWN_FIELD`. An omitted claim is recorded as
  `method: unspecified`. Results list provenance in the reserved
  `QueryResult.fact_provenance` as `ProvenanceEntry { fact: FactRef,
  provenance }`. Evidence a reader cannot see is removed, and coverage becomes
  `partial`. Invalid provenance labels fail with `E_PROVENANCE`, and too much
  evidence fails with `E_BUDGET`. Fixtures:
  `positive/fact-provenance-model.json`,
  `positive/claimed-provenance-import.json`,
  `positive/provenance-entry-unspecified.json`,
  `negative/claimed-provenance-forged-writer.json`,
  `negative/reserved-node-provenance.json`.

Until A2, the existing claimed labels `Assertion.source` and
`Edge.assertion_source` and the per-commit `Event.actor` are the only
provenance on the wire.

## 7. Embeddings

Defined by [ADR-0006](../architecture/ADR-0006-embedding-facts-and-spaces.md)
(accepted). Records: `weave_contract::embedding`. Active on the wire from A4.

- **`embedding.space`.** `EmbeddingSpace { id, revision, encoder,
  encoder_version, preprocessing, dimensions, metric }` is an immutable
  descriptor. `dimensions` is between 1 and 4096. `metric` is
  `cosine_distance` or `euclidean`. Two spaces are compatible only when they
  are equal in every field. A mismatch fails with `E_SPACE`, even when the
  dimensions are equal. Fixtures: `positive/embedding-space.json`,
  `negative/embedding-space-encoder-version.json`.
- **`embedding.fact`.** `EmbeddingFact { id, space, subject, vector,
  valid_time, readers, derived_from, derived_nodes, derived_snapshots,
  provenance }` is a temporal, permissioned fact. `subject` is `entity`,
  `node` (document chunks included), `edge` or `assertion`. `vector` holds
  finite binary32 components of exactly the space's dimension (`E_SPACE`
  otherwise). Selection by valid time, recorded time and permission comes
  before ranking. Fixtures: `positive/embedding-fact.json`,
  `negative/embedding-dimension-mismatch.json`,
  `negative/reserved-graph-embeddings.json`,
  `negative/reserved-expression-nearest.json`.
- **`embedding.candidate`.** `CandidateRelation { id, predicate:
  "weave:similar_to", from, to, space, score, exactness, valid_time, origins }`
  is a similarity-derived *result value*. It is never an asserted fact or an
  identity, and it never appears in ordinary traversal unless requested.
  `score` is a distance (lower is closer). `exactness` is `exact`;
  `approximate` is reserved for C5. Fixture:
  `positive/candidate-relation.json`.

The untyped science `vector` property keeps its current semantics and is
never reinterpreted as an embedding fact.

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
  `commit`, `commit_batch`, `query`, `join`, `bind`, `evaluate`,
  `recorded_range` and `accepted_range`. Fixtures:
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

  Derived values carry their exact source origins and are readable only by
  the evaluating principal until a release policy exists (B2). Fixture:
  `positive/selection-operators.json`.

A3 adds `expand` (k-hop traversal) and `changes` (change sets). A4 adds
`nearest` and `similar_candidates`. These names are reserved
([section 11.6](#116-reserved-names)).

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
    `selected_context` and `source_revisions`, each omitted when empty.

  Unknown, partial, denied and empty stay distinguishable. When any selected
  input is missing, hidden or unresolved, the result has `coverage: partial`
  with a generic diagnostic. A result MUST NOT be an empty `complete` result
  for unavailable input. `complete` describes the authorized input, not the
  world. Fixture: `positive/result-envelope.json`.

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
| `E_CONFLICT` | `expected_head` differs from the branch head, or a descriptor conflicts with one already registered |
| `E_INTERVAL` | A valid-time interval is empty or inverted |
| `E_ID` | An identifier is empty, too long or otherwise invalid |
| `E_FORBIDDEN` | The host has not granted the required write authority |
| `E_UNAVAILABLE` | A requested object is unavailable to this principal. "Missing" and "denied" are not distinguished |
| `E_BUDGET` | A size, depth, work or read budget is exhausted. Never a truncated complete result |
| `E_INTEGRITY` | Stored data failed an integrity check |
| `E_HISTORY_SELECTOR`, `E_HISTORY_UNAVAILABLE` | An invalid recorded or accepted selector, or a selected observation that is unavailable |
| `E_CONTEXT_REQUIRED` | A contextual claim was consumed without matching context selection |
| `E_RESERVED_NAMESPACE` | A write targets an engine-reserved graph or descriptor (for example identity or governance storage) |
| `E_PROVENANCE` | Invalid provenance, or a missing origin (ADR-0005) |
| `E_SPACE` | An invalid or mismatched embedding space or vector (ADR-0006) |
| `E_STORAGE`, `E_FORMAT` | Internal storage or serialization failure |

Extension features define further codes in their version notes. Those codes
are stable under the same rule.

## 11. Compatibility policy

### 11.1 Version identifiers

These versions identify different boundaries. None advances another
automatically:

| Identifier | Current | Where |
|---|---|---|
| Protocol (`Program.version`, `QueryResult.version`) | 0.21.0 | `weave_contract::VERSION` |
| Core profile | `core-v1` | This document |
| Contract crate | 0.21.0, always equal to the protocol | `crates/weave-contract/Cargo.toml` |
| Store schema | 29 | `STORAGE_VERSION` |
| Capsule format | 0.4 | [CAPSULES.md](../CAPSULES.md) |
| Science API | 0.1.0 | [SCIENCE_INTERFACE.md](../SCIENCE_INTERFACE.md) |
| Artifact formats | `weave-view-registration/1`, `weave-handler-registration/1` | 0.16 and 0.18 notes |

### 11.2 Semantic versioning of the protocol

- **`compat.versions`.** A program declares exactly one protocol version.
  There is no range negotiation. The engine accepts the closed set
  `SUPPORTED_VERSIONS` (0.1.0 through 0.21.0 today) and executes each with
  the exact semantics it was published with. A feature used under an older
  version fails with `E_VERSION` before any write. Fixtures:
  `positive/legacy-version-accepted.json`,
  `negative/legacy-version-feature-gate.json`.
- **Before 1.0.0 (now).** Every wire change bumps MINOR (0.22.0, 0.23.0 and
  so on). Each change MUST be additive (section 11.3). PATCH is not used on
  the wire: wire versions are always `X.Y.0`. Clarifications that change no
  bytes need no bump.
- **1.0.0.** Protocol 1.0.0 is cut at the Phase A release (A6). It freezes
  core v1 together with its activated provenance and embedding fields. After
  that, MINOR means additive and MAJOR means breaking.
- **Core v1 activation.** A2 and A4 activate the reserved core-v1 fields
  through additive minor bumps. Each implementing PR records its exact version
  number. A1 itself changes no wire bytes, so the protocol stays 0.21.0.

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
  activation. Protocol 0.21.0 rejects them (`E_UNKNOWN_FIELD` or
  `E_UNKNOWN_VARIANT`), and no extension may use them for anything else:

  | Name | Kind | Owner |
  |---|---|---|
  | `provenance` on `Node`, `Edge`, `StructuralEdge`, `Assertion` and `MetadataAttachment` | field (`ClaimedProvenance`) | ADR-0005, A2 |
  | `QueryResult.fact_provenance` | result field (`ProvenanceEntry[]`) | ADR-0005, A2 |
  | `Node.valid_time` | field (`Interval`) | A2 |
  | `GraphData.embedding_spaces`, `GraphData.embeddings` | fields | ADR-0006, A4 |
  | `QueryResult.candidates`, `QueryResult.exactness` | result fields | ADR-0006, A4 |
  | `assert`, `retract`, `correct` | command `op` | A2 write API |
  | `expand`, `changes` | expression `kind` | A3 |
  | `nearest`, `similar_candidates` | expression `kind` | A4 |
  | `weave:` | predicate and ID namespace (`weave:similar_to`) | engine |

  An epic MAY choose different names, in which case its PR releases the
  reservation here. Fixtures: `negative/reserved-node-provenance.json`,
  `negative/reserved-graph-embeddings.json`, `negative/unknown-command.json`,
  `negative/reserved-expression-nearest.json`.

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
             "results": [{"kind": "queried", "coverage": "complete", "nodes": [], "edges": [], "edge_properties": {}, "recorded_observations": 1, "revision": "sha256:…"}],
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
| `crates/weave-engine/tests/core_v1_fixtures.rs` | Every `engine` expectation; this document cites every fixture and concept; `weave-engine run` codes; the engine accepts exactly `SUPPORTED_VERSIONS` | Engine CI |

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
`positive/embedding-space.json`, `positive/embedding-fact.json`,
`positive/candidate-relation.json`.

Negative: `negative/unknown-version.json`, `negative/missing-version.json`,
`negative/unknown-version-checked-first.json`,
`negative/unknown-field-program.json`,
`negative/reserved-node-provenance.json`,
`negative/reserved-graph-embeddings.json`,
`negative/claimed-provenance-forged-writer.json`,
`negative/unknown-command.json`,
`negative/reserved-expression-nearest.json`,
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
- Provenance and embedding records are not yet accepted by any program. Their
  fixtures test the records alone until A2 and A4.
- Error messages are diagnostic text. They are not stable and not part of the
  contract.
