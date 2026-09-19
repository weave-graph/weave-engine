# Native accepted identity mappings

Status: implemented native trusted-host API on protocol 0.11, SQLite schema 7. This is a bounded acceptance profile, not completion of engine §6 or language §2.2. Signed remote acceptance, language resolver syntax and authenticated governance synchronization remain open.

The resolver must distinguish a proposal from an accepted identity decision. A source can claim two independently assigned entity IDs refer to the same entity; the claim itself cannot install a policy, gain authority, rewrite those IDs or merge accepted state.

## Host boundary

1. A trusted administrator installs an immutable named policy revision with explicit decision makers, allowed spaces and resource ceilings. Installation is outside graph plans and signed proposal ingress. A policy label is not authority by itself.
2. A candidate pins source NodeRefs, supporting assertion evidence, an exact valid-time interval and a policy revision. Receipt stores a candidate independently of the accepted mapping registry. Its data cannot preempt structural/schema identities or replace an accepted mapping.
3. A decision maker can accept, supersede or split a mapping using strict expected-head comparison. The immutable decision revision records the policy, candidate and source pins, actor, valid time and prior decision. Acceptance and its durable occurrence event must commit atomically; replay is an exact receipt, not a fresh decision.
4. A resolver selects a named immutable accepted mapping revision and a source manifestation. It exposes only authorized counterpart adjacency in the requested target space. Original entity IDs, manifestation IDs and local properties remain intact. The accepted bridge decision is an additional proof; it does not silently rewrite an entity ID or synchronize state.

A split is a new decision over explicit memberships; it does not delete historical accepted revisions. Historical selection continues to use its pinned mapping revision. Pairwise private identifiers remain usable because the accepted link is separate from the local identifiers.

## Proof and privacy boundary

Persisted virtual bridges and aggregate values can depend on private isolated nodes. AssertionRef-only gates cannot represent those inputs. The cycle/budget-safe NodeRef dependency gate covers query filtering, endpoint traversal, capsule closure and signed read retries. Raw plans cannot supply a result-origin envelope as proof of authorization.

Discoverability filters source and candidate memberships before producing adjacency. No endpoint API reports a total count including inaccessible or disconnected records. Do not eagerly materialize a global clique. Bound source loads, candidate scans, retained proof bytes and result objects; limits return explicit errors or generic partial coverage, not a false complete empty answer.

An accepted bridge keeps both source-node and decision proofs, exact context and time applicability. Context defaults are not broadcast into pinned worlds. Mathematical transforms, learned mappings and similarity remain distinct domains. Contradictory candidates stay evidence until the selected policy adjudicates them.

## Acceptance checks

- Candidate receipt does not advance a mapping head or publish adjacency.
- An uninstalled or forged policy and an unauthorized decision maker cannot accept.
- CAS conflicts and replayed decision bodies preserve a single accepted occurrence. Identity-specific process-death acceptance remains pending; the implementation uses one SQLite savepoint for graph, event, registry and receipt writes.
- Independently assigned entity IDs can become counterparts through an explicit accepted revision; later supersession/split changes current adjacency and preserves historical selection.
- Hidden source nodes, candidate membership and proof paths produce no global count or identifying error. Persisted bridge copies cannot shed source restrictions by clearing readers.
- Source properties remain unchanged. Coordinate transforms and similarity claims cannot satisfy identity acceptance.
- Initial trusted-host implementation remains explicitly separate from remote signed admission; broader operation scopes and governance adapters are subsequent integration work.

## Decision representation

A candidate replaces the explicit membership partition for one mapping. Every member is an exact graph/revision/node reference and occurs in at most one group. Singleton groups are permitted after a split. The system does not infer transitive links between groups or merge the original entity IDs. A new candidate with revised time or evidence produces a distinct decision body even if its members match an earlier decision.

Accepted storage uses one membership assertion per member, with source-node dependency protection. Partition association stays in the trusted internal SQL registry; public member records contain no group tag, membership digest or total count. It has no separate public node containing a total group count or full member list. A trusted registry binds the mapping ID, decision revision and named policy revision to an accepted head. Selecting a graph with similar data cannot forge a registry entry. Internal mapping graph identifiers need protection against raw commit/import preemption; the private acceptance path must not expose a client-selectable bypass flag.

A resolver finds the authorized source membership and emits adjacency only to authorized target-space memberships in that selected partition. Each returned bridge carries both membership assertion pins and both original NodeRefs. It retains original entity IDs, exact source NodeRefs, context and applicability. Returned member wrappers have explicit synthetic IDs and source-pointer properties; they do not pretend to be unchanged original nodes. There is no eager pairwise clique, universal counterpart count or property synchronization. Mapping revocation and current authorization remain admission conditions even when historical decision content is pinned.

The reserved `weave:identity:` graph namespace rejects raw commits, batch commits, capsule receipt and branch acceptance. Only the private validated acceptance route can write it. Current policy guards apply to original and pinned queries, metadata, assertion and node proofs, exact resolution and capsule export. Historical pins do not bypass revocation.

All membership queries and resolver values return the same generic `Partial` coverage diagnostic because they enumerate only authorized and available records, never a global directory. Primitive reader checks happen before source-proof traversal; hidden members cannot change visible payload, layout or this coverage marker. Mapping revisions still expose that a decision version changed; timing, resource use and full activity hiding are not claimed.

Policy installation, revocation and head inspection are trusted administrator APIs. Source records and evidence are rechecked for submission, acceptance and receipt retries. Candidate groups are full replacement partitions with at most 128 total exact NodeRefs. Candidate receipt is capped at 1,000 proposals/64 MiB per proposer; decision receipts at 10,000 per actor. Queries also share the runtime read and materialization budgets. Revocation is irreversible for an exact policy revision.

Evidence: `crates/weave-engine/tests/identity_acceptance.rs` exercises receipt vs acceptance, immutable policies/candidates, authorization, CAS/replay/no-op events, explicit split/history, independent IDs, private-target omission, repersisted reader stripping, revocation, reserved-namespace poisoning, hidden-member payload/coverage parity, and repeated graph-value union. `crates/weave-contract/src/algebra.rs` checks canonical synthetic-node idempotence without fabricated source origins. Full mobile/browser persistence, remote signed decisions, accepted-view governance and authenticated capsule synchronization remain open.
