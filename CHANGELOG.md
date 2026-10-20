# Changelog

Release notes for tagged releases of weave-engine. Phases and epics are
defined in [docs/ROADMAP.md](docs/ROADMAP.md). The supported profile is in
[docs/STATUS.md](docs/STATUS.md).

## 1.0.0: Phase A, the shared core (2026-10-20)

Phase A delivers a local, embeddable temporal graph database with per-fact
provenance and a time-aware embeddings layer, usable from Rust, JSON,
Python, the optional Weave language and a read-only MCP preview. It closes
A6 ([#16](https://github.com/weave-graph/weave-engine/issues/16)).

### Versions

| Identifier | Version |
|---|---|
| Protocol (`Program.version`), `weave-contract` and `weave-engine` crates | **1.0.0** |
| Store schema (`STORAGE_VERSION`) | 31 |
| Science API (`weave-science`, Python `weave_science`) | 0.1.0 |
| MCP preview (`weave-mcp`) | 0.1.0 |
| Core profile | `core-v1` |
| Capsule format | 0.4 |

Protocol 1.0.0 freezes [core contract v1](docs/contract/core-v1.md) with the
0.24.0 feature set ([1.0 note](docs/contract/v1.0/README.md)). It adds no wire
feature. Programs declaring 0.1.0 through 0.24.0 stay accepted with their
published semantics, and stored data and revision IDs are unchanged. From
1.0.0 on, MINOR versions are additive and MAJOR versions are breaking (core
v1 §11). Feature gates now read one version table
(`weave_contract::versions`), so a future version changes that table and no
gate.

### What Phase A delivers

- **A1, core contract v1.** One normative specification of the core subset,
  with golden fixtures shared with the language, stable decode error codes, a
  compatibility policy, and ADR-0005 (provenance) and ADR-0006 (embeddings).
- **A2, incremental bitemporal fact store (0.22.0, store 30).** The `write`
  command asserts, corrects and retracts individual facts in O(change) fact
  revisions. It adds node valid time and engine-recorded per-fact provenance,
  indexed reads, and a `FactBackend` storage trait with SQLite and in-memory
  backends (ADR-0004). The 100k-object per-graph cap is removed and 1M-fact
  graphs run in CI. A store 29 is upgraded in one crash-safe transaction.
- **A3, time travel, traversal and change sets (0.23.0).** It adds k-hop
  `expand`, `as_of` reads at a recorded time, and `changes` with a pull
  feed. Every cited fact is explained in `fact_provenance`, coverage is
  explicit, and paired tests check that hidden input changes nothing.
- **A4, embeddings and hybrid retrieval (0.24.0, store 31).** Embedding
  spaces and embeddings are temporal, permissioned facts. It adds exact
  `nearest`, which streams in O(k) memory under its own scan budget, composes
  with traversal (`candidates`, `seeds_from`) and, with candidates, costs in
  proportion to the candidates. `similar_candidates` proposes candidate
  relations with provenance, which are never written. A migration from the
  untyped `vector` property must be run explicitly. Python helpers and a
  tutorial are included.
- **A5, hardware reference model.** The `weave-hardware` crate has the
  schema, `SeriesRef`, idempotent CSV and JSON importers through fact
  writes, and a deterministic generator for a 5,000-unit fleet.
- **A7, read-only MCP preview.** `weave-mcp` serves `query`, `expand`,
  `as_of`, `what_changed`, `hybrid_search` and `explain` over stdio as a
  fixed reader.
- **A6, acceptance:**
  - **Exit scenario.** The whole [Phase A exit scenario](docs/PHASE_A_EXIT.md)
    (5,000 units, 1,000 updates, questions 1 to 5, explanations, reopening, and
    MCP questions 2 to 4) passes in CI on Linux, macOS and Windows.
  - **Fuzzing.** cargo-fuzz targets cover fact writes, the A3 and A4 plans
    and MCP JSON-RPC input.
  - **Migration.** A populated store 29 reaches store 31, directly and
    through the store-30 binary, with process kills at every step.
  - **Benchmarks.** [Baseline benchmarks](docs/benchmarks/README.md) are
    published.
  - **Quickstart.** A five-command [README quickstart](README.md#quickstart-a-hardware-fleet-and-hybrid-retrieval)
    covers the hardware model and hybrid retrieval.
- **Language parity (LA1, weave-language#3).** The compiler emits 0.24.0 and
  has syntax for time travel, traversal, change sets, fact writes,
  `nearest` and `similar_candidates`. Each of the five exit questions fits in
  at most 15 lines of `.weave`, and engine CI runs all five.

### Measured (one host, not capacity promises)

These were measured on one AMD Ryzen 9 7950X Linux host, optimized build, on
an on-disk WAL store. They describe what was measured; they are not service
levels.

| Operation | 100k facts | 1M facts |
|---|---:|---:|
| single-fact commit, median | 0.84 ms | 0.85 ms |
| `expand`, 2 hops | 1.6 ms | 1.7 ms |
| `changes`, one fact | 0.45 ms | 0.47 ms |

`nearest` k=10 within a 2-hop neighborhood takes 2.7 ms at both 20k and 200k
vectors. Exit-scenario timings on all three CI operating systems are in the
benchmark index.

### Limits

- **Local and single-user.** There is no server, no authentication and no
  tenant isolation. The CLI and MCP preview trust their caller, and the MCP
  preview is read-only ([SECURITY.md](SECURITY.md)).
- **Permissions are per-record reader lists.** Groups, graph-level ACLs and
  agent identities are B1. Leakage protection for derived facts covers
  results, counts, ranks and explanations as tested. Influence through
  timing, scan budgets (hidden embeddings count toward `MAX_SCAN_COMPONENTS`)
  and revision IDs (which change with hidden writes) is documented and left
  to B2.
- **Language source writes cannot set `readers` until LB1.** A source
  `correct` of a restricted fact publishes it. Write restricted facts
  through JSON or Rust.
- **Exact vector search only.** A whole-space scan costs about 28 µs per
  32-dimensional vector, mostly decoding JSON-stored vectors. 100,000 vectors
  of 1,536 dimensions take about 7 s and 2.1 GB. Approximate indexes are C5,
  and a binary vector encoding is C4.
- **Large fact revisions are re-verified on read.** A read at a head written
  as one 50,000-change batch costs 0.2 to 0.5 s, so write in batches of a few
  thousand. Caching the verification is C4 (#15).
- **Snapshot commits** stay bounded at 16 MiB per input. Use `write` for
  large graphs.
- **Replay of saved experiment records** requires the same engine contract.
  Records saved against a 0.24.0 engine report a contract mismatch on exact
  replay under 1.0.0, while their programs still run unchanged.
- **Native profile only.** The supported profile is native Linux, macOS and
  Windows. The browser and mobile hosts are experimental, and the browser
  workflow runs on manual dispatch only (E10 is deferred).

### Deferred scope

- **Phase B:** principals, groups and agent identities (B1); derived-fact
  leakage protection (B2); an authenticated service (B3); the write-capable
  agent interface (B4); optional review (B5).
- **Phase C:** telemetry ingestion and series linkage, push change feeds and
  connectors, incremental derivations, storage scale, retention and erasure,
  backup and an approximate vector index.
- **Not scheduled:** capsules and mounts (E08), offline branches and peer
  sync (E09), browser and mobile (E10), clustering and zoom (E13), and the
  external-effect broker. They keep their tests.

### For the language repository: contract files to vendor for 1.0.0

`vendor/weave-contract/` mirrors `crates/weave-contract/` byte for byte. The
1.0.0 update is:

- **new:** `src/versions.rs` (the version table and `version_at_least`);
  `fixtures/core-v1/positive/protocol-1-0-0.json`;
- **modified:** `Cargo.toml` (1.0.0); `src/lib.rs` (`VERSION`, the `versions`
  module and its re-exports); `src/wire.rs` (`SUPPORTED_VERSIONS` and
  `is_supported_version` moved to `versions` and re-exported);
  `src/handler_registration.rs` and `src/view_registration.rs` (template
  protocol is `VERSION`; protocol gates use `version_at_least`);
  `tests/core_v1_fixtures.rs`; `fixtures/core-v1/manifest.json` (protocol
  1.0.0 and hashes); `fixtures/core-v1/negative/unknown-version.json` (1.1.0)
  and `negative/unknown-version-checked-first.json` (2.0.0);
- **documentation:** `docs/contract/v1.0/README.md` (new) and
  `docs/contract/core-v1.md` (§11 and the §12 row).

The compiler may keep emitting 0.24.0 programs; the engine accepts them with
unchanged meaning. Emitting 1.0.0 is a separate language decision.
