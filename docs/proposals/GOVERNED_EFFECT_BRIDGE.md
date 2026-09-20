# Governed effect-intent bridge: native interface for review

Base: paired engine `edd58fd` / compiled-handler freeze `9b444ff`. This design is approved for native implementation; the implementation remains pending. Protocol remains0.18: no new Program operation, expression, compiler artifact or remote admission action is proposed. SQLite **store17** is required, because older dispatchers would ignore its binding and current-authorization guards.

## Requirement and trust boundary

This advances R10–R14, R21, R25–R26 and R38 (engine paper4.2–4.4/10.1/10.2). Existing0.18 sealed handlers already produce restricted immutable graphs. Existing governance already accepts exact publications under owner/quorum policies. Existing effects already have an unknown-before-I/O fence. The missing connection is a kernel-validated accepted request with an atomic intent and delivery acknowledgment, followed by guarded broker dispatch.

Use the existing **typed governance delivery** stream as input. A caller supplies adapter/event/lease IDs, not a decision selector or payload. The kernel obtains the real publication occurrence from its outbox and protected decision registry. No fabricated graph-commit event or assertion is needed. A policy-change control event can be acknowledged without an effect; it never means “execute the previous request again.”

The grant authorizes any exact governed canonical request satisfying its installed schema/view/source scope. It does **not** attest that a particular compiler or handler produced it. Compiled output is one supported producer, but ordinary attribution metadata never proves compiler origin. A future compiler-bound grant must validate trusted preparation/completion records and the exact resulting revision separately.

Graph acceptance is not execute authority or declassification. A separate trusted host installation explicitly authorizes this bridge. The initial destination is a named, same-principal reference sink; no arbitrary network address, script, LLM call, device control, payment, or cross-principal release is supported. Graph strings remain opaque data. The runtime's native installation boundary remains trusted and is not a signed remote Execute capability or sandbox.

## Fixed request and payload profile

An installed grant specifies the complete expected `GraphSchema`, graph/branch scope and governance view. The accepted source must have exactly that descriptor and pass ordinary graph validation and current whole visibility. Payload format `weave-governed-graph-effect/1` is a bounded canonical typed envelope containing the exact source GraphRef, genuine publication decision GraphRef, and the original immutable GraphData. The only encoder/effect class is `CanonicalGraphV1` / `store_graph`; it has no graph-selected destination or executable interpretation.

This permits existing compiled handler output—including an empty, partial-computation, or unknown-support graph—to be the request. Its protected attribution does not certify completeness. A host requiring particular semantic evidence must use a different explicitly reviewed request profile; this bridge never treats any warning or accepted graph as physical-control authority.

The entire fixed request/decision dependency closure is checked, including snapshot/assertion/node/attachment gates and typed-context witnesses. All reachable records must be current-authorized and integrity-valid. Live graph handles and unpinned Object metadata are unsupported in this first profile. Exact pins, not current graph heads or equal replacement content, bind the request. Metadata cycles remain permitted where the existing kernel permits them; mixed proof cycles fail closed through the shared traversal context.

## Proposed native API

All configuration below is trusted host input, not deserializable plan authority. Private stored wire mirrors remain strict and bounded.

