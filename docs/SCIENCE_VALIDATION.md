# Native scientific validation

This acceptance profile checks the [native experiment completion contract](NATIVE_SCIENCE_PLAN.md)
DS01–DS09. It uses the real native binary and SQLite store, without a browser,
mobile application, network service or mandatory Python scientific packages.
The full original white-paper roadmap remains separately tracked.

## Run the independent checks

From the repository root:

```sh
cargo build --release --locked -p weave-science
python3 scripts/check_science.py --engine target/release/weave-science --report science-acceptance.json
python3 scripts/benchmark_science.py --engine target/release/weave-science --report science-benchmark.json
```

Use `weave-science.exe` on Windows. The acceptance controller also works with
the debug executable used in native CI. `--revision COMMIT` records the source
revision the caller verified when building; when omitted, the report labels the
current checkout and records its dirty status. Every report records the exact
binary SHA-256, protocol/science versions, current source tree digest, host,
request and response digests, snapshots, parameters and deterministic seed.
The binary digest is authoritative when a source-to-binary mapping has not
been verified.

The acceptance controller makes each request in a fresh native process. The
database survives these process boundaries. Default seed `1337` and six small
generated graphs produce 16 grouped cases; native subprocess calls are counted
separately, rather than presented as independent test cases. The controller
uses Python's standard library. If NetworkX and its PageRank dependencies are
already installed, add `--networkx` for an additional reference comparison.

## Independent references and acceptance cases

| Behavior | Reference or invariant | Contract |
|---|---|---|
| Directed and undirected shortest paths, strong and weak components | Floyd–Warshall all-pairs dynamic programming, independent of the engine's BFS and Kosaraju algorithms | DS05 |
| PageRank | Direct solution of `(I - alpha Pᵀ) score = (1 - alpha) / n`, with Gaussian elimination; no repeated PageRank implementation | DS05 |
| Parallel assertions, self-loops, isolated nodes and negative evidence | Explicit five-node fixture, exact degree/incidence counts, disconnected paths and component membership | DS05 |
| Exact Euclidean and cosine neighbors | Exhaustive Python distances; equal-distance ties ordered by manifestation ID | DS06 |
| Authorization before analytics | Denied identifiers never appear in selected topology, resolved metadata or neighbors; changing hidden vectors and paths leaves visible analytics unchanged | DS04 |
| Node and edge graph-valued metadata | Both point to the same exact evidence revision, which resolves once with denied evidence removed; the entire query envelope survives analysis | DS03–DS04 |
| Valid-time selection | Half-open interval boundaries checked at `-1`, `0`, `9`, `10`; analysis samples time while retaining the selected node universe | DS02 |
| Corrections and recorded knowledge | A correction shortens the current path; the immutable old revision and recorded checkpoint reproduce the earlier path after reopening | DS01–DS02 |
| Stale writes and host authority | Compare-and-swap rejects an old head; an independent host without a graph write grant cannot import a replacement | DS01, DS04 |
| Temporal graph joins and reuse | Independent premises `[0,10)` and `[5,15)` produce `[5,10)`; touching disjoint intervals produce no assertion; the joined result can be analyzed directly | DS03 |
| Provenance, snapshots and numeric transport | Joined sources name both premise revisions; reopened query bytes have identical canonical content; the integer `9007199254740993` remains exact | DS02–DS03, DS07 |
| Invalid vectors and bounded work | Mismatched dimensions, zero cosine vectors, exhausted node limits and unavailable path sources fail explicitly | DS06, DS08 |
| Partial knowledge | Missing evidence produces partial coverage and diagnostics; analysis refuses it until `allow_partial` is explicit, then retains the original envelope | DS08 |
| Missing snapshots and pure analysis | Missing graph/revision is unavailable; any mutation in an analysis program is rejected before executing it | DS08 |
| Empty and seeded graph cases | Defined empty results plus six deterministic small multigraphs compared with the independent references | DS05, DS09 |

These are finite, reproducible acceptance cases. They supplement the existing
native runtime suite, which covers persistence, recovery, policy, geometry,
clustering and graph algebra. They do not prove general noninterference or
distributed correctness. The hidden-input comparison checks released analytics;
source revision digests legitimately differ when the underlying graph changes.

Shortest paths count unweighted edge hops. PageRank gives each positive parallel
edge a transition contribution; a self-loop is one incoming and one outgoing
incidence. Negative assertions remain in the original query envelope and are
excluded from topology analytics. Optional `analysis.valid_at` filters positive
edge intervals and preserves all selected authorized nodes. A core query with
`query.valid_at` can instead select only surviving edge endpoints; this difference
is explicit in the interface and fixture.

