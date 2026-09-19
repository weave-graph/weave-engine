# Implementation status and evidence

This is a **foundation checkpoint**, not completion of the full project. Original paper reconciliation, full runtime semantics and public release remain open.

## Implemented checkpoint

- Portable `weave-contract` serde types and explicitly versioned JSON plans.
- Native SQLite immutable revisions, whole-snapshot commits with compare-and-swap, host recording time and half-open valid-time filtering.
- Entity/manifestation/space separation, graph-valued node and edge metadata, pinned metadata expansion and explicit missing-dependency coverage.
- Trusted host write grants and principal read filters. Transitive derivation restrictions withhold unauthorized/unavailable conclusions; incomplete derivations mark generic partial coverage. This is not signed capability authorization or a remote authentication boundary.
- Whole-program atomic graph changes and durable event rows. A reference audit adapter supports pause, retry, dead letters, explicit dead-letter replay and durable duplicate suppression for local logical effects.

## Verified evidence

On the local macOS host, `cargo test --workspace --locked` passes 12 integration tests: 9 in `crates/weave-engine/tests/runtime.rs` and 3 independently authored review tests in `crates/weave-engine/tests/root_review.rs`. `cargo clippy --workspace --all-targets --locked -- -D warnings` and `cargo fmt --all -- --check` pass.

The orchestrator independently compiled the language example, executed it using the engine CLI, reopened the database in a separate process and checked pinned results, incomplete metadata coverage, half-open interval boundaries, denied write authority and stale-head rejection. Its local integration harness remains orchestrator-owned. These checks do not establish CI success on GitHub, mobile/browser behavior or distributed conformance.

## Gate status

| Gate | Status | Evidence / remaining scope |
|---|---|---|
| E00 | blocked | Papers require recovery/reconciliation; public organization/repositories handled by orchestrator |
| E01 | in progress | 0.1 typed contract implemented and consumed by language; broader contract/golden compatibility work pending |
| E02 | in progress | Atomic immutable SQLite snapshots tested; n-ary model, migrations and general branch lifecycle pending |
| E03 | in progress | Pinned filtered graph queries and provenance tested; joins, general graph functions and recursive rules pending |
| E04 | in progress | Basic trusted-host read/write boundaries plus transitive evidence restriction tests; full capability/topology privacy not implemented |
| E05 | in progress | Atomic event storage, ordering of event enumeration and durable delivery records; crash-injection matrix and streaming/checkpoint protocol pending |
| E06 | in progress | Local reference audit adapter; full lifecycle/isolation and external effect broker pending |
| E07–E15 | proposed | See complete requirements and workflow; not satisfied by this checkpoint |

E01–E06 proceed provisionally on known explicit requirements while E00 remains blocked. This override permits implementation work but does not waive source reconciliation or final gate dependencies.

## Limits

No production authentication, signed capabilities, revocation, general joins, accepted-view governance, decentralized sync, capsules, automatic clustering, geometry operators, mobile/browser persistent engine or arbitrary external adapter execution is claimed. System timestamps record host wall time; queries select historical system state through immutable revision IDs, not yet a system-time range operator. Metadata values are JSON values with pinned graph references; rich typed schemas are future work. Event enumeration is ordered, but callers manually selecting events can deliver out of order. Revision IDs and graph presence may disclose topology to local callers; a remote deployment must complete E04 first.