```rust
struct GovernedEffectGrant {
    id: String,
    revision: String,
    principal: String,
    view_id: String,
    source: SubscriptionScope,       // exact allowed graph + branch
    request_schema: GraphSchema,     // complete descriptor, not label-only
    destination_id: String,          // independently installed host sink name
    destination_principal: String,   // must equal principal
    encoder: EffectEncoder,          // only CanonicalGraphV1
    execution_id: String,            // explicit execution/replay namespace
    start: EffectStart,              // AfterInstallation | ReplayHistory
    not_before_ms: i64,
    expires_at_ms: i64,
}
struct GovernedEffectReceipt {
    duplicate: bool,
    ordinal: u64,                    // existing recipient-local ordinal
    disposition: GovernedEffectDisposition,
}
enum GovernedEffectDisposition {
    Intent { intent_id: String },
    PolicyChange,
    SupersededPublication,
}
struct GovernedEffectDispatch {
    intent_id: String,
    attempt_id: String,
    destination_id: String,
    idempotency_key: String,
    payload: Vec<u8>,                // exact stored canonical envelope bytes
}

Engine::install_governed_effect(&self, manifest: &AdapterManifest,
    grant: &GovernedEffectGrant, host: &HostContext) -> Result<()>;
Engine::enqueue_governed_effect(&self, adapter: &str, event: &str,
    lease: &str, host: &HostContext) -> Result<GovernedEffectReceipt>;
Engine::read_governed_effect(&self, intent: &str, host: &HostContext)
    -> Result<GovernedEffectStatus>;
Engine::begin_governed_effect(&self, intent: &str, host: &HostContext)
    -> Result<GovernedEffectDispatch>;
Engine::reconcile_governed_effect(&self, intent: &str, attempt: &str,
    outcome: ReconciledOutcome, evidence: &SinkEvidence, host: &HostContext)
    -> Result<()>;
Engine::cancel_governed_effect(&self, intent: &str, host: &HostContext)
    -> Result<()>;
Engine::revoke_governed_effect_grant(&self, adapter: &str, host: &HostContext)
    -> Result<()>;
```

`GovernedEffectStatus` exposes the opaque intent/attempt IDs and pending/unknown/confirmed/failed/canceled state, not the private closure or payload. Payload disclosure is only the first successful `begin`, after the durable unknown fence. `ReconciledOutcome` is Confirmed or Failed. `SinkEvidence` is a bounded opaque reference/hash from the explicitly trusted broker; it does not pretend to independently attest a remote system. Reference tests verify it against the fake sink's durable ledger.

Installation is one transaction with the existing adapter and a single governance subscription. Require host = manifest = grant = destination principal, exactly the declared source subscription, no graph outputs, exactly the declared destination, `projection_replay=false`, valid artifact/config identities and grant interval. The bridge's artifact digest is the domain-separated complete normalized grant digest; the manifest/config is also bound in the registry. A legacy or compiled-projection adapter cannot be adopted. Reinstallation is exact-only; revoked grants and canceled subscriptions cannot be reactivated. A durable UNIQUE(principal, destination_id, execution_id) constraint reserves the execution namespace across all adapter IDs, including revoked or removed history. Registry rows are never deleted by lifecycle cleanup. A new adapter or grant label cannot reuse a historical execution namespace.

`AfterInstallation` captures the current internal governance-stream checkpoint atomically, so ordinary installation does not execute historical acceptance. `ReplayHistory` is an explicit host-authorized new execution namespace; it must never be inferred from a new adapter ID alone. Existing generic subscribe behavior is unchanged for legacy adapters. No global checkpoint is returned. Installing a grant for a currently unreadable view fails; bootstrap an initial policy/view through existing APIs first.

## Identity and atomic enqueue

Private domains distinguish grant identity, registration binding, payload bytes, immutable intent authorization context and receipt body. Do not change legacy request/effect hashes. A durable intent and attempt use independent opaque random IDs. An idempotency key binds installed execution identity + source replica/typed occurrence + destination; it must reveal no private graph/policy content. Bind the full tuple durably and reject key/body mismatch. A new occurrence/explicit execution namespace is distinct; delivery attempts, renewed leases and imported copies are not new authorizations.

At enqueue, reserve the writer and capture one operation clock and SQL snapshot. Verify registered grant, adapter lifecycle, owning principal, active bound subscription, lease, genuine typed event, current view/policy, exact publication registry/source and descriptor. Creating or reusing an intent requires the event's publication to remain the **effective current publication** of the view. A policy-transition head may retain that publication, but a later publication supersedes it even if source bytes are equal. Reuse the protected registry's publication binding, not source-only equality or approval-time policy. Historic-but-superseded requests cannot execute under this first grant profile.

