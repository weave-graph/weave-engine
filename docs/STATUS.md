# Implementation status and evidence

This is a **foundation checkpoint**, not completion of the full project. Original papers are recovered and reconciled; full runtime semantics and public release remain open.

## Implemented checkpoint

- Portable `weave-contract` serde types and explicitly versioned JSON plans.
- Native SQLite immutable revisions, whole-snapshot commits with compare-and-swap, host recording time and half-open valid-time filtering.
- Entity/manifestation/space separation, graph-valued node and edge metadata, pinned metadata expansion and explicit missing-dependency coverage.
- Trusted host write grants and principal read filters. Transitive derivation restrictions withhold unauthorized/unavailable conclusions; incomplete derivations mark generic partial coverage. This is not signed capability authorization or a remote authentication boundary.
- Contract 0.2 exact identity-space path joins, interval intersections, deterministic output IDs and complete multi-revision snapshot vectors; 0.1 plans retain explicit compatibility.
- Contract 0.3 immutable program-local graph values and recursive query/join/filter/reference expressions, preserving leaf provenance without hidden persistence. Cumulative byte budgets reject repeated-reference amplification and roll back earlier commits.
- Hash-verified unsigned snapshot capsules, idempotent quarantined receive, explicit acceptance and offline forks through the trusted Rust host API.
- Whole-program atomic graph changes and durable event rows. A reference audit adapter supports pause, retry, dead letters, explicit dead-letter replay and durable duplicate suppression for local logical effects.

## Verified evidence

On the local macOS host, `cargo test --workspace --locked` passes 24 integration tests: 20 in `crates/weave-engine/tests/runtime.rs` and 4 independently authored review tests in `crates/weave-engine/tests/root_review.rs`. `cargo clippy --workspace --all-targets --locked -- -D warnings` and `cargo fmt --all -- --check` pass.

The orchestrator independently compiled the language example, executed it using the engine CLI, reopened the database in a separate process and checked pinned results, incomplete metadata coverage, half-open interval boundaries, denied write authority and stale-head rejection. Its local integration harness remains orchestrator-owned. `cargo check -p weave-contract --target wasm32-unknown-unknown --locked` also passes for the contract only. The native SQLite engine is not yet a portable WASM runtime. These checks do not establish CI success on GitHub, mobile/browser behavior or distributed conformance.

## Gate status

| Gate | Status | Evidence / remaining scope |
|---|---|---|
| E00 | in progress | Original papers recovered, hashed and reconciled; publication handled by orchestrator |
| E01 | in progress | 0.1 compatibility and 0.2 typed contract implemented and consumed by language; broader contract/golden compatibility work pending |
| E02 | in progress | Atomic immutable SQLite snapshots tested; n-ary model, migrations and general branch lifecycle pending |
| E03 | in progress | Pinned filtered queries, provenance and two-input identity-space path joins tested; general graph functions and recursive rules pending |
| E04 | in progress | Basic trusted-host read/write boundaries plus transitive evidence restriction tests; full capability/topology privacy not implemented |
| E05 | in progress | Atomic event storage, ordering of event enumeration and durable delivery records; crash-injection matrix and streaming/checkpoint protocol pending |
| E06 | in progress | Local reference audit adapter; full lifecycle/isolation and external effect broker pending |
| E07 | proposed | Incremental views and live subscriptions remain open |
| E08 | in progress | Whole-snapshot capsule export/receive and dependency boundaries tested; mounts and complete portable graph fragments pending |
| E09 | in progress | Independent local branches and explicit accepted heads tested; signed peer exchange and governance pending |
| E10–E15 | proposed | See complete requirements and workflow; not satisfied by this checkpoint |

Source reconciliation is complete against the original papers. Implementation gates remain open according to their acceptance evidence; publication and final gates remain orchestrator-owned.

## Limits

No production authentication, signed capabilities, revocation, general join planning, accepted-view governance, decentralized sync, signed/full capsules, automatic clustering, geometry operators, mobile/browser persistent engine or arbitrary external adapter execution is claimed. System timestamps record host wall time; queries select historical system state through immutable revision IDs, not yet a system-time range operator. Metadata values are JSON values with pinned graph references; rich typed schemas are future work. Event enumeration is ordered, but callers manually selecting events can deliver out of order. Revision IDs and graph presence may disclose topology to local callers; a remote deployment must complete E04 first.

The orchestrator independently verified actual compiled 0.2 joins and 0.3 join→filter→join execution, leaf provenance, input snapshot preservation and no hidden persistence. Same-transaction cyclic metadata remains unimplemented because current revision digests include pinned references; [ADR 0001](architecture/ADR-0001-revision-identity-and-content.md) records the future identity/content separation requirement.

Resource checks also run directly against `Engine::query` and `Engine::join`: incremental metadata, output-node, edge, provenance and origin accounting rejects oversized materialization during construction, before program output is retained. The byte ceiling is a serialized-data bound, not a measured process-RSS guarantee; a hardened resource-isolated host remains part of E14.
