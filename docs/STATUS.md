# Current status

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
| Engine contract | 0.22.0; existing graph, temporal, metadata and authorization semantics, plus fact-level writes, node valid time and per-fact provenance ([0.22](contract/v0.22/README.md)). The core subset is specified in [core contract v1](contract/core-v1.md) |
| Native storage | Store 30; persistent local SQLite graph revisions and history, plus the fact log ([ADR-0004](architecture/ADR-0004-fact-log-and-storage-backend.md)) |
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

- **Golden fixtures.** 34 golden fixtures in
  `crates/weave-contract/fixtures/core-v1/` are pinned by a SHA-256 manifest.
  The contract harness checks decoding and canonical re-encoding. The engine
  harness checks execution outcomes. Both fail on drift.
- **Stable decode error codes.** `weave_contract::decode_program` reports
  `E_VERSION`, `E_UNKNOWN_FIELD`, `E_UNKNOWN_VARIANT`, `E_DECODE` and
  `E_BUDGET`, and checks the version before the structure. The
  `weave-engine run` CLI reports these codes and now rejects duplicate JSON
  keys; it previously reported `E_INPUT` for every decode failure. Plans over
  16 MiB still report `E_INPUT`, and the science envelope keeps `E_INPUT`.
- **Not yet active.** Embedding facts and spaces
  ([ADR-0006](architecture/ADR-0006-embedding-facts-and-spaces.md)) are
  accepted. Their records exist in `weave-contract`, but no program carries
  them. Their wire fields are reserved until A4.

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
- **Scenario.** The parts of the Phase A exit scenario that the current
  operators reach run on it in CI at full size (`hardware-fleet-5k`): import,
  zero-fact re-import, 1,000 single-update writes, question 2 in both forms
  against an independent oracle, reader restrictions and reopening.

**Limits.** Imports above 100,000 changes are written in several revisions.
Rows missing from an export are not retracted. Recorded-time selection
requires a reader who can see the whole revision (ADR-0002), so restricted
readers pin revisions instead. Series overlap is filtered after an indexed
read, not indexed itself. Embeddings for maintenance notes are A4.

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
  trusted interface; CPU/RSS process isolation and remote authentication need a host design.
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
- **Queries (Phase A).** There is no k-hop traversal operator. "What changed" is
  computed by diffing two results; there is no stored change feed.
- **Embeddings (Phase A).** Science kNN uses an untyped `vector` node property
  without time awareness and is not part of the plan algebra. Typed embedding
  spaces support only pairwise distance.
- **Sharing (Phase B).** There is no server, no authentication on the main API,
  no groups or agent identities, and leakage protection is enforced at read time
  only.
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
