# Three native peers: executable paper scenario and next boundary

Design only, based on engine `8d359df` (protocol0.16 candidate), inspected 2026-09-20. No new protocol, storage schema, API or build is implemented here. The source/compiler pair still needs its own fixture and independent release verification. This design follows engine paper [§12](../source/Weave_Engine_White_Paper_v0.1.md) and the orchestration `verification/COMPLETION_PATH.md`.

## Proposed first experiment

Run a controller and three short-lived native host processes against separate SQLite files: **P** is a phone-like offline store, **W** a workstation, **T** a team peer. Each process uses the real Engine APIs; the controller may inspect invariants through explicit trusted test commands but never copies database rows between peers. Restart processes between stages. Transfer bounded serialized messages through local pipes/files first, with a deterministic disconnect/drop/duplicate/reorder controller. No simulator, browser, production endpoint, credentials or extra target cache is needed.

Use fixed test-only signing keys, independent peer audiences and explicit installed trust roots. Pin exact graph revisions, schemas, adapter artifact/configuration identities and the approved source/compiler commits in a run manifest. Authority time comes from host-installed ManualClocks; fact time is a separate field. A recorded timestamp in a graph cannot authorize reconnect or revive a revoked capability.

The initial graph set is deliberately small: one installation graph containing a device, gateway and connection; operational and physical manifestations linked by an explicit counterpart assertion; an evidence graph with baseline positive connection-quality evidence; an independent private-annotations graph. Put required installation/evidence members in the shareable logical batch. Private annotations and inaccessible counterpart links stay outside both that batch and the selected required closure. A whole logical manifest containing an inaccessible member must fail export; hiding one member is not selective disclosure.

The first successful run can use one authenticated owner identity across the three stores and an owner-governed accepted view on T. Peer identities remain distinct. That demonstrates offline operation and scoped acceptance, not cross-user release of private handler outputs. An explicit multi-user negative case must show the latter is denied. Do not make the example pass by clearing readers or by treating a governance signature as declassification.

## Runnable stages and exact gaps

| Stage | Existing native execution | Required observations and limits |
|---|---|---|
| Seed and working set | W `execute(CommitBatch)`; `export_capsule`; P `admit_proposal` then explicit host `integrate_proposal`; `attach_mount`/`detach_mount`; `fork_branch` | Initial source/history pins match; signed isolated receipt does not install revisions/heads/events; host promotion emits its own accepted occurrence. Current export is trusted-host only. Mounting and detaching neither accept nor delete content. |
| Prepare dependency histories | Explicit host accepts each chosen dependency root on its declared local branch | Integration advances only the requested root head, not every imported dependency. Merely possessing a pinned evidence block does not establish Read+Traverse accepted-history scope. Record all additional acceptances; no implicit forest acceptance or one atomic multi-root sync claim. |
| Go offline and revise evidence | Stop all transfer. P `execute(CommitBatch)` writes a new evidence revision and rebinds the installation's named edge/assertion evidence attachment using logical same-batch references | Both old/new pins remain queryable. Inject pre/post-commit death using existing recovery boundary where available. All source changes/events commit together or not at all. Private annotations remain separate. |
| Durable diagnostic reaction | Install scoped `AdapterManifest`; `poll_adapter`; evaluate pinned Query→Metadata→Reason/Support; `complete_handler` commits warning GraphData and receipt/checkpoint | Use explicit negative quality evidence, not absence, to derive the warning. Preserve exact measurement, attachment and rule premises. Replay returns one historical receipt without new graph events. Existing wire events are `graph.committed`/`graph.accepted`, **not** the paper's typed `org.weave.meta.graph-rebound.v1`; the handler compares authorized pinned before/after attachment values. This adapter is explicit host code, not compiled reactor syntax or sandboxed module execution. |
| Local organization | A second scoped adapter reacts to the committed warning; host `cluster_navigation` produces a reusable graph and `complete_handler` persists it | Candidate layout/membership retains warning and evidence influences. This is deterministic bounded native clustering, not incremental clustering or an inference that cluster proximity proves a failure cause. |
| Reconnect to W | Existing trusted export + signed isolated `admit_proposal` + explicit `integrate_proposal` can demonstrate two-way transfer/replay today | Root branch CAS conflict retains both branches and the isolated proposal. W's distinct accepted event is not P's event replay. There is no current signed export endpoint, authenticated revision negotiation or selective proof transport. Label the current relay a trusted harness, not authenticated peer synchronization. |
| Larger organization/history | W queries its broader permitted set, computes another cluster organization and saves a separate derived revision | Preserve P's cluster proposal and original evidence pins. Different organizations coexist; arrival order never rewrites source assertions or resolves a semantic conflict. Source authority must hold for all output influences. |
| Team proposal and acceptance | Send ungoverned evidence/proposal snapshots to T using signed Propose and explicit import; then `propose_governance`, signed `record_governance_approval`, `accept_governance`; exact `query_accepted_view` / AcceptedGraph and Explain | The team creates its **own** genuine decision assertion. Never transmit/install another peer's reserved governance registry as authority. Prior accepted occurrence stays historically selectable subject to current policy/source checks. Threshold approval can be tested, but meaningful independent reviewer access to owner-private warnings needs a separate release policy. |
| Maintenance request with lost response | A scoped effect adapter uses `request_effect`, `begin_effect_dispatch`, an instrumented local fake destination, then `reconcile_effect` | Persist unknown before I/O; kill after one observed destination action but before acknowledgment. Restart refuses automatic second dispatch. Record independent destination evidence before reconciliation. This proves the ledger boundary, not exactly-once arbitrary external notification. |
| Detach/revoke/restart | Detach routes, expire/revoke admission/governance policy, restart every peer, retry cached reads/receipts/leases | Fresh remote access and acceptance stop under current installed policy. Previously disclosed offline plaintext is not recalled. Current authority remains separate from original signed admission evidence. |

