# ADR 0004: fact log, indexes, checkpoints and the storage backend trait

Status: **accepted** on 2026-10-19 (A2, [#3](https://github.com/weave-graph/weave-engine/issues/3)).
The backend trait, the SQLite backend, the in-memory reference backend and
their conformance suite are in `crates/weave-engine/src/fact_store/` and
`crates/weave-engine/tests/fact_store_conformance.rs`. Later A2 parts wire them
into the engine; this ADR is the design they implement.

## Context

Every write today stores a complete `GraphData` snapshot in `revisions.data`
and is a compare-and-swap of that snapshot (ADR-0001). Writes cost in
proportion to the graph, reads re-parse the whole snapshot, and a graph is
capped at 100,000 nodes or edges and 16 MiB per revision
([audit](../AUDIT_2026-10-17.md), gaps 1 to 5). Goals G1 to G3 need writes
that cost in proportion to the change, indexes for reads, valid time on
nodes, per-fact provenance (ADR-0005), and a store whose backend can change
in C4 without a rewrite.

Constraints:

- Revision identity stays content-derived (ADR-0001). Existing `sha256:` and
  `logical:` revisions keep their IDs, bytes and verification.
- Recorded time and writer come from the engine only (ADR-0002, ADR-0005).
- Per-record `readers` and derivation carriers keep their exact semantics.
- Stored data and saved plans are never reinterpreted.

## Decision

### 1. Two kinds of revision

| Kind | ID | Content | Created by |
|---|---|---|---|
| **Snapshot revision** | `sha256:…` (ADR-0001) or `logical:{batch}:{graph}` | The full `GraphData` in `revisions.data` | `commit`, `commit_batch`, handler and view writes, capsule acceptance, every pre-A2 revision |
| **Fact revision** | `fact:sha256:<64 hex>` | The parent revision plus a list of fact operations in the fact log | The fact write API (A2) |

A snapshot revision's stored payload is authoritative and is never rewritten.
Its exact bytes are what its content-hash ID verifies, and array order inside
it is client-defined, so it cannot be re-derived from facts. It acts as a
*base checkpoint*: the state a fact line starts from.

A fact revision's content is the fact log. Its full `GraphData` is
*derived*: it is materialized on demand from the line state described below,
and it may be cached as a checkpoint (section 5). Dropping every cached
checkpoint changes no result.

### 2. Fact identity and operations

A **fact** is one record: a node, an edge, a structural edge, an explicit
assertion or a metadata attachment (embeddings join in A4, ADR-0006). Facts
live in two identity namespaces per graph: `node` (node IDs) and `record`
(edges, structural edges, assertions, attachments and, later, embeddings),
matching core v1 `identity.assertion`. A fact key is `(namespace, id)`.

A write is one `write` command (protocol 0.22.0, core v1 `fact.write`) holding
a list of changes on distinct keys:

- `assert`: the key is not live; add this version.
- `correct`: the key is live with the same kind; this version supersedes it.
  Structural identity stays immutable: an edge keeps its `from`, `to` and
  `predicate`, and a node keeps its `entity_id` and `space_id`. A different
  structure needs a new ID.
- `retract`: the key is live; it stops being live. The old version stays in
  history.

Asserting a version byte-identical to the live one is a no-op. A write whose
operations are all no-ops returns `unchanged` and creates no revision, as
snapshot commits do today. Optimistic concurrency is optional at two levels:
`expected_head` on the branch, and per operation an expectation that the key
is absent or unchanged since a named revision. Either mismatch fails the
whole write with `E_CONFLICT`.

The time-invariant node property map stays `Node.properties`. A
time-varying property is a metadata attachment hosted on the node with a
literal value and its own `valid_time`, readers and provenance. Each such
value is one fact, so a firmware change is one `correct` or `assert` and not
a rewrite of the node.

### 3. Revision identity mapped to log positions

The fact revision ID is

```
"fact:sha256:" + hex(SHA-256(json(["weave-fact-revision-v1", graph_id, branch_id, parent, ops])))
```

where `ops` is the write's effective operations sorted by `(namespace, id)`,
each `{"assert": <canonical record>}` or `{"retract": {"kind", "id"}}`. The
canonical record is the contract record's own serde encoding, including its
claimed `provenance`. Writer, recorded time and concurrency expectations are
not content, so the same change on the same parent has the same ID on every
replica. The `fact:` prefix keeps the kinds apart: no code ever verifies a
fact revision as a snapshot hash or the reverse.

Every revision row still exists in `revisions` (heads, events and head
observations reference it), with `data` empty. A fact revision is identified
by its row in the backend, not by the empty payload. Code that is unaware of
fact revisions therefore sees an unavailable payload and fails closed.

The **log** is append-only. Each operation gets a store-wide **position**: an
integer that strictly increases in commit order and is never reused. A
revision maps to the contiguous position range of its own operations,
`[first_position, first_position + op_count)`. Positions are a storage
detail, never part of a revision ID, and they are not exposed on the wire.

### 4. Lines: the unit of linear history

A **line** is a linear chain of fact revisions in one graph. Each revision has
`(line, seq)`, with `seq` counting from 1 along the line.

- A write whose parent is the *tip* (highest `seq`) of a line continues that
  line.
- Any other write starts a new line: a new graph (no parent), a parent that is
  a snapshot revision, or a parent that is an older fact revision (a fork, or
  a branch moved back to an old revision). The new line copies its parent's
  state as **base** entries at `seq` 0: the graph header (profile, schema,
  influence, context typing) and every live fact, each with its original
  recording.

Lines make every line self-contained, so the state at `(line, seq)` depends on
that line's rows only, and every index lookup is a single range scan. The
cost is that the first write after a fork or on top of a snapshot copies the
parent state once. A snapshot is bounded by the 16 MiB request limit. Forks
of large fact graphs are rare: branching and sync are deferred (E09). C4 may
replace copying with shared ancestry behind the same trait.

The graph header is fixed per line. Fact writes inherit it. Changing the
profile or schema takes a snapshot commit, after which the next fact write
starts a new line. A graph built only from fact writes uses the legacy profile
and no schema.

### 5. Tables, indexes and checkpoints (SQLite backend)

```
fact_lines(line PK, graph_id, header, base_parent)
fact_revisions(revision PK, graph_id, branch_id, parent, line, seq,
               first_position, op_count, writer, on_behalf_of, recorded_at_ms,
               UNIQUE(line, seq))
fact_log(position PK, line, seq, op IN ('base','assert','retract'), kind,
         namespace, fact_id, body, writer, on_behalf_of, recorded_at_ms,
         recorded_in)
fact_index(position PK → fact_log, line, namespace, fact_id, kind,
           from_seq, until_seq, entity, subject, object, predicate,
           valid_from, valid_to, recorded_at_ms)
fact_checkpoints(revision PK → fact_revisions, body)
```

`fact_log` rows never change. `fact_index` is derived and could be rebuilt
from the log. It holds one row per asserted version, live on its line for
`from_seq <= seq < until_seq` (`until_seq` NULL while live). A later
`correct` or `retract` on the same line sets `until_seq`. This is the
transaction-time (recorded) interval of the version on that line.

The index keys come from the record, and the engine extracts them, because the
backend never parses fact bodies:

| Key | Node | Edge / structural edge | Assertion | Attachment |
|---|---|---|---|---|
| `entity` | `entity_id` | – | – | host entity ID |
| `subject` | `id` | `from` | `edge_id` | host node, edge or assertion ID |
| `object` | – | `to` | – | – |
| `predicate` | – | `predicate` | – | `key` |
| valid time | `valid_time` (none: always) | `valid_time` | `valid_time` | `valid_time` |

SQLite indexes: by key `(line, namespace, fact_id, from_seq)`, and
`(line, entity)`, `(line, subject)`, `(line, object)`, `(line, predicate)`,
`(line, valid_from)` and `(line, recorded_at_ms)`. These give the roadmap's
indexes by (graph, entity), (graph, predicate), valid time and recorded time,
because a line belongs to one graph.

`fact_checkpoints` caches the materialized `GraphData` of a fact revision.
Checkpoints are written only by an explicit maintenance call, never as a side
effect of a read. A read uses a checkpoint when one exists and otherwise
materializes from the index. Both give the same bytes, which the conformance
suite and the engine tests check.

**Materialization order.** A fact revision's `GraphData` lists each fact kind
sorted by ID. Snapshot revisions keep their stored order.

### 6. The backend trait

`fact_store::FactBackend` is the only path to the fact log, indexes and
checkpoints. Engine code outside `fact_store/sqlite.rs` issues no SQL against
them, and `scripts/check_fact_store_boundary.py` enforces this in CI. The
trait works on opaque record bodies plus extracted index keys, so it holds no
contract semantics.

Every backend MUST meet these guarantees, and the conformance suite checks
each one against the SQLite backend and the in-memory reference backend:

- **Atomicity.** Everything appended between `begin` and `commit` becomes
  visible together, or nothing does after `rollback`. A failed `append`
  changes nothing.
- **Durability.** After `commit` returns, a durable backend survives process
  exit. SQLite uses the engine's connection and transaction, so fact rows
  commit atomically with `revisions`, `heads`, events and head observations.
  The in-memory backend reports `durable() == false` and is the semantic
  reference only.
- **Isolation.** Reads see committed state plus the caller's own uncommitted
  writes.
- **Immutability.** Log entries never change. Only derived index metadata
  (`until_seq`) and checkpoints change.
- **Positions.** They strictly increase in commit order, are contiguous within
  one revision and are never reassigned.
- **Recorded time.** The backend stores the engine's `writer` and
  `recorded_at_ms` verbatim and never generates either. Along a line,
  `recorded_at_ms` never decreases (`E_HISTORY_CLOCK`).
- **Lines.** Continuing a tip takes no base state. Starting a line requires
  one (`E_INTEGRITY` otherwise). A revision ID is never reused
  (`E_CONFLICT`).
- **Operations.** Keys within one revision are distinct. A retract or a
  superseding assert must match the live fact's kind (`E_CONFLICT`). A
  retract of a non-live key fails with `E_CONFLICT`.
- **Reads.** `get` and `select` return exactly the versions live at the named
  revision, in `(kind, id)` order. `select` fails with `E_BUDGET` rather than
  truncating when more than its limit match. Unknown revisions fail with
  `E_UNAVAILABLE`.

The engine owns everything semantic: validation, authorization, provenance
merging, revision IDs, header handling and the request budgets. C4 adds a
backend by implementing this trait and passing the same suite.

### 7. Reads

- `load(graph, revision)` keeps its signature. A fact revision is
  materialized from its line (or checkpoint) and charged to the read budget
  by the bytes it reads, so every existing operator works unchanged.
- `query` on a fact revision with a `predicate`, `from` or `to` filter reads
  only the matching facts through the index. It assembles the subset those
  facts need (their endpoint nodes, the attachments on them and graph-hosted
  attachments) and runs the same authorization and filter code on it.
  Differential tests compare it with full materialization.
- Recorded time selection (`recorded_query`, `recorded_range`) is unchanged:
  fact writes advance heads through the same `advance_head`, so ADR-0002
  observations cover them.
- Integrity: reading a fact revision recomputes its ID from its logged
  operations and fails with `E_INTEGRITY` on a mismatch.

### 8. Valid time on nodes

`Node.valid_time` (reserved in core v1) is optional. Absent means valid at
every time, which is exactly how every existing node behaves. With
`valid_at = t`, a node whose `valid_time` excludes `t` is removed, together
with the edges and attachments that depend on it. With `valid_at` as the only
filter, a node whose explicit `valid_time` contains `t` is kept even when it
has no edge at `t`. No existing node has a `valid_time`, so no existing
result changes.

### 9. Provenance (ADR-0005)

- Each log entry stores the engine-recorded `writer` and `recorded_at_ms`, and
  the revision that first recorded that version (`recorded_in`). Base copies
  keep the original recording. The claim is inside the body.
- **First recording is listed.** A fact's provenance at revision R is the
  recording of the version live at R. Re-asserting identical content is a
  no-op and records nothing. A changed claim is a new version.
- Facts of snapshot revisions report the revision's commit event actor and
  recorded time, the record's claim, or its legacy `source` label with method
  `unspecified`. A snapshot commit restates the whole graph, so its committer
  is the writer of every fact in it. A revision with no recorded actor gives
  unavailable provenance with partial coverage, as ADR-0005 requires. The
  migration therefore writes nothing for provenance.

### 10. Migration from store 29

Store 30 creates the empty fact tables in the schema transaction. It rewrites
no existing row. Every pre-migration revision stays a snapshot revision with
its ID, bytes and verification, so pinned reads are unchanged. A line starts
lazily at the first fact write on a graph. A process kill before the schema
commit leaves store 29 untouched. A kill after it leaves a complete store 30.
Store-29 binaries refuse store 30 with `E_STORAGE_VERSION`.

### 11. Room for group-resolved readers (B1)

B1 (ADR-0007) adds groups and agent identities. The fact model leaves room
without committing to a design:

- **No visibility in the index.** The index holds no per-principal visibility
  and no reader columns. Authorization is evaluated at read time from the
  record's `readers`, so B1 can resolve groups at read time with no reindex.
- **Readers are not reinterpreted.** A `readers` entry stays a literal
  principal ID forever, so no existing record changes meaning when groups
  arrive. The record field name `reader_groups` is reserved in core v1 for
  B1's group references, as an additive field on every fact kind.
- **Writer stays a principal.** `writer` is the `HostContext.principal`
  string, never a key, a display name or a group. Store 30 reserves a
  nullable `on_behalf_of` column in `fact_log` and `fact_revisions`
  (`Recording.on_behalf_of` in the trait). It is engine-recorded, outside
  content and always empty until B1, so B1 needs no migration for it.
- **ADR-0007 section 13.** `reader_groups` is omitted when empty, lives in
  the record body and the revision hash and never in an index column; it is
  rejected (`E_UNKNOWN_FIELD`) until B1. Record visibility goes through one
  predicate (`visibility::readers_admit`), and reader intersections for
  derived records through one helper (`visibility::intersect_readers`). The
  earlier copies in `governance.rs`, `governance_graph.rs` and
  `accepted_history.rs` now call the predicate. Fact writes to `weave:`
  graphs fail with `E_RESERVED_NAMESPACE`.

### 12. Limits and non-guarantees

- No per-graph object cap. Per-request limits stay: 16 MiB requests, the read
  budget (128 MiB and 4,096 reads per operation), the 32 MiB result budget
  and at most 100,000 operations per write. Reading a whole fact revision
  bigger than the read budget fails with `E_BUDGET`. Indexed queries are the
  path for large graphs.
- ADR-0002 caps recorded head observations at 100,000 per store, which would
  limit a store to 100,000 head-advancing writes. That cap is in A2 scope: a
  later A2 part lifts or restructures it (for example with an index by
  recorded time instead of the linear walk) and records the measured practical
  limit in `docs/STATUS.md`.
- Fact revisions cannot be erased by retention, and capsule export rejects
  them. Log compaction, retention and erasure are C4.
- Integrity is checked per revision (its own operations), not along the whole
  chain on every read.

## Alternatives rejected

- **Convert every snapshot into log entries during migration.** Retention may
  have erased old payloads. Snapshot order is client-defined, so exact bytes
  could not be re-derived anyway.
- **Revision ID from the full state.** Hashing the whole graph on every write
  is O(graph), which defeats the purpose.
- **Shared ancestry instead of line copies.** Each lookup would walk a chain of
  ancestor lines. Lines keep reads to one range scan. C4 can revisit this
  behind the trait.
- **A per-principal visibility index.** It would leak through counts and need
  reindexing on every reader or group change.
- **Lazy checkpoints written by reads.** A read would take a write lock and
  could fail on storage errors that have nothing to do with the read.
