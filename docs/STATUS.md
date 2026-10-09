## Coordinated accepted selection and history ranges (0.21 / store21)

Canonical `AcceptedHistory` and named `RecordedRange`/`AcceptedRange` commands
preserve actual local selection witnesses, including empty valid-time outputs.
Current policy and whole-input authority still apply, and persisted results reject
forged or missing witnesses. Existing0.20 artifacts remain compatible. Store21
preserves original history and prevents old runtimes from dropping the new fields.

The actual SDK/SystemClock history controller passes with8 compiler and15 runtime
processes; the existing recorded pin/view controller passes with8/12. The complete
source/peer/effect trace passes with214 runtime processes,12 compiler processes and
22 controlled deaths. Independent composition, range and preflight rollback
oracles pass. The development engine passes513 tests and strict lint/format;
compiler passes198. Four populated store20 upgrades and a real old compiler/runtime
journal upgrade pass. Archive and hosted evidence are recorded separately against
their exact publication revisions. See [contract0.21](contract/v0.21/README.md).

Accepted historical collection queries do not install a global clock or grant
remote governance authority. Retention, lifecycle, broader incremental/transport,
complete portable applications and the original assurance gates remain mandatory.

# Implementation status and evidence

## Native accepted-view history

Native explicit decision/date/range selection now separates governed acceptance
from source recording and replica receipt. Signed original ordering links and
protected occurrence bodies are checked, current whole authority is retained,
and persistent clock regression cannot publish an acceptance. Six independent
oracles cover late acceptance, equal time/restart, half-open ranges/overflow,
foreign/missing/future cuts, lost/corrupt intermediates and expired historical
approvals versus current policy. See [ADR 0003](architecture/ADR-0003-accepted-view-history.md).

The full workspace checkpoint passes 505 tests and strict lint/format. Aggregate
range byte bounds are checked separately. The actual source-transfer trace passes
with 214 runtime processes, 12 compiler processes and 22 controlled deaths. An
independent eight-process compatibility controller starts with real prior store20
governance, verifies unchanged original rows and lets the prior runtime read the
original decision after a new acceptance. Canonical/source accepted-time and range selection, retention and all
remaining original requirements stay mandatory; this is a native extension.

## Coordinated recorded selection (protocol 0.20, store20)

Canonical `RecordedQuery` now selects an actual replica-local checkpoint through
`LocalTime` or explicit observer/checkpoint criteria. Query results preserve the
selected observations through empty outputs, graph algebra, temporal selection and
scalar derivation. Cached values revalidate these exact witnesses and their whole
input snapshots under current authority. Source `recorded_handle` declarations are
lazy; `pin` executes once, and view templates preserve the recorded criterion while
their tick controls valid time. Pure handler recipes cannot hide these reads.

Store20 preserves store19 history without adding baselines, and refuses to
reconstruct a missing history table. Older stores retain the store19 current-time
baseline migration. Capsule0.4 and historical0.19 artifacts remain compatible.
The full original requirements remain mandatory: source ranges/accepted-view cuts,
retention/GC, lifecycle, incremental execution, transport and broader assurance are
still open. See [protocol 0.20](contract/v0.20/README.md) and [local verification](VERIFICATION_020.md).

## Current snapshot

The preceding implementation uses protocol0.19, SQLite marker19 and capsule0.4. It adds alternative influence on whole values, nodes and attachments, per-branch snapshot premises, and pure Window/Sequence graph operations. The language vendors the exact canonical contract from native freeze `a6adb94`. See [the current contract](contract/v0.19/README.md) and [joint verification](VERIFICATION_019.md).

Store19 adds atomic replica-local branch observation checkpoints and bounded
recorded-time query/range APIs. Migration keeps all original rows and establishes
current-time baselines instead of inventing old acceptance dates. Native history
selection follows authenticated predecessor records and checks current whole
snapshot authority. See [ADR 0002](architecture/ADR-0002-recorded-head-observations.md).
Source range/accepted-view selection, retention horizons and GC remain open.


Local joint validation covers actual source compilation and persistent execution, native/WASM parity, authorization after detached persistence, six populated historical migration suites and sixteen real browser persistence/upgrade cases. Commit-specific hosted results are tracked separately; local results alone do not establish CI success. E00 is complete; E01–E14 remain in progress. These bounded profiles do not complete either paper. See [workflow gates](workflow.json) and [reconciled requirements](RECONCILIATION.md).

