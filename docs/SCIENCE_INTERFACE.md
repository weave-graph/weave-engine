# Native science interface

`weave-science` runs graph experiments through the existing Weave engine. Its
JSON CLI and Rust API share the same commits, immutable revisions, authorization,
query operators and analytics. Use this interface to integrate a native program
or drive experiments with JSON; use the [Python client](PYTHON_SCIENCE.md) for
notebooks, table helpers and experiment records.

The science interface version is **0.1.0** and the engine contract version is
**0.21.0**. These are separate versions. The Rust
[request types](../crates/weave-science/src/lib.rs) define the science schema;
the [engine contract index](contract/README.md) explains base and extension
semantics. Canonical [contract types](../crates/weave-contract/src/lib.rs) define
`Program`, `GraphData` and query results. Browser and mobile applications are
outside this delivery.

## Build and invoke the CLI

From the repository root:

```sh
cargo build --release --locked -p weave-science
printf '%s\n' '{"operation":"capabilities"}' | target/release/weave-science --db experiment.db --actor researcher
```

On Windows the executable is `target/release/weave-science.exe`. In PowerShell:

```powershell
'{"operation":"capabilities"}' | .\target\release\weave-science.exe --db experiment.db --actor researcher
```

| Argument | Meaning |
|---|---|
| `--db PATH` | Required SQLite file; each invocation opens it independently. |
| `--actor PRINCIPAL` | Required nonempty principal supplied by the trusted host. |
| `--write GRAPH` | Grant write authority to one graph; repeat for additional graphs. |
| `--help` or `-h` | Print usage and exit successfully. |

Use a persistent file for a sequence of CLI calls. `:memory:` loses its contents
when the process exits; an in-memory Rust session can instead retain state across
multiple requests. Opening a file initializes or upgrades the native store as
needed, including for a read request. See [storage and recovery](STORAGE_RECOVERY.md).

Each invocation reads **one JSON object** from stdin and writes **one JSON
response** followed by a newline to stdout. There is no `run` subcommand or
JSON Lines streaming mode. Requests are limited to 16 MiB. Duplicate object keys,
unknown request fields, malformed JSON and trailing input reject before storage
opens. Graph and plan validation remains the engine's responsibility.

`--actor` and `--write` are trusted local authority. The CLI is an embedding tool,
not a remote authentication service: request JSON cannot choose a different
host principal or grant itself write access. Current authorization governs both
new reads and reads of older revisions, including topology and dependencies.

## Runnable import, query and analysis

Save this as `native_walkthrough.py`. It needs only Python's standard library,
passes JSON directly to the CLI and obtains actual revision IDs from receipts.
Each run creates a fresh directory and keeps its database, requests and results
there. It does not require the Python SDK. Times `0`, `5`, `10` and `15` are toy
Unix-epoch millisecond values; use real epoch milliseconds for dated datasets.