Warning/cluster output readers are currently exactly the installed adapter principal. `complete_handler` cannot broaden them. Graph-valued accepted governance, including a policy transition, does not remove original source reader/proof restrictions. Governance delivery acknowledges a typed occurrence but cannot atomically submit an arbitrary governance decision and graph-producing handler output under that acknowledgement. Both release policy and governed-output completion are genuine remaining stages, not hidden harness conveniences.

## Minimal next implementation slice: authenticated whole-capsule export

Recommend implementing one narrowly scoped **signed exact-revision export admission** plus the process reference exchange, before adding declassification or a general sync daemon. It closes the missing read-side authority boundary using existing signed Propose/import on the receiving side. Whole authorized snapshots are sufficient for this first direct-pair reference experiment; they do not satisfy the paper's selective block-proof requirement.

Proposed native-only shape for review, not a version reservation:

```rust
struct CapsuleExportRequest {
    format: String, // one reviewed explicit request domain/version
    root: GraphRef,
    branch_id: String,
}
struct CapsuleExportReceipt {
    capsule: Capsule, // replay flag remains in the existing Admitted envelope
}
fn admit_capsule_export(
    &mut self,
    proof: &AdmissionProof,
    request: &CapsuleExportRequest,
) -> Result<Admitted<CapsuleExportReceipt>>;
```

The original admitted closure and request/subject/epoch binding are private durable receipt fields, not a discovery response. Exact request fields and the transport envelope still need review before naming any protocol version.

The operation uses explicit Read+Traverse scopes and signs the canonical **tagged export body**, distinct from QueryPlan JSON. Require the exact root to be reachable from the authorized branch. No ambient head or known-hashes inventory is accepted initially. An exact current branch/head update needs a separate fresh root request. Bound IDs/body before serialization; no caller-selected resource quota.

Inside one immediate transaction and one trusted clock sample: verify current proof/audience/epoch/body/nonce; establish exact root history; authorize capsule members and complete influencing dependency closure; verify integrity; produce a bounded whole capsule; atomically record the immutable response and admitted closure before returning. Required metadata must be present or export unavailable; live handles remain unsupported. Apply primitive visibility before dependency traversal. Private members cannot contribute discoverable inventory, counts or out-of-scope identifiers. A missing/denied requested root has one generic denial; physical I/O timing and monolithic revision-version observability are not hidden.

Two details require implementation tests, not an assumption that the existing query preflight is sufficient:

1. Capsule closure includes ancestry and complete logical-manifest siblings as well as graph/proof/context references. Every disclosed revision must satisfy the recipient's current scope and ordinary reader/proof policy. Parents or logical siblings not traversed by an ordinary Query must not escape export admission.
2. Cached retries must recheck the **original disclosed capsule and admitted closure**, under the current proof/policy and current source guards. Do not export a new closure on retry or authorize old bytes using a narrower new query. Missing/changed nonce bodies reject; capacity errors leave no nonce/receipt. Historical success never bypasses current revocation.

