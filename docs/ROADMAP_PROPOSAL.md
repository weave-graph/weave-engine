# Proposed GitHub issue changes (engine)

Status: **draft for owner approval. Nothing below has been applied on GitHub.**
It implements [ROADMAP.md](ROADMAP.md). Language-repository changes are in
[weave-language/docs/ROADMAP_PROPOSAL.md](https://github.com/weave-graph/weave-language/blob/main/docs/ROADMAP_PROPOSAL.md).

## How to apply, once approved

1. Create the labels `phase-a`, `phase-b`, `phase-c`, `deferred` and `epic`, and
   the milestones `Phase A: shared core`, `Phase B: permissions and agents` and
   `Phase C: telemetry scale`.
2. **Preserve history.** Before replacing any issue body, post the old body as a
   comment titled "Historical checkpoint evidence (moved from the issue body on
   <date>)". Existing evidence links stay reachable.
3. Apply the operations below in order: retitle and rebody, create new issues,
   then close merged issues with a link to the issue that absorbs them.
4. Update the "Builds on" issue numbers in `ROADMAP.md` if GitHub assigns
   different numbers to the new issues.

## Summary of operations

| Op | Issue | New title | Label / milestone |
|---|---|---|---|
| retitle + rebody | #2 E01 | A1: Core contract v1 | phase-a |
| retitle + rebody | #3 E02 | A2: Incremental bitemporal fact store | phase-a |
| retitle + rebody | #4 E03 | A3: Time travel, traversal and provenance queries | phase-a |
| retitle + rebody | #13 E12 | A4: Embeddings layer and hybrid retrieval | phase-a |
| **create** | — | A5: Hardware reference model and importers | phase-a |
| retitle + rebody | #16 E15 | A6: Phase A acceptance and release | phase-a |
| **create** | — | B1: Principals, groups and agent identities | phase-b |
| retitle + rebody | #5 E04 | B2: Derived-fact leakage protection | phase-b |
| **create** | — | B3: Authenticated Weave service | phase-b |
| **create** | — | B4: Agent interface (MCP and SDK) | phase-b |
| retitle + rebody | #12 E11 | B5: Optional review of shared knowledge | phase-b |
| **create** | — | B6: Phase B acceptance | phase-b |
| **create** | — | C1: Telemetry ingestion and series linkage | phase-c |
| retitle + rebody | #6 E05 | C2: Change feed, event bus and connectors | phase-c |
| **close as merged** | #7 E06 | → #6 (C2); effect broker deferred | deferred |
| retitle + rebody | #8 E07 | C3: Incremental derivations | phase-c |
| retitle + rebody | #15 E14 | C4: Storage scale, retention and erasure | phase-c |
| **create** | — | C5: Approximate vector index | phase-c |
| **create** | — | C6: Phase C acceptance | phase-c |
| label + prepend note | #9 E08 | (unchanged title) | deferred |
| label + prepend note | #10 E09 | (unchanged title) | deferred |
| label + prepend note | #11 E10 | (unchanged title) | deferred |
| label + prepend note | #14 E13 | (unchanged title) | deferred |
| none | #1 E00 | already closed | — |

**Deferral note** to prepend to #9, #10, #11 and #14:

