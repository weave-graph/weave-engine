# Accepted identity mappings: next implementation boundary

Status: design proposal, not an implemented acceptance API. The declared counterpart selector in contract 0.10 is a prerequisite, not completion of engine §6 or language §2.2.

The resolver must distinguish a proposal from an accepted identity decision. A source can claim two independently assigned entity IDs refer to the same entity; the claim itself cannot install a policy, gain authority, rewrite those IDs or merge accepted state.

## Proposed host boundary

1. A trusted administrator installs an immutable named policy revision with explicit decision makers, allowed spaces and resource ceilings. Installation is outside graph plans and signed proposal ingress. A policy label is not authority by itself.
2. A candidate pins source NodeRefs, supporting assertion evidence, an exact valid-time interval and a policy revision. Receipt stores a candidate independently of the accepted mapping registry. Its data cannot preempt structural/schema identities or replace an accepted mapping.
3. A decision maker can accept, supersede or split a mapping using strict expected-head comparison. The immutable decision revision records the policy, candidate and source pins, actor, valid time and prior decision. Acceptance and its durable occurrence event must commit atomically; replay is an exact receipt, not a fresh decision.
4. A resolver selects a named immutable accepted mapping revision and a source manifestation. It exposes only authorized counterpart adjacency in the requested target space. Original entity IDs, manifestation IDs and local properties remain intact. The accepted bridge decision is an additional proof; it does not silently rewrite an entity ID or synchronize state.

A split is a new decision over explicit memberships; it does not delete historical accepted revisions. Historical selection continues to use its pinned mapping revision. Pairwise private identifiers remain usable because the accepted link is separate from the local identifiers.

## Required proof and privacy work

Persisted virtual bridges and aggregate values can depend on private isolated nodes. AssertionRef-only gates cannot represent those inputs. A cycle/budget-safe NodeRef dependency gate is required before this API can publish derived adjacency. It must cover query filtering, endpoint traversal, capsule closure and signed read retries. Raw plans cannot supply a result-origin envelope as proof of authorization.

Discoverability filters source and candidate memberships before producing adjacency. No endpoint API reports a total count including inaccessible or disconnected records. Do not eagerly materialize a global clique. Bound source loads, candidate scans, retained proof bytes and result objects; limits return explicit errors or generic partial coverage, not a false complete empty answer.

An accepted bridge keeps both source-node and decision proofs, exact context and time applicability. Context defaults are not broadcast into pinned worlds. Mathematical transforms, learned mappings and similarity remain distinct domains. Contradictory candidates stay evidence until the selected policy adjudicates them.

## Acceptance to implement

- Candidate receipt does not advance a mapping head or publish adjacency.
- An uninstalled or forged policy and an unauthorized decision maker cannot accept.
- CAS conflicts, replayed decision bodies and process death before/after commit preserve a single accepted occurrence.
- Independently assigned entity IDs can become counterparts through an explicit accepted revision; later supersession/split changes current adjacency and preserves historical selection.
- Hidden source nodes, candidate membership and proof paths produce no global count or identifying error. Persisted bridge copies cannot shed source restrictions by clearing readers.
- Source properties remain unchanged. Coordinate transforms and similarity claims cannot satisfy identity acceptance.
- Initial trusted-host implementation remains explicitly separate from remote signed admission; broader operation scopes and governance adapters are subsequent integration work.
