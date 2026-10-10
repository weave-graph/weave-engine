# Weave Engine

A native runtime for reproducible experiments with multidimensional temporal knowledge graphs. It combines durable immutable graph revisions, temporal queries and provenance with a Rust analysis interface and an installable Python client. The current delivery scope is [native data science](docs/NATIVE_SCIENCE_PLAN.md); the broader decentralized white-paper roadmap remains open.

## Data-science experiments

Build the experiment engine and install the dependency-free Python client:

```sh
cargo build --release --locked -p weave-science
python3 -m venv .venv
.venv/bin/python -m pip install ./python
WEAVE_SCIENCE_BINARY="$PWD/target/release/weave-science" .venv/bin/python examples/science/temporal_vectors.py --output experiment-output
```

The example imports a temporal graph with vectors, analyzes selected times,
compares an immutable revision with a later correction, and saves a reproducible
experiment. The [Python guide](docs/PYTHON_SCIENCE.md) covers notebooks, CSV/table
imports, graph expressions, result export and exact replay. The native JSON CLI
also works without Python; its request schema and semantics are documented in
[the science interface](docs/SCIENCE_INTERFACE.md).

Native analytics include degree statistics, weak/strong components, unweighted
shortest paths, PageRank and exact cosine/Euclidean vector neighbors. Existing
engine plans provide temporal joins, graph-valued metadata, provenance,
clustering and geometry. Every analysis reads through the engine's current
authorization rules and records its selected snapshots. Algorithms use bounded
in-memory inputs; [validation and measurements](docs/SCIENCE_VALIDATION.md)
describe tested behavior and practical limits.

Browser/mobile applications are deferred. No app, compiler, server or network
service is required to run native JSON/Python experiments.

Implemented: immutable SQLite graph snapshots with optimistic concurrency, distinct entity/space/manifestation IDs, first-class edges, graph-valued node and edge metadata, pinned temporal queries, reusable graph values and exact identity-space temporal path joins, transitive provenance visibility, atomic graph commits and durable events, unsigned hash-verified capsules with explicit reception/acceptance and offline branches, schema-safe graph algebra with four-valued support, named cyclic metadata, durable scoped dispatch and effect receipts, and principal-scoped live views with explicit ticks and freshness.

```sh
cargo test --workspace --locked
cargo run -p weave-engine -- run --db demo.db --actor demo --write demo examples/demo.json
```

The CLI is a **trusted local host**: `--actor` selects a principal and `--write` grants graph write authority. It is not an authentication service or safe remote multi-user endpoint. Plans cannot grant themselves authority. Every plan is transactional; repeated initial commits reject instead of overwriting an existing graph.

The companion [Weave language](https://github.com/weave-graph/weave-language) compiles into the shared versioned contract. The engine owns the canonical I/O-free [`weave-contract`](crates/weave-contract/src/lib.rs) crate; the language vendors an exact copy for independent builds.

See [current evidence and limitations](docs/STATUS.md), [implementation plan](docs/IMPLEMENTATION_PLAN.md), [workflow DAG](docs/WORKFLOW.md), [contract](docs/contract/v0.21/README.md) and [source provenance](docs/SOURCES.md). Broader joins, selective peer synchronization, complete permission/governance semantics, broader geometry and incremental clustering remain future work. Browser/mobile application acceptance is outside the current delivery scope. The [original white papers](docs/source/README.md) have been recovered and [reconciled](docs/RECONCILIATION.md). MIT licensed.

Native store29 adds [local causal dispatch and scoped lag](docs/CAUSAL_DISPATCH.md). Kernel-bound ancestry suspends actual mutual adapter feedback before leasing the blocked source. Explicit owner policy changes preserve history; bounded diagnostics recheck current visibility. Full remote/taxonomy/resource and original assurance requirements remain open.

The native [request2 host interface](docs/HOST_LIFECYCLE.md) exposes owner lifecycle,
compiled reconstruction/version transfer, recorded actors and scoped diagnostics
through the existing Rust/C/Swift embedding. Genuine old actor histories and two
actual compiler sources have separate [recovery evidence](docs/VERIFICATION_HOST_LIFECYCLE.md).
Protocol0.21, store29 and the original request1/legacy C entry points remain compatible.

The experimental [generic browser host](docs/BROWSER_HOST.md) transports raw
request2/source artifacts through the shared Rust facade and fences complete
SQLite plus SDK inventory generations in IndexedDB. Actual compiled completion,
reconstruction, version transfer and rollback have separate
[browser recovery evidence](docs/VERIFICATION_BROWSER_HOST.md). Full portable
actor/effect execution and browser/mobile application acceptance remain open.

Native store28 adds [effect-aware recorded actor cancellation](docs/ACTOR_DISPOSITION.md). Unknown outcomes require actual reconciliation; undispatched intents can be retired atomically with state/checkpoint, pause/rebuild and audit. Exact old retries preserve later work. Future delivery requires explicit owner initialization.

Native store27 adds [recorded actor version transfer and historical observation](docs/ACTOR_LIFECYCLE.md). Rollback restores an actual recorded pair and defaults historical deliveries to original receipt observation without new tool calls, graph writes or external actions. Source/portable and broader lifecycle/assurance requirements remain open.

Native store26 adds [recorded native actor state and effect recovery](docs/RECORDED_ACTORS.md), with actual stored tool artifacts, terminal effect outcomes and atomic output/state/private checkpoint pairing. The independent native journal/sink profile observes one physical action across lost acknowledgment and exact retries. Actor lifecycle and source/portable execution remain required.

Native store25 adds kernel-computed pure stateless snapshot reconstruction after replay expiry, alongside compatible source-compiled stateless upgrade, recorded-pair rollback and [explicit owner cancellation and pure state/artifact/checkpoint upgrade and rollback](docs/ADAPTER_LIFECYCLE.md), preserving prior retention data and immutable output provenance. Full source/portable lifecycle and actor/effect/resource profiles remain open.

Trusted native store22 administration adds conservative reachability collection, genuine owned pins, verifiable erasure anchors and explicit atomic projection/view rebuild after expired replay. See [retention limits and remaining work](docs/RETENTION.md). Protocol0.21 and capsule0.4 remain unchanged.

Replica-local recorded and genuine governed acceptance history now have explicit source selectors and bounded half-open ranges. Empty values retain their exact observation witnesses; current authority governs historical reads and cached reuse. See [0.21 verification](docs/VERIFICATION_021.md).

Finite rule closure is available through the [0.7 contract](docs/contract/v0.7/README.md); signed host admission has a separate [security boundary](docs/ADMISSION.md).

Exact context selection and qualified metadata access are described in the [0.8 contract](docs/contract/v0.8/README.md).

Authorized assertion-backed geometry and graph-valued explanations are described in the [0.9 contract](docs/contract/v0.9/README.md).

Declared counterpart bridge selection is available in the [0.10 contract](docs/contract/v0.10/README.md); it does not yet implement governed identity merging or splitting.
