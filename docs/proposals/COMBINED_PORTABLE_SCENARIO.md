# Combined source-to-effect scenario and trusted host facade

Design for review, based on browser freeze `d8e223c`, protocol 0.18/store17,
compiler SDK ABI 1 and the existing native services. No protocol, schema, ABI or
implementation change is reserved by this document. The parent owns browser CI
and publication. The first deliverable is a reusable embedding boundary; the
acceptance controller supplies artifacts and operations at run time.

## Scope and present seams

Paper §12 requires offline evidence and metadata rebinding, durable reaction,
cluster proposals, authorized synchronization, explicit acceptance and an effect
with uncertain-outcome recovery. Existing native traces exercise these in pieces.
The compiler SDK already accepts arbitrary bounded source/modules and returns
complete artifacts. The native C ABI currently exposes only open/Program/close;
the browser host is a fixed fixture. Neither exposes the combined services.

Use the existing Engine and compiler throughout. Add one strict trusted-host
facade shared by native and Emscripten builds, then run the same scenario through
native SQLite, persistent browser images, and the existing iOS simulator host.
A browser run alone does not prove mobile persistence. No new interpreter, hidden
source resolver, automatic approval, or general network effect is required.

The first cluster adapter is explicitly trusted host code. It executes an actual
compiled graph recipe through Engine; it is not a compiled pure handler. The
canonical handler validator intentionally rejects Cluster and arbitrary Query.
Do not loosen that validator to make this scenario pass.

## Proposed boundary and authority layers

Proposed host transport format: `weave-host-request/1`, separate from Program
protocol. A strict tagged request enum delegates to typed Engine APIs. There is
no reflective method lookup, caller SQL, filesystem path per operation, callback
code, or generic transaction script. One request invokes one documented operation;
it does not promise atomicity across several native methods.

1. **Embedding configuration:** a trusted application opens an opaque session
   with fixed principal/write scope, local store identity/profile and independently
   installed policy/key/signer/destination providers. Installation and changes to
   these providers are separate privileged entry points, never ordinary request
   verbs. Engine-owned operation clocks remain authoritative. Manual time and
   fixed signing keys exist only in the test host. A source valid-time literal is
   data, not an authorization clock.
2. **Bounded operational requests:** execute Programs, operate only authorized
   adapter IDs, submit signed evidence, read exact accepted occurrences, or operate
   existing intent IDs. Fields cannot replace the session actor, grants, clock,
   signing key, paired peer, governance root or destination registry. Approvals
   are independently signed or explicitly requested from a trusted approver;
   compilation, receipt of a capsule and host bootstrap never approve a proposal.
3. **Untrusted source/artifacts/peer bytes:** compiler output is validated using
   canonical Program/template helpers and normal native installation. A graph's
   attribution attachment is informational; it cannot authenticate compiler or
   handler origin. Receiving a capsule never installs peer governance/effect
   registries or makes imported decisions locally authoritative.

The privileged plane is a trusted embedding API, not a new remote capability
protocol. A worker isolates execution and persistence, not a malicious embedding
application or compromised same-origin code. Production credential custody and
untrusted adapter sandboxing remain separate requirements.

Proposed Rust seam: a session holding Engine and trusted configuration, with
bounded `call(request_bytes) -> response_bytes`; native and WASM wrappers share
its decoder/dispatch. The facade adds a new entry point and keeps the existing
Program-only ABI compatible. Use opaque ASCII handle tokens, checked for exact
syntax/range, never JSON Number handles. Sessions cannot change identity through
calls. Panic/trap/unknown storage state invalidates the session.

| Operational family | Existing native path and boundary |
|---|---|
| Graph work | `execute(Program, HostContext)`; original protocol preflight, command and materialization limits. |
| Compiled projection | install through privileged manifest/output binding; poll, prepare, complete, and lifecycle under durable adapter ownership. |
| Trusted cluster adapter | compiled read recipe through execute, exact captured output/CAS through guarded raw completion; never use raw completion for compiled/bridge IDs. |
| Transfer | `admit_capsule_export`, paired response verification, `admit_proposal`, explicit `integrate_proposal`; keys/policies are installed out of band. |
| Governance | propose, record signed approval, accept exact proposal/head, query exact occurrence; root installation is privileged. |
| Effect bridge | typed poll, enqueue, status, begin, cancel and reconcile; grant installation/revocation and sink mapping are privileged. |

Several current native adapter methods intentionally take only an adapter ID.
They cannot be forwarded unchecked through a principal-bound session. Add narrow
native guarded wrappers that check durable manifest/registration ownership within
the operation snapshot before normal processing and duplicate fast paths. Do not
use a session-only ownership map that loses meaning on reopen. Preserve all
compiled/bridge raw-bypass guards. Foreign IDs must return generic denial.

## Complete artifacts and exact bytes

