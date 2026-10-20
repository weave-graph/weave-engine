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

**Phase A is complete** ([1.0.0 release notes](CHANGELOG.md)). The shared
core is released as protocol **1.0.0**, which freezes
[core contract v1](docs/contract/core-v1.md). The engine has 758 Rust tests,
crash-injection and migration checks, fuzz smoke runs, and CI on Linux, macOS
and Windows. That CI includes the whole
[Phase A exit scenario](docs/PHASE_A_EXIT.md) on a 5,000-unit fleet. The
optional [Weave language](https://github.com/weave-graph/weave-language) has
reached Phase A parity: each of the five exit questions fits in at most 15
lines of `.weave`, and engine CI runs all five. Phase B (multi-user,
permissions and agents) is in progress: the principal registry, the
authenticated service and the agent interface have landed. Phase C
(telemetry scale) is not started.

| Works today | Not yet (roadmap phase) |
|---|---|
| Immutable, durable graph revisions in SQLite with incremental fact-level writes; valid-time and recorded-time ("as known at") selection | |
| k-hop traversal (`expand`), as-of reads and "what changed" change sets with a pull feed (`changes`) | Write-time leakage protection for derived facts, agent quotas, optional review (Phase B) |
| Per-record read permissions, re-checked through derived results | High-volume telemetry ingestion, approximate vector index (Phase C) |
| Joins, rules, metadata graphs, per-fact provenance and explanations | |
| Embeddings as temporal, permissioned facts in typed spaces; exact `nearest` composed with traversal; similarity candidate relations with provenance ([guide](docs/EMBEDDINGS.md)) | |
| Exact vector neighbors and graph analytics (PageRank, components, paths) from Python, JSON or Rust | |
| A read-only local MCP preview for agents (`weave-mcp`: query, expand, as-of, what changed, hybrid search, explain) as a fixed reader ([guide](docs/MCP_PREVIEW.md)) | |
| Users, groups, agents and graph ACLs; an authenticated multi-tenant service with an audit log ([guide](docs/SERVICE.md)) | |
| An MCP agent interface on the service: agents assert, retract, query, time-travel, search and explain with their own identity and provenance ([guide](docs/AGENT_INTERFACE.md)) | |

The supported profile is described in [docs/STATUS.md](docs/STATUS.md): science
**0.1.0**, contract **1.1.0**, store **32**. The CLI and the science
interface run locally and trust their caller; `weave-service` is the
authenticated network interface (see [SECURITY.md](SECURITY.md)).

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

## Quickstart: a hardware fleet and hybrid retrieval

These five commands generate a synthetic fleet, import it with the
[hardware model](docs/HARDWARE_MODEL.md), apply 100 field updates (each one a
fact write), embed the maintenance notes and run hybrid retrieval over the
graph. Run them from a checkout on macOS or Linux, with the same prerequisites
as above:

```sh
cargo build --release --locked -p weave-hardware -p weave-science
target/release/weave-hardware generate --out fleet --units 200 --updates 100
target/release/weave-hardware import --db fleet.db --actor etl-importer --graph fleet --csv fleet/base
target/release/weave-hardware apply-updates --db fleet.db --actor etl-importer --graph fleet --updates fleet/updates.jsonl
python3 examples/hardware/hybrid_quickstart.py --science target/release/weave-science --db fleet.db --fleet fleet
```

On Windows, use `target\release\weave-hardware.exe`,
`target\release\weave-science.exe` and `py -3`.

The last command writes one embedding fact per maintenance note, incident and
unit. Each fact keeps its row's readers. The command then answers two
questions as `fleet-analyst`:

- Which maintenance notes are similar to the latest incident, within two hops
  of the affected unit and valid when the incident opened? This is one
  `nearest` plan over an `expand` neighborhood.
- Which two units are similar but not linked in the graph? This is one
  `similar_candidates` plan.

Each hit is printed with its source: the note text and the writer and method
recorded for its embedding. Add `--reader fleet-admin` to see the
customer-restricted units that the analyst cannot. The script embeds with a
small word-hashing function so it needs no model or network. Weave stores
and ranks the vectors you supply; it does not generate them. CI runs these
steps on Linux, macOS and Windows. The same questions are also available to
agents through the [MCP preview](docs/MCP_PREVIEW.md).

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
