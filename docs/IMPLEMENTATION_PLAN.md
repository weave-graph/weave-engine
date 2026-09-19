# Weave Engine implementation plan

Status: implementation in progress; executable foundation described in [STATUS.md](STATUS.md), full scope remains open. Owner: engine agent, coordinated by the parent orchestrator. License target: MIT. Public repository target: `weave-graph/weave-engine`.

## Scope and source authority

The engine is the local-first execution, persistence, event, adapter, replication and policy runtime for Weave. Completion means the full requirements below pass their acceptance gates, including portable execution, clustering and governed reconnection. An early vertical slice is a checkpoint, not project completion.

The complete original language and engine papers are now available in [docs/source](source/README.md), with exact hashes. [RECONCILIATION.md](RECONCILIATION.md) records source requirements, current implementation gaps and gate assignments. Their syntax is illustrative; their requirements and explicit research/optional boundaries are authoritative for this plan.

The user explicitly requests multidimensional manifestations, automatic recursive clustering and zoom, decentralized attachment/detachment, permissions, optional governance, 3D and embedding spaces, mobile/personal offline branching, an event bus with registered adapters, and graph-valued metadata on nodes and edges. Temporal joins and parametrized knowledge graphs originate in the initial request. Bitemporal semantics, capsules, capability machinery, transactional outbox and specific algorithms are proposed mechanisms requiring explicit architecture records, not independently mandated vendor choices.

## Adopted architecture direction

Use a Rust semantic core for one set of invariants across native command-line tools, embedded applications, WASM and mobile bindings. Keep persistence, clocks, randomness, identity signing, networking and effects behind host traits. A small portable core prevents a server-only API from masquerading as an offline engine. The orchestrator adopted this direction for the foundation; concrete dependency choices remain documented and revisable.

Native persistence starts with SQLite transactions, an append-only logical revision journal and an outbox in the same transaction. Immutable content-addressed blocks support capsules and revision exchange; stable semantic IDs permit cyclic knowledge graphs without cyclic hash construction. Canonical encoding, hash domain/version and migration semantics must be shared and tested before persistence is durable. WASM uses the same semantics with an in-memory baseline followed by transactional browser persistence; mobile needs a native binding and real device/emulator acceptance, not only a successful cross-compile.

Modules:

| Module | Owns | Must not own |
|---|---|---|
| `weave-model` | IDs, nodes, edges/assertions, manifestations, graph refs, intervals, restrictions, diagnostics | Host effects or globally mutable clock |
| `weave-store` | Atomic commits, immutable revisions, branch heads, snapshot reads, outbox | External effect execution |
| `weave-query` | Versioned plan interpretation, joins, provenance, incremental views | Hidden networking or LLM calls |
| `weave-policy` | Discover/read/traverse/propose/publish/delegate checks, restriction propagation | Claims of objective truth |
| `weave-bus` | Durable event log, subscriptions, checkpoints, retries, dead letters | Exactly-once claims for arbitrary external effects |
| `weave-adapters` | Registration, lifecycle, manifest validation, budgets and effect broker | Unrestricted authority inherited from the host |
| `weave-sync` | Capsule boundaries, signed revisions, permitted replication, quarantine and integration | Automatic acceptance of received claims |
| `weave-spaces` | Counterparts, typed bridges, coordinate and embedding validation | Treating similarity as identity |
| `weave-clusters` | Versioned grouping, lineage, semantic zoom and authorized summaries | Destructive replacement of source data |
| `weave-host` / CLI | Configuration, persistence wiring, transport, local operations and diagnostics | A second semantic implementation |

The bus provides at-least-once durable delivery, stable event IDs and ordering within a declared stream. It uses transactional graph+outbox commits, adapter checkpoints and deduplication. Adapters producing graph changes use a commit idempotency key. External effects require an effect ledger plus a destination idempotency key when available; destinations without idempotency expose uncertain outcomes and a reconciliation state. Replay defaults to observation/reconstruction; reissuing external actions requires explicit policy.

## Shared contract with the language project

Both projects must adopt one versioned contract and identical golden fixtures. The language compiler owns parsing, type/effect checking and plan production; the engine validates every plan and enforces authorization and data-dependent checks. The engine never trusts a compiler as a security boundary.

The contract must specify:

- Separate `EntityId`, `ManifestationId`, `SpaceId`, `GraphId`, `BranchId`, `RevisionId`, `AssertionId`, `EventId` and `PeerId`; a manifestation references one entity and space.
- First-class directed edges/n-ary assertions with identities. Both nodes and edges carry scalar metadata and `GraphRef { graph_id, revision }` metadata. Metadata graph cycles and shared references are legal, with bounded traversal and missing/denied/cyclic diagnostics.
- Valid-time half-open intervals and runtime-assigned system revisions/times; explicitly pinned snapshot vectors. Remote snapshots do not imply a globally atomic distributed cut.
- Plan operators for graph inputs, authorized scans, filters, joins, projection/emission, graph metadata traversal, union and explicit policy resolution. Effects are declared separately from pure plans. Recursion requires resource bounds or a terminating fragment.
- Result envelope containing graph, provenance, snapshot vector, assumptions, completeness/coverage, unavailable dependencies and structured diagnostics. Unknown, unsupported, denied and empty are distinguishable.
- Event envelope containing version, event ID, type, graph/branch/revision, stream sequence, actor, causation/correlation IDs, payload reference and restrictions. No confidential payload is leaked through routing metadata.
- Deterministic canonical encoding and hashing; exact handling of numeric values, Unicode, map ordering, invalid intervals, unknown schema versions and timestamps. Golden fixtures include rejection cases.
- A negotiated protocol/capability version. Unsupported features fail explicitly; no silent lowering of policy or consistency.

Adopted contract ownership: the engine repository owns `crates/weave-contract` and canonical `docs/contract/v0.1`; the language repository owns syntax and plan production. The language vendors the exact standalone contract crate with a hash manifest until a published package strategy exists. Both owners coordinate changes and compare fixtures.

## Requirement-to-deliverable-to-test map

`U` means an explicit user requirement; `P` means a source proposal adopted for implementation planning. Gate IDs reference [the workflow](WORKFLOW.md). Every row remains in scope until explicitly changed by the user.