The preceding public pair, engine `21c2728` and compiler `82a45f6`, passed all hosted checks at protocol0.18/store17. Those results describe that historical revision.

The current native runtime includes genuine accepted-governance graph reads, a trusted operation clock, exact Query/Filter membership maintenance, and durable coalesced view scheduling. Earlier notes describing accepted graphs as unavailable or scheduling as unpublished apply only to their historical checkpoints.

Protocol0.16 adds: exact accepted-occurrence expressions, definition-matched RequireCurrent view expressions, and separate canonical host registration artifacts. Source manifests persist through initial/full/incremental/fallback evaluation. SQLite marker14 protects compiled registration identity. The [0.16 contract](contract/v0.16/README.md) records the native boundary; paired compiler acceptance is now executed in integration CI.

## Resumed source-backed native acceptance

The recovered export repair and retained cluster implementation now have a combined
actual compiler→offline evidence/rebind→sealed diagnostic→retained cluster→signed
P/W/T exchange→team acceptance→unknown effect/reconciliation process trace. Both
source variants preserve exact original SDK artifacts, old history and private
reader gates. The journal preserves scoped Partial cluster coverage and checks
whole exact inputs separately, before fresh and historical completion.
See [native compiled acceptance](NATIVE_COMPILED_SCENARIO.md). This is unpublished
local evidence; broader lifecycle/retention, history, incremental, transport and
portable application requirements remain open.

The same combined trace now passes in iOS26.4 simulator-target Rust processes.
A separate [Swift facade app](SWIFT_HOST.md) passes actual source-backed offline
edits, sealed diagnostic recovery and privacy in its own application container
across 24 launches. These are distinct profiles; the app does not yet bind the
cluster/peer/governance/effect services. Exact archived engine/compiler workspaces
pass 480/191 checks respectively; no full paper gate or hosted result is inferred.

## Published 0.18 boundaries and historical checkpoints

Sealed pure handler artifacts and native immutable install/prepare/complete are published, with SQLite marker16. Exact event/preloaded input gates survive empty outputs and generated records. Output CAS, current authority and historical receipt replay are checked atomically; caller-built completion cannot bypass a compiled binding. Genuine historical0.16 and0.17 templates and signed receipts retain their original bytes through migration. Source/runtime joint verification and publication are complete for this bounded profile. See [contract](contract/v0.18/README.md) and [native handler profile](proposals/COMPILED_HANDLERS.md). This stage does not close complete reactor/effect semantics or a paper gate.

The governed canonical graph effect bridge is published with SQLite marker17 and unchanged protocol0.18. Owner and independent authorization tests, the actual source-handler→governance→reference-sink trace and populated historical recovery passed, including hosted acceptance. No remote Execute capability, arbitrary destination, declassification or general exactly-once effect claim is made. See [the native bridge profile](proposals/GOVERNED_EFFECT_BRIDGE.md).

Public0.17 introduced exact snapshot and movable-attachment influence, capsule0.3, and bounded shared mixed-proof traversal. Its native383-check historical full-suite checkpoint, independent populated migration, exact compiler vendoring and hosted CI are evidence for that revision, not a claim about the current0.18 candidate.

## Implemented boundaries