For a publication, construct the canonical payload and exact closure from authenticated stored records. Persist it with the exact source/event/decision/registration/execution binding and checksum. Atomically insert one intent, store the bridge receipt, invoke the private governance acknowledgment core, advance its private checkpoint and delete pending delivery. No network I/O occurs inside this transaction. A policy-change event instead stores PolicyChange and acknowledges atomically. A genuine publication superseded before enqueue stores SupersededPublication and acknowledges atomically, without an intent or payload disclosure. This disposition still requires the active grant, current view and historical source authorization, exact genuine publication binding, allowed source/branch, fixed closure and expected schema. Missing, denied, malformed or wrong-scope requests remain unacknowledged. Thus two accepted publications before polling, and explicit history replay, can advance past authorized obsolete work without executing it. A later supersession never rewrites an existing Intent receipt: retry and dispatch fail current-publication checks; pending intents remain explicitly owner-cancelable.

A duplicate enqueue rechecks all current guards before the receipt path. It returns the same intent and follows existing acknowledgment lease/epoch rules, including denial after lifecycle invalidates the old epoch. It does not recompute payload, renew the execution grant, create another intent or refresh historical authority. The existing reference-sink or broker can separately inspect an already known intent under current authorization.

## Current authorization and bypass closure

Before payload inspection, enqueue retry, or pending-to-unknown dispatch, verify the complete immutable binding and reconstruct its closure from current stored contents. A trimmed cached closure vector or changed payload cannot remove checks. Compare every exact stored record/integrity binding; apply current governance roots, source reachability, readers, influence/context gates, grant validity/revocation, destination mapping and effective publication. Missing/denied request/decision paths use one generic unavailable diagnostic. No prior approval time, source valid time, event time or caller `now` provides current authority.

Bounded private loading/factoring is required at these existing paths:

- Raw `request_effect`, `complete_handler` and their recovery hooks reject a bridge adapter **before duplicate fast paths**.
- Raw `acknowledge_governance` rejects a bridge subscription before receipt return; only the bridge's private in-transaction core may acknowledge it.
- Raw `poll_adapter` rejects bridge IDs: the bridge consumes typed governance events, not ordinary request-graph events. Generic governance polling applies the registered grant/single-view guard before delivery. Additional raw subscriptions cannot widen a bound adapter.
- Raw `effect_intent`, `begin_effect_dispatch` and `reconcile_effect` reject bridge intent IDs before payload/receipt access. Internal bounded decoders are shared without exposing a bypass flag. All legacy effect behavior remains unchanged.
- Capsule/import/ordinary graph metadata cannot install any grant or effect registry. Effect payloads are opaque external data, not a way to install governance roots at a peer.

Pause denies new enqueue/dispatch; resume may dispatch a still-pending intent only after fresh full checks. It cannot reuse an invalidated governance lease. Draining accepts an already pending delivery and permits draining already recorded pending intents, but starts no new governance delivery. Grant revoke is terminal and disables/cancels the bound subscription. Removal and cancellation deny further dispatch.

Pending intent cancellation is a terminal owner cleanup, allowed after read authority expires because it returns no payload and performs no I/O. It records a private audit disposition. **Unknown cannot be canceled, reset to pending, or automatically redispatched.** Reconciliation of an already-issued attempt is also owner/broker cleanup, allowed after revocation solely to record what occurred; it must match the persisted attempt ID and never disclose payload or authorize more I/O. Same outcome/evidence retry is exact-idempotent; conflicting reconciliation fails. General graph-handler stale-CAS cancellation, upgrades and retention are a separate lifecycle gate, not silently solved here.

## Storage and bounds

Proposed private tables: `governed_effect_bindings(adapter PRIMARY KEY, principal, destination_id, execution_id, body, digest, revoked, UNIQUE(principal,destination_id,execution_id))`, `governed_effect_receipts(adapter,event_id,body,digest, PRIMARY KEY(adapter,event_id))`, and `governed_effect_context(intent_id PRIMARY KEY, adapter, principal, body, digest, attempt_id, reconciliation_digest)`. Reuse the existing effect-intent state machine while adding bridge-only canceled state handling; legacy code cannot see those rows through its guarded public APIs.

