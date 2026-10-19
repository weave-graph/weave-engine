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
| Engine contract | 0.21.0; existing graph, temporal, metadata and authorization semantics. The core subset is specified in [core contract v1](contract/core-v1.md) |
| Native storage | Store 29; persistent local SQLite graph revisions and history |
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
- **Not yet active.** Per-fact provenance
  ([ADR-0005](architecture/ADR-0005-per-fact-provenance.md)) and embedding
  facts and spaces
  ([ADR-0006](architecture/ADR-0006-embedding-facts-and-spaces.md)) are
  accepted. Their records exist in `weave-contract`, but no 0.21.0 program
  carries them. Their wire fields are reserved until A2 and A4.
- **Language CI.** Language CI does not run the fixtures yet; that follows in
  LA0.

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

- **Storage (Phase A).** Each commit stores a full graph snapshot, and there is
  no fact-level write. Graphs are capped at 100,000 nodes or edges and 16 MiB per
  revision. Nodes have no valid time. Provenance records the writer per commit,
  not per fact.
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