- Immutable SQLite snapshots/CAS, atomic logical batches, schema and structural identity, explicit assertion records, half-open fact time, principal-filtered pinned reads, graph-valued named metadata and provenance. Live handles pin explicitly; contextual traversal and metadata wrappers preserve source restrictions.
- Portable graph algebra, four-valued time-specific support, finite range-restricted rules, exact context selection and typed axis witnesses, explanation graphs, canonical exact Decimal/nominal Quantity values, finite Float, and assertion-backed geometry. Richer semantics remain partial; see [contract history](contract/).
- Persistent whole-value, node and assertion influence, alternative proof groups, current-policy checks on stored and generated values, and generic partial denial. Clearing reader lists does not remove retained source gates. See [influence](INFLUENCE.md).
- Signed bounded native Query/Publish/Propose and exact whole-capsule export admission with durable replay receipts and isolated proposal quarantine. Mount lifecycle and explicit signed integration preserve separate acceptance and retention boundaries. No remote arbitrary-Program authority is implied. See [admission](ADMISSION.md), [signed export](SIGNED_CAPSULE_EXPORT.md) and [mounts](MOUNTS_AND_INTEGRATION.md).
- Native accepted identity mappings and owner/threshold governance with signed approvals, predecessor-policy transitions, atomic CAS/receipts, genuine protected decision assertions and reusable acceptance influence. Current policy/source authority uses one trusted clock sample per storage operation, including historical decisions. See [governance graphs](GOVERNANCE_GRAPHS.md) and [operation clock](OPERATION_CLOCK.md).
- Scoped durable adapter dispatch and typed governance delivery, leases/lifecycle/dead letters, atomic writes/receipts/checkpoints, and an explicit unknown external-effect fence. Historical duplicate receipts recheck authority; they are not new reads. See [dispatch](DISPATCH.md) and [governance delivery](GOVERNANCE_DELIVERY.md).
- Durable principal-scoped views with full recomputation oracle, explicit freshness/ticks and retractions. Opt-in Query/Filter membership maintenance reuses predicate decisions; whole snapshot loading, hashing/index rebuilding and output repinning remain O(input)/O(output). Durable bounded scans coalesce work; failed work rotates fairly and publication/checkpoint commits atomically. No background thread or general incremental operator engine is claimed. See [selection](INCREMENTAL_SELECTION.md) and [scheduling](VIEW_SCHEDULING.md).
- Hash-verified capsules with whole authorized logical-manifest transport, isolated receive and explicit acceptance, ancestry/equivocation checks, and current authorization on export/reuse. Selective proofs and generic remote authority installation remain unsupported. See [capsules](CAPSULES.md).
- Portable clustering core, authorized native navigation and historical lineage, with explicit overlapping perspectives and measured synthetic quality/churn boundaries. Incremental clustering/hysteresis and broader recall acceptance remain open. See [cluster service](CLUSTER_SERVICE.md).
- Transactional migrations, bounded stored reads, integrity checking and SQLite backup/recovery. Native C/Swift hosts execute macOS and iOS-simulator persistence. Nine fixed portable semantic groups execute natively and in WASM with identical bytes at the resumed archive checkpoint. The separately published experimental browser host persists bounded whole-image generations through IndexedDB under one worker and an exclusive Web Lock. Its historical failure/abort/quota/reload matrix passed, but it remains an 8 MiB fixed-scenario host rather than full portable paper acceptance. Facade A and its Swift wrapper add a generic principal-bound Program/diagnostic artifact boundary; complete browser/mobile scenario service bindings remain unfinished. See [storage](STORAGE_RECOVERY.md), [native host](NATIVE_HOST.md), [Swift host](SWIFT_HOST.md) and [portable parity](PORTABLE_PARITY.md).

## Verification and remaining work

The public marker13 checkpoint includes focused independent selection/scheduler oracle tests, queued-tick freshness regressions, actual scheduler process-death recovery, and marker12→13 death/restart/old-binary refusal. Native workspace tests, strict lint, compiler integration and fuzz CI passed at public `7de6839`. Historical suite totals in linked milestone reports apply to those exact revisions; they are not current cumulative verification counts. Small local measurements separate membership counters from end-to-end costs and make no speedup or production SLO claim.

Required work remains across broader structural/assertion typing and algebra, graph-backed numeric conversions, richer context compatibility, full governance and permission topology semantics, authenticated peer synchronization and selective transport, complete reactor/source effects, generalized incremental evaluation, clustering quality/incremental maintenance, generic persistent browser scenario bindings, mobile/platform breadth and complete integrated multi-peer paper acceptance. The existing native three-process trace now covers offline evidence/reaction/conflict/governance/effect fencing, including signed whole-capsule exchange; it leaves broader transport, release and reactor semantics open. Local administration APIs remain trusted embedding-host operations. System history selects exact revisions; general system-time range queries remain open. Serialized budgets are not measured RSS isolation.

N-ary relations and reward-based traversal learning are optional in the papers. NAT/rendezvous services are optional operational infrastructure; their absence alone does not prevent a direct reference peer transport from satisfying its scoped acceptance.

Protocol0.16 native8d359df passed344 workspace test/doctest checks, strict all-target/all-feature lint/fmt and contract WASM compilation. Root independently passed seven compiled-view/binding tests, actual compiler-to-host acceptance, schema13-to14 process-death rollback/restart/refusal, exact26-file canonical vendor equality and source native/WASM parity. The compiler freeze25b1da8 passed134 tests and a locked/offline source-archive build; its21 historical example plans/source identities remain unchanged except the declared protocol version. Hosted results are recorded in the public issues after completion. No full gate is closed by this bounded checkpoint.
