# Python experiments with the native engine

The `weave_science` package gives notebooks and scripts access to the existing
Weave engine and the native science algorithms. It needs no Python dependencies
for import, query, analysis or export. The native process owns persistence,
revision assignment, temporal semantics, authorization and computation.

## Install and run

From a clean repository checkout with a Rust toolchain and Python 3.10 or newer:

```sh
cargo build --release -p weave-science
python3 -m venv .venv
.venv/bin/python -m pip install ./python
export WEAVE_SCIENCE_BINARY="$PWD/target/release/weave-science"
.venv/bin/python examples/science/temporal_vectors.py --output experiment-output
```

On Windows, use `py -m venv .venv`, `.venv\Scripts\python.exe`, and
`$env:WEAVE_SCIENCE_BINARY = "$PWD\target\release\weave-science.exe"` in PowerShell.
An already built native binary needs no Rust toolchain to run. Pass its path as
`Engine(..., binary="...")` or put `weave-science` on `PATH`. Editable source
installs also discover an existing repository `target/release` or `target/debug`
binary. The client never builds the engine implicitly.

The example creates a persistent database, imports four vector-bearing nodes,
compares temporal connectivity before and after a late correction, retrieves
exact cosine neighbors, computes PageRank, exports tables, and replays the old
experiment after reopening the database. Its vectors are fixed toy coordinates
in a named space. Supply vectors from your own model for an embedding experiment.
Use a new output directory on each run.

## Import, correct and read a dataset

```python
from weave_science import Engine, node, edge, query

engine = Engine("experiment.sqlite", actor="scientist", write_graphs=["observations"])
nodes = [
    node("a", entity_id="person-A", space_id="semantic-model-v1", vector=[1.0, 0.0]),
    node("b", entity_id="person-B", space_id="semantic-model-v1", vector=[0.8, 0.2]),
    node("c", entity_id="person-C", space_id="semantic-model-v1", vector=[0.0, 1.0]),
]
edges = [edge("ab", "a", "b", predicate="collaborates", valid_from=0, valid_to=10)]
first = engine.import_graph("observations", nodes=nodes, edges=edges)
old = engine.query("observations", revision=first.revision)

# This is a full replacement snapshot under compare-and-swap revision control.
edges[0]["valid_time"]["end"] = 6
second = engine.import_graph("observations", nodes=nodes, edges=edges,
                             expected_head=first.revision)
new = engine.query("observations", revision=second.revision)
assert old.snapshots != new.snapshots
assert engine.query("observations", revision=first.revision).graph == old.graph
```

`expected_head=None` creates a new branch. Updating requires the current revision;
a stale revision rejects instead of overwriting concurrent work. A successful
import returns `CommitReceipt(revision, event_id, changed, raw)`. Reimporting an
unchanged snapshot with the correct expected head returns an unchanged receipt.
The previous immutable snapshot remains available.

Node `id` is a manifestation ID; `entity_id` is stable semantic identity and
`space_id` names the space. Supply different manifestation IDs for the same entity
in different spaces. Equal vectors do not merge entities. Edge IDs remain stable
assertion IDs. Metadata references, reader restrictions and other existing
contract fields can be passed through `node`, `edge` and `graph_data`, or supplied
as an ordinary `GraphData` dictionary to `import_graph(graph_id, data)`.

Time is a signed 64-bit integer in the dataset's declared unit. Edge intervals are
half-open `[start, end)`; `end=None` is unbounded. Valid time is supplied by the
dataset. Recorded time and revisions are assigned by the engine. The database
parent directory must exist. The SDK requires a persistent path because each
operation opens a native process; `:memory:` would lose state between calls.

## Native analyses

```python
connected = engine.analyze(old, algorithm="components", mode="weak", valid_at=8)
degree = engine.analyze(old, algorithm="degree", valid_at=8)
paths = engine.analyze(old, algorithm="shortest_paths", source="a", directed=True,
                       valid_at=8)
rank = engine.analyze(old, algorithm="pagerank", damping=0.85, tolerance=1e-10,
                      max_iterations=200, valid_at=8)
neighbors = engine.nearest(old, [1.0, 0.0], space_id="semantic-model-v1",
                           property="vector", metric="cosine", k=3)
print(connected.analysis)
print(neighbors.analysis)
print(rank.semantics)
```