```python
import copy
import json
import subprocess
import sys
import tempfile
from pathlib import Path

binary = sys.argv[1]
directory = Path(tempfile.mkdtemp(prefix="weave-native-example-"))
database = directory / "experiment.db"

def call(request, *, write=False):
    args = [binary, "--db", str(database), "--actor", "researcher"]
    if write:
        args += ["--write", "experiment"]
    completed = subprocess.run(args, input=json.dumps(request), text=True,
                               encoding="utf-8", capture_output=True, timeout=30, check=False)
    response = json.loads(completed.stdout)
    if completed.returncode or not response["ok"]:
        raise RuntimeError(response.get("error", completed.stderr))
    assert response["science_version"] == "0.1.0"
    assert response["contract_version"] == "0.21.0"
    return response["result"]

graph = {
    "nodes": [
        {"id": "a", "entity_id": "A", "space_id": "features", "properties": {"vector": [1, 0]}},
        {"id": "b", "entity_id": "B", "space_id": "features", "properties": {"vector": [0, 1]}},
        {"id": "c", "entity_id": "C", "space_id": "features", "properties": {"vector": [1, 1]}},
        {"id": "isolated", "entity_id": "I", "space_id": "features"}
    ],
    "edges": [
        {"id": "ab", "predicate": "link", "from": "a", "to": "b", "valid_time": {"start": 0, "end": 10}},
        {"id": "bc", "predicate": "link", "from": "b", "to": "c", "valid_time": {"start": 5, "end": 15}}
    ]
}
receipt = call({"operation": "import", "graph_id": "experiment", "data": graph}, write=True)
revision = receipt["results"][0]["revision"]
program = {"version": "0.21.0", "commands": [
    {"op": "query", "query": {"graph_id": "experiment", "revision": revision}}
]}
query_request = {"operation": "execute", "program": program}
selected = call(query_request)["results"][0]["result"]
path_request = {"operation": "analyze", "program": program, "analysis": {
    "algorithm": "shortest_paths", "source": "a", "target": "c", "valid_at": 5
}}
paths = call(path_request)
assert paths["analysis"]["path"] == ["a", "b", "c"]
assert paths["analysis"]["distances"]["isolated"] is None

neighbors = call({"operation": "analyze", "program": program, "analysis": {
    "algorithm": "nearest_vectors", "space_id": "features",
    "query": [1, 0], "metric": "cosine", "k": 2
}})
assert [row["id"] for row in neighbors["analysis"]["neighbors"]] == ["a", "c"]
expired_request = copy.deepcopy(path_request)
expired_request["analysis"]["valid_at"] = 10
assert call(expired_request)["analysis"]["path"] is None

# Replace current state using compare-and-swap; the old revision remains.
corrected = copy.deepcopy(graph)
corrected["edges"][0]["valid_time"]["end"] = 5
new_receipt = call({"operation": "import", "graph_id": "experiment",
                    "expected_head": revision, "data": corrected}, write=True)
new_revision = new_receipt["results"][0]["revision"]
assert new_revision != revision
current_path_request = copy.deepcopy(path_request)
current_path_request["program"]["commands"][0]["query"]["revision"] = new_revision
current_paths = call(current_path_request)
assert current_paths["analysis"]["path"] is None
assert call(query_request)["results"][0]["result"] == selected
assert call(path_request) == paths

for name, value in {"import_receipt": receipt, "query_request": query_request,
                    "selected": selected, "path_request": path_request,
                    "paths": paths, "neighbors": neighbors,
                    "correction_receipt": new_receipt,
                    "current_path_request": current_path_request,
                    "current_paths": current_paths}.items():
    (directory / f"{name}.json").write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
print(json.dumps({"directory": str(directory), "revision": revision,
                  "path": paths["analysis"]["path"], "corrected_path": current_paths["analysis"]["path"],
                  "neighbors": neighbors["analysis"]["neighbors"]}, indent=2))
```

Run it from the repository root:

```sh
python3 native_walkthrough.py target/release/weave-science
```

On Windows, use `python native_walkthrough.py target/release/weave-science.exe`.
Every call starts a new process, so the final equality checks also verify
reopening after a correction. The printed revision names the old snapshot;
the correction receipt records the new one. Generated `query_request.json` and
`path_request.json` can be submitted directly using stdin redirection, the
printed database path and the same actor. No hash needs replacing by hand.

## Requests and responses

The science discriminator is **`operation`**. Nested engine commands use **`op`**;
graph expressions use **`kind`**. These fields are not interchangeable.

| Operation | Required fields | Optional fields and defaults | `result` on success |
|---|---|---|---|
| `capabilities` | None | None | Supported operations, algorithms, semantics, versions and limits. |
| `import` | `graph_id`, `data` | `branch_id: "main"`, `expected_head: null` | `{"results": [commit receipt]}`. |
| `execute` | `program` | None | `{"results": [ordered engine command results]}`. |
| `analyze` | `program`, `analysis` | `result_index: 0`, `allow_partial: false`, `limits` as below | `{"input": QueryResult, "analysis": metrics, "semantics": declared semantics}`. |

Import commits a **full snapshot**, not a patch. A null or absent expected head
creates an empty branch's first revision. Updating requires its current revision
as `expected_head`; a stale head fails with `E_CONFLICT`. A content-identical
update can return `kind: "unchanged"` with a revision and no event ID.