Keep the complete compiler response and its artifact fingerprint, including scalar
values and every view/handler template. Program execution does not imply template
installation. The host requires an explicit installation selection and host-owned
manifest/output binding; unused artifacts remain available and are reported.

Provide a bounded Rust artifact-selection helper which takes the original SDK
response bytes and returns selected Program/template bytes plus a complete artifact
inventory. It is format/type validation, not source interpretation or authority.
Use typed decoding/raw JSON slices, with duplicate/unknown field rejection and
canonical template validation; keep the full original response. Do not reduce a
bundle to Program and silently drop templates. Coordinate this envelope with the
language SDK owner; no engine dependency on compiler implementation is required.

Requests/responses, signatures and persisted replay bodies cross JS as UTF-8 byte
arrays or exact strings. No `JSON.parse`/`stringify` cycle of graph/artifact bodies,
i64 values, quantities, signatures or u64 ordinals. JS may decode a small separately
typed control envelope and pass its opaque payload bytes unchanged. C/Swift copies
bounded buffers with explicit ownership. Test values above 2^53 and i64 endpoints.

## Combined runnable trace

Use independent P (phone), W (workstation) and T (team) stores plus a separate
reference-sink ledger. The positive first profile uses the same principal across
stores; a distinct reviewer exercises denial. This is not declassification or
multi-user production collaboration evidence.

1. Compile supplied source and exact modules with the real SDK. Preserve compiler
   responses and all manifests. Install the diagnostic handler through normal
   compiled installation; bootstrap admission/governance/effect policies only via
   explicit trusted controller operations.
2. W creates operational/physical manifestations and pinned edge evidence, with
   private annotations outside the selected working closure. Signed exact export,
   paired verification, Propose quarantine and explicit integration establish P's
   offline branch. A supplied peer image is never used as graph transport.
3. Disconnect transport. An actual compiled Program commits a new measurement and
   named edge-metadata rebind in one logical batch. Poll and prepare the compiled
   Metadata→Reason diagnostic; complete it once, retaining original exact evidence
   and owned proof-bearing wrappers through restart. Existing `graph.committed`
   events drive this step; typed `MetaGraphRebound` is still a distinct open gap.
4. A trusted cluster adapter consumes the warning occurrence. Compile its exact
   pinned graph recipe with the SDK, evaluate through Engine, preserve the entire
   protected cluster GraphData and create an owner-scoped completion Program with
   a captured output CAS. Persist that exact recipe/input/output/completion record
   before calling raw completion. Retries reuse the same bytes and expected head;
   they do not query a newer head, rebase, or rewrite provenance. The record is
   host-owned adapter state, never caller-supplied proof of authorization. Recheck
   the captured result's current authority under a native operation snapshot before
   completing; use a narrow guarded wrapper around the existing result-authority
   and in-transaction completion helpers if necessary. This wrapper must not bypass
   raw completion's compiled/bridge rejection. No new persistent kernel registry is
   assumed: if safe composition needs one, stop for schema review.
5. Reconnect with controlled loss/duplication. Signed export must cover the complete
   emitted ancestry, manifests and all proof/metadata dependencies. Scope narrowing,
   expired request/response expectation and key rotation deny. Explicit integration
   preserves conflicts and prior evidence; W may generate a distinct larger cluster
   while retaining P's proposal. Transport has no implicit Publish or Accept action.
6. T receives a proposal, obtains an explicit signed owner/quorum approval, and
   accepts the exact source under its own genuine protected decision registry. Read
   both the current and historical accepted occurrence, and Explain the original
   evidence. Peer approvals/decision graphs do not become local trust roots.
7. A separately installed CanonicalGraphV1 grant consumes the real typed governance
   publication, creates the intent and acknowledges atomically. Fence unknown before
   releasing the dispatch ticket to the same-principal reference sink. Lose the sink
   response, restart, and reconcile from its independent ledger. The runtime must
   not issue another ticket. Also retain the non-idempotent destination's unresolved
   outcome rather than inventing evidence. Superseded publications get the existing
   authenticated no-effect disposition; revoked pending work remains owner-cancelable.

The cluster adapter journal belongs to the trusted host, with a small versioned
format and content checksum. In browser storage it is included in the same IDB
generation transaction as the database image; native/mobile persist the immutable
record before invocation. A lost completion response retries the existing kernel
receipt, so this does not claim a distributed transaction across host journal and
SQLite. Bound retained records and stop at quota; never silently forget unknown
completion or stale-CAS work. Test journal loss/corruption as fail-closed recovery,
not permission to regenerate a different Program.

## Persistence and failure contract

Native/mobile reuse file-backed SQLite and existing transactional Engine methods.
The browser reuses the reviewed single-worker, lifetime Web Lock, DELETE-journal
image profile, explicit create/reopen sentinel and one IDB generation transaction.
An acknowledged generation covers the database plus any host replay journal state.
Opening/migration and every call, including reads that may write replay receipts,
leases or caches, follow the same fence. Assume no verb is read-only by its name.

