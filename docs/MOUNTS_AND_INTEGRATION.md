# Durable mounts and signed proposal integration: implementation boundary

Status: implemented native slice under independent review; SQLite schema 8. A mount exposes an existing locally stored pinned source through a principal-owned route. It neither advances accepted graph heads nor accepts identity claims. An integration decision promotes an already signed, isolated proposal under explicit host write authority and branch CAS. The APIs remain separate.

## Mount API

`attach_mount(MountSpec {id, reference: GraphRef}, expected_generation, nonce, host)` creates an immutable principal-owned route after a current authorized pinned query. Route IDs are scoped by principal. Reusing a route ID with another source rejects; detach/reattach changes the route generation, not its pinned meaning. Identical nonce/body attach retries return the original receipt only after current source and original proof authority is checked. Owner detach cleanup remains available after source revocation and returns no graph content. Changed bodies reject replay. Active-route reads always run the ordinary pinned query again under the current host; no cached authorization is stored.

`detach_mount(id, expected_generation, nonce, host)` removes the active route using generation CAS. It records an owner-visible durable lifecycle occurrence atomically with state and receipt. Detach stops use of that route; direct reads, previously disclosed copies, accepted heads and retained revisions are unchanged. No garbage collection is triggered. Local retention remains conservative until a complete reachability collector exists.

`mount_changes(after_cursor, limit, host)` exposes only that principal's lifecycle stream with an owner-local cursor and bounded output. The current stream retains up to 10,000 owner lifecycle events; reaching capacity returns explicit backpressure. Initially this is a native durable lifecycle stream; integration with the generic graph adapter bus is a subsequent step. It must not forge graph mutation events for attaching a route or expose global sequence gaps caused by other principals.

## Signed integration API

`integrate_proposal(IntegrationRequest {proposal_id, branch_id, expected_head, nonce}, proof, now_ms, host)` retrieves the exact isolated proposal, checks its digest and subject, and revalidates a signed Propose proof over those exact capsule bytes under the current installed admission policy. The proof selects a prior authorized proposal; it does not grant acceptance authority. The trusted host must separately grant write authority for every imported graph and explicitly select the root destination branch.

One outer SQLite transaction encloses verified receipt into the local revision store, immutable structural/schema registration, root branch CAS, accepted occurrence event and integration receipt. Nested capsule operations must use savepoints, so an inner success cannot survive failure or process death before the outer commit. Conflicting identities, missing required metadata, reserved identity namespaces, stale CAS or changed replay bodies roll back all promotion effects. The original isolated proposal remains independently available for later host decisions.

Replays recheck current signature, policy and source authorization before returning the exact original integration receipt. Historical bytes or an earlier successful signature do not authorize a request after revocation. Proposals received under an obsolete epoch need fresh admissible signed evidence; arbitrary subject labels are not authentication. Acceptance records are local decisions, not a claim that peers agree about truth.

The initial integration preserves the proposed immutable snapshot in a selected offline branch. It does not silently merge concurrent scalar metadata, collapse identity candidates, or merge graphs by arrival order. Protocol/capsule formats and schemas use existing strict version/integrity validation. Requests concern explicitly named roots; no global catalog or peer discovery broadcast is added.

## Acceptance and limits

Two-engine tests must cover signed isolated receipt without accepted heads or registry poisoning; explicit promotion; conflicting local branch CAS; partial/missing metadata; local fork independence; restart and exact retry; changed nonce bodies; current revocation; reserved namespace rejection; private source filtering; and process termination before/after the outer transaction. Mount tests must distinguish route lifecycle from graph acceptance, enforce principal-local cursors and prove detach does not delete shared evidence.

Selective Merkle disclosure, encryption, transport/NAT/rendezvous, automatic replication scheduling, retention/GC, semantic conflict merging and remote governed acceptance remain mandatory later work. This profile does not claim a complete peer protocol, mobile synchronization or exact-once external transport.

## Verified evidence

`tests/mounts.rs` covers received-but-unaccepted route attachment, generation CAS, exact replay, restart, detach without deletion, owner-local cursors and original proof revocation. `tests/integration.rs` uses two engines and signed requests to check quarantine, explicit authority, offline branch publication, restart/replay/revocation, private-content rollback and structural identity conflicts. It also verifies that capsule export and acceptance use bounded integrity-checked storage loads.

`scripts/check_integration_recovery.py` runs the feature-gated `integration_probe` with deterministic local test keys. Exit 86 after all decision SQL but before the outer commit leaves no imported revision, registry record, accepted head, event or integration receipt; the isolated signed proposal remains. Exit 87 after commit followed by restart produces the exact prior receipt and one accepted occurrence. Changed replay bodies and revoked current keys reject. This proves local transaction recovery, not network transport exactly-once delivery.

Mount limits are 1,000 routes and 64 MiB of stored authorization per principal, plus 10,000 lifecycle events/receipts. Integration receipts cap at 10,000 per host actor. Capsule input stays bounded at 16 MiB and reuses existing graph/read/materialization limits. Mount detach does not collect any revision. The lifecycle stream reports only that owner's chosen route ID, generation, active state and occurrence ID; it does not expose source graph content or other principals' sequence gaps.