In the ordinary graph profile, nodes require `id`, `entity_id` and `space_id`;
edges require `id`, `predicate`, `from`, `to` and `valid_time`. Endpoints name
manifestation node IDs in that snapshot. `valid_time.start` is required; an absent
or null `end` means an open interval. Properties default to an empty object,
metadata to an empty list and edge polarity to `positive`. Node/edge `metadata`
lists contain pinned `GraphRef` values naming graph ID and revision. Named
metadata `attachments` can instead use `value.kind: "live_graph"` with graph and
branch IDs; that form resolves the current dependency head when queried.
Advanced schemas, assertions, attachments and provenance use the existing
contract unchanged. See [graph concepts](SCIENCE_CONCEPTS.md).

`execute` supports existing graph expressions, temporal joins, histories,
geometry and clustering. Current examples use Program `"version": "0.21.0"`
and an ordered `commands` list. Existing engine compatibility rules still apply;
the science host does not translate unsupported plans into weaker semantics.

An analysis Program must be read-only. Any `commit` or `commit_batch` rejects
before any command executes. `result_index` indexes the ordered **command result
array**, not just query results. The selected result must have `kind: "queried"`;
history ranges are not graph analysis inputs. Analysis reexecutes the Program
under current authority; it does not accept a copied graph as proof of access.

Successful envelope:

```json
{
  "ok": true,
  "science_version": "0.1.0",
  "contract_version": "0.21.0",
  "result": {
    "results": [{
      "kind": "committed",
      "revision": "sha256:5d3385e7df004cd13555eb8e8f17745662fa646dc0213742517e9aa702b3270e",
      "event_id": "commit:sha256:5d3385e7df004cd13555eb8e8f17745662fa646dc0213742517e9aa702b3270e"
    }]
  }
}
```

This receipt is from the walkthrough fixture. Consumers should use the ID their
own import returns. A read uses `kind: "queried"` with graph, snapshots,
provenance, metadata graphs, coverage and diagnostics. Analysis retains that full
envelope as `input` alongside metrics, so scores retain their selected evidence.

Failure envelope, with nonzero CLI exit status:

```json
{
  "ok": false,
  "science_version": "0.1.0",
  "contract_version": "0.21.0",
  "error": {"code": "E_SCIENCE_READ_ONLY", "message": "analysis programs must contain only read operations"}
}
```

Check both envelope and process status. Help is human-readable, and OS or stdout
failures can terminate the process without a valid JSON envelope.

## Algorithms

Topology algorithms accept optional `valid_at`, absent/null by default. They use
only positive edges and retain all nodes in the selected authorized result.

| Algorithm | Parameters | Analysis fields |
|---|---|---|
| `degree` | Optional `valid_at`. | `nodes` keyed by ID with `in_degree`, `out_degree`, `total_degree`; `node_count`, `edge_count`. |
| `components` | Required `mode: "weak"` or `"strong"`; optional `valid_at`. | Sorted `components` of sorted node IDs; `mode`, `node_count`, `edge_count`. |
| `shortest_paths` | Required `source`; optional `target`, `directed: true`, `valid_at`. | `distances` and `predecessors` by node ID; target `path`; `source`, `target`, `directed`. |
| `pagerank` | Optional `damping: 0.85`, `tolerance: 1e-10`, `max_iterations: 100`, `valid_at`. | `scores` by ID, `iterations`, `converged`, `residual`; nonempty results also report parameters and `residual_norm`. |
| `nearest_vectors` | Required `space_id`, numeric `query`, `metric: "cosine"` or `"euclidean"`; optional `property: "vector"`, `k: 10`. | Ordered `neighbors` with `id`, `entity_id`, `space_id`, `distance`; `candidate_count`, `dimensions`, `metric`, `exact: true`. |

Every analysis identifies its `algorithm`. Unreachable distances/predecessors
are null. `path` is null if no target was requested or it is unreachable;
an absent source/target is an error. Paths count **unweighted hops**. Edge
properties such as `weight` do not affect native topology algorithms.

Parallel edges each contribute to degree and PageRank transition probability.
A self-loop contributes one incoming and one outgoing incidence, hence two to
total degree. Weak components ignore direction; strong components retain it.
Isolates form singleton components. The empty graph produces empty degree,
components and scores; empty PageRank is converged with zero iterations.

