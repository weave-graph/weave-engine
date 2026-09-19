# Implementation status and evidence

Public MIT implementation in progress; recovered original papers are reconciled. This checkpoint implements a verified subset, not completion of the papers.

## Implemented

- Versions 0.1–0.4: immutable SQLite snapshots/CAS, half-open time, principal-filtered pinned queries, interval-aware identity-space joins and reusable pure graph values.
- Version 0.4 adds immutable embedded schemas, typed joins, named contextual metadata, atomic logical snapshot manifests with constructible metadata cycles, required dependency checks, live handle pinning, and no-op suppression. See [contract](contract/v0.4/README.md).
- Trusted host read/write boundaries, transitive derivation restrictions, generic partial coverage, bounded materialization and whole-program rollback.
- Durable graph event rows and reference local audit adapter with retry, deduplication, pause and dead-letter replay.
- Unsigned hash-verified legacy snapshot capsules, quarantine receive, explicit acceptance and offline branches. Logical manifest transport is the next capsule format; unsupported exports fail explicitly.

## Evidence

`cargo test --workspace --locked`, formatting, clippy and the contract WASM target are checked locally. Independent executable acceptance is committed under `scripts/root_*`: language compilation into runtime, revision/CAS/time checks, joins, graph values, ten metadata checks, and eight original-paper example checks. Public CI executes native tests and contract WASM checks. Runtime SQLite remains native, not a browser/mobile persistence implementation.

## Gates

E00 is passed: original source files/hash provenance, reconciliation and public MIT repository baseline are verified. E01–E06 and E08–E09 remain in progress. Other gates remain open as recorded in [workflow](workflow.json). N-ary relations and reward-based traversal learning are optional in the papers; neither is a mandatory completion blocker.

Remaining required work includes separate structural-edge/source-assertion identity, richer graph algebra/schema semantics, accepted-view governance, signed capability and topology privacy, scoped dispatcher/adapter recovery, subscriptions, authenticated sync, selective capsules, clustering/geometry, full portable runtime and system-level conformance. Existing reader filters operate inside a trusted local host boundary, not production remote authentication. System history selects revisions; general system-time range queries remain open. Serialized byte budgets are not measured process-RSS isolation guarantees.
