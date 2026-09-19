# Authorized typed governance delivery

Current authority time comes from the [constructor-installed operation clock](OPERATION_CLOCK.md). Explicit native `now` arguments have been removed; fixture callers must migrate to an installed test clock.

Native governance events now participate in the existing adapter identity and lifecycle boundary. This is a typed acknowledgment stream over the separate governance outbox. It does not fabricate graph commits, execute graph-producing handlers, or run external effects. Accepted graph-value influence and remote subscriber admission remain separate requirements.

A trusted host installs the existing immutable `AdapterManifest`, starts the adapter, and calls `subscribe_governance(adapter, view, now, host)`. The principal must exactly match the manifest. This profile adds a governance subscription to an existing adapter; the manifest's existing graph subscription is not repurposed as a fake governance graph. At most 32 retained governance subscriptions per adapter are allowed. A canceled subscription cannot be silently reactivated under the same identity.

`poll_governance` returns an event identity, view/decision identity, event kind, recorded time, replica source identifier, recipient-local occurrence ordinal, and lease. It includes no source graph expansion, member roster, vote count, proposal digest, global sequence number, or fabricated GraphRef. The immutable outbox event identity survives retries, while leases rotate after expiry or lifecycle invalidation. Newly created governance decisions use opaque occurrence IDs; existing persisted decision/receipt IDs are unchanged. This does not retroactively rewrite old commitments or recall prior disclosure.

## Current authority and historical events

Registration, polling, acknowledgment, exact acknowledgment retry, and dead-letter replay recheck the current view policy and source authority under one local SQLite snapshot. Historical publication events independently recheck their original source revision and transitive proof closure under the current policy. A policy-change event inherits the source selected at that historical point: the implementation walks bounded predecessor decisions to find it, rather than substituting the current accepted source. Changing to a public head later does not make a private historical source event visible.

Invisible or currently unavailable historical events advance only an internal checkpoint. Such skipped history is not retro-delivered if authority or availability later changes; a new adapter/subscription identity is needed to rescan. Cancellation of the old subscription is terminal. Visible occurrences receive contiguous local ordinals; unrelated views and hidden events do not expose global gaps. Current authorization of the view's accepted head is also required, a conservative whole-view availability boundary. Policy expiry or revocation with no graph-head movement still prevents pending payload disclosure and receipt retry.

Adapter pause invalidates governance leases and prior acknowledgment epochs atomically with the lifecycle state change. Resume can redeliver the same occurrence with a new lease and the same local ordinal. Draining delivers already pending work but starts no fresh occurrence. Removal requires resolving or explicitly canceling pending deliveries. Cancellation requires the owning adapter principal and remains available after view read authority is revoked; it discloses no event. Removed identities, canceled subscriptions, and old leases cannot acknowledge work.

## Delivery and recovery semantics

Each subscription has one pending occurrence. Lease duration, retry limit, and visible backlog limit use the installed adapter manifest. Expired attempts eventually enter a dead-letter state. Explicit host replay checks current authority and renews the lease without inventing a new event. Bounded busy errors may be retried by the host.

Acknowledgment atomically stores its receipt, advances the private checkpoint, and removes pending work. It returns an exact durable duplicate acknowledgment only for the same occurrence/lease and unchanged subscription epoch, after current authorization checks. A receipt retry may succeed after its lease timeout if it was already committed; an uncommitted expired lease cannot succeed. No graph mutation or external action is coupled to this acknowledgment. Exactly-once external delivery is not claimed.

The native SQLite schema advances from 9 to 10 within the existing migration transaction. Adapter manifests are SQL-bounded to 64 KiB before parsing. Event pages are bounded to 1,001 rows; historical source traversal is bounded to 1,000 predecessors and shares the engine operation read budget. Retained receipts are limited to 10,000 per subscription. A bounded poll may exhaust the shared read budget while checking a large historical queue, even when its record count is within quota; it fails without checkpoint/lease changes. Guaranteed progress for every in-quota history is not claimed. These limits describe logical state and serialized inputs, not process-memory or database-file isolation. Compaction and receipt retention policy remain open.

## Evidence

[Native tests](../crates/weave-engine/tests/governance_delivery.rs) cover restart, lease rotation and stale acknowledgments, current reader revocation of pending and acknowledged occurrences, hidden publication/policy-change history without visible ordinal gaps, deadline/dead-letter recovery, drain/cancel/removal, and unchanged graph event counts.

[Process-death acceptance](../scripts/check_governance_delivery_recovery.py) uses a feature-only [test host](../crates/weave-engine/examples/governance_delivery_probe.rs). Exit 90 after acknowledgment SQL but before commit leaves the original pending lease and no receipt/checkpoint advancement. Exit 91 after commit but before response permits durable exact retry without another event. Changed lease and revoked policy retries fail.

E05/E11 remain in progress. Required broader work includes graph-valued accepted decisions with complete influence propagation, governed handler outputs/effect execution, remote authenticated subscriber operations, retention, and supported distributed consistency profiles.
