# Current status

**Phase A is complete** (20 October 2026). The shared core is released as
protocol **1.0.0**, which freezes [core contract v1](contract/core-v1.md)
with the 0.24.0 feature set ([release notes](../CHANGELOG.md),
[1.0 note](contract/v1.0/README.md)). **Phase B has started:** B1 is complete:
the principal registry with users, groups and agents, group readers, graph
ACLs and Discover, as protocol 1.1.0 and store 32
([below](#principals-groups-agents-and-graph-acls-b1)),
B3 is complete with the authenticated multi-tenant `weave-service` ([below](#authenticated-service-b3)),
B2 is complete with a generated noninterference harness over every operator,
write-time propagation as protocol **1.2.0**, and audited release policies
([below](#derived-fact-leakage-protection-b2)), B4 part 1 serves the
agent interface over MCP ([below](#agent-interface-b4)), and B5 part 1 adds
optional review of shared knowledge in the engine, as protocol **1.3.0** and
store **33** ([below](#optional-review-b5)). The language reached Phase A parity in
LA1 (weave-language#3): each of the five exit questions fits in at most 15
lines of `.weave`, and engine CI runs all five through `check_phase_a.py`.
Source writes cannot set `readers` until LB1. A source `correct` of a
restricted fact under 1.1.0 therefore publishes it; under 1.2.0 it fails
with `E_READERS_WIDENED`. Write restricted facts through JSON or Rust.

As of **10 October 2026**, the native data-science delivery is implemented and
merged in [PR #33](https://github.com/weave-graph/weave-engine/pull/33). Its finite
completion contract is [DS01–DS09](NATIVE_SCIENCE_PLAN.md). The broader original
white-paper requirements remain tracked separately in the
[implementation plan](IMPLEMENTATION_PLAN.md) and [workflow](WORKFLOW.md).

## Supported experiment profile

| Component | Version / scope |
|---|---|
| Science API and CLI | `weave-science` 0.1.0; native Rust and one-request JSON process |
| Python client | `weave_science` 0.1.0; Python 3.10+, no mandatory runtime dependencies |
| MCP preview | `weave-mcp` 0.1.0; read-only local stdio MCP server (A7, [guide](MCP_PREVIEW.md)) |
| Service | `weave-service` 0.1.0; authenticated multi-tenant HTTP/JSON server over TLS, one store per tenant, API tokens and a read/write audit log (B3, [guide](SERVICE.md)) |
| Agent interface | MCP at `weave-service` `POST /v1/mcp`, agent tools 1.0.0: assert, retract, query, expand, as-of, what changed, hybrid search, explain, describe schema (B4 part 1, [guide](AGENT_INTERFACE.md)) |
| Engine contract | 1.3.0: core v1 as frozen in 1.0.0 with the 0.24.0 feature set ([1.0](contract/v1.0/README.md)), plus `reader_groups` and `FactProvenance.on_behalf_of` ([1.1](contract/v1.1/README.md)), write-time propagation ([1.2](contract/v1.2/README.md)) and `FactProvenance.review` ([1.3](contract/v1.3/README.md)); 0.1.0–0.24.0 and 1.0.0–1.2.0 programs stay accepted with their published semantics. Existing graph, temporal, metadata and authorization semantics, plus fact-level writes, node valid time and per-fact provenance ([0.22](contract/v0.22/README.md)), k-hop `expand`, `as_of` selection and `changes` change sets ([0.23](contract/v0.23/README.md)), and embedding facts with exact `nearest` and `similar_candidates` ([0.24](contract/v0.24/README.md)). The core subset is specified in [core contract v1](contract/core-v1.md) |
| Native storage | Store 33; persistent local SQLite graph revisions and history, plus the fact log ([ADR-0004](architecture/ADR-0004-fact-log-and-storage-backend.md)), which also holds embedding spaces and embeddings |
| Platforms | Linux, macOS and Windows in the native CI profile |

Experiments support durable import and correction with optimistic concurrency,
revision and temporal selection, existing graph expressions, authorized graph
analytics, exact vector neighbors, result/table export and saved experiment
records. Start with the [README quickstart](../README.md#run-your-first-experiment),
[Python tutorial](PYTHON_SCIENCE.md) or [native JSON guide](SCIENCE_INTERFACE.md).

## Core contract v1

[Core contract v1](contract/core-v1.md) is the consolidated specification of
the core subset (A1). It defines the core concepts normatively and classifies
every contract feature 0.1–0.21 as core v1, extension or deferred. It also sets
the compatibility policy (semantic versioning, additive rules, the relationship
between store and contract versions, deprecation and reserved names).

- **Golden fixtures.** 54 golden fixtures in
  `crates/weave-contract/fixtures/core-v1/` are pinned by a SHA-256 manifest.
  The contract harness checks decoding and canonical re-encoding. The engine
  harness checks execution outcomes. Both fail on drift.
- **Stable decode error codes.** `weave_contract::decode_program` reports
  `E_VERSION`, `E_UNKNOWN_FIELD`, `E_UNKNOWN_VARIANT`, `E_DECODE` and
  `E_BUDGET`, and checks the version before the structure. The
  `weave-engine run` CLI reports these codes and now rejects duplicate JSON
  keys; it previously reported `E_INPUT` for every decode failure. Plans over
  16 MiB still report `E_INPUT`, and the science envelope keeps `E_INPUT`.
- **Embeddings.** Embedding facts and spaces
  ([ADR-0006](architecture/ADR-0006-embedding-facts-and-spaces.md)) are
  active from protocol 0.24.0 (see [embeddings](#embeddings-and-hybrid-retrieval-a4)).

## Fact-level writes (A2)

Protocol 0.22.0 and store 30 add the fact store of
[ADR-0004](architecture/ADR-0004-fact-log-and-storage-backend.md):

- **Writes.** The `write` command asserts, corrects or retracts individual
  nodes, edges, structural edges, assertions and attachments in one atomic
  *fact revision* (`fact:sha256:…`), with optional `expected_head` and
  per-fact `expected` concurrency. A write stores only its changes. Every
  earlier revision stays readable. The science `import` request accepts
  `changes` for incremental batches (`Engine.import_facts` in Python); full
  `data` snapshots remain available.
- **Node valid time.** `Node.valid_time` scopes a manifestation in valid time;
  `valid_at` excludes nodes outside it. Time-varying properties are
  node-hosted attachments with their own valid time.
- **Provenance.** Results of 0.22.0 programs carry `fact_provenance`: for
  every cited stored fact, the engine-recorded writer and recorded time plus
  the claimed source, method and evidence. Hidden evidence is removed and
  makes coverage partial. A client cannot supply the writer.
- **Indexed reads.** A `query` on a fact revision with `predicate`, `from` or
  `to` reads only what the filter needs through the indexes by entity,
  subject, object, predicate, valid time and recorded time.
- **No per-graph caps.** The 100,000 node/edge cap and the 100,000
  head-observation store cap are removed. Recorded-time selection is indexed.
- **Migration.** Store 29 upgrades to store 30 by adding the empty fact store;
  no existing row is rewritten. A process kill before or after the schema
  commit is safe, and store-29 binaries refuse store 30.

**Measured** on one Linux host (AMD Ryzen 9 7950X, optimized build, on-disk
WAL store; [report](benchmarks/fact-writes-2026-10-19.json)):

| Graph size | Load (100k-change writes) | Single-fact commit, median / p95 | Mean growth per single-fact commit | Store size |
|---|---|---|---|---|
| 10,000 facts | 0.5 s | 0.89 / 1.04 ms | 2.5 KB | 6 MB |
| 100,000 facts | 5.6 s | 0.90 / 1.20 ms | 2.5 KB | 59 MB |
| 1,000,000 facts | 63 s | 0.90 / 1.08 ms | 2.6 KB | 592 MB |

The growth per commit includes the revision, event, head observation and
causal-root rows, not just the fact. The `fact-scale-1m` CI job re-checks a
1M-fact graph (create, indexed query, reopen, O(change) growth) and 120,000
single-fact revisions in one store on every PR.

**Limits.**
- Per-request limits remain. A whole fact revision is materialized for
  unfiltered queries and most algebra, so the 128 MiB read budget bounds it
  (a 1M-fact graph fails with `E_BUDGET` unless the query filters by
  `predicate`, `from` or `to`).
- Fact writes inherit the graph header (profile, schema, influence, context
  typing) from their parent; changing it takes a snapshot commit.
- Retention never erases fact revisions, its planner still inventories at most
  3,000 revisions, and capsules cannot carry fact revisions (C4, E08).
- A filtered fact read reports partial coverage only for records the filter
  selects.

## Hardware reference model (A5)

The `weave-hardware` crate and CLI provide the
[hardware reference model](HARDWARE_MODEL.md):

- **Schema.** Twelve node types (organization, team, user, site, model,
  firmware version, configuration, unit, part, sensor, maintenance event,
  incident) and fourteen relations, enforced by an engine graph schema
  (`weave.hardware` revision 1) on every write.
- **`SeriesRef`.** A typed literal on an `hw:series` sensor attachment whose
  valid time is the referenced range. No raw samples are stored. References
  are selected by half-open range overlap.
- **Importers.** CSV and JSON tables map to facts with IDs derived from each
  row's key. `Engine::reconcile_facts` (new, needs write authority) plans the
  asserts and corrections, which are written as A2 fact writes with claimed
  provenance (`method: import`). An identical re-import writes zero facts and
  no revision.
- **Generator.** A deterministic, seeded fleet generator: by default 5,000
  units (about 151,600 facts, roughly a fifth customer-restricted) plus 1,000
  updates (rollouts, relocations, replacements, maintenance, incidents and
  corrections), byte-identical on every platform.
- **Scenario.** The whole Phase A exit scenario runs on it in CI at full size
  (see [Phase A acceptance](#phase-a-acceptance-a6)).

**Limits.** Imports above 100,000 changes are written in several revisions.
Rows missing from an export are not retracted. `recorded_query` requires a
reader who can see the whole revision (ADR-0002); restricted readers use the
A3 `as_of` expression, which reads the visible subset as known at a time. Series overlap is filtered after an indexed
read, not indexed itself. The importer writes no embeddings; the caller
embeds notes as embedding facts (see `tests/fleet_hybrid.rs`).
## Traversal, time travel and change sets (A3)

Protocol 0.23.0 adds three graph expressions
([guide](TRAVERSAL.md), [0.23](contract/v0.23/README.md)); the store stays 30:

- **`expand`.** The exact k-hop neighborhood (1 to 16 hops) of seed nodes,
  with predicate, direction and valid-time filters and a result-size `limit`
  (`E_BUDGET`, never truncated). It reads a pinned revision, the head, or the
  branch `as_of` a recorded time. On a fact revision each hop reads only the
  frontier's edges through the subject and object indexes.
- **`as_of`.** `{kind: "as_of", query, unix_millis}` reads any query as this
  replica knew the branch at a recorded time, and `expand` and `changes` take
  the same selection. It uses the `(graph, branch, recorded_at_ms)` index
  without a history walk and authorizes record by record, so a reader who
  cannot see restricted records (for example a fleet analyst) gets the
  visible subset exactly as a read pinned to that revision.
  `recorded_query` keeps its whole-revision rule (ADR-0002, amended).
- **`changes`.** The facts asserted, retracted or corrected between two
  authorized states (as of a time, a cursor revision or the head), scoped by
  entity or predicate. It is the pull change feed: `to_revision` is the next
  cursor. Between two revisions of one fact line it reads only the logged
  operations between them and the records whose visibility depends on them.
- **Explanations.** Every element of `expand`, `changes` and join results is
  listed in `fact_provenance` with its writer, recorded time and claim.
- **Coverage.** A missing or hidden seed (`E_SEED_UNAVAILABLE`) and a denied
  derivation in the result's region (`E_DERIVATION_UNAVAILABLE`) make the
  result partial; an empty complete result means nothing visible matched.
- **Interfaces.** Rust programs, `weave-engine run`, the science JSON
  envelope (`execute`, `analyze`, listed in `capabilities`) and the Python
  client (`Engine.expand`, `as_of`, `changes`, `explain`, with exact replay
  through `pinned_program`); see the
  [tutorial](PYTHON_SCIENCE.md#traversal-as-of-reads-and-change-sets). The A5
  fleet scenario answers "as known at K" for the restricted analyst with
  `At::AsOf`.
- **Hidden input.** Paired tests (`tests/traversal.rs`) show that facts a
  reader cannot see do not change any `expand` or `changes` output for that
  reader, apart from content-derived revision IDs.

Evidence: `tests/traversal.rs` checks `expand` against an independent BFS on
40 random graphs (1,440 expansions over fact and snapshot revisions) and
`changes` against the difference of two model states on 1,000+ random
histories with corrections, retractions and snapshot commits.
`tests/traversal_scale.rs` checks k = 1..4 on a 100,000-fact graph with
valid-time, as-of and reader filters in the release CI job. As-of selection
is measured at 10,000 and 1,000,000 revisions
([report](benchmarks/as-of-selection-2026-10-20.json)).

**Limits.**
- A change set across a snapshot revision or two fact lines reads both states
  whole, within the read budget.
- The read budget counts stored records an operator reads, including hidden
  ones, as every read does. Revision IDs change with hidden writes.
- The change feed is pull only (push is C2).
- Selecting the edges or attachments of a node scans every version recorded
  under it, so a node corrected very many times is slower to traverse.

## Read-only MCP preview (A7)

`weave-mcp` is a local stdio MCP server over one existing store
([guide](MCP_PREVIEW.md), [#37](https://github.com/weave-graph/weave-engine/issues/37)):

- **Tools.** `query`, `expand` (with `seeds_from`), `as_of`, `what_changed`
  (the A3 `changes` expression), `hybrid_search` (the A4 `nearest`, with a
  neighborhood as `candidates`, and `similar_candidates`) and `explain`. Each
  runs one read-only `evaluate` program through the same session as the
  science JSON `execute` operation, so its `result` equals that operation's
  result. Nested expressions are limited to those read operators.
- **Principal.** `--actor` is required and fixes the reader for every call.
  There is no default identity and no tool argument can change it; per-record
  `readers` apply to every result, rankings and candidates included. The
  server holds no write grants.
- **Data/code boundary.** Tool descriptions, schemas and instructions are
  static. Store content appears only inside a result envelope marked
  `"trust": "untrusted_data"`.
- **Bounds.** Messages up to 1 MiB, arguments up to 32 levels deep with
  strings up to 4,096 bytes and at most 4 nested expressions (`E_MCP_INPUT`),
  results up to 16 MiB; engine bounds and codes pass through.

Evidence: `crates/weave-mcp/tests/mcp.rs` checks every tool against the
science JSON API on the same store, paired stores that differ only in hidden
facts and embeddings (identical output for the restricted reader, no hidden
ID or text in any response, hidden embeddings never ranked), a scripted
session of the real binary that calls every listed tool and leaves every
store table unchanged, the input bounds and the CLI's refusal to run without
a reader. `tests/fleet_exit_questions.rs` drives the binary as a scripted MCP
client on the A5 fleet and answers exit questions 2, 3 and 4 for the
restricted and the unrestricted reader, checked against the `weave-hardware`
helpers (question 2) and an independent ranking (questions 3 and 4).

**Limits.** No authentication, local single-user use only. One request at a
time, no cancellation, stdio only. The preview supplies no encoder: agents
pass query vectors from the space's encoder or a stored embedding ID.

## Embeddings and hybrid retrieval (A4)

Protocol 0.24.0 and store 31 activate the embedding records of
[ADR-0006](architecture/ADR-0006-embedding-facts-and-spaces.md)
([guide](EMBEDDINGS.md), [0.24](contract/v0.24/README.md)):

- **Embedding facts.** Spaces (encoder, encoder version, preprocessing,
  dimensions, metric) and vectors (binary32) are facts of the fact log,
  written incrementally with `write` with readers, conservative gates, valid
  time and claimed provenance (normally `method: model`). Spaces are
  immutable; comparisons across spaces fail with `E_SPACE` even at equal
  dimension. Ordinary reads never return embeddings.
- **`nearest`.** Exact k-nearest-neighbor ranking over one space at a pinned,
  `as_of` or head revision, with `valid_at` and an optional candidate set
  from any graph expression. Selection and authorization come before ranking;
  ties break by subject and embedding ID. The result carries ranked
  `neighbors`, the hit subjects with their attachments, `exactness: exact`
  and `fact_provenance`.
- **Composition.** Graph to vector: `nearest` restricted to an `expand`
  neighborhood. Vector to graph: `expand` seeded from `nearest` hits
  (`seeds_from`).
- **Candidate relations.** `similar_candidates` proposes `weave:similar_to`
  relations between visible embeddings, optionally only those absent from the
  graph, with score, valid time and both embedding origins. They are result
  values, never written and never traversed.
- **Compatibility.** The untyped science `vector` property keeps its
  binary64 semantics. `Engine::vector_property_changes` plans an explicit,
  idempotent conversion into a declared space (`method: import`).
- **Migration.** Store 30 upgrades to 31 by moving the marker; no row is
  rewritten and store-30 binaries refuse store 31.

Evidence: `tests/embeddings.rs` compares `nearest` with an exhaustive oracle
on 24 random stores with exact duplicates, positive multiples (cosine ties)
and one-ulp neighbors (2,592 rankings, over 6,000 adjacent ties), checks valid
and recorded time, space mismatches on each descriptor field, both
compositions against a BFS oracle, candidate provenance, and paired stores
whose hidden embeddings (duplicates of visible vectors, closer vectors,
vectors on hidden subjects, a hidden edge) change no output, error or tie.
`tests/fleet_hybrid.rs` answers Phase A exit questions 3 and 4 on the
generated fleet (in CI at 5,000 units) against an independent oracle, with
restricted units hidden from the analyst.

- **Scale.** `nearest` streams through the selected embeddings with O(k)
  memory under its own scan budget (2^28 vector components, about 174,000
  vectors of 1,536 dimensions). **Measured** on one AMD Ryzen 9 7950X Linux
  host ([report](benchmarks/embeddings-2026-10-20.json)): 100,000 embeddings
  of 1,536 dimensions load in 100 writes of 1,000 at about 330 ms each, flat
  from first to last; one more embedding is a 1.2 ms write; an exact top-10
  over the whole space takes about 7.1 s (3.6 s with `valid_at` selecting
  half), without raising peak memory.
- **Python.** `embedding_space`, `embedding`, `Engine.nearest_embeddings`,
  `Engine.similar_candidates` and `expand(seeds_from=...)`, with exact replay,
  and an end-to-end [tutorial](PYTHON_SCIENCE.md#hybrid-retrieval-with-caller-supplied-embeddings)
  (`examples/science/hybrid_retrieval.py`, run by the native tests). The
  science `capabilities` list both operators.

**Limits.**
- Ranking is exhaustive. An exact whole-space top-10 over 100,000 vectors
  of 1,536 dimensions takes about 7.1 s on the measured host, mostly
  decoding vectors stored as JSON (about 2.1 GB for that space). A binary
  vector encoding is C4 (storage backend) and approximate indexes are C5;
  until then, restrict large rankings with a candidate set or `valid_at`.
- The scan budget counts every stored embedding a ranking streams, hidden
  ones included, as the read budget does for every read.
  `similar_candidates` holds its vectors in memory and stays bounded by the
  read budget and its comparison limit.

- Candidate relations are never persisted by the engine; persisting one is
  an explicit write of an ordinary edge citing the embeddings as evidence.
- Views and handler recipes reject `nearest` and `similar_candidates`.

## Phase A acceptance (A6)

A6 is complete ([#16](https://github.com/weave-graph/weave-engine/issues/16)):

- **Exit scenario.** `scripts/check_phase_a_exit.py` runs the whole
  [exit scenario](ROADMAP.md#phase-a-rock-solid-shared-core) at 5,000 units
  and 1,000 updates and gives one pass or fail: questions 1 to 5, source-fact
  explanations, reproducibility after reopening, and questions 2 to 4 through
  the MCP preview. It runs in the `phase-a-exit` CI job on Linux, macOS and
  Windows ([how it works](PHASE_A_EXIT.md)).
- **Fuzzing.** cargo-fuzz targets for fact `write` decode and apply
  (`fact_write`), the A3 plans (`traversal_plans`), the A4 plans
  (`vector_plans`) and MCP JSON-RPC input (`mcp_jsonrpc`) run as bounded
  smoke runs in `fuzz.yml` ([fuzzing](FUZZING.md)).

- **Store 29 to the current store.** The `store29-to-current`
  compatibility job upgrades a store 29, populated by the exact store-29
  binary, along both routes: directly, and through the store-30 binary. Each
  step is killed before and after its commit. Every older binary refuses the
  result. Pinned queries stay byte-identical, and the Phase A operators work
  over the migrated lines ([upgrade path](STORAGE_RECOVERY.md#phase-a-upgrade-path-store-29-to-the-current-store)).
- **Baseline benchmarks.** Single-fact commit, `expand`, `as_of`, `changes`
  and `nearest` at 100k and 1M facts were measured on one host, along with
  the exit-scenario timings on Linux, macOS and Windows
  ([results](benchmarks/README.md#phase-a-baseline-a6)). Commit, traversal
  and change-set latencies stay flat from 100k to 1M facts. A whole-space
  `nearest` grows with the size of the space. A restricted `nearest` did
  too when that baseline was recorded; it has since been fixed (next item).
- **README quickstart.** Five commands cover the hardware model and hybrid
  retrieval ([README](../README.md#quickstart-a-hardware-fleet-and-hybrid-retrieval)).
  CI runs them on all three OSes.
- **Scoped `nearest`.** A `nearest` with `candidates` now costs in proportion
  to its candidates: 2.7 ms for a 2-hop neighborhood at 20k and at 200k
  vectors, down from 24 and 251 ms
  ([report](benchmarks/scoped-nearest-2026-10-20.json)).

- **Protocol 1.0.0.** One ordered version table in `weave_contract::versions`
  drives every feature gate through `version_at_least`.
  `tests/version_gates.rs` shows each gate accepts the same versions as
  before, and that 1.0.0 behaves like 0.24.0
  ([note](contract/v1.0/README.md)).
- **Release.** The [CHANGELOG](../CHANGELOG.md) has the 1.0.0 release notes,
  covering limits and deferred scope.

## Principals, groups, agents and graph ACLs (B1)

B1 ([#38](https://github.com/weave-graph/weave-engine/issues/38),
[ADR-0007](architecture/ADR-0007-identity-groups-and-tenancy.md), accepted)
is complete ([guide](REGISTRY.md)):

- **Registry.** An opt-in, one-way principal registry per store, stored as
  facts in the reserved graph `weave:registry`, with users, groups (a team
  is a group), nested membership (acyclic, at most 4 deep, at most 1,024
  groups per principal, enforced at write time with `E_BUDGET`), statuses,
  group administrators and the built-in `weave:administrators` group. IDs
  are never reused. Registry history and "who was in G at T" are ordinary
  `as_of` queries of the registry graph.
- **Group readers.** `reader_groups` (protocol 1.1.0) on every fact record,
  resolved against current membership when each request runs. Removing a
  member hides group facts on the next request, including pinned and as-of
  history, with no stored revision rewritten. Writes may name only groups
  the writer can discover; anything else fails with one `E_UNAVAILABLE`.
- **Graph ACLs and Discover.** Discover ≤ Read ≤ Write ≤ Administer per
  graph, for users and groups, plus protected branches with branch write
  ACLs. Graphs are closed by default. A graph without Discover gives exactly
  the nonexistent-graph error for every operation and is absent from
  `Engine::list_graphs`; Discover without the needed permission gives
  `E_FORBIDDEN`.
- **Agents.** Each agent has one owner (a user or an agent, at most 8
  links to a user) and a delegation: graph scopes up to Write, exercised
  owner identities, an optional expiry and `may_delegate`. Its authority on
  every request is its owner's current authority ∧ its scope ∧ its own
  identity, so it never exceeds its owner, also after the owner loses
  access. Administer is not delegable. Escalating sub-agents and
  delegations are refused at write time, and owners revoke, narrow or retire
  agents with effect on the agent's next request. Agent fact writes record
  `writer` and `on_behalf_of` (1.1.0 results only); agents cannot make
  snapshot commits.
- **Permission matrix.** User, group member and agent × Discover, Read,
  Write and Administer × graph, protected branch and record
  (`tests/registry_agents.rs`).
- **Writes need Read on what they change.** In a registry store, a
  `correct` or `retract` of a fact the writer cannot read (its own
  `readers` and `reader_groups`, and for an edge its endpoints') fails
  exactly as for a fact that does not exist. The B3 paired HTTP tests found
  that part 1 let a writer change such facts.
- **Local stores** without a registry behave exactly as before; the existing
  authorization and influence suites pass unchanged.
- **Store 32.** Store 31 upgrades by moving the marker; no row is rewritten
  and store-31 binaries refuse store 32 (`check_registry_store_migration.py`
  in the compatibility workflow, against the v1.0.0 binary).

## Authenticated service (B3)

B3 ([#39](https://github.com/weave-graph/weave-engine/issues/39),
ADR-0007 sections 6 to 9) is complete, as the `weave-service` crate and
binary ([guide](SERVICE.md)):

- **Server.** A long-running HTTP/1.1 server over TLS (axum on hyper, rustls
  with the ring provider), serving the core v1 JSON contract: `write`,
  `query`, `evaluate` and `bind` programs of protocol 1.0.0 and later, graph
  listing and creation, the registry API and token management. Plaintext is
  served only on loopback, for development.
- **Authentication.** Bearer API tokens (256 random bits, stored only as a
  SHA-256 digest in the control store, with expiry and revocation) mapped to
  an active registry user or agent of one tenant. An agent token
  authenticates only the agent, under its owner's current ceiling; owners
  issue and revoke their agents' tokens, and suspending, retiring or
  expiring an agent, or suspending its owner, fails its next request. Every route, including unknown
  ones, rejects unauthenticated requests with one byte-identical `401`.
  Revocation, expiry and registry suspension apply on the next request.
- **Tenants.** One store per tenant (its own engine database and audit
  database), with the tenant taken from the token, never the request. A
  request of one tenant never opens another tenant's store.
- **Audit.** A per-tenant append-only, hash-chained audit log of every
  authenticated request: principal, operation, outcome, graphs, revisions,
  the digest of the response, and the facts read (from `fact_provenance`) or
  written. `GET /v1/audit/facts` answers "who read fact F" and "who wrote
  fact F". Responses are withheld when their entry cannot be written.
- **Visible manifest.** `GET /v1/manifest` serves the caller's
  `weave-visible/1` manifest (weave-language LB1): the graphs it can
  discover with its permission and branches, and exactly the groups it may
  name in `reader_groups` (`Engine::registry_nameable_groups`). Language CI
  runs LB1's `check_permissions.py` and `check_agent_repair.py` against the
  engine.
- **No ID probing.** Graphs get engine-assigned IDs unless a tenant
  administrator chooses one, and non-administrators may assert only record
  IDs the service minted for them, which closes the `E_CONFLICT` occupancy
  channel for them.
- **Limits and logs.** Bounded bodies, request heads, connections, per-tenant
  queues, programs, minted IDs and audit pages; JSON request logs without
  tokens, bodies or graph IDs (tested).

- **OIDC (part 2).** ID and access tokens from operator-configured issuers
  (RS256, ES256, EdDSA; keys from a configured JWKS, never fetched), mapped
  to registry users through administrator-managed `(iss, sub)` bindings; the
  tenant comes from the `(issuer, audience)` configuration. Group claims are
  never live; an audited import turns an IdP snapshot into registry
  memberships.
- **Signed admission (part 2).** `POST /v1/admit` runs a program signed with
  a `weave-policy` capability chain, under the tenant's own audience, with a
  root derived from the registry and keys bound to the agent's ownership
  chain; the program then runs as the agent under the registry ceiling.
  Replays fail `E_REPLAY`; key revocation, suspension and owner permission
  loss apply on the next request.

- **Load (part 3).** 50 concurrent TLS clients on the full authentication
  path: one tenant serves about 710 requests/s at 20% writes (p99 about
  90 ms), the same with OIDC; five tenants serve 2,300 requests/s. A tenant
  runs one request at a time, so per-tenant throughput does not grow with
  clients ([report](benchmarks/README.md#authenticated-service-b3)). A CI
  smoke test runs the same harness with 50 clients.

B3 is complete.

## Derived-fact leakage protection (B2)

B2 ([#5](https://github.com/weave-graph/weave-engine/issues/5),
[ADR-0008](architecture/ADR-0008-write-time-propagation-and-release-policies.md),
accepted) is complete:

- **Noninterference harness.** Generated pairs of stores that differ only in
  facts the readers under test cannot see, in local and registry stores
  (literal readers, `reader_groups`, nested groups, hidden registry state and
  an undiscoverable graph). Every `GraphExpression` operator (28 kinds) and
  every science analysis (5) is checked: values, counts, ranks,
  explanations, coverage, diagnostics, `fact_provenance` and errors must be
  identical for each reader under test, after revision identity is
  normalized (ADR-0008 section 7). Each operator must also be reached by the
  hidden facts. Cases per operator: 40 in the debug suite, 500 in the PR CI
  job and 10,000 in the scheduled full-count job
  ([measured counts](measurements/2026-10-20-noninterference.json)).
- **Declared channels.** `recorded_query`, `accepted_graph` and
  `accepted_history` answer one bit about hidden records in the revision
  they select; revision identity, work budgets, timing and size, write-path
  ID occupancy and disclosed writer identities are listed in
  [SECURITY.md](../SECURITY.md).
- **Fixes.** `reason` explanations no longer choose among equivalent
  witnesses in an order that follows revision IDs, and proof conjunctions
  are flat and canonically ordered, so hidden records no longer change a
  visible witness or push a result over the carrier depth budget.
- **Writes never compare with hidden versions (every protocol version).** A
  live version the writer cannot read is absent for `correct`, `retract` and
  `expected`, which fail exactly as for a missing fact. An `assert` of its ID
  fails with `E_CONFLICT` "fact ID is not available", whatever its kind or
  content, and `reconcile_facts` plans it like an absent key. In a registry
  store, a reference to a record the writer cannot read fails as a reference
  to a missing record; a local store resolves references against every live
  record, and its writers may act on versions they recorded themselves
  (trusted importers). Generated writes in local and registry stores give
  identical outcomes with and without hidden facts, except for the two
  declared bits: occupancy of an asserted ID, and a hidden dependent
  refusing a retraction with the same `E_DEPENDENT_FACT` as a visible one.
- **Write-time propagation (protocol 1.2.0).** A correction that would make
  a fact readable by more principals fails with `E_READERS_WIDENED` unless it
  sets `widen_readers: true`. A fact whose provenance evidence cites a
  restricted record the writer can see must carry it as an influence carrier
  or be readable only by a subset of its readers and groups, else
  `E_DERIVED_READERS`. Both compare lists, never membership; 1.1.0 and older
  programs keep their published meaning. A derived conjunction of group
  clauses keeps its first clause exactly and enforces the rest through its
  carriers, instead of the B1 meet; disjoint literal readers derive
  `weave:no-reader`, never a public record. Error codes for clients are in the
  [1.2 note](contract/v1.2/README.md#error-codes-for-clients).
- **Agents.** Registry worlds of the harness add an agent of `alice` as a
  reader under test; an agent never sees more than its owner, and narrowing
  the owner only removes records from the agent's output. Revocation reaches
  compiled views, change sets and rankings on the next request.

- **Persisted analytics and rankings.** The science operation `persist`
  (`Engine::persist_result`) writes an analysis or a `nearest` ranking back as
  attachment facts on the analysed nodes. The engine computes their readers:
  every input node and edge is an influence carrier, and the own clause lies
  within every ranked embedding's clause, so a persisted value is readable
  only by readers of every input, also after revocation. An embedding citing
  restricted text is no broader than the text.
- **Release policies.** `Engine::install_release_policy` (tenant
  administrators; the trusted host without a registry), `Engine::release` and
  `Engine::release_audit`. A policy releases the number of distinct entities
  in a graph, evaluated under a named evaluator's authority, to its audience
  only when it is at least `min_k`, and withholds it otherwise. Every
  evaluation is audited in `weave:release`. Policies have one kind,
  `aggregate_count`.

## Agent interface (B4)

B4 ([#40](https://github.com/weave-graph/weave-engine/issues/40)) is in
progress. Part 1 serves the agent interface as MCP over the authenticated
service ([guide](AGENT_INTERFACE.md)):

- **Endpoint.** `POST /v1/mcp`, MCP Streamable HTTP without sessions or
  event streams: one JSON-RPC message per request, authenticated and audited
  like every service request. The principal comes from the credential and
  the tenant from the control store; revocation, suspension and membership
  changes apply to the next tool call.
- **Tools.** `assert_fact`, `retract_fact`, `query`, `expand`, `as_of`,
  `what_changed`, `hybrid_search`, `explain` and `describe_schema`. Every
  tool runs one program through the same path as `POST /v1/execute`; read
  tools return exactly its result. The tool layer is the `weave-mcp`
  library, shared with the read-only local preview.
- **Writes.** `assert_fact` mints every record ID, requires a stated method
  (model and version, rule, import or human; never `unspecified`, never
  `human` from an agent), evidence references and an explicit visibility,
  and the engine records the writer, `on_behalf_of` and recorded time.
- **Schema.** `describe_schema` lists the caller's graphs and nameable
  groups, and summarizes one graph's visible predicates, node spaces, types,
  property and attachment keys and embedding spaces, with runnable example
  calls.
- **Data/code boundary.** Static tool text; store content only under
  `result` in an untrusted-data envelope; closed argument types without
  writer, grant, ID or record-reader fields; `Origin` headers refused.

Evidence: `crates/weave-service/tests/agent_mcp.rs`, `agent_hidden.rs`
(paired worlds for every tool, writes included, for an agent of another
team and one without access) and `agent_injection.rs`.

**Not yet:** per-agent quotas and rate limits, the Python SDK and a compile
tool. Derived-fact restriction follows the engine's current propagation
(B2).

## Optional review (B5)

B5 ([#12](https://github.com/weave-graph/weave-engine/issues/12)) is in
progress. Part 1 is the engine side ([guide](REVIEW.md)):

- **Modes.** Each graph of a registry store is `direct` (the default:
  writes publish immediately) or `reviewed`: every direct write path fails
  with `E_REVIEW_REQUIRED` or has no write authority, and principals with
  Write (agents included) propose fact changes instead.
- **Reviewers.** A policy names users or groups and a threshold. Eligible
  reviewers are active users with graph Read, never agents and never the
  proposer. A vote binds the exact version digest (`E_REVIEW_CHANGED`
  otherwise); a rejection decides at once; the threshold of approvals
  publishes the proposal as its proposer, atomically, or fails with
  `E_REVIEW_STALE` and records nothing.
- **Provenance.** Accepted facts keep `writer` and `on_behalf_of` and add
  `review` (proposal, decision, approving reviewers, times) in protocol
  1.3.0 results.
- **Visibility.** Proposals live outside every graph: pending, rejected and
  withdrawn proposals never reach a graph read. A reviewer sees a proposal
  only within its own read scope, which is otherwise identical to a missing
  one; the proposer and its owner chain see their own.
- **Policy changes.** Tightening a direct graph is immediate for an
  Administer holder; any change of a reviewed graph is a proposal decided by
  the policy it replaces, so a policy never authorizes itself.
- **Store 33.** Five append-only review tables; store 32 upgrades by creating
  them, and store-32 binaries refuse store 33
  (`check_review_store_migration.py`, against the B2 binary).

Evidence: `crates/weave-engine/tests/review.rs`,
`tests/review_store_migration.rs`, and the noninterference harness, whose
registry worlds now include a reviewed graph with an accepted proposal in
both stores and pending, rejected and withdrawn proposals of public facts
in the store with hidden input
(`pending_proposals_would_change_reader_outputs_once_accepted` shows they
are not vacuous).

**Not yet:** the service endpoints and MCP tools to propose, list and review
(part 2). Proposals do not expire, and decided ones are kept without
retention.

## Verified evidence

The implementation at `3bbd9d87f708c7aa3d5a2fb2afccb01be0789340` was merged at
`f0deca06f78448f652be5468462b10f5561e2566`. All three jobs in the
[native CI run](https://github.com/weave-graph/weave-engine/actions/runs/38032492105)
passed on that implementation commit: workspace Rust tests, strict Clippy,
formatting, installed Python tests and independent scientific acceptance on
Linux, macOS and Windows. This establishes the named native CLI/SDK profile;
application and device acceptance have separate gates.

The local integration recorded 615 Rust checks with no failures or ignored
checks, 25 installed Python checks including four native integration cases, and
16 grouped independent scientific acceptance cases. The installed example
completed correction, export, vector search, PageRank and old-snapshot replay.

Release benchmarks on the recorded Apple M4 Max host passed 1k and 10k input-node
workloads. At 10k, query/analysis medians were 172–194 ms, including the complete
query and provenance JSON. Read [validation and measurement methods](SCIENCE_VALIDATION.md)
and the [raw report index](benchmarks/README.md) for workload shape, binary/source
identities, sample counts and limitations. These measurements describe one host;
production capacity requires separate evidence.

## Boundaries that affect experiments

- Analytics use bounded in-memory selected graphs and exhaustive vector search.
  No approximate index, embedding generator or distributed analytics service is supplied.
- Analysis rejects partial input by default. Explicit partial analysis retains
  coverage and diagnostics; unavailable knowledge does not become false evidence.
- Old snapshots are read under current authority. Exact replay requires retained
  revisions and unchanged selected input, including live metadata and policy.
  Some service/view expressions are recordable but are not eligible for exact SDK replay.
- The native host supplies actor identity and graph write grants. It is a local
  trusted interface. Remote authentication is `weave-service` ([guide](SERVICE.md));
  CPU/RSS process isolation is not provided.
- Browser/mobile applications, portable source actors, distributed networking,
  approximate indexes and the full formal/cryptographic assurance program remain
  outside this delivery's acceptance scope.

See [experiment concepts](SCIENCE_CONCEPTS.md) and the interface guides for the
precise temporal, identity, replay and resource semantics.

## Fit against the three goals (audit of 17 October 2026)

The [state audit](AUDIT_2026-10-17.md) compares the code with the goals in
[VISION.md](../VISION.md). These are the main gaps the [phased roadmap](ROADMAP.md)
addresses:

- **Storage (Phase A).** Addressed by A2: fact-level writes, node valid time,
  per-fact provenance, indexed reads and no per-graph object cap (see
  [fact-level writes](#fact-level-writes-a2)). Snapshot commits remain bounded
  by the 16 MiB request limit.
- **Queries (Phase A).** Addressed by A3: k-hop `expand`, indexed `as_of`
  selection and `changes` change sets with a pull feed (see
  [traversal](#traversal-time-travel-and-change-sets-a3)).
- **Embeddings (Phase A).** Addressed by A4: temporal, permissioned
  embedding facts in typed spaces, a streaming exact `nearest` operator in
  the plan algebra, candidate relations, and Python helpers (see
  [embeddings](#embeddings-and-hybrid-retrieval-a4)). Approximate indexes
  remain for C5.


- **Sharing (Phase B).** `weave-service` (B3 part 1) serves the engine to
  authenticated users and agents of isolated tenants with API tokens and an
  audit log. Users, groups, agents and graph ACLs exist (B1); OIDC and signed
  agent capabilities do not yet, and leakage protection is enforced at read
  time, at write time for declared derivations and corrections (protocol
  1.2.0), on persisted analytics and rankings, and declassified only through
  audited release policies (B2).
- **Scale (Phase C).** There is no telemetry ingestion and no approximate vector
  index. Measured workloads are 10k–100k objects.

## Earlier milestones

The runtime also contains earlier adapter, capsule, governed-history, geometry,
clustering, C/Swift host and experimental browser functionality. These features
have their own finite profiles and verification records; availability through
an engine plan does not establish every original architecture requirement.

[Historical status](STATUS_HISTORY.md) preserves the original milestone ledger,
including checkpoint-specific test counts and pending notes. Use the
[documentation index](README.md) to find a feature's reference and evidence.