Exact vector search uses one explicitly selected space and raw numeric node
properties. The caller records encoder identity and preprocessing in the
experiment. Search does not silently map incompatible spaces or merge entities.

## Workload measurements

The benchmark defaults to 1,000 and 10,000 input nodes, three samples per
read/analysis operation, eight-dimensional vectors and deterministic seed
`1337`. Each node has a directed ring edge and a seeded chord; every twentieth
node has a self-loop. Two visibility partitions hide every tenth node, and the
authorized query removes incident unavailable topology. Node and edge metadata
share a sixteen-node evidence graph at depth one. Chord intervals overlap at
the declared sample time `75`.

The report measures import, pinned query, degree, weak components, shortest
paths, PageRank and exact nearest vectors. It verifies visible node/edge counts,
degree conservation, component partitioning, source distance, PageRank
normalization/convergence and neighbors against exhaustive distances.

Latency includes a fresh native process, SQLite reopening, authorization,
query materialization, analytics, complete provenance-bearing JSON serialization
and output transfer. Fixture generation, builds and Python JSON decoding are
excluded. These are end-to-end native experiment timings, not isolated kernel
timings. Reported memory uses per-child `wait4` high-water RSS on POSIX hosts;
it is explicitly unavailable on hosts without that API and excludes the harness.

Three samples report median and maximum. A nearest-rank p95 is included only
when `--samples` is at least five. Request size, response size, workload density,
visibility, metadata shape and exact snapshot appear alongside each sample.
Budget rejection is recorded as rejection and never presented as demonstrated
capacity. Increase scales with `--sizes` only within the declared native input,
output and work limits. These synthetic workloads establish a baseline on the
reported host and do not create a production performance promise.

For a quick harness check:

```sh
python3 scripts/benchmark_science.py --engine target/debug/weave-science --report science-benchmark-smoke.json --sizes 1000 --samples 1
```

## Verified release baseline

On 10 October 2026, the optimized native binary passed all 16 grouped acceptance
cases in 0.93 seconds, using 158 native process operations. Both benchmark sizes
passed every import, query and analytics check in 5.45 seconds overall. The
host was Apple M4 Max, arm64, 14 logical CPUs, 36 GiB RAM, Darwin 25.6.0 and
Python 3.14.6. The input shapes were 1,000 nodes/2,050 edges and 10,000
nodes/20,500 edges; authorized reads selected 900 nodes/1,604 edges and 9,000
nodes/16,077 edges respectively.

| Operation | 1k median ms | 1k peak child RSS MiB | 10k median ms | 10k peak child RSS MiB |
|---|---:|---:|---:|---:|
| Import | 23.52 | 18.45 | 181.70 | 123.45 |
| Pinned query | 23.57 | 20.27 | 172.22 | 150.50 |
| Degree | 23.73 | 22.08 | 193.60 | 165.61 |
| Weak components | 23.64 | 20.83 | 179.93 | 152.47 |
| Shortest paths | 23.66 | 21.25 | 181.90 | 156.81 |
| PageRank | 23.60 | 20.97 | 181.51 | 153.14 |
| Exact vectors | 23.75 | 20.44 | 172.54 | 151.41 |

Import has one measured sample per size; the other operations have three. Peak
RSS is the largest measured child value. At 10k, analytics responses were
approximately 13.3–13.8 MB because they retained the full selected query and
provenance; these timings include that output. The evidence describes a build
from the recorded dirty integration checkout, with full source-tree and binary
digests. It does not assert an exact clean release-commit mapping.

Recorded SHA-256 identities:

The [acceptance report](benchmarks/science-acceptance-2026-10-10.json) and
[benchmark report](benchmarks/science-benchmark-2026-10-10.json) preserve the
raw measurements, workload definitions, source-tree identities and operation
digests for this baseline.

```text
native binary: d111a9b263d2a52ab01400e52c3e4bfd8c3e30f407bf6ea2317f72d98b7a7fda
science-acceptance.json: a025cb2b8b3fbd42cb31d6cc5d26ae6e78b32583d3f8e9a662d1acc504e7337d
science-benchmark.json: 042549d0fe28a2b4c92317a299159703f9b6a83906258cb9658824e70de57155
```

CPU model and physical RAM were read on the same host immediately after these
runs and added to the reports; elapsed measurements were preserved. No larger
dataset capacity, tail-latency SLO or app-platform behavior is inferred.