Reuse existing 16 MiB capsule, 1000 revision, depth32 and native cumulative read limits, plus admission receipt count/byte quotas. Cache and return only complete authorized response bytes within those limits; do not add a payload-size check after committing a response that cannot be returned. Backpressure is explicit; no nonce GC or unbounded automatic retries.

The reference serving process must also authenticate its response. Proposed paired transport envelope signs a separately reviewed domain containing request digest/nonce, requester identity, serving peer identity/audience, selected protocol/capsule version and capsule digest. The trusted host owns the serving key; serialized requests never install it. Receiver validates the paired serving key and exact request/root binding, then submits the capsule through its independently scoped Propose admission. A sender signature authenticates those bytes and the sender, not every embedded assertion's claimed author or truth. Wrong audience/version/root, substituted or truncated response, stale nonce and signature failure must cause no receiver mutation.

This envelope is a proposal requiring explicit cryptographic-format review. Do not claim confidentiality from signatures; the initial local-pipe test transport is only an isolated test surface. A deployed direct connection needs authenticated confidential transport and key lifecycle. NAT, relays and rendezvous are optional operational infrastructure in paper§8, not universal gate prerequisites.

No remote root/policy installation, accepted-head mutation, object-reader clearing, reserved decision import, automatic dependency-head promotion, inventory discovery, partial Merkle proofs, semantic merge, background retry/retention or shared protocol/source syntax is bundled into this slice.

## Concrete acceptance for that slice

Use actual P/W/T child processes and their persistent databases, not three Engine objects in one process. Test keys and endpoints are deterministic and local. The controller records every operation identity, exact root, event/receipt count and current policy epoch for audit; none of these trusted counters becomes an unprivileged response field.

1. **Offline independence:** terminate W/T while P commits/rebinds, reacts and clusters. Restart P between stages; old pins, new logical batch and processed receipts remain exact.
2. **Missing read-side authority:** unsigned export and forged subject/audience fail; a valid root-only grant fails when required evidence/ancestry/manifest siblings are outside scope. Private unrelated annotations never appear in bytes or topology summaries.
3. **Exact replay:** lost export response followed by restart returns identical bytes only with valid current authority; changed body, narrower scopes, epoch change, expired grant and revoked keys deny. Kill before export receipt COMMIT leaves no consumed nonce; kill after COMMIT preserves the exact response.
4. **Transport authentication:** wrong paired server signature/version/audience/request/root or altered capsule is rejected before receiver proposal storage. Dropped/reordered duplicate messages cannot produce another accepted occurrence.
5. **Quarantine and conflict:** receiving signed bytes alone changes only isolated proposal/receipt state. Concurrent workstation branch update yields explicit CAS conflict, retains both histories, and leaves no partial identity/head/event promotion. Any later host-selected branch integration is a separately recorded decision.
6. **Re-export closure:** after explicit dependency-root acceptance at W, a W→T export independently checks W's current scopes; a source-admission signature from P is never transitively reused as W or T's authority. Reserved governed snapshots remain nonportable authority.
7. **Acceptance and history:** T's own exact decision and source explanation retain measurement→attachment→warning/cluster provenance, including empty/scalar proof carriers. Revocation blocks new reads/receipt access while prior source pins remain stored. Different source/proposal histories are not silently retargeted.
8. **Unknown external effect:** the local fake destination records exactly one attempted action, engine state is unknown after the response-loss kill, automatic retry is refused, and explicit evidence-driven reconciliation is durable.
9. **Multi-user boundary:** an independent team member cannot read/release an owner-private diagnostic output merely because its graph was mounted, imported or accepted. This is an expected denial and an explicit remaining-governed-release test, not a successful shared-workflow claim.
10. **Measured limits:** record bytes transferred, revision/event amplification, operation durations and fixture peak RSS with a small fixed workload. Include budget/backpressure denial and restart; do not extrapolate to production/mobile capacity or claim delta-only I/O.

The first fixture can be built from current APIs with a visibly trusted export step to expose additional concrete failures. After the export slice, rerun the same trace through the signed endpoint/envelope. Passing it advances E05/E08/E09/E11 integration evidence; it does not close those gates, the exact MetaGraphRebound event contract, declared source reactors, selective cryptographic replication, broader governed output release, portable persistent hosts or full paper conformance.