| ID | Source | Requirement and deliverable | Acceptance evidence | Gate |
|---|---|---|---|---|
| R01 | U | Nodes and directed edges as referenceable objects; n-ary extension optional | Round trip identities, cycles, edge arguments and edge queries | E01,E02 |
| R02 | U | Scalar and graph-valued metadata on both nodes and edges | Shared, recursive, missing, denied and offline metadata tests; bounded traversal | E02,E03 |
| R03 | U | Same entity manifested in multiple spaces, discoverable counterparts | Identity stable across manifestations; state distinct; hidden counterparts not leaked | E02,E04 |
| R04 | U/P | Temporal assertions, immutable bitemporal revisions | Late arrival/correction queries distinguish valid/system time; historical revision unchanged | E02 |
| R05 | U | Joinable parameterized graph computations | Compile language fixture and evaluate into reusable graph result | E03,E12 |
| R06 | P | Identity/context/time-aware joins | Disjoint intervals yield no simultaneous conclusion; scope mismatch rejects; mappings pinned | E03 |
| R07 | P | Union, derivation and conflict resolution are separate | Positive/negative support coexist; resolver never erases originals | E03 |
| R08 | P | Provenance and reproducibility | Same plan, inputs and snapshots reproduce output; explain traces all premise revisions | E03 |
| R09 | P | Explicit partial/unknown coverage | Unavailable peer/metadata never becomes a complete empty answer | E03,E08 |
| R10 | U | Event bus and registered reactive adapters | Commit event reaches registered adapter and adapter produces authorized result | E05,E06 |
| R11 | P | Atomic graph commit and durable event publication | Crash before/after every transaction boundary has neither ghost event nor lost committed event | E05 |
| R12 | P | Delivery retries, ordering, replay and duplicate handling | Duplicate, reorder, disconnect, restart and poison event matrix; checkpoint persists | E05 |
| R13 | P | Adapter lifecycle and resource/effect isolation | Register/start/pause/resume/upgrade/remove; time/memory quotas; denied effect fails closed | E06 |
| R14 | P | Controlled external actions | Idempotent destination receives one effect; uncertain outcome enters reconciliation; replay safe | E06 |
| R15 | P | Incremental materialized views and retraction | Differential full recompute equivalence after corrections, deletions and rolling-window changes | E07 |
| R16 | U | Attach and detach independently of merge | Mounted graph queryable without acceptance; detach preserves retained authorized snapshot | E08 |
| R17 | U/P | Portable capsules with explicit boundaries | Export/import identities, schemas, history, metadata and dependencies; truncated/tampered capsule rejects | E08 |
| R18 | U | Offline branches for personal compute | Independent local creation/query/evidence edits survive process restart with network disabled | E09 |
| R19 | U | Offline branches for mobile compute | Same scenario on actual mobile host/emulator; storage quota/crash/reconnect outcomes recorded | E10 |
| R20 | U/P | Decentralized peer sync, preserving identities/history | Three peers partition/reconnect; duplicates do not duplicate assertions or evidence strength | E09 |
| R21 | P | Receiving, integrating and accepting are separate | Unauthorized/invalid proposal quarantined; accepted view changes only through current policy | E09,E11 |
| R22 | U | Permissions for visibility and traversal | Matrix tests for discover/read/traverse/propose/publish/delegate, metadata and counterpart topology | E04 |
| R23 | P | Restriction propagation to outputs/explanations/indexes | Pairwise noninterference tests: hidden facts cannot influence visible output absent release policy | E04,E07,E13 |
| R24 | P | Scoped capabilities and revocation boundary | Invalid signature/scope/expiry/delegation denied; reconnect checks current authority; offline limits documented | E04,E09 |
| R25 | U | Optional governance | Personal immediate publication and reviewed community publication both work | E11 |
| R26 | P | Versioned policy and immutable approval subjects | Old/changed proposal approval rejected; policy change cannot self-authorize; exclusive decisions ordered | E11 |
| R27 | U | Automatic recursive clustering without fixed depth | Lazy expansion across variable depths under budget; overlapping memberships preserved | E13 |
| R28 | P | Clusters are graph objects with lineage | Members, evidence snapshot, algorithm version, splits/merges queryable; originals unchanged | E13 |
| R29 | U/P | Semantic zoom and geometric zoom | Expand aggregate/evidence separately from camera; finite evidence boundary; exact query unaffected | E13 |
| R30 | P | Stable authorized clustering and summaries | Hysteresis/seed/version reproducibility; hidden input does not alter public grouping/count/layout | E13 |
| R31 | U | 3D geometry and directional vectors | Frame/unit validation, explicit transforms and valid time; moving display does not reverse relation | E12 |
| R32 | U | Embedding spaces and cross-dimensional bridges | Encoder/version/metric mismatch rejects; identity/relation/transform/learned bridges remain distinct | E12 |
| R33 | P | Approximate navigation cannot suppress exact matches | Approximate retrieval marked; exact result compared to exhaustive oracle on adversarial clusters | E12,E13 |
| R34 | P | Schema discovery and machine-readable diagnostics | Authorized describe API and stable error codes; unknown schema/version fails usefully | E01,E03 |
| R35 | P | Portable deterministic semantic core | Same golden corpus on native and WASM; no hidden wall-clock/network dependency | E10 |
| R36 | P | Storage migration, backup and recovery | Upgrade fixture, backup/restore, corrupt block detection and interrupted migration rollback | E14 |
| R37 | P | Resource limits and untrusted inputs | Fuzz capsule/plan/event parsing; graph cycle, huge interval, recursion and amplification budgets | E14 |
| R38 | P | Observable runtime | Structured trace links commit→event→adapter→effect; secrets and denied topology excluded | E05,E14 |
| R39 | U | Fully public MIT open source delivery | Public repository, license, build instructions, contribution/security guidance and reproducible CI | E00,E15 |
| R40 | U | Two projects completed under orchestration | Language/engine integration scenario and all requirement gates pass with evidence links | E15 |

## Stages and completion gates

