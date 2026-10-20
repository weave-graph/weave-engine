# Weave Engine

**Weave is an open-source temporal knowledge graph for hardware-heavy industries
and the AI agents that work in them.**

Teams that build and operate hardware keep telemetry, hardware metadata (parts,
serials, firmware, configurations, maintenance) and user metadata (owners,
operators, customers, teams) in separate systems. Weave relates them in one
graph where every fact has a valid time and a recorded time. You can ask how
things were related at any moment, what was known at any moment, and what
changed in between.

Weave aims to serve three goals on one core:

1. **Temporal graph database** for telemetry, hardware metadata and user
   metadata, with time travel and "what changed" queries.
2. **Shared fact store for AI agents** across people and teams. Agents write and
   query facts directly, with permissions and provenance, and derived results
   never leak what a reader may not see.
3. **Time-aware hybrid RAG.** Embeddings are linked to the graph, so vector search
   surfaces relations the graph lacks and graph traversal selects what to search.

Read the [vision](VISION.md) and the [phased roadmap](docs/ROADMAP.md).

## Status: what works today and what does not yet

The engine has been tested extensively: 615 Rust tests, crash-injection and
migration checks, and CI on Linux, macOS and Windows. It is **pre-release**.

| Works today | Not yet (roadmap phase) |
|---|---|
| Immutable, durable graph revisions in SQLite with incremental fact-level writes; valid-time and recorded-time ("as known at") selection | Python helpers and tutorial for hybrid retrieval (A4, next part) |
| k-hop traversal (`expand`), as-of reads and "what changed" change sets with a pull feed (`changes`) | Multi-user server, authentication, groups, agent interface (Phase B) |
| Per-record read permissions, re-checked through derived results | High-volume telemetry ingestion, approximate vector index (Phase C) |
| Joins, rules, metadata graphs, per-fact provenance and explanations | |
| Embeddings as temporal, permissioned facts in typed spaces; exact `nearest` composed with traversal; similarity candidate relations with provenance ([guide](docs/EMBEDDINGS.md)) | |
| Exact vector neighbors and graph analytics (PageRank, components, paths) from Python, JSON or Rust | |
| A read-only local MCP preview for agents (`weave-mcp`: query, expand, as-of, what changed, hybrid search, explain) as a fixed reader ([guide](docs/MCP_PREVIEW.md)) | |

The supported profile is described in [docs/STATUS.md](docs/STATUS.md): science
**0.1.0**, contract **0.24.0**, store **31**. It runs locally. There is no
server, and the CLI trusts its caller (see [SECURITY.md](SECURITY.md)).

## Run your first experiment

Start in a checkout of this repository. Building requires stable Rust with Cargo
and your platform's C compiler/linker for bundled SQLite: Xcode Command Line
Tools on macOS, a C build toolchain on Linux, or Visual Studio C++ Build Tools
for Rust's MSVC target on Windows. The client requires Python 3.10 or newer and
has no mandatory third-party runtime dependencies.

On macOS or Linux:

```sh
cargo build --release --locked -p weave-science
python3 -m venv .venv
.venv/bin/python -m pip install ./python
.venv/bin/python examples/science/temporal_vectors.py \
  --binary target/release/weave-science --output experiment-output
```

On Windows, in PowerShell:

```powershell
cargo build --release --locked -p weave-science
py -3 -m venv .venv
.venv\Scripts\python.exe -m pip install ./python
.venv\Scripts\python.exe examples/science/temporal_vectors.py `
  --binary target/release/weave-science.exe --output experiment-output
```

The example imports four nodes with toy vectors and three temporal edges. At
valid time `8`, a late correction changes weak connectivity from **two components
to three**. It then reopens the database and replays the original experiment
against its old revision. It also computes exact cosine neighbors and PageRank.

Read `experiment-output/report.json` for the summary. The directory contains the
SQLite database, full query JSON, CSV tables and saved experiment records. Use a
new output directory for each run; the example rejects an existing database.
The vectors are supplied coordinates; the engine does not train a model or
generate embeddings.

Continue with the [Python tutorial](docs/PYTHON_SCIENCE.md), including notebook
setup, imports, temporal joins, export and replay. For a Python-free interface,
use the [native JSON walkthrough and API reference](docs/SCIENCE_INTERFACE.md).

## What you can experiment with

| Task | Interface and behavior |
|---|---|
| Import and correct datasets | Node/edge records, typed CSV or optional pandas frames; full immutable snapshots and compare-and-swap updates |
| Select temporal data | Exact revisions, branch heads, half-open valid-time intervals and replica-local recorded checkpoints |
| Derive graph values | Existing engine plans for joins, union, diff, windows, rules, metadata, explanations, clustering and geometry |
| Analyze topology | Degree, weak/strong components, unweighted shortest paths and PageRank on authorized positive multigraph edges |
| Retrieve vectors | Exhaustive cosine or Euclidean neighbors in one explicit space, retaining manifestation and entity IDs |
| Record experiments | Parameters, selected input, provenance, coverage, versions and executable hash; replay under current authority |
| Model a hardware fleet | `weave-hardware`: reference schema, series references, idempotent CSV/JSON importers and a seeded 5,000-unit fleet ([hardware model](docs/HARDWARE_MODEL.md)) |

Read [the experiment concepts](docs/SCIENCE_CONCEPTS.md) before interpreting
results. Filtering edge validity during analysis retains selected isolated nodes,
while filtering a core query can remove them. A saved root revision does not
freeze live metadata or bypass current policy. Replay reports input drift explicitly.

Analytics materialize bounded inputs in memory. The
[validation guide](docs/SCIENCE_VALIDATION.md) describes independent correctness
checks, measured 1k/10k workloads and practical limits. Normal native CI covers
Linux, macOS and Windows; benchmark numbers describe their recorded host.

## Documentation and development

The [documentation index](docs/README.md) separates tutorials, references,
architecture, evidence and historical proposals. [Current status](docs/STATUS.md)
identifies supported behavior and remaining work.

For contributors and AI agents, start with [AGENTS.md](AGENTS.md) and
[CONTRIBUTING.md](CONTRIBUTING.md). The companion
[Weave language](https://github.com/weave-graph/weave-language) optionally compiles
source into the shared JSON contract; this repository owns the runtime and
canonical [`weave-contract`](crates/weave-contract/src/lib.rs) types.

The CLI is a trusted local host. Actor selection and graph write grants come
from its caller; request JSON cannot authenticate a remote user or grant itself
authority. See [SECURITY.md](SECURITY.md) for the security boundary.

MIT licensed. Original papers and their unchanged hashes are documented in
[source provenance](docs/SOURCES.md).
