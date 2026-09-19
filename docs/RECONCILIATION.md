# Original paper reconciliation

Baseline: engine commit `283d4fb`; original papers dated 11 September 2026, supplied 19 September 2026. `docs/source/manifest.json` authenticates the copied bytes against the supplied bundle, not the scientific validity of their claims. All source sections below are architecture requirements unless explicitly labelled optional/research in the papers.

| Paper section | Requirement | Baseline evidence and gap | Gate |
|---|---|---|---|
| Engine 2; Language 2 | Separate entity, manifestation, structural edge and source assertion identities | Entity/space/node IDs exist; edge currently conflates relationship and assertion; structural endpoint/predicate immutability needs enforcement | E02 |
| Engine 2; Language 3.2 | Addressable named metadata with assertion context, policy and revision; hosts include graph/entity/node/edge | Legacy anonymous GraphRef vectors insufficient; add independently attributable attachments without silently reinterpreting old hashes | E02,E04 |
| Engine 2.1; Language 3.3 | Cyclic logical references resolved through snapshot manifest, storage DAG acyclic | Existing hash-as-revision model cannot construct new cyclic metadata; ADR 0001 replacement now supported by authoritative source | E02 |
| Engine 2.2; Language 3.1 | Atomic inline metadata graph+binding creation | Program transactions atomic; planned logical batch addresses cross-graph same-transaction references | E02 |
| Engine 2.2 | Reachability GC, shared references, retention roots, tombstones and expired replay | No GC, retained checkpoint or compaction protocol implemented | E02,E05,E14 |
| Engine 3.1 | Atomic commits and outbox publication | SQLite transaction+event tested; systematic crash injection still required | E05 |
| Engine 3.2–3.3 | CloudEvents-compatible versioned scoped envelope, namespace authority, causal context, meaningful taxonomy | Minimal envelope and commit/accept events; no scoped alias or full event schema registry | E05,E04 |
| Engine 3.4 | Durable checkpoints, ordering scope, retention, backpressure and lag | Reference delivery rows/retries exist; checkpoint expiration and dispatcher scheduling pending | E05 |
| Engine 4–4.2 | Manifest-pinned adapter lifecycle and atomic command receipts | Only trusted local audit adapter; no sandboxed SDK, manifest upgrades or general command receipts | E06 |
| Engine 4.3 | Authorized effect-intent ledger and unknown outcomes | Not implemented; no arbitrary exactly-once external-effect claim | E06 |
| Engine 4.4 | Replay safety, no-op change suppression, causal loop controls | Dead-letter replay exists; no-op commits currently generate changes and must be corrected; loop scheduler pending | E05,E06 |
| Engine 5; Language 3.1 | Live handles pinned once per evaluation, reverse-dependency indexes, freshness watermark | Pinned refs/explicit partial coverage exist; live metadata and watermark controls pending | E03,E07 |
| Engine 5; Language 4–5 | Complete graph-result manifest, schema/plan/rule/visibility fingerprints, terminating rule profile | Composition/leaf provenance implemented; richer schemas/fingerprints, rule engine and temporal sequences pending | E01,E03 |
| Engine 6; Language 2.1 | Authorized counterpart virtual adjacency, accepted identity mappings/splits | Shared entity IDs exist, but no resolver/candidate policy/history of mapping splits | E04,E12 |
| Engine 6.1; Language 6 | Typed geometry, vectors, embedding versions and explicit bridges | Plain properties are not typed geometry support | E12 |
| Engine 7; Language 7 | Event-reactive overlapping clusters, metadata membership/lineage, lazy progress, exact-search safety | Not implemented; select measured baseline algorithms without inventing quality guarantees | E13 |
| Engine 8; Language 8 | Signed portable manifests and distinct mount/replicate/fork/integrate/detach | Unsigned integrity capsules and local forks exist; mounts, authentication, verified partial disclosure and semantic merge pending | E08,E09 |
| Engine 9; Language 5,8 | Replica receipt and accepted-view times; semantic-category reconciliation | Local recorded time and immutable history exist; source clocks intentionally untrusted; concurrent values/attachment conflicts need actual integration algebra | E09 |
| Engine 10; Language 9 | Scope-specific capabilities, use-time checks, subscription privacy, trusted policy installation | Trusted local HostContext only; must not be exposed as remote auth | E04,E11 |
| Engine 10.2 | Governed proposals and preceding-policy-authorized transitions | Owner acceptance exists; reviewer/threshold governance absent | E11 |
| Engine 11 | Observable lag, stale views, missing dependencies, unknown effects, independent backpressure | Generic diagnostics and input/output budgets exist; complete operational observability pending | E05–E14 |
| Engine 12 | Complete phone evidence→adapter→cluster→peer→governance→effect scenario | Only local slices implemented; complete end-to-end and mobile acceptance remain required | E15 |
| Engine 13.1; Language 12 | Crash, duplicate/reorder, metadata cycles, security and convergence experiments | 24 local integration tests; not equivalent to required systematic experiments | E14 |
| Engine 13.2 | Formal models, reviewed cryptographic partial replication, clustering quality, identity splits | Explicit research/assurance work; cannot be marked solved from code alone | E14 |
| Engine 13.2 | Reward-based traversal learning | Explicit optional experimental extension, outside core completion | optional |
| Language 2 | N-ary relation form | Explicitly optional; ordinary first-class edges are required, n-ary syntax is not a core blocker | optional |

## Adopted next changes

1. Preserve old contract and content-hash decoding; add logical revision batches whose graph refs resolve through an immutable snapshot manifest with separate content digests.
2. Add named contextual metadata attachments and portable schema descriptors. Runtime validates what the compiler checks statically; host authority never comes from source data.
3. Enforce structural edge identity and suppress no-op change events. Keep assertion support/history distinct from structural identity.
4. Continue gate sequence through durable dispatcher/adapters, signed selective mobility, governed views, geometry/clustering and portable hosts. Each gate remains evidence-based; source recovery does not make any runtime feature complete.

The complete R01–R40 mapping remains in `IMPLEMENTATION_PLAN.md`. Source reconciliation resolves the source-recovery blocker, while public repository creation and final acceptance remain orchestrator-owned.