1. **E00 — Source and project baseline.** Original papers recovered and reconciled; keep provenance, approve contract ownership and architecture ADRs; public MIT scaffold and repository workflows. Missing source attachment is tracked rather than silently substituted.
2. **E01 — Contract freeze.** Shared versioned schemas, compatibility policy, golden positive/negative fixtures and a cross-repository contract check. Zero ambiguous time, metadata or identity semantics in baseline.
3. **E02 — Durable graph core.** Model, atomic native store, revisions, branches and metadata graph resolution. Crash/restart and bitemporal scenarios pass.
4. **E03 — Query execution.** Language-produced plans, temporal/context joins, conflicts, provenance and coverage. Golden execution corpus and independent expected results pass.
5. **E04 — Authorization core.** Policy boundaries cover reads, topology and derivation before external adapters or peers ship. Threat model and negative tests required.
6. **E05 — Durable events.** Transactional outbox, subscriptions, replay, ordering and dedup checkpoints. Crash-injection matrix passes.
7. **E06 — Adapters and effects.** Lifecycle, quotas and effect broker; reference safe logger and graph enrichment adapters; demonstrable external-effect reconciliation.
8. **E07 — Incremental views.** Changes include removals and changed support; full-recompute oracle agrees after arbitrary mutation sequences.
9. **E08 — Capsules and mounts.** Portable bounded graph fragments and mount semantics, including metadata graphs and unavailable dependencies.
10. **E09 — Branches and sync.** Local writes during partition, permitted revision exchange, integration/conflicts, no replication-induced evidence inflation.
11. **E10 — Browser/mobile portability.** Native/WASM semantic parity and real persistent mobile/browser offline scenarios. Document supported hosts and measured constraints.
12. **E11 — Governance.** Optional accepted views, immutable proposal approvals and policy transitions; coordinated exclusive outcome demonstrates its ordering boundary.
13. **E12 — Spaces and geometry.** Typed 3D/embedding spaces, bridges, query operators and clear approximate coverage; shared language integration.
14. **E13 — Clustering and zoom.** Versioned overlap/lineage, on-demand semantic expansion, budget handling and visibility-safe summaries; exact queries never prune unsafely.
15. **E14 — Hardening and performance.** Fuzzing, recovery, migrations, threat-model review and reproducible workload measurements. Publish baseline numbers; do not fabricate an SLO before measurement.
16. **E15 — Full integration and public release.** All R01–R40 have passing evidence or an explicit user-approved scope change; clean installation on supported hosts; tagged source/build artifacts and public docs verified by parent orchestrator.

## Integrated acceptance scenario

An offline phone holds a capsule for an installation. A device has operational, physical and semantic manifestations. An edge's metadata references an evidence graph. The phone adds a time-scoped correction and evidence offline; the local transaction creates one committed revision and durable event. A registered adapter derives a restricted result without gaining extra authority. A living view retracts stale support. Semantic zoom opens a cluster and its evidence without revealing private counterparts. After reconnection, peers exchange permitted immutable revisions; the team reviews the exact proposal revision under a pinned governance policy. Integration preserves competing evidence and valid/system history. Replaying or resynchronizing never repeats an external action or counts replicated evidence twice. Queries disclose missing dependencies, selected snapshots and explanation graphs. The language repository compiles the scenario source and the engine executes it on native and portable hosts.

## Research and unresolved decisions

- Full papers are reconciled: reward-based traversal learning and n-ary relations are explicitly optional extensions. Required experiments and formal/cryptographic assurance work remain open.
- Select canonical wire encoding and integer/decimal/time precision. Decide whether schema migrations produce new revisions or independent storage transformations.
- Define deletion/redaction, retention and content addressing when legal or operational erasure is required; references cannot promise recall of already copied plaintext.
- Decide capability signing/key rotation and revocation freshness policy without making offline access dependent on constant connectivity.
- Contract ownership is resolved above; define later package publication and compatibility upgrades without divergent schemas.
- Choose browser persistence and mobile ABI after a portability spike. WASM compilation alone is insufficient for durable transactions.
- Evaluate incremental algorithms and clustering methods on evolving, overlapping, permission-filtered graphs. Record selection criteria and measured costs rather than declaring one algorithm universally correct.
- Define cross-peer logical ordering and exclusive-governance consistency. A snapshot vector is not global atomicity, and generic conflict-free replication does not preserve every domain invariant.
- Define deterministic handling of floating point embeddings, stochastic clustering, model versions and GPU results across hosts.
- Quantify limits for capsule size, metadata depth, events, fan-out, untrusted adapter work and exact/approximate queries before release.

## Evidence policy

Each gate gets an evidence record containing commit SHA, contract revision, command, host, result, artifact path and unresolved findings. A passing unit test does not imply a platform or distributed acceptance gate. CI status, public visibility and release availability must be checked live when publication occurs. No task is marked complete solely because code exists or a build succeeds.