After Engine returns, retain the outcome privately, validate bounded serialization,
export a quiescent image, and await IDB transaction completion before replying.
This includes **errors after possible mutation**, not just success. A known engine
rejection may be returned only after a coherent image is durably fenced; do not
assume all service errors rolled back every earlier action. If serialization,
image export, quota, transaction completion or worker state is ambiguous, return a
host uncertainty error where possible and poison the session. No payload, receipt,
lease or effect ticket may escape. Reopen the last committed generation and inspect
state; never automatically replay an arbitrary Program or effect begin.

For a lost ticket response after durable unknown, inspection reports unknown and
no second ticket. For failure before the image commits, no ticket was released;
old pending state may be inspected after reopen. Worker/process death during IDB
transaction may yield a complete old or new generation, never a mixed database and
host journal. Browser storage eviction/power loss is not proven by process tests.
No new rollback-resistance or trusted wall-clock claim follows from this host.

## Bounds, evidence and implementation order

Keep existing kernel budgets. Initial facade: one in-flight call per session,
16 MiB raw request, existing 32 MiB materialized result plus a small fixed envelope,
128 KiB host configuration (larger installed policy objects use their existing
bounded native paths), no unbounded operation batches. Preserve the current
16-command Program limit. Artifact helper accepts the SDK's bounded full response.
Start with at most 16 retained host cluster records/8 MiB aggregate, counting actual
serialized bytes before copying; these are proposed host limits, not new kernel
quotas. Separate compiler/runtime workers so their heaps do not accumulate together.

Browser remains an experimental 8 MiB **whole durable generation** profile; journal
bytes count too. Measure per-peer image/journal size, largest capsule, maximum
request/result and end-to-end SQL+copy/hash+IDB time. Do not truncate history/proofs
or increase caps silently to pass. If the meaningful bounded scenario exceeds the
cap, report the capacity boundary and request a reviewed storage step. Keep the
current 256 MiB runtime heap cap and inherited proof/read limits. These are not
peak-RSS guarantees. Reuse installed SDK, browser, native/iOS targets and caches;
no new toolchain, simulator, cache or heavy dependency is needed for the first seam.

| Gate | Required evidence |
|---|---|
| Generic facade | Two different runtime-supplied sources/modules and changed handler artifacts; no graph names, recipes, keys or scenario opcodes baked into runtime. Complete artifact inventory; foreign-ID, malformed/oversized, exact-number and raw-bypass negatives. |
| Native oracle | Full trace with exact source/module/template/revision links, persisted cluster completion body/CAS and receipts; controlled deaths and duplicate transport; exact historical explanation. |
| Persistent browser | Same trace/bytes and semantic assertions with page/worker/process restart, actual IDB abort/quota, exclusive-lock contention and lost responses at preparation/completion/enqueue/begin/reconcile boundaries. Ticket leakage counter stays zero before the fence. |
| Mobile | Same host requests and artifacts on existing iOS simulator, app-process reopen and unknown recovery. State separately which tests are process termination versus physical device/OS power loss. |
| Authority | Reviewer denial, hidden annotations omitted without inventory leakage, revocation/missing exact inputs, empty/scalar/attachment gates after envelope/readers stripping, stale output CAS, forged approval/peer key, narrowed scope, supersession and owner-only cleanup. |
| Parity/reporting | Compare deterministic compiler artifacts and graph/proof semantics. Opaque occurrence/lease/attempt IDs and real clock samples are expected to differ; verify their exact references within each run, never assert byte-equal whole databases. Record process counts, store generations, sink actions and measured resources. |

Implement in reviewable order: (A) generic facade and principal/byte/durability
contract with native tests; (B) actual SDK diagnostic + trusted cluster journal
through native; (C) transfer/governance/effect native oracle; (D) same requests through
browser; (E) iOS bindings and process recovery. Root reviews each seam before widening.
No public protocol/store change is expected merely for wrappers; any required
persistent authorization field needs a separate compatibility/old-binary decision.

Engine owns facade/native guards, generic browser host, trusted cluster journal and
native/mobile operation adapters. Language owns source/modules, SDK extraction
agreement and compiler acceptance; proposed temporal Window/Sequence work belongs
in its isolated canonical module and is not a scenario prerequisite. Root owns
independent adversarial orchestration, baseline preservation, CI and publication.
Only this proposal is edited now; no builds while the sibling owns the slot.

This advances persistent E10 scenario evidence, not full paper completion. Typed
metadata event contracts, true incremental clustering, general lifecycle/retention,
expired replay/rebase, feedback-loop control, untrusted adapter isolation,
cross-principal release and production destination semantics remain requirements
outside this slice. NAT/rendezvous, broker bridges, a visual UI, registry packaging
and a general source cluster-reactor DSL are optional for this bounded acceptance.
