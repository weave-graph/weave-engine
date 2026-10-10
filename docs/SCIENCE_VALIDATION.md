# Native scientific validation

This page explains how to check a native installation, reproduce the measured
workloads and interpret the stored evidence. The [native experiment completion
contract](NATIVE_SCIENCE_PLAN.md) defines DS01–DS09. Its evidence combines runtime
tests, Python client tests, independent scientific acceptance and installation
checks; one controller does not establish every requirement by itself.

For request fields and result semantics, read the [native interface](SCIENCE_INTERFACE.md).
For notebooks, imports, exports and saved experiments, use the [Python guide](PYTHON_SCIENCE.md).
The [benchmark index](benchmarks/README.md) separates the current science baseline
from historical engine measurements.

## Choose a validation profile

| Profile | What it establishes | Where it runs |
|---|---|---|
| Current native Rust suite | Runtime/storage behavior, authorization, graph algebra, geometry, clustering and science API checks | `cargo test --workspace --all-features --locked` and native CI |
| Installed Python client checks | Client transport, imports/exports, experiment records and actual native replay scenarios | [Python client workflow](../.github/workflows/ci.yml), using the installed package and selected binary |
| Independent science acceptance | Observable graph/vector answers, temporal revisions, metadata, authorization and rejection cases | `scripts/check_science.py`, locally and in native CI |
| Science workload benchmark | End-to-end timing and memory for the declared dataset shape, with numerical/visibility invariants | `scripts/benchmark_science.py`, separately from correctness CI |