`analyze` accepts a graph ID, existing graph expression, read-only Program or
`QueryResult`. A result is re-read through the engine with exact pinned inputs
and current authorization. An analysis returns `AnalysisResult.input` (the full
authorized `QueryResult`), `.analysis` (the computed output), and `.semantics`.
`Engine.capabilities()` reports supported algorithms, versions, conventions and
finite resource budgets. Pass `limits={...}` to analysis for lower workload budgets.

Degree, weak/strong components, unweighted shortest paths and PageRank use positive
directed multigraph topology. Parallel assertions and self-loops are retained.
PageRank counts edge multiplicity and reports convergence; it does not claim that
an unconverged iterate is a converged answer. Negative assertions remain in the
input envelope and are excluded from positive topology. Nearest-neighbor search
is exhaustive in one explicitly named space, using cosine or Euclidean distance.
It requires consistent numeric dimensions and rejects undefined cosine vectors.
It does not generate embeddings or infer a cross-space mapping.

Use an unfiltered query plus `analysis.valid_at` for a fixed-time topology
experiment that retains isolated nodes. A core `query(..., valid_at=8)` returns
matching edge endpoints and can omit isolates. Without analysis time, topology
uses the interval union; relationships from different times may coexist in that
union. This is useful for historical connectivity but is not simultaneous state.

Partial coverage rejects by default. Set `allow_partial=True` deliberately to
compute from available inputs; `.input.coverage` and diagnostics remain partial.
An empty graph, denied topology and an unavailable dependency have different
engine meanings.

## Temporal joins and the existing engine algebra

The SDK exposes the existing engine contract directly, so graph-valued metadata,
temporal joins, history, rule evaluation, clustering and geometry do not need a
new Python implementation. Convenience builders create JSON expressions:

```python
from weave_science import join, union, diff, window, explain, recorded_query

# Both inputs are pinned; join uses entity-and-space endpoint matching.
derived = engine.evaluate(join(query("left", revision="LEFT_REVISION"),
                               query("right", revision="RIGHT_REVISION"),
                               output_predicate="connected_through"))
change = engine.evaluate(diff(query("observations", revision=first.revision),
                              query("observations", revision=second.revision)))
interval = engine.evaluate(window(query("observations", revision=first.revision), 2, 9))
proof = engine.evaluate(explain(query("observations", revision=first.revision)))

# Recorded time is a replica-local history cut, distinct from edge valid time.
historical = engine.evaluate(recorded_query("observations", recorded_at_ms=RECORDED_MILLIS))
```

Pass any existing graph-expression dictionary to `evaluate`, or any existing
versioned Program to `execute`. `execute` returns `{"results": [...]}` in native
command order. `evaluate` accepts read-only Programs and selects a query result by
`result_index`; use `execute` for commits. For a full Program, analysis also selects
the result by `result_index`. Service expressions such as accepted history,
clustering and geometry use their existing native installed-state and selector
requirements; passing a JSON reference grants no authority. See the contract
types and [native interface](SCIENCE_INTERFACE.md) for those schemas.

## Tables, CSV and optional packages

`import_graph(..., nodes=records, edges=records)` accepts ordinary dictionaries or
engine-shaped pandas frames. The schema is explicit: nodes need `id`, `entity_id`
and `space_id`; edges need `id`, `predicate`, `from`, `to` and `valid_time`.
Nested `properties`, `metadata` and `readers` retain their JSON types.

For an arbitrary CSV table, choose conversion types and map its columns explicitly:

```python
from weave_science import read_csv

rows = read_csv("measurements.csv", json_columns=["vector"], integer_columns=["sample_time"])
manifestations = [node(row["id"], entity_id=row["entity"], space_id="model-v2",
                      vector=row["vector"], properties={"sample_time": row["sample_time"]})
                  for row in rows]
engine.import_graph("measurements", nodes=manifestations)  # grant this graph at Engine construction
```

`Engine.import_csv(graph_id, nodes_path, edges_path, node_options=..., edge_options=...)`
imports engine-shaped CSVs using the same typed reader. JSON columns must be
declared, for example `node_options={"json_columns": ["properties", "readers"]}`
and `edge_options={"json_columns": ["valid_time", "properties", "readers"]}`.
Empty typed cells become `None`. Unspecified columns remain strings; the reader
rejects duplicate headers, inconsistent row widths and non-finite numbers.