> **Deferred (roadmap of <date>).** This epic is not scheduled in Phases A–C; see
> [ROADMAP.md](https://github.com/weave-graph/weave-engine/blob/main/docs/ROADMAP.md#deferred-kept-but-not-scheduled). Existing code and
> tests are kept and must keep passing. Reopen scheduling only by owner decision.

**Closing comment** for #7:

> Merged into #6 (C2: change feed, event bus and connectors) under the phased
> roadmap. The adapter lifecycle evidence above remains valid. The external-effect
> broker (R14) is deferred; see docs/ROADMAP.md.

## Common footer for every epic body

> **Definition of done.** Every criterion has a linked test or reproducible
> measurement, full CI passes, user docs and `docs/STATUS.md` are updated in the
> same PR, limits are documented, and contract changes carry golden fixtures plus
> the language vendor update. Implementation PRs reference this issue. Progress
> notes go in comments, not this body.

---

## A1: Core contract v1 (rebody #2)

**Goal.** Publish one consolidated specification of the core contract that
Phases A–C build on, replacing the reading of 21 additive version notes.

**Scope.**
- A specification document `docs/contract/core-v1.md` for: identities (entity,
  manifestation/node, edge, assertion, graph, revision), facts and polarity,
  valid and recorded time, per-fact provenance fields (as decided in ADR-0005),
  embedding facts and spaces (ADR-0006), query and selection operators, the
  result envelope (provenance, coverage, diagnostics) and error codes.
- A compatibility policy covering semver for the contract, which fields may be
  added, how store and contract versions relate, and the deprecation process.
- Golden positive and negative fixtures checked in both repositories.

**Acceptance criteria.**
- [ ] `core-v1.md` exists and every listed concept has a normative definition
  with at least one fixture.
- [ ] Each existing contract feature (0.1–0.21) is classified as *core v1*,
  *extension* or *deferred* in a table in the spec.
- [ ] Golden fixtures run in engine CI and in language CI through the vendored
  contract, and fail on drift.
- [ ] Unknown versions and unknown fields fail with stable error codes, with a
  test for each.
- [ ] ADR-0005 and ADR-0006 are accepted, or explicitly referenced as pending
  with the fields reserved.

**Out of scope.** Changing the semantics of deferred extensions such as
capsules, actors or governance effects.

---

## A2: Incremental bitemporal fact store (rebody #3)

**Goal.** Replace full-snapshot commits with fact-level writes, keeping immutable
history, so ingestion and agent writes cost in proportion to the change rather
than to the graph.

**Problem today.** `revisions.data` stores the whole graph per commit; the only
write is compare-and-swap of a full snapshot. Caps are 100,000 nodes or edges per
graph and 16 MiB per revision. Nodes have no valid time. The writer is recorded
per commit only (see the [audit](https://github.com/weave-graph/weave-engine/blob/main/docs/AUDIT_2026-10-17.md)).

**Scope.**
- ADR-0004: an append-only fact log with indexes by (graph, entity), (graph,
  predicate), valid time and recorded time. It must define how revision identity
  and content addressing (ADR-0001) map to log positions, and how snapshots
  become derived, cacheable checkpoints.
- A write API to assert, retract or correct one or many facts (node, edge,
  assertion, property or attachment) in one atomic transaction, with optional
  optimistic concurrency on the branch head or on individual facts.
- Valid time on nodes and node properties.
- ADR-0005: per-fact provenance. Each fact records the engine-recorded writer
  principal, recorded time, claimed source, method (human, rule, model plus
  version, or import) and evidence references. The engine sets the writer and
  recorded time; clients cannot forge them.
- Existing per-record `readers` and derivation references carry over unchanged.
- Migration from store 29, preserving every existing revision and its identity.

**Acceptance criteria.**
- [ ] Appending one fact to a graph of 1M facts writes O(change) bytes, verified
  by a test that measures database growth.
- [ ] A benchmark records single-fact commit latency at graph sizes of 10k, 100k
  and 1M.
- [ ] The 100,000-object per-graph cap is removed. A graph of at least 1M facts
  is created, queried and reopened in a test. Per-request limits remain.
- [ ] Queries pinned to any pre-migration revision return byte-identical results
  after migration from a populated store 29 fixture. The migration survives a
  process kill before and after commit.
- [ ] Every fact returned by a query exposes writer, recorded time, source and
  method. A client-supplied writer is ignored or rejected, with a test.
- [ ] Node valid time works: a unit installed on day 10 and decommissioned on
  day 50 is absent from `valid_at=5` and `valid_at=60` and present at
  `valid_at=20`.
- [ ] Late-arriving corrections keep valid and recorded time separate. "As known
  at recorded time K" and "valid at V" are each correct in a 4-quadrant fixture.
- [ ] The existing read-authorization and influence tests pass unchanged.
- [ ] The science `import` API accepts incremental batches. Full-snapshot import
  remains available.

---

## A3: Time travel, traversal and provenance queries (rebody #4)

**Goal.** Make the questions "how was it related", "what did we know then" and
"what changed" single, efficient queries.

**Scope.**
- An `Expand` plan operator: k-hop neighborhood from seed nodes with predicate
  and direction filters, a depth limit, valid-at and as-of selection, and a
  result-size budget.
- A `Changes` operator: the set of facts asserted, retracted or corrected
  between two recorded times (or since a cursor), optionally scoped to entities
  or predicates. This is the pull change feed used later by C2.
- As-of selection by recorded time using an index (no linear walk of history).
- `Explain` from any result element to its source facts and their provenance.
- Coverage on every result (complete, partial or unavailable), kept as today.

**Acceptance criteria.**
- [ ] `Expand` returns the exact k-hop neighborhood for k=1..4 on a
  100k-fact fixture, matching an independent BFS oracle, including valid-time and
  as-of filters.
- [ ] `Changes(K1, K2)` equals the set difference of the two as-of snapshots on
  randomized histories, checked by a property test with at least 1,000 cases.
- [ ] Recorded-time selection is O(log n) in history length, with a benchmark at
  10k and 1M revisions or log entries.
- [ ] Every element of `Expand`, `Changes` and join results can be explained to
  source facts with full provenance.
- [ ] Hidden facts (per-record readers) never appear in, and do not change, any
  `Expand` or `Changes` output for a reader who cannot see them. Paired tests
  cover this.
- [ ] Python client and JSON API expose `expand`, `changes`, `as_of` and
  `explain`, with tutorial docs.

---

## A4: Embeddings layer and hybrid retrieval (rebody #13)

**Goal.** One time-aware embeddings layer linked to the graph. Vector search
surfaces relations absent from the graph, and graph traversal seeds vector search.

**Problem today.** Science kNN reads an untyped node property with no encoder
descriptor, valid time or plan integration. Typed `weave-spaces` vectors support
only pairwise distance (see the [audit](https://github.com/weave-graph/weave-engine/blob/main/docs/AUDIT_2026-10-17.md)).

**Scope.**
- ADR-0006: embedding facts. A vector is a temporal fact attached to an entity,
  node, edge, assertion or document chunk, in a typed embedding space (encoder,
  version, preprocessing, dimensions, metric), with provenance and readers. This
  one model replaces the untyped `vector` property, which keeps a compatibility
  path.
- Document chunk nodes: text artifact reference and offsets, linked to the
  entities they mention.
- A `Nearest` plan operator (exact mode): query vector or query fact, space, k,
  valid-at and as-of selection, an optional candidate set from a graph
  expression, and authorization filtering applied **before** ranking.
- Composition inside one plan:
  - graph → vector: `Nearest` over vectors attached to the output of `Expand`
    or a query;
  - vector → graph: `Expand` from `Nearest` hits.
- Candidate relations: an operator that emits `similar_to` edges as *derived
  candidates*, with score, space, model version and provenance, never as identity
  or asserted fact.
- An exactness flag in the result (`exact` now, `approximate` reserved for C5).

**Acceptance criteria.**
- [ ] kNN results equal an independent exhaustive oracle on fixtures with
  adversarial ties and near-duplicates.
- [ ] Time awareness: a vector valid only in [10, 20) is returned at
  `valid_at=15` and not at 25. A vector recorded after K is not returned as-of K.
- [ ] Mismatched encoder, version, metric or preprocessing is rejected even at
  equal dimension.
- [ ] One plan runs "2-hop neighborhood of unit U → nearest 10 maintenance-note
  chunks", and one runs "nearest 10 chunks → owning units → their sites". Both
  match oracle answers.
- [ ] Candidate relations carry provenance and are excluded from ordinary
  `Expand` unless explicitly requested.
- [ ] A vector whose readers exclude the caller never appears in results and does
  not change the rank or score of visible results. Paired tests cover this.
- [ ] Embedding upsert is incremental through A2, measured at 100k vectors of
  1536 dimensions.
- [ ] A Python tutorial shows end-to-end hybrid retrieval with caller-supplied
  embeddings.

**Out of scope.** Generating embeddings (the caller owns the encoder), ANN
indexes (C5), and geometry beyond the existing operators (frozen).

---

## A5: Hardware reference model and importers (new)

**Goal.** Make Weave immediately usable for hardware-heavy teams with a
reference schema, importers and a realistic dataset.

**Scope.**
- A reference schema covering: asset/unit, model, part, serial, firmware or
  software version, configuration, site, sensor, `SeriesRef` (series store URI,
  series ID, time range, digest), maintenance event, incident, user,
  organization and team. Relations include installed-in, runs, located-at,
  owned-by, measured-by and replaced.
- `SeriesRef` defined as a typed metadata value. No raw samples are stored in
  the graph during Phase A.
- CSV and JSON importers mapping tabular exports (for example from ERP or PLM)
  to incremental fact writes, with idempotency keys.
- A synthetic but realistic fleet dataset generator (default 5,000 units),
  including histories, corrections and maintenance notes.

**Acceptance criteria.**
- [ ] The schema is documented in `docs/HARDWARE_MODEL.md`, with every type and
  relation shown in an example.
- [ ] Re-running an import is idempotent: zero new facts on an identical second
  run, verified by a test.
- [ ] The generator produces the dataset deterministically from a seed, and the
  Phase A scenario runs on it.
- [ ] `SeriesRef` round-trips and is queryable by time-range overlap.

---

## A6: Phase A acceptance and release (rebody #16)

**Goal.** Prove the shared core end to end and publish a tagged release.

**Acceptance criteria.**
- [ ] The Phase A exit scenario in [ROADMAP.md](https://github.com/weave-graph/weave-engine/blob/main/docs/ROADMAP.md#phase-a-rock-solid-shared-core)
  passes as an automated test in CI on Linux, macOS and Windows.
- [ ] Fuzz targets exist for the A2 write path, the A3 operators and the A4
  operator, with bounded smoke runs in CI.
- [ ] Migration from a populated store 29 database passes in CI.
- [ ] Baseline benchmarks are published (single-fact commit, `Expand`,
  `Changes`, `Nearest` at 100k and 1M facts) with host details. Benchmarks
  describe what was measured and are not capacity promises.
- [ ] The README quickstart covers the hardware model plus hybrid retrieval in
  under 10 commands.
- [ ] A tagged release exists, with release notes listing limits and deferred
  scope.

---

## B1: Principals, groups and agent identities (new)

**Goal.** Give every person, team and agent a first-class identity, and make
permissions resolve through groups.

**Scope.**
- ADR-0007: a principal registry with kinds user, group/team and agent. Each
  agent has an owner principal and a delegated, narrowed scope that cannot
  exceed its owner's.
- Readers and ACLs may name groups. Membership is resolved at read time, so a
  revocation takes effect on the next request.
- Graph-level read and write ACLs, and branch-level write ACLs.
- The `Discover` permission (whether a graph exists) is distinct from `Read`.
- Registry changes are themselves audited facts.

**Acceptance criteria.**
- [ ] A permission-matrix test covers user, group and agent × discover, read,
  write and administer × graph, branch and record.
- [ ] Removing a user from a group hides group-restricted facts on their next
  query, with no rewrite of stored revisions.
- [ ] An agent can never read or write beyond its owner's scope; delegation
  escalation attempts fail, and tests cover them.
- [ ] Graphs without Discover are indistinguishable from nonexistent graphs in
  errors and listings.

---

## B2: Derived-fact leakage protection (rebody #5)

**Goal.** Derived facts never reveal restricted inputs to readers who cannot see
those inputs, unless an explicit release policy allows it.

**Problem today.** The read-time influence model is strong. However, a writer can
commit derived content with broader readers and no `derived_from`. Analytics and
kNN outputs carry no influence, and noninterference coverage is a finite set of
paired tests (see the [audit](https://github.com/weave-graph/weave-engine/blob/main/docs/AUDIT_2026-10-17.md)).

**Scope.**
- ADR-0008: write-time propagation. Engine-computed derivations get readers equal
  to the intersection of their inputs' readers. Declared derivations whose
  readers are broader than their inputs are rejected unless a release policy
  applies.
- Influence records on analytics outputs (scores, components, paths), kNN
  results, candidate relations and embeddings computed from restricted text.
- Release policies: named, audited, scoped declassification, for example
  "aggregate count with k ≥ 10 may be released to group G".
- A property-based noninterference harness. For every query operator, generate
  pairs of databases that differ only in facts hidden from reader R, and assert
  that R's outputs are identical (values, counts, ranks, explanations, errors,
  coverage).

**Acceptance criteria.**
- [ ] The harness covers every `GraphExpression` operator, every science
  analysis and `Nearest`, with at least 10,000 cases per operator in nightly CI
  and 500 in PR CI.
- [ ] A derived fact committed with readers broader than its inputs is rejected,
  with a test for each derivation path.
- [ ] Persisting an analytic or kNN result yields records whose readers are no
  broader than their inputs.
- [ ] Release policies are enforceable and audited. A test shows a k-threshold
  aggregate released and a below-threshold aggregate withheld.
- [ ] Documented non-guarantees (timing, size, resource side channels) are listed
  in SECURITY.md.

---

## B3: Authenticated Weave service (new)

**Goal.** A long-running, multi-user Weave server, so knowledge is shared
through one authenticated endpoint rather than a trusted local process.

**Scope.**
- A server binary with an HTTP/JSON API covering the A2–A4 operations. The
  transport (HTTP/JSON first, gRPC optional) is chosen in ADR-0007.
- Authentication mapped to B1 principals: API tokens and OIDC at minimum. Agent
  tokens are bound to the agent principal.
- Reuse of the existing signed capability admission (`weave-policy`) for
  delegated agent requests.
- Tenant isolation, either one store per tenant or a verified tenant key on
  every row.
- An append-only audit log of reads and writes: principal, operation, graph and
  result digest.
- Request limits and budgets, health checks and structured logs. Secrets and
  denied topology are excluded from logs.

**Acceptance criteria.**
- [ ] Unauthenticated requests are rejected on every endpoint, and a test
  enumerates the endpoints.
- [ ] A cross-tenant access attempt fails and is audited.
- [ ] The audit log answers "who read fact F" and "who wrote fact F".
- [ ] A load test publishes concurrent read and write throughput for 50 clients.
- [ ] SECURITY.md is updated with the new trust boundary.

---

## B4: Agent interface (MCP and SDK) (new)

**Goal.** Agents write and query shared knowledge directly through a standard
tool interface.

**Scope.**
- An MCP server backed by B3, with tools: `assert_fact`, `retract_fact`,
  `query`, `expand`, `as_of`, `what_changed`, `hybrid_search`, `explain` and
  `describe_schema`.
- An SDK (Python first) with the same operations.
- Every agent write records agent identity, method and model version, and
  evidence references, through A2 provenance.
- Data/code boundary: fact content can never be interpreted as instructions or
  authority, and tool outputs mark untrusted content.
- Per-agent quotas and rate limits.

**Acceptance criteria.**
- [ ] An end-to-end test: agent X asserts a fact from a document, agent Y (same
  team) finds it via `hybrid_search` and `explain` shows X as the source.
- [ ] Agent Z (other team, no access) cannot see the fact or any effect of it.
- [ ] `describe_schema` lets an agent build a valid query without hard-coded
  field names, shown in a scripted test.
- [ ] A fact containing prompt-injection text cannot change permissions or
  trigger writes. A test covers this.
- [ ] Docs include a quickstart for connecting an MCP client.

---

## B5: Optional review of shared knowledge (rebody #12)

**Goal.** Teams can choose whether agent-written facts become shared immediately
or only after review.

**Scope.**
- Per-graph policy: `direct` (personal or trusted graphs publish immediately) or
  `reviewed` (writes land as proposals; reviewers accept or reject).
- Reuse the existing governance machinery (immutable approval subjects, M-of-N,
  policy transitions) behind a simpler API.
- Accepted facts keep the proposer's provenance and add the reviewer's.

**Acceptance criteria.**
- [ ] Both modes work end to end through B3 and B4.
- [ ] An approval of a changed proposal is rejected, and a policy change cannot
  self-authorize. The existing tests are retained.
- [ ] A reviewer sees proposals only within their own read scope.
- [ ] Rejected proposals stay in history and are never visible as accepted facts.

---

## B6: Phase B acceptance (new)

**Acceptance criteria.**
- [ ] The Phase B exit scenario in [ROADMAP.md](https://github.com/weave-graph/weave-engine/blob/main/docs/ROADMAP.md#phase-b-permissions-and-agent-knowledge-sharing)
  passes as an automated test against a running B3 server.
- [ ] The B2 harness passes at nightly scale.
- [ ] A threat model is written down and reviewed: authentication, tenant
  isolation, derived leakage, agent delegation and injection.
- [ ] A tagged release exists, with deployment docs for a single-node server.

---

## C1: Telemetry ingestion and series linkage (new)

**Goal.** Relate high-rate telemetry to the graph without turning Weave into a
time-series database.

**Scope.**
- ADR-0009: native columnar series storage versus external stores (for example
  Parquet or a TSDB) referenced by `SeriesRef`. Decide using measured ingest and
  query costs.
- A batch append path for high-rate derived facts (events, state changes,
  aggregates) with idempotency.
- Linking: query "all series and derived events for units in this subgraph,
  valid in [t1, t2)".

**Acceptance criteria.**
- [ ] The ADR is accepted, with measurements from at least two options.
- [ ] A sustained ingest benchmark reports facts per second and series samples
  per second on a recorded host, and the Phase C target is set from it.
- [ ] Derived event facts link to their exact series range and pass `explain`.

---

## C2: Change feed, event bus and connectors (rebody #6, absorbs #7)

**Goal.** Push changes to subscribers and ingest from industrial sources through
registered adapters.

**Scope.**
- Push subscriptions over the A3 `Changes` feed, with at-least-once delivery,
  durable cursors and permission filtering per subscriber. Reuse the existing
  outbox and lease machinery.
- An adapter interface for ingestion connectors, with reference connectors (for
  example MQTT and a generic HTTP webhook) chosen by user demand.
- The external-effect broker is out of scope (deferred).

**Acceptance criteria.**
- [ ] A crash-injection matrix shows no lost and no ghost events. The existing
  process-death tests are retained.
- [ ] A subscriber never receives an event about a fact it cannot read.
- [ ] Each reference connector ingests into the A5 hardware model with
  idempotent replay.

---

## C3: Incremental derivations (rebody #8)

**Goal.** Keep derived facts (rules, aggregates, embedding refreshes, candidate
relations) current under continuous ingestion.

**Acceptance criteria.**
- [ ] Incremental output equals full recomputation after randomized insert,
  correct, retract and expire sequences, checked by a property test with at
  least 1,000 cases.
- [ ] A measured speedup over full recomputation is published for the C1
  workload. If there is no speedup, the epic is not done.
- [ ] Derived outputs keep B2 restriction semantics.

---

## C4: Storage scale, retention and erasure (rebody #15)

**Goal.** Bounded storage and predictable performance at telemetry scale.

**Scope.**
- Partitioning by graph and time, compaction of the fact log and checkpoints,
  retention policies, and audited erasure (with documented limits on recalling
  already-copied data), plus backup and restore.
- A storage-backend decision record: whether SQLite remains adequate or another
  backend is needed, based on measurement.
- The existing fuzzing, recovery and migration work continues as part of this
  epic.

**Acceptance criteria.**
- [ ] A 24-hour soak at the C1 target rate shows bounded storage growth under
  retention.
- [ ] An erased fact is unreadable through every API, and the erasure is itself
  audited.
- [ ] Backup and restore round-trip a populated database with identical query
  results.
- [ ] Latency of A3 and A4 queries at 10M or more facts is published.

---

## C5: Approximate vector index (new)

**Goal.** Fast retrieval at scale without silently changing exact semantics.

**Acceptance criteria.**
- [ ] An ANN index (for example HNSW) sits behind the A4 `Nearest` operator.
  Results are marked `approximate`, and the exact mode remains available.
- [ ] Recall@10 against the exact oracle is measured and published at 1M and
  10M vectors, and the threshold is declared.
- [ ] Permission and time filtering stay correct with the index. A paired test
  shows hidden vectors do not change visible results.
- [ ] Index maintenance is incremental with A2 and C3 writes.

---

## C6: Phase C acceptance (new)

**Acceptance criteria.**
- [ ] The Phase C exit scenario in [ROADMAP.md](https://github.com/weave-graph/weave-engine/blob/main/docs/ROADMAP.md#phase-c-scale-for-high-volume-telemetry)
  passes, with the measurements published.
- [ ] The B2 harness and B6 scenario still pass at Phase C scale.
- [ ] A tagged release exists, with capacity guidance derived from measurements.
