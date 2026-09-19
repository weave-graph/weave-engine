# Weave Engine

A local-first runtime for multidimensional temporal knowledge graphs. This repository contains an executable Rust foundation and a full implementation roadmap; it is not yet the complete white-paper runtime.

Implemented: immutable SQLite graph snapshots with optimistic concurrency, distinct entity/space/manifestation IDs, first-class edges, graph-valued node and edge metadata, pinned temporal queries, reusable graph values and exact identity-space temporal path joins, transitive provenance visibility, atomic graph commits and durable events, unsigned hash-verified capsules with explicit reception/acceptance and offline branches, schema-safe graph algebra with four-valued support, named cyclic metadata, durable scoped dispatch and effect receipts, and principal-scoped live views with explicit ticks and freshness.

```sh
cargo test --workspace --locked
cargo run -p weave-engine -- run --db demo.db --actor demo --write demo examples/demo.json
```

The CLI is a **trusted local host**: `--actor` selects a principal and `--write` grants graph write authority. It is not an authentication service or safe remote multi-user endpoint. Plans cannot grant themselves authority. Every plan is transactional; repeated initial commits reject instead of overwriting an existing graph.

The companion [Weave language](https://github.com/weave-graph/weave-language) compiles into the shared versioned contract. The engine owns the canonical I/O-free [`weave-contract`](crates/weave-contract/src/lib.rs) crate; the language vendors an exact copy for independent builds.

See [current evidence and limitations](docs/STATUS.md), [implementation plan](docs/IMPLEMENTATION_PLAN.md), [workflow DAG](docs/WORKFLOW.md), [contract](docs/contract/v0.5/README.md) and [source provenance](docs/SOURCES.md). General joins, signed peer synchronization, capabilities/governance, real mobile/browser persistence, geometry and automatic clustering remain open. The [original white papers](docs/source/README.md) have been recovered and [reconciled](docs/RECONCILIATION.md). MIT licensed.
