# Implementation status and evidence

Public MIT implementation in progress; recovered original papers are reconciled. This checkpoint implements a verified subset, not completion of the papers.

## Implemented

- Versions 0.1–0.8: immutable SQLite snapshots/CAS, half-open time, principal-filtered pinned queries, interval-aware identity-space joins and reusable pure graph values.
- Version 0.4 adds immutable embedded schemas, typed joins, named host/time-qualified metadata, atomic logical snapshot manifests with constructible metadata cycles, required dependency checks, live handle pinning, and no-op suppression. See [contract](contract/v0.4/README.md).
- Signed per-operation native Query/Publish/Propose admission with durable nonce receipts, accepted-branch scope checks, isolated proposal quarantine and subject-scoped egress; [security boundary](ADMISSION.md). No remote arbitrary-program authority or transport server is implied.
- Trusted host read/write boundaries, transitive derivation restrictions, generic partial coverage, bounded materialization and whole-program rollback.
- Version 0.6 explicitly separates structural edges from source assertions, preserving source/context/identity through graph values; source manifests and portable computation/explanation identities are included.
- Version 0.8 exact context selection, qualified metadata paths and scoped derived status/explanation nodes prevent implicit default/pinned mixing; [profile](contract/v0.8/README.md).
- Version 0.7 finite range-restricted graph rules with temporal proof closure, host work budgets, explicit negative evidence and live-view retraction; [profile](contract/v0.7/README.md).
- Version 0.5 bounded union/diff/project and time-specific four-valued support, with typed composition, pinned node origins and persisted alternative derivation groups.
- Durable principal-scoped named views, exact-snapshot recomputation, explicit freshness, time ticks and membership retractions; [view boundaries](VIEWS.md).
- Scoped durable adapter dispatch with leases, lifecycle/retry/dead letters, atomic graph writes plus receipt/checkpoint, and explicit unknown external effect intents; [recovery boundary](DISPATCH.md).
- Unsigned hash-verified capsules, quarantine receive, explicit acceptance and offline branches. Capsule 0.2 carries whole authorized logical manifests, rejects equivocation and cross-receipt ancestry cycles; [transport boundaries](CAPSULES.md). Live handles, signatures and selective manifest proofs remain open.

- Portable typed physical/embedding geometry, explicit frame/unit transforms, restriction-preserving lineage and display-only projection; [geometry boundary](SPACES.md). Pure host-authorized API; graph service integration remains open.
- Stored revision/manifest integrity checks, transactional schema initialization and backup/legacy-upgrade acceptance; [recovery evidence](STORAGE_RECOVERY.md).

## Evidence

`cargo test --workspace --locked`, formatting, clippy and the contract WASM target are checked locally. Independent executable acceptance is committed under `scripts/root_*`: language compilation into runtime, revision/CAS/time checks, joins, graph values, ten metadata checks, eight original-paper example checks, and seven process-death dispatch/effect checks. Public CI executes native tests and contract WASM checks. Runtime SQLite remains native, not a browser/mobile persistence implementation.

## Gates

E00 is passed: original source files/hash provenance, reconciliation and public MIT repository baseline are verified. E01–E09, E12 and E14 remain in progress. Other gates remain open as recorded in [workflow](workflow.json). N-ary relations and reward-based traversal learning are optional in the papers; neither is a mandatory completion blocker.

Remaining required work includes broader structural-object queries and typed assertion properties, richer graph algebra/schema semantics, accepted-view governance, broader capability operations and topology privacy, scoped dispatcher/adapter recovery, subscriptions, authenticated sync, selective capsules, clustering/geometry, full portable runtime and system-level conformance. Existing local APIs remain trusted host administration; the signed per-operation facade is separately bounded and does not constitute a production remote service. System history selects revisions; general system-time range queries remain open. Serialized byte budgets are not measured process-RSS isolation guarantees.
