# Weave Engine

A native engine for experiments with temporal knowledge graphs. Store immutable
graph revisions in SQLite, select what was valid or known at a particular time,
derive graph results with provenance, and run graph or vector analyses from
Python, JSON or Rust.

The supported [native experiment profile](docs/STATUS.md) uses science **0.1.0**,
contract **0.21.0** and store **29**. Browser and mobile applications are deferred;
experiments run locally without an app, compiler or server. The broader
[white-paper roadmap](docs/IMPLEMENTATION_PLAN.md) remains open.

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

For contributors, start with [CONTRIBUTING.md](CONTRIBUTING.md). The companion
[Weave language](https://github.com/weave-graph/weave-language) optionally compiles
source into the shared JSON contract; this repository owns the runtime and
canonical [`weave-contract`](crates/weave-contract/src/lib.rs) types.

The CLI is a trusted local host. Actor selection and graph write grants come
from its caller; request JSON cannot authenticate a remote user or grant itself
authority. See [SECURITY.md](SECURITY.md) for the security boundary.

MIT licensed. Original papers and their unchanged hashes are documented in
[source provenance](docs/SOURCES.md).