PageRank starts uniformly and redistributes dangling mass uniformly. Residual is
the absolute L1 difference between successive vectors. Damping must satisfy
`0 <= damping < 1`, tolerance must be positive and finite, and iterations must
be from `1` through `10000`, inclusive. Hitting the iteration limit returns scores with
`converged: false`, rather than claiming convergence.

Negative assertions stay in `input` and are excluded from topology. The
`semantics` object reports excluded negative and out-of-time edges. These counts
describe the authorized selection, not hidden data or an evidence resolution.
Manifestation IDs stay distinct even when entity IDs match; vector similarity
does not merge identities.

### Time and vectors

Valid times use signed 64-bit integer Unix-epoch milliseconds, as specified by
the [base contract](contract/v0.1/README.md). The core compares these integers
without interpreting a separate dataset unit. Recorded observation time is a
distinct, host-assigned axis; an experiment cannot assign it through graph data.

`analysis.valid_at` samples half-open edge intervals `[start, end)` while
retaining the selected node universe. Without it, topology uses the union of
selected intervals, which does not imply simultaneous validity. A core query's
time, predicate or endpoint filter can remove isolates because it keeps matching
edge endpoints. Query an unfiltered snapshot and use analysis time for
isolate-preserving statistics.

Vector search reads `node.properties[property]` in the chosen space. Other
spaces and nodes missing the property are skipped. Every selected candidate
must have the query's dimension and finite numeric coordinates; a malformed
candidate rejects the operation. `k` must be positive and at most
`limits.max_nodes`. Fewer than `k` candidates returns all; none returns an empty
list. Euclidean distance is the vector norm of coordinate differences; cosine
distance is `1 - cosine_similarity`, with similarity clamped to `[-1, 1]` for
rounding. Cosine requires nonzero query and candidate vectors. A finite-coordinate
Euclidean distance that exceeds finite `f64` range rejects.

Neighbors sort by increasing distance, then lexical node ID. The caller records
encoder identity and preprocessing; the interface does not generate embeddings
or infer transformations between spaces. Algorithms use fixed node and operation
order, without randomness or wall-clock inputs; compare platform math using
numerical tolerances.

Current authorization can change a historical read. Live metadata can change
dependency selections even with a pinned root revision: retain and compare the
complete input snapshot manifest. The [Python replay guide](PYTHON_SCIENCE.md)
describes drift checks and supported exact-replay selections.

## Resource limits and errors

Only `analyze` accepts `limits`. Omitted fields retain these defaults:

| Limit | Default | Maximum accepted override |
|---|---:|---:|
| `max_nodes` | 100,000 | 1,000,000 |
| `max_edges` | 1,000,000 | 5,000,000 |
| `max_work` | 100,000,000 | 1,000,000,000 |
| `max_vector_dimensions` | 16,384 | 65,536 |
| `max_output_bytes` | 16,777,216 (16 MiB) | 67,108,864 (64 MiB) |

Node/edge limits apply to the whole query graph before polarity/time filtering.
Work counts abstract node/edge visits and vector arithmetic; sorting remains
bounded by structural limits. It is not a CPU instruction, wall-time or RSS
quota. Output includes the JSON envelope and full original query input.
Other operations use the default 16 MiB output limit. CLI input remains capped
at 16 MiB, and the core engine's own plan/read/materialization budgets still
apply. Raising science limits does not guarantee dataset capacity.

For example, add `"limits": {"max_work": 5000000}` to reduce compute work while
retaining other defaults. Partial input rejects by default. An explicit
`"allow_partial": true` permits computation with the original partial coverage
and diagnostics intact. Complete coverage is relative to the authorized
selection, not every fact in the database.

