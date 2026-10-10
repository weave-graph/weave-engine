# Native science interface

`weave-science` is a trusted local host for the existing Weave runtime, with a
small Rust API in `crates/weave-science`. The embedding supplies an actor and
graph write grants. Request JSON cannot grant authority. The CLI is not a remote
authentication service.

```sh
printf '%s\n' '{"operation":"capabilities"}' | weave-science --db experiment.db --actor researcher
```

One bounded JSON request is read from stdin and one JSON envelope is written to
stdout. Success has `ok: true`, `science_version`, `contract_version` and `result`.
Failure has `ok: false` and an engine-style `error` containing `code` and `message`,
and the CLI exits unsuccessfully. `capabilities` reports the supported operations,
algorithm semantics and resource limits. Native scientific packages are optional;
the Rust engine performs the computations.

## Operations

- `import`: `graph_id`, optional `branch_id`, `expected_head`, and the existing
  contract `GraphData` as `data`. An absent/null expected head creates a new
  branch only; updating requires the current revision. Add `--write GRAPH` to
  grant that graph write authority. Returns the normal engine commit receipt.
- `execute`: `program`, an existing protocol0.21 JSON Program. Returns the normal
  ordered engine command results. This includes the existing graph expression,
  temporal history, geometry and clustering operators.
- `analyze`: read-only `program`, optional `result_index` (zero by default),
  `analysis`, optional `limits`, and `allow_partial` (false by default). The
  selected command must produce a graph query result. Returns that complete
  input envelope, the numeric/graph analysis and declared semantics. Commit and
  commit-batch commands reject before any command executes.
- `capabilities`: no additional fields.

An analysis config is tagged by `algorithm`: `degree`, `components` (mode `weak`
or `strong`), `shortest_paths` (source, optional target and directed flag),
`pagerank` (optional damping/tolerance/iteration limit), or `nearest_vectors`
(property, explicit space ID, numeric query vector, metric and k). The Rust
request types are the canonical local interface. Unknown request fields reject.

```json
{
  "operation": "analyze",
  "program": {
    "version": "0.21.0",
    "commands": [{"op": "query", "query": {"graph_id": "experiment", "revision": "EXACT_REVISION"}}]
  },
  "analysis": {"algorithm": "components", "mode": "strong", "valid_at": 5}
}
```

## Scientific semantics

Topology analyses use positive directed edges. Parallel edge assertions retain
their multiplicity; self-loops remain visible. Negative assertions are retained
in the query envelope and excluded from positive topology, with the exclusion
reported. Shortest paths use hop count. PageRank distributes transition mass by
positive edge multiplicity and reports its convergence state. Components retain
the selected isolated nodes. Node IDs are manifestation identities; equal entity
IDs and vector similarity do not collapse distinct manifestations.

The optional topology `analysis.valid_at` selects half-open edge intervals and
retains the selected node universe. Without it, analysis uses the interval union;
that union is not a claim of simultaneous validity. A core query with `valid_at`
selects matching edge endpoints and can remove isolates. Select the unfiltered
graph and set analysis time when isolate-preserving statistics are intended.

Exact vector search uses numeric properties on authorized nodes in one explicitly
named space. It does not generate embeddings or map between spaces. Incompatible
dimensions, non-finite values and undefined cosine inputs reject. Results preserve
node, entity and space IDs; ties use stable lexical node IDs.

The input includes exact snapshots, provenance, metadata, coverage and diagnostics.
Partial coverage rejects by default. An explicit `allow_partial` request permits
computation while preserving the partial envelope, rather than relabeling missing
knowledge as a complete empty answer. Current authorization still governs reads
of older revisions.

Requests, materialized query data, node/edge/vector work, algorithm iterations and
serialized output have finite budgets. The host reports exhausted budgets. The
limits describe bounded local experimentation; they do not promise isolation of
arbitrary external actors or distributed processing.

`execute` can contain ordinary engine commits. If execution succeeds but its
response exceeds the wire budget, `E_SCIENCE_OUTPUT_AFTER_EXECUTION` explicitly
reports that durable commits remain. Query the relevant heads before retrying.
Use small transactions and separate reads when importing large datasets. A
transport timeout also requires checking durable state before assuming rollback.

## Verification profiles

Normal pull-request CI runs the complete current native Rust suite, strict lint,
installed Python client checks and independent scientific acceptance on Linux,
macOS and Windows. Branch pushes do not trigger duplicate copies. Historical
store/compiler compatibility runs when those implementations change, and remains
available manually. Browser persistence is manual while application delivery is
deferred. A benchmark command measures a deterministic workload separately from
correctness acceptance; see [validation](SCIENCE_VALIDATION.md).