Store17 creates these tables and marker atomically. Old store16 runtimes must refuse before opening a bridge-bearing database. Do not backfill legacy intents/receipts as governed. Populate migration fixtures with actual0.18 compiled registrations/preparations/receipts plus historical0.16/0.17 views/export receipts, preserving their bytes.

Initial limits:128 bridge registrations/16MiB registered bytes per principal;256 retained bridge intents/dispositions and64MiB retained bridge bytes per principal;1MiB canonical payload;2MiB complete authorization context;64KiB grant/schema descriptor;64KiB reconciliation evidence. Bound and charge each SQL read before copying/decoding. Use current128MiB/4,096-read operation limits and shared mixed-proof budget; bound closure to1,000 exact pins and10,000 visited dependency occurrences before queuing. No automatic deletion at quota: explicit backpressure. These are serialized-state limits, not sandbox/RSS guarantees.

## Crash and authorization acceptance matrix

| Boundary or change | Required observable result |
|---|---|
| Before enqueue commit, including observer unwind | No intent, disposition, acknowledgment or checkpoint movement; same Engine/reopen can retry. |
| After enqueue commit, before response | Same immutable intent/disposition and acknowledgment on authorized exact retry; no duplicate action. |
| Before pending→unknown commit | No ticket; intent remains pending. |
| After unknown commit, before ticket response/I/O | Unknown persists; no automatic second ticket. Broker/operator determines whether any I/O occurred. |
| After fake destination action, before reconciliation | Unknown survives restart; durable sink lookup proves one idempotent action or explicit non-idempotent uncertainty. |
| Before/after reconciliation commit | Before leaves unknown; after exact evidence retry succeeds; changed evidence fails. |
| Current policy/grant expires, is revoked, or publication is superseded | No fresh payload/ticket/receipt disclosure; no graph or intent mutation except explicit owner cleanup. |
| Pause/resume/drain/cancel/remove or stale lease | Existing lifecycle fences retained; unknown never resets; no hidden new subscription or replay. |
| Two publications before poll or ReplayHistory with obsolete publications | Authorized superseded publication yields explicit no-effect receipt and atomic checkpoint; current publication yields one intent. No starvation or silent re-execution. |
| New adapter/grant with historical execution namespace, including removed/revoked adapter | Durable uniqueness rejects installation; explicit new execution identity is required for history replay. |
| Forged event, policy-change event, changed grant/schema/destination/payload/closure | Generic denial or explicit no-effect control disposition; no external action. |
| Partially visible request, empty request, mixed proof cycle or missing exact revision | Whole-profile partial/missing/cycle denial; genuinely empty authorized fixed request succeeds with real exact source/decision context. |
| Old binary opens store17; death during migration | Refusal; transaction rollback leaves old marker and no new registry, restart preserves historical bodies. |

Reference acceptance runs separate producer/engine and sink processes with independent SQLite files, fixed test identities and no real network endpoint. The idempotent sink binds key→payload digest/response and rejects changed-body reuse; the non-idempotent sink deliberately exposes a lost-response ambiguity. Count actual sink actions and persisted states after each controlled kill. A structured trace links source commit → compiled handler output → exact governance publication → typed event → intent → attempt → reconciliation without private member lists/global sequence gaps.

## Ownership and remaining scope

Engine owns `governed_effects.rs`, narrow private factoring/guards in dispatch/governance delivery and protected publication lookup, migration/bounds, reference sink/probe and recovery script. Canonical0.18 types and compiler artifacts stay untouched. Language independently owns its source-only interval/time gate; no effect DSL dependency is introduced. Parent owns independent adversarial tests, scope review, paired CI and publication.

Still open: signed remote Execute/Subscribe capabilities; general cross-principal egress and declassification; untrusted adapter isolation and CPU/memory enforcement; transactional intent creation across stores; destination-specific production adapters; causal loop budgets/timers; general handler cancellation/upgrade/retention and replay-window expiration; complete paper/mobile scenario. This bounded bridge must not close E05/E06/E11 or language reactive conformance wholesale.
