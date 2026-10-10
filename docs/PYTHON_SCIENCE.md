# Python experiments with Weave

Use `weave_science` from scripts or notebooks to import datasets, query immutable
revisions, compare temporal graphs and run native graph/vector analyses. The
Python package has no required runtime dependencies. Rust performs persistence,
authorization, temporal selection and analysis.

For identity, time and policy terminology, see [Science concepts](SCIENCE_CONCEPTS.md).
For the JSON protocol and algorithm output schemas, see [Native science interface](SCIENCE_INTERFACE.md).

## Install

Run these commands from the repository root. Building from source requires a Rust
and native compiler toolchain; Python requires version 3.10 or newer. Use a virtual
environment so installation also works with externally managed system Python.

macOS or Linux:

```sh
cargo build --release --locked -p weave-science
python3 -m venv .venv
.venv/bin/python -m pip install ./python
export WEAVE_SCIENCE_BINARY="$PWD/target/release/weave-science"
.venv/bin/python examples/science/temporal_vectors.py --output experiment-output
```

Windows PowerShell:

```powershell
cargo build --release --locked -p weave-science
py -3 -m venv .venv
.venv\Scripts\python.exe -m pip install ./python
$env:WEAVE_SCIENCE_BINARY = "$PWD\target\release\weave-science.exe"
.venv\Scripts\python.exe examples/science/temporal_vectors.py --output experiment-output
```

Use a fresh output directory for each example run. The script creates a SQLite
database, corrects a temporal graph, searches fixed toy vectors, computes
PageRank, exports tables and replays an earlier experiment. Inspect its
`report.json`, `before.json` and `after.json` files.

With an already built binary, omit the Cargo step and set `WEAVE_SCIENCE_BINARY`
to its executable path. Alternatively pass `binary=...` to `Engine` or put
`weave-science` on `PATH`. Editable source installs can discover a repository
`target/release` or `target/debug` binary. The client never builds it implicitly.
No compiler, network service or browser is required during an experiment.

## A complete experiment

Run the following Python blocks in order, as notebook cells or one script. The
first block creates a new temporary workspace, so restarting the experiment does
not overwrite an existing database. To keep your own datasets, replace `work`
with an existing project directory.

```python
import tempfile
import time
from pathlib import Path
from weave_science import Engine, node, edge, query

work = Path(tempfile.mkdtemp(prefix="weave-experiment-"))
engine = Engine(work / "experiment.sqlite", actor="scientist",
                write_graphs=["observations"])
nodes = [
    node("a", entity_id="person-A", space_id="model-v1", vector=[1.0, 0.0]),
    node("b", entity_id="person-B", space_id="model-v1", vector=[0.8, 0.2]),
    node("c", entity_id="person-C", space_id="model-v1", vector=[0.0, 1.0]),
    node("d", entity_id="person-D", space_id="model-v1", vector=[-1.0, 0.0]),
]
edges = [
    edge("ab", "a", "b", predicate="links", valid_from=0, valid_to=10),
    edge("bc", "b", "c", predicate="links", valid_from=0, valid_to=20),
]
first = engine.import_graph("observations", nodes=nodes, edges=edges)
old = engine.query("observations", revision=first.revision)
recorded = engine.recorded_query("observations",
                                 recorded_at_ms=time.time_ns() // 1_000_000)
print(work)
print(first.revision)
```

Node `id` names a manifestation; `entity_id` names the entity and `space_id` names
its space. Distinct manifestations can share an entity ID. Vector similarity does
not merge identities. The example's vectors are supplied coordinates in one named
space; Weave does not generate embeddings. `write_graphs` grants this trusted local
host write access only to `observations`; request JSON does not add grants.