| Code | Meaning or recovery |
|---|---|
| `E_INPUT` | Invalid CLI flags/strict JSON or input above 16 MiB. |
| `E_FORBIDDEN` | Host has not granted the graph write. |
| `E_CONFLICT` | Stale expected head; query current state before deciding how to update. |
| `E_UNAVAILABLE` | Selected graph/revision unavailable under current visibility. |
| `E_VERSION` | Unsupported engine Program version or feature. |
| `E_SCIENCE_READ_ONLY` | Analysis includes a mutation; no command from that Program ran. |
| `E_SCIENCE_RESULT` | Result index is invalid or selects a non-query result. |
| `E_SCIENCE_PARTIAL` | Complete input required; inspect diagnostics before opting into partial analysis. |
| `E_SCIENCE_LIMIT` | Override exceeds the host maximum. |
| `E_SCIENCE_BUDGET` | Selected graph or algorithm work exceeds its budget. |
| `E_SCIENCE_TOPOLOGY` | Duplicate node IDs or positive edge with an unavailable endpoint. |
| `E_SCIENCE_SOURCE`, `E_SCIENCE_TARGET` | Requested path endpoint absent from selected nodes. |
| `E_SCIENCE_PARAMETER` | Invalid PageRank parameters. |
| `E_SCIENCE_VECTOR` | Invalid vector configuration/dimensions/coordinates or undefined cosine. |
| `E_SCIENCE_NUMERIC` | Computed distance exceeds finite `f64` range. |
| `E_SCIENCE_OUTPUT` | Read/analysis response exceeds its output budget. |
| `E_SCIENCE_OUTPUT_AFTER_EXECUTION` | A mutating Program succeeded but its response is too large; commits remain. |

Existing storage, schema, dependency and authorization errors also propagate.
Treat codes as machine-readable values and messages as explanation.

An engine Program is transactional: an engine error rolls back its writes and
events. The science host checks wire size **after successful execution**.
`E_SCIENCE_OUTPUT_AFTER_EXECUTION` therefore means commits survived: query heads
before retrying. A killed process or transport timeout can leave an unknown
commit outcome. Prefer small imports and separate reads for large experiments;
a missing response does not prove rollback.

## Rust embedding

Consumers can depend on the library's path in a repository checkout. This
example also uses `weave-engine` for `HostContext` and `serde_json` for JSON
construction. Its public methods are exercised by the
[native tests](../crates/weave-science/tests/science.rs).

```rust
use serde_json::json;
use weave_engine::HostContext;
use weave_science::{parse_request, Request, ScienceSession};

fn main() -> Result<(), weave_engine::Error> {
    let host = HostContext::new("researcher", ["demo".to_owned()]);
    let mut session = ScienceSession::memory(host)?;
    let imported = session.handle(parse_request(br#"{
        "operation":"import", "graph_id":"demo", "data":{
            "nodes":[
                {"id":"a","entity_id":"A","space_id":"s"},
                {"id":"b","entity_id":"B","space_id":"s"}
            ],
            "edges":[{"id":"ab","predicate":"link","from":"a","to":"b",
                      "valid_time":{"start":0}}]
        }
    }"#)?)?;
    let revision = imported["results"][0]["revision"].clone();
    let request: Request = serde_json::from_value(json!({
        "operation":"analyze",
        "program":{"version":"0.21.0","commands":[
            {"op":"query","query":{"graph_id":"demo","revision":revision}}
        ]},
        "analysis":{"algorithm":"degree","valid_at":5}
    }))?;
    let result = session.handle(request)?;
    println!("{}", result["analysis"]);
    Ok(())
}
```

`ScienceSession::open(path, host)` persists to disk;
`ScienceSession::new(engine, host)` wraps an existing engine. `handle(Request)`
returns the operation payload or engine `Error`; `respond(Request)` returns the
versioned envelope. `parse_request(bytes)` applies the CLI's strict JSON/input
bound. When constructing typed requests or deserializing directly, the embedding
owns its transport parsing boundary; runtime validation and analysis limits
still apply.

## Validation and scope

Configured native CI runs the current Rust suite, strict lint, installed Python
checks and independent scientific acceptance on Linux, macOS and Windows.
Historical store/compiler checks run for relevant changes and remain available manually;
browser persistence is manual while app delivery is deferred. This describes
the workflow, rather than asserting an unobserved platform run passed. See
[validation and measurements](SCIENCE_VALIDATION.md) for independent oracles and
practical limits, and [the completion contract](NATIVE_SCIENCE_PLAN.md) for scope.