```python
old.export_json("authorized-result.json")                # full envelope
old.export_json("snapshot.json", graph_only=True)        # importable GraphData
node_csv, edge_csv = old.export_csv("authorized-tables")  # nested cells contain JSON
```

CSV exports contain node/edge records. Keep the JSON envelope beside them for
provenance, coverage, metadata graphs, snapshots and lossless scalar types. CSV
requires an explicit schema; null and empty text share a blank cell. For notebook conversion,
install `pip install './python[tables]'` and call `old.to_pandas()`. For an
independent graph oracle, install `pip install './python[networkx]'` and call
`old.to_networkx()`. The NetworkX export is a `MultiDiGraph` keyed by manifestation
and edge IDs, excluding negative assertions by default. These optional exports
operate on already authorized data.

## Reproducible records and replay

```python
from weave_science import Experiment

record = connected.save_experiment("trial.json", label="connectivity at t=8",
                                    parameters={"dataset": "measurements-v2", "seed": 17})
reopened = Engine("experiment.sqlite", actor="scientist")
replayed = Experiment.load("trial.json").replay(reopened)
assert replayed.analysis == connected.analysis
```

Artifacts include the original request, exact selected snapshot vector, full
authorized result, algorithm configuration/semantics, caller parameters, actor,
SDK/science/contract versions and the native executable's SHA-256. The JSON record
has its own integrity hash. Saving fixes unambiguous live query inputs to the
revisions observed by that successful result. Recorded-time reads pin their actual
checkpoint witness. A later head change cannot silently change those inputs.

Exact replay requires the same actor, protocol versions and native executable by
default. To compare native builds deliberately, use
`record.replay(engine, require_same_binary=False)`; retain a new experiment record
for the comparison. The same database must still retain the referenced revisions
and dependencies. Current policy is rechecked; artifacts never restore authority.
Binary replacement during an `Engine` session rejects; create a new session after
a rebuild so its executable identity is accurate.

Pinning a root revision cannot freeze a live metadata attachment. Replay compares
the complete current authorized input envelope with the recorded input, including
graph values, dependencies, provenance and coverage. A changed live metadata head
or changed current policy raises `ReproducibilityError` instead of claiming an
exact reproduction. Analysis of a prior `QueryResult` performs the same check.
Use pinned metadata references for experiments that must survive later head changes.

Ambiguous unpinned revisions, mutating Programs and runtime service/view expressions
whose complete immutable selection is not verified remain recordable but have
`replay_request=None` with an explicit reason. They are never advertised as exact
replays. Raw graph exports and scalar analysis outputs do not themselves carry
write/read grants or establish a distributed atomic snapshot.

## Errors and verification

`NativeError.code`, `.message` and `.envelope` preserve native rejection details.
`QueryResult.diagnostics`, `.coverage` and `.provenance` preserve the result's
semantic evidence. `ProtocolError` identifies malformed or unsupported protocol
responses. `ReproducibilityError` reports input drift. Replays reject mutating
Programs before launching the engine, including externally supplied artifacts
with recomputed hashes. The client supports native science0.1.0/contract0.21.0 explicitly.

The subprocess timeout is finite and configurable with `Engine(..., timeout=60)`.
On `OperationTimeout`, a mutating operation's `.commit_status` is `"unknown"`;
no automatic retry occurs. Check durable graph state before retrying. Native
execution can also report an output budget failure after committing; preserve
that distinction when processing `NativeError`. Each call checks the native
response against `max_response_bytes` after reception; native resource/output
budgets govern computation.

Run the dependency-free Python tests against the installed package and binary:

```sh
WEAVE_SCIENCE_BINARY="$PWD/target/release/weave-science" .venv/bin/python -m unittest discover -s python/tests -v
```

The native integration cases verify import/correction/CAS, old snapshots after
reopen, exact artifact replay and hidden vector/topology boundaries. SDK tests
also exercise protocol errors, uncertain timeouts, typed CSV ingestion, stable
input objects and replay pinning. The independent scientific acceptance suite is
described in [SCIENCE_VALIDATION.md](SCIENCE_VALIDATION.md).