Valid time describes when a relationship holds in your dataset. These intervals
are half-open: `ab` holds at time 9 and expires at time 10. The contract uses signed
64-bit Unix-epoch milliseconds; these are small toy millisecond values, and
`valid_to=None` means unbounded. The engine compares integers without inferring
units or parsing dates. See [time conventions](SCIENCE_CONCEPTS.md#valid-time-and-recorded-knowledge)
for abstract experimental axes. Recorded time describes what this replica knew
at a system-assigned observation, also in Unix milliseconds. It is a separate axis.

Now make a late correction and measure its effect at valid time 8:

```python
before = engine.analyze(old, algorithm="components", mode="weak", valid_at=8)
edges[0]["valid_time"]["end"] = 6
second = engine.import_graph("observations", nodes=nodes, edges=edges,
                             expected_head=first.revision)
current = engine.query("observations", revision=second.revision)
after = engine.analyze(current, algorithm="components", mode="weak", valid_at=8)
assert before.analysis["components"] == [["a", "b", "c"], ["d"]]
assert after.analysis["components"] == [["a"], ["b", "c"], ["d"]]
assert engine.query("observations", revision=first.revision).graph == old.graph
```

Import commits a **full replacement snapshot**, not a row append or partial patch.
`expected_head=None` creates a new branch; updating an existing branch requires
its current revision. This compare-and-swap (CAS) check rejects stale writes with
`NativeError`, preserving concurrent changes. An unchanged snapshot with the
correct head returns `changed=False`. The old immutable revision remains queryable.
Node/edge helpers copy their inputs; editing `edges` above does not edit `old`.

Run additional analyses against the old snapshot:

```python
degree = engine.analyze(old, algorithm="degree", valid_at=8)
paths = engine.analyze(old, algorithm="shortest_paths", source="a", valid_at=8)
rank = engine.analyze(old, algorithm="pagerank", valid_at=8,
                      damping=0.85, tolerance=1e-10, max_iterations=200)
neighbors = engine.nearest(old, [1.0, 0.0], space_id="model-v1",
                           metric="cosine", k=3)
assert [row["id"] for row in neighbors.analysis["neighbors"]] == ["a", "b", "c"]
assert rank.analysis["converged"]
print(degree.analysis)
print(paths.analysis)
```

Analyses use positive directed multigraph topology: parallel assertions count
separately and self-loops remain. Shortest paths count hops; PageRank weights
transitions by edge multiplicity and reports convergence. Negative assertions
remain in the query envelope but are excluded from positive topology. Exact
vector search uses one explicit space and consistent numeric dimensions, with
cosine or Euclidean distance. Undefined cosine vectors reject.

For topology statistics that retain isolated nodes, query the unfiltered graph
and set **analysis** `valid_at`, as above. `engine.query(..., valid_at=8)` selects
matching edge endpoints and can omit isolates. Without analysis time, topology
uses the interval union, which can combine relationships from different times.
Partial inputs reject unless you deliberately set `allow_partial=True`; this
preserves partial coverage and diagnostics in the input envelope.

## Temporal joins and recorded history

Join the two-hop path in the same pinned graph, then recover the observation
captured before the correction:

```python
from weave_science import join, diff, window, explain

pinned = query("observations", revision=first.revision, predicate="links")
derived = engine.evaluate(join(pinned, pinned, output_predicate="two_hops"))
assert len(derived.edges) == 1
assert {n["entity_id"] for n in derived.nodes} == {"person-A", "person-C"}

witness = recorded.raw["recorded_observations"][0]
known_before = engine.recorded_query("observations", observer=witness["observer"],
                                     checkpoint=witness["checkpoint"], valid_at=8)
assert known_before.snapshots["observations"] == first.revision
assert len(known_before.edges) == 2

change = engine.evaluate(diff(query("observations", revision=first.revision),
                              query("observations", revision=second.revision)))
interval = engine.evaluate(window(pinned, 2, 9))
proof = engine.evaluate(explain(pinned))
```

Joins match entity-and-space endpoints and enforce temporal compatibility. Derived
manifestations have new IDs; inspect their `entity_id` and provenance to identify
source entities. `diff` produces a source-aware graph difference with membership
metadata, rather than a table-cell patch. Different source revisions can give
otherwise equal records different provenance-bearing carriers.

`evaluate` accepts an existing graph-expression dictionary or read-only Program
and returns its query result. `execute` accepts the full versioned JSON Program,
including authorized commits, and returns ordered `{"results": [...]}`. Select
query/analysis outputs with `result_index` when a Program has several results.
Rule, metadata, accepted-history, clustering and geometry expressions retain their
existing native selector and installed-state requirements; see the
[native interface](SCIENCE_INTERFACE.md).

## Results, exports and replay

A `QueryResult` returned by `Engine` carries both the native envelope and the SDK
request context needed for reuse. These are the main inspection fields:

| Field | Meaning |
|---|---|
| `graph`, `nodes`, `edges` | Copies of the selected authorized records |
| `coverage`, `diagnostics` | Complete/partial status and structured explanations |
| `provenance` | Source assertion references |
| `snapshots` | Graph-ID-to-revision map; use `raw["input_snapshots"]` for all pins, including several revisions of one graph |
| `raw` | Full native result, including metadata graphs and history witnesses |
| `pinned_program` | Source Program with unambiguous query inputs pinned to their observed revisions |

`AnalysisResult` provides `.input` (the selected `QueryResult`), `.analysis` (metrics)
and `.semantics` (algorithm conventions). Analyzing a prior result re-reads its
pinned Program through current authorization and checks that the selected input
has not changed.

```python
from weave_science import Experiment

old.export_json(work / "authorized-result.json")
old.export_json(work / "snapshot.json", graph_only=True)
node_csv, edge_csv = old.export_csv(work / "tables")
record = before.save_experiment(work / "trial.json", label="connectivity at t=8",
                                parameters={"dataset": "toy-model-v1", "seed": 17})
reopened = Engine(work / "experiment.sqlite", actor="scientist")
replayed = Experiment.load(work / "trial.json").replay(reopened)
assert replayed.analysis == before.analysis
```

The result JSON preserves the native envelope; graph-only JSON is an importable
`GraphData` snapshot. Neither contains the complete SDK request context. For
replay, use `save_experiment` and `Experiment.load`, rather than constructing a
new `QueryResult` from exported JSON. Artifacts store the original request,
algorithm configuration/semantics, exact input pins, full result, actor, versions,
executable SHA-256 and caller parameters. The example's seed is a recorded caller
parameter; these native algorithms do not consume a random seed.

Replay requires the same actor, supported protocol versions and binary by default,
and retained revisions/dependencies in the database. It verifies the full current
authorized input against the record. A different returned input raises
`ReproducibilityError`; current policy may instead reject the read with a native
error before comparison. Pinning a root revision cannot freeze a live metadata
attachment. Use pinned metadata references when later changes must not affect
your experiment.

For a deliberate build comparison, call `record.replay(engine,
require_same_binary=False)` and save a new result record. This relaxes the binary
check, not the protocol or input checks. Replay verifies the selected input; compare
returned metrics explicitly, as the assertion above does. Floating-point results
from different platforms/builds should be compared with suitable tolerances.

Ambiguous unpinned inputs and service/view expressions whose complete immutable
selection is not verified are saved with `replay_request=None` and an explicit
reason. Such records cannot be replayed automatically. Mutating replay requests
reject before execution, even in externally supplied artifacts with recomputed
hashes. The artifact hash detects edits; it is not an authorship signature or an
access grant. Rebuilding/replacing the binary requires a new `Engine` instance.

## CSV and optional notebook libraries

The SDK accepts engine-shaped dictionaries or pandas frames through
`import_graph(..., nodes=..., edges=...)`. For an arbitrary source table, specify
CSV types and map its columns to the graph schema. This example is self-contained:

```python
import csv
import tempfile
from pathlib import Path
from weave_science import Engine, read_csv, node

csv_work = Path(tempfile.mkdtemp(prefix="weave-csv-"))
source = csv_work / "measurements.csv"
with source.open("w", newline="", encoding="utf-8") as stream:
    writer = csv.writer(stream)
    writer.writerow(["id", "entity", "vector", "sample_time"])
    writer.writerow(["m1", "sensor-A", "[1.0, 2.0]", 5])
rows = read_csv(source, json_columns=["vector"], integer_columns=["sample_time"])
manifestations = [node(row["id"], entity_id=row["entity"], space_id="model-v2",
                      vector=row["vector"], properties={"sample_time": row["sample_time"]})
                  for row in rows]
csv_engine = Engine(csv_work / "measurements.sqlite", actor="scientist",
                    write_graphs=["measurements"])
csv_engine.import_graph("measurements", nodes=manifestations)
assert csv_engine.query("measurements").nodes[0]["properties"]["sample_time"] == 5
```

Engine-shaped CSVs can use `Engine.import_csv(graph_id, nodes_path, edges_path,
node_options=..., edge_options=...)`. Declare every JSON-valued column, such as
node `properties`/`readers` and edge `valid_time`/`properties`/`readers`, in
`{"json_columns": [...]}`. Nodes require `id`, `entity_id`, `space_id`; edges require
`id`, `predicate`, `from`, `to`, `valid_time`. Unspecified CSV columns remain strings;
empty typed cells become `None`. CSV exports need an explicit column schema and
cannot distinguish null from empty text. Keep the full JSON for lossless types,
metadata, provenance and coverage. Duplicate headers, malformed rows and
non-finite numeric conversions reject.

For optional pandas/NetworkX interop, install `./python[tables,networkx]` using your
virtual environment's `python -m pip`. `result.to_pandas()` returns node and edge
frames. `result.to_networkx()` returns an authorized `MultiDiGraph` with manifestation
and edge IDs; it excludes negative assertions by default. Supply engine-shaped
records on reimport, including nested JSON fields; flattening a frame is not an
implicit schema conversion.

## Notebook setup

Use the same virtual environment for the SDK and notebook kernel. From the
repository root on macOS/Linux:

```sh
.venv/bin/python -m pip install ipykernel
.venv/bin/python -m ipykernel install --user --name weave-science --display-name "Weave Science"
```

On Windows replace `.venv/bin/python` with `.venv\Scripts\python.exe`. Select
**Weave Science** as the notebook kernel. Set `WEAVE_SCIENCE_BINARY` before launching
your notebook process, or pass an absolute `binary=` path to `Engine`. A notebook
launched from an existing app may not inherit your terminal's environment.

Check the active interpreter, then run the complete experiment's Python cells:

```python
import sys
import weave_science
print(sys.executable)
print(weave_science.__file__)
```

Notebook cells share variables. Rerun the first experiment cell to get a new
database; rerunning only its import line against an existing branch will fail CAS.
The database parent must exist. Each call opens a native subprocess, so the SDK
requires a persistent database path and rejects `:memory:`.

## API, errors and verification

| Operation | Typical use |
|---|---|
| `import_graph(graph_id, data, expected_head=...)` | Commit a full native `GraphData`; alternatively provide `nodes` and `edges` |
| `query(graph_id, revision=..., valid_at=..., predicate=...)` | Select graph records; `from_id`, `to_id`, metadata depth and branch selectors are also supported |
| `recorded_query(graph_id, recorded_at_ms=...)` | Select a replica-local recorded-time cut, or use `observer` plus `checkpoint` |
| `analyze(value, algorithm=..., **parameters)` | Analyze a graph ID, expression, read-only Program or returned `QueryResult` |
| `nearest(value, vector, space_id=..., metric=..., k=...)` | Exact vector search, using property `vector` by default |
| `capabilities()` | Inspect supported algorithms, semantics and native budgets |

`NativeError` preserves `.code`, `.message` and `.envelope`; use the code rather
than parsing error text. `ProtocolError` rejects malformed/unsupported responses.
The client supports science0.1.0 and contract0.21.0 explicitly. `ValueError` covers
invalid helper inputs and unavailable replay selections; `FileNotFoundError`
identifies a missing native executable.

The default subprocess timeout is 30 seconds; configure `Engine(..., timeout=60)`
for longer workloads. `OperationTimeout.commit_status` is `"unknown"` for imports
and arbitrary execution. It is `"read_only"` for analysis. No retry occurs: inspect
durable state before retrying a mutation. Native output-budget errors can also
occur after a commit. `limits={...}` sets finite native analysis budgets;
`max_response_bytes` checks the response size after reception rather than isolating
native memory usage. See the [native interface](SCIENCE_INTERFACE.md) for these boundaries.

Run installed-client tests with the configured binary:

```sh
.venv/bin/python -m unittest discover -s python/tests -v
```

Set `WEAVE_SCIENCE_BINARY` to enable native integration cases; otherwise those
cases are skipped. See [Scientific validation](SCIENCE_VALIDATION.md) for independent
algorithm oracles and measured workloads.