The [Native science workflow](../.github/workflows/ci.yml) is configured for
Linux, macOS and Windows. Its acceptance artifact belongs to the exact workflow
run and commit; inspect [that run's checks](https://github.com/weave-graph/weave-engine/actions/workflows/ci.yml)
when assessing a platform. The stored performance reports below were measured
locally on the stated Mac host. They contain no Linux or Windows timing result.
Historical compiler/store compatibility and application scenarios have separate
profiles. Browser/mobile applications and the full decentralized white-paper
roadmap are outside this native experiment completion contract.

## Reproduce the independent checks

Use a stable Rust toolchain and Python 3.10 or later. Run these commands from the
repository root. The controllers require only Python's standard library; installing
the Python client is not necessary for these two controllers. Reports are written
outside the checkout so they do not change its source-tree fingerprint.

```sh
cargo build --release --locked -p weave-science
python3 scripts/check_science.py \
  --engine target/release/weave-science \
  --report ../weave-validation/science-acceptance.json \
  --seed 1337 --random-cases 6 --timeout 60
python3 scripts/benchmark_science.py \
  --engine target/release/weave-science \
  --report ../weave-validation/science-benchmark.json \
  --sizes 1000 10000 --samples 3 --dimensions 8 --seed 1337 --timeout 120
```

On Windows PowerShell, use `python` and the `.exe` binary; the report parent
directory is created automatically:

```powershell
cargo build --release --locked -p weave-science
python scripts/check_science.py --engine target/release/weave-science.exe --report ../weave-validation/science-acceptance.json --seed 1337 --random-cases 6 --timeout 60
python scripts/benchmark_science.py --engine target/release/weave-science.exe --report ../weave-validation/science-benchmark.json --sizes 1000 10000 --samples 3 --dimensions 8 --seed 1337 --timeout 120
```

The acceptance controller also supports the debug executable used in native CI.
Build with `cargo build --locked -p weave-science` and use `target/debug` for that
profile. Debug and optimized timing results should be labeled separately.

Both reports contain `status`, protocol/science versions, exact binary SHA-256,
source identity, host information and individual case results. For acceptance,
`status: "passed"` means the declared grouped cases passed, including expected
error responses. The benchmark records budget rejection as `status: "rejected"`
with per-sample diagnostics. Its process can finish successfully while recording
rejection, so automation must inspect the report's status as well as its exit code.
An assertion, timeout or transport failure produces a failed report and a nonzero
controller exit.

Keep the executable unchanged while a controller runs. On a clean checkout, a
fresh locked build followed by the controller provides a clear source context.
Optional `--revision COMMIT` records a caller-verified build revision; it is a
label, not a verification performed by the harness. When omitted, the harness
records the current checkout, dirty status, tracked diff digest and a digest of
tracked plus nonignored working files. Those fingerprints describe the files at
controller startup. The binary digest is authoritative when its exact mapping
to a clean source commit has not been established.

The acceptance controller uses temporary databases and a fresh native process
for each request. The database survives these process boundaries during the run
and is removed afterward. Default seed `1337` and six small generated graphs
produce 16 grouped cases. Native subprocess operations are counted separately;
158 process operations are not 158 independent test cases. If NetworkX and its
PageRank dependencies are installed, add `--networkx` for an additional reference
comparison. That optional comparison was not used in the stored baseline.

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

Each dataset is imported once into a temporary SQLite store. Each subsequent
sample starts a fresh native process and reopens that same store. Filesystem
caches are not flushed, and the harness does not discard warmup samples.
Metadata setup is separate from the dataset import sample.

Individual operation latency includes process startup, SQLite reopening,
authorization, query materialization, analytics, full provenance-bearing JSON
serialization and file output. Fixture generation, builds and Python JSON
decoding are excluded from those operation timers. The report's overall elapsed
time includes fixture/store setup and verification. These are
end-to-end native experiment timings, not isolated kernel timings. On POSIX,
the controller polls completion every 5 ms, so timing also includes the completion
observation delay.

Memory is per-child `wait4` high-water RSS on POSIX hosts; it is explicitly
unavailable on hosts without that API. It excludes the Python harness and is
not an algorithm allocation measurement. The units in the table are MiB
(`1 MiB = 1,048,576 bytes`); raw reports store bytes.

Three samples report median and maximum. A nearest-rank p95 is included only
when `--samples` is at least five. Request size, response size, workload density,
visibility, metadata shape and exact snapshot appear alongside each sample.
Budget rejection is never presented as demonstrated capacity. `--sizes` counts
input nodes, not total graph objects or authorized result nodes. The native
request limit is 16 MiB; runtime materialization and science output/work limits
also apply. A nominal algorithm node limit does not guarantee a dataset will fit:
vectors, metadata and repeated provenance can exhaust byte budgets first. See
[interface limits](SCIENCE_INTERFACE.md) and [cumulative runtime reads](READ_BUDGETS.md).
Larger `--sizes` values are explicit experiments, not previously verified capacity.
These synthetic workloads establish a baseline on the reported host and do not
create a production performance promise.

For a quick harness check:

```sh
python3 scripts/benchmark_science.py --engine target/release/weave-science --report ../weave-validation/science-benchmark-smoke.json --sizes 1000 --samples 1
```

## Recorded optimized baseline: 10 October 2026

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

The [acceptance report](benchmarks/science-acceptance-2026-10-10.json) and
[benchmark report](benchmarks/science-benchmark-2026-10-10.json) preserve the
raw measurements, workload definitions, source-tree identities and operation
digests for this baseline.

Both record science API `0.1.0`, engine contract `0.21.0`, seed `1337` and base
commit `7cc0c96d5e80d9e0c6f09cf0b5c0cbe15ba406ae` with uncommitted integration
changes. That base commit alone does not identify the new science implementation.
Their source-tree digests differ because files changed between controller starts;
their native binary digests are identical. The saved evidence is preserved as
measured rather than relabeled as a later clean source build.

Recorded SHA-256 identities:

```text
native binary: d111a9b263d2a52ab01400e52c3e4bfd8c3e30f407bf6ea2317f72d98b7a7fda
science-acceptance-2026-10-10.json: a025cb2b8b3fbd42cb31d6cc5d26ae6e78b32583d3f8e9a662d1acc504e7337d
science-benchmark-2026-10-10.json: 042549d0fe28a2b4c92317a299159703f9b6a83906258cb9658824e70de57155
```

CPU model and physical RAM were read on the same host immediately after these
runs and added to the reports; elapsed measurements were preserved. No larger
dataset capacity, tail-latency SLO or app-platform behavior is inferred.
