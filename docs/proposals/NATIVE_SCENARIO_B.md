# Native scenario B: compiled diagnosis and retained cluster completion

Design-only implementation handoff, 2026-09-20. Start after the coherent 0.19 pair
is published; the reviewed native candidate is `a6adb94`. No runtime edit, Cargo
run, protocol reservation or store migration is part of this proposal. This refines
step B of [the combined scenario](COMBINED_PORTABLE_SCENARIO.md); transfer,
governance, effects, browser bindings and mobile execution remain later steps.

## Deliverable

One reproducible process trace runs real compiler SDK responses through the existing
safe Rust host boundary and SQLite engine. It proves an atomic offline evidence
rebind, a compiled Metadata→Reason warning handler, and a trusted host's durable
cluster completion. Kill/reopen tests retain the exact input pins, preparation,
completion body, expected head and historical receipt. A separate reader principal
cannot obtain the private diagnosis or proposal.

Use one active P store (`phone.db`) and a separate host journal (`phone-host.db`).
W/T names are reserved for the later transfer stage, not simulated network peers
in this report. A seed snapshot represents the already available working set;
B makes no signed-transfer claim. All processes are local and test-owned. The
principal is `collector`; the negative reader is `reviewer`. Time, principal,
write scopes, adapter manifest and output mapping are trusted fixture configuration.
No source text, artifact or operational request can install that configuration.

## Exact fixture inventory

Place source inputs in language `examples/native_scenario/`, with the engine
controller accepting the fixture directory as input rather than embedding sources.
Preserve each SDK request and its original complete response bytes in the trace.

| Input | Content and expected artifact |
|---|---|
| `seed.weave` | One `transaction seed` creates Evidence and Installation. Evidence has positive `quality_ok` from baseline to gateway over [0,100). Installation has operational/physical manifestations, `connection`, and edge attachment `evidence-binding` pointing to `logical:seed:Evidence`. Include integer 9007199254740993 and Unicode in inert properties. |
| `diagnostics.weave` | Pinned module containing a pure function: Metadata on the current `connection` edge/key `evidence`, then a rule with explicit **negative** `quality_ok` body and positive `warning` head, then warning selection at valid-time 7. Absence alone is not a warning. |
| `handler.weave` | Imports that exact module hash and emits `Diagnostic` handler, Installation/main, metadata depth4, `graph.committed`, output slot `warnings`, pinned replay. Also emits scalar values and an unused second handler to prove complete inventory survives selection. |
| `offline.weave.in` | Controller substitutes escaped exact seed revisions into source `graph … replace revision …` declarations. One `transaction offline` adds a negative measurement over [5,20) and changes `evidence-binding` to `logical:offline:Evidence`. Compile after substitution; do not patch the returned Program. |
| `cluster.weave.in` | `cluster_navigation Proposal source graph "Warnings" revision "<exact warning event revision>" relation "warning" at 7 levels 2 context default;`. The host serializes the exact returned revision as a source literal before compiling. No latest-head query, interpreter or Cluster in the sealed handler allowlist. |
| Variant2 | Change graph IDs, module/rule revision, warning predicate and output mapping coherently. It must produce distinct artifacts and still pass; runtime code cannot dispatch by fixture names. |

A separate trusted fixture commit places reviewer-only annotations outside this
working closure. The report distinguishes this host-authored negative setup from
compiler-authored domain graphs. Graph-valued metadata is required for navigation;
a Literal attachment remains inert and is not a substitute graph.

## Runnable sequence

1. Invoke the real native compiler SDK ABI1 in a short child process for each
   source/module bundle. Feed the original success response to
   `weave_native::artifacts::ArtifactBundle::parse`. Record complete inventory,
   raw SHA256 and selected Program/template bytes. Execute source Programs through
   `HostSession::call`; install only Diagnostic through its existing privileged
   typed installation method. Unused values/templates stay in the original bundle.
2. Run the initial diagnostic event completely; it may produce an empty warning
   value, whose exact snapshot influence must remain representable. Capture its
   output head. Execute the compiled offline transaction once. Query both old
   exact revisions and the new head: old metadata still resolves old Evidence;
   the new attachment resolves precisely the offline logical sibling. Two graph
   changes commit atomically, not one rebind followed by an independent write.
3. Poll/prepare Diagnostic using existing principal-bound host verbs. Kill before
   and after the preparation commit, reopen, renew the lease and complete the
   original preparation. Kill before and after completion COMMIT. Receipt retry
   is historical/idempotent, not a fresh query. Exactly one warning revision and
   acknowledgment result; Explain retains both the negative measurement and exact
   evidence-binding attachment, plus the module/rule/template identity.
4. A separately installed **trusted host** cluster adapter subscribes to Warnings,
   with one output `ClustersPhone`, no effects and a pinned host recipe identity.
   Poll its exact event, create the runtime-pinned source above, compile via SDK,
   and execute its one Cluster bind/evaluation. Require Complete coverage. Preserve
   the complete QueryResult and original artifact bundle, including source manifests.
5. Capture the output head once and construct one canonical Commit Program from
   the full protected cluster graph, with all records restricted to the installed
   principal. Persist the retained record before calling completion. A fresh
   completion or historical retry revalidates the retained input/result under the
   current principal before returning any output or receipt. Never recompile,
   re-evaluate a changed head, regenerate IDs or replace the expected head on retry.
6. Query the exact committed cluster revision, its provenance and historical
   diagnostic; compare with the recorded output and pins. Perform authority/CAS,
   journal-failure and process-death negatives below. Leave a machine-readable
   report and retained artifact hashes; temporary stores are cleaned on success.

The acceptance controller may inspect a returned wrapper ID and submit that exact
ID in a later metadata plan. It cannot infer aliases from `derived_nodes`. Ordinary
annotations, template labels and source manifests are descriptive, not authority.

## Minimal new trusted host seam

Existing APIs cover compilation, artifacts, Programs and compiled handlers. The
missing safe composition is the externally retained cluster completion:
`complete_handler` reauthorizes its event and prior **Queried** receipt results,
but a commit-only caller Program has no stored QueryResult guard. A preflight
check followed by a separate completion transaction is insufficient.

Proposed Rust-only host API shape (names provisional, no C/worker verb expansion
in B):

- `ClusterJournal::prepare(session, adapter, envelope, artifact_bundle, output_binding)`
  validates the installed owner/subscription; accepts only a write-free Program
  with one named Cluster result and its required source manifest, with the Cluster
  exact source matching the delivered event. It evaluates once, requires Complete,
  captures CAS, charges bytes, and durably inserts an immutable retained record.
- `ClusterJournal::complete(session, record_id, current_lease)` loads that record
  with bounded reads and verifies its binding/digest. The caller supplies no result,
  Program, principal, clock or revised expected head. Lease renewal is transport
  state outside the immutable computation identity.
- A narrow Engine trusted-host helper performs BEGIN IMMEDIATE → durable adapter
  ownership and ordinary-adapter checks → exact event/record binding → current
  retained-result and full source-closure authorization → existing
  `complete_handler_in_transaction` → COMMIT. Checks precede duplicate receipt
  return. Compiled-handler and governed-effect adapter IDs remain rejected.

The helper must compare the sole Commit payload/output/CAS/source manifest to the
retained result plus the explicitly specified principal restriction transform;
no extra commands, writes or effects. Reconstruct all semantic dependency pins
from the retained result and exact source records, not only a caller/journal Vec.
Check existence, integrity, current protected guards and full visibility where
whole input was consumed. Carry the actual source snapshot gate for empty output.
Descriptive `input_snapshots` must not silently become authority gates. Preserve
all current OR branches and original-record guards under one operation budget.

The journal is trusted embedding state, not a signed permission credential.
An attacker controlling the trusted process already controls raw Engine APIs;
B does not sandbox that process or revoke its existing raw capability. Untrusted
facade requests gain no arbitrary raw-completion verb. If the implementation needs
kernel-persisted managed-adapter guards to make a stronger guarantee, stop for an
explicit schema/old-binary review instead of claiming host routing enforces it.

Before implementation, review this helper's concrete input type and equality rules
in a small first checkpoint. Do not expose `require_current_result_authority` as a
standalone check-and-use operation or accept a caller's empty witness as proof.

## Host journal and crash matrix

Use a separate bounded SQLite journal with the **same linked rusqlite/SQLite copy**
as Engine, reusing the locked dependency version and bundled library. This is a
host-local format, not an added Engine table or schema version. Avoid concurrent
opening of the same file through a second SQLite library in the test process.
Independent inspection runs in a separate process. Native journal persistence uses
its own FULL-synchronous committed transaction before Engine invocation; there is
no claimed atomic transaction across the two databases.

Identity is `(configured_store_identity, principal, adapter, event_id)` plus a
separate domain hash `weave-native-cluster-journal-v1` over the immutable body.
The body retains exact SDK response bytes/hash, selected Program, event GraphRef,
manifest/config identity, output binding/CAS, complete result, full source manifest,
constructed completion Program/hash and closure digest. Persisted closure metadata
is checked against reconstructed record content. The hash detects corruption, not
authenticity. A separate observed receipt record may be appended after completion;
it never changes the computation body.

| Death/error boundary | Required restart behavior |
|---|---|
| Before journal commit | No completion was invoked; re-evaluate only if no retained record exists, under the original exact event. |
| After journal commit, before Engine call | Load the same bytes and CAS; current authority checked again. |
| Before Engine completion COMMIT | No output revision/receipt/checkpoint; same retained record remains pending. |
| After Engine COMMIT, before response or journal receipt | Retry the identical retained Program; one historical receipt, one output revision, no second acknowledgment. |
| After observed receipt commit | Return only after current authority recheck; retain original receipt identity. |
| Output head changes before completion | Conflict, retained record unchanged, event unacknowledged. No automatic rebase or skip. |
| Source guard revoked or exact premise missing | Deny both fresh and historical completion; no result/receipt payload. Reopen does not restore authority. |
| Journal body/checksum/closure differs, missing record, or quota exhausted | Fail closed. If kernel receipt exists but journal identity is missing, do not regenerate. Trusted inspection/recovery is explicit. |

Keep 16 retained records, 8MiB aggregate **serialized** retained bytes and 2MiB per
record, charged before decode/copy/retention; also bound SQL text length and source
response raw size by ArtifactBundle's existing separate cap. Exceeding either is a
reported capacity rejection, not truncation. Source/compiler caps, kernel proof
budgets and HostSession request/response limits continue to apply. No automatic
retention eviction. Reaching the small host quota is backpressure; lifecycle and
stale-CAS cancellation are separate future work.

## Files, commands and acceptance

Engine owns new `crates/weave-native/src/cluster_journal.rs`, its bounded host
methods, the narrow Engine completion helper, `crates/weave-native/examples/native_scenario.rs`,
`crates/weave-native/tests/cluster_journal.rs` and
`scripts/check_native_scenario.py`. Language owns the source fixtures and reviews
actual source/module identities. Root owns independent fault/authority tests and
CI/publication. No one edits shared protocol enums for B. Source artifacts are
loaded at runtime; fixture graph names, programs and keys stay out of library code.

Planned commands after approval (these files do not yet exist):

```sh
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 nice -n 10 cargo test --locked -p weave-native --test cluster_journal
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 nice -n 10 cargo build --locked -p weave-native --example native_scenario --features recovery-testing
python3 scripts/check_native_scenario.py --compiler-sdk "$COMPILER_SDK" --host target/debug/examples/native_scenario --fixtures "$LANGUAGE_ROOT/examples/native_scenario" --report native-scenario-b.json
```

`recovery-testing` would only forward the existing engine feature and expose
controlled test-process death hooks. The no-build Python controller launches every
runtime action in a fresh process with a timeout, compiles through the real SDK,
and passes opaque artifact bytes without a JSON round-trip. Requests containing
i64 literals remain exact. Process counts/deaths, compiler/template/source hashes,
all result pins, journal/result sizes, CAS and receipt IDs, elapsed times and peak
child RSS are measured, not promised in advance. It cleans only its own temporary
stores/processes and closes SQLite connections explicitly on all platforms.

Mandatory negatives: foreign adapter; raw compiled completion before/after receipt;
changed module hash; duplicate SDK keys; incomplete artifact inventory; live input
rejected; partial cluster input; missing exact evidence; private branch/attachment
restrictions after copied output readers/envelope are stripped; narrowed current authorization; stale
CAS; a trimmed closure with recomputed checksum; journal/body corruption; both
journal and Engine commit deaths; one replay after an independently committed
receipt; and a permanently blocked record never silently acknowledged or replaced.

B closes the source→offline rebind→durable diagnostic→retained cluster path only.
Typed MetaGraphRebound delivery, general source cluster reactors, cluster incremental
maintenance, production adapter isolation, cross-principal release, retention/rebase,
signed P/W/T transfer, governance/effects and browser/mobile durability remain
explicitly outside B. Temporal .19 operators may be used by a second supplied recipe,
but B does not depend on inventing a new temporal or replica-observation API.

## Concrete helper checkpoint for review

The following is the proposed first implementation boundary, still docs-only.
Names are native Rust APIs, not Program DTOs or ordinary JSON operations.

```rust
// Native Engine types; no compiler crate dependency.
struct RetainedClusterCompletion {
    runtime_source: String,
    adapter_manifest_digest: String,
    event: GraphRef,
    recipe: Program,
    result: QueryResult,
    completion: Program,
}
enum HandlerSlotState {
    Pending,
    Completed { request_hash: String }, // no historical result payload
}
impl Engine {
    fn runtime_source_identity(&self) -> Result<String>;
    fn inspect_handler_slot_for(
        &self, adapter: &str, event: &str, authority: &HostContext,
    ) -> Result<HandlerSlotState>;
    fn complete_retained_cluster_for(
        &mut self, adapter: &str, event: &str, lease: &str,
        retained: &RetainedClusterCompletion, authority: &HostContext,
    ) -> Result<HandlerReceipt>;
}
```

`runtime_source_identity` reads the existing singleton `engine_identity.source`,
which also appears in genuine dispatch envelopes; it cannot be supplied by a
request. No new UUID column or migration is needed. The completion helper compares
it to the retained binding inside the operation transaction and compares the actual
scoped event graph/branch/revision to both the captured envelope and Cluster source.
The retained body will include the branch alongside the GraphRef above (the sketch
uses the existing event ID argument to avoid inventing a second occurrence ID).

At trusted host open, the application registry binds a configured host store ID to
one Engine storage locator and its actual runtime source identity. Journal header
and every record contain both identities. A journal mismatch fails before loads
are returned; completion repeats the runtime comparison. Opening a distinct store
cannot adopt the journal merely by passing its filename or copying the configured
ID in operational JSON. Copying a DB to a second registered host store gets a new
host-store binding and fails record identity even if the DB copied its old runtime
UUID. Explicit restore into the original registered store is a separate trusted
recovery operation, not a request verb. This is not protection against a malicious
embedding application cloning its entire registry/store/journal, nor rollback
resistance. Tests cover two independently created stores and a copied DB opened
under a different registered host-store ID.

`inspect_handler_slot_for` uses an operation read snapshot and durable owner,
adapter-kind, event and current-authority checks. It reveals only Pending or a
bounded existing request hash to the owner. Before preparing any missing journal
record, call it: Completed means `E_HOST_JOURNAL_MISSING`, with no recipe evaluation,
new record, output or acknowledgment. If a record exists, load it and use normal
current-authority completion, regardless of whether the optional observed-receipt
journal entry exists. A checkpoint beyond the event without its expected receipt
is inconsistent state and fails closed, not Pending. The test host serializes all
journal operations for this store; a second concurrent trusted journal writer is
rejected by the host lock. It is not claimed to constrain arbitrary privileged
Engine callers outside that host.

The sole accepted recipe shape is one source-authored `Bind { name, value: Cluster }`
(or exactly one equivalent Evaluate), with no references, additional bindings,
writes, native view/governance reads or live metadata. Its Cluster source equals
the exact warning occurrence. Context, relation, levels and valid_at are retained
verbatim; the host-installed recipe policy restricts their allowed profile. The
helper checks that shape and manifest ownership again on replay. This does not
make Cluster eligible for a compiled pure handler.

The sole completion is one Commit to the manifest's one configured output graph
and branch with the recorded expected head. Source revisions equal the conflict-
checked union of recipe and computed result sources. The expected GraphData is
exactly the captured result graph with only each supported record's readers set
to `[manifest.principal]`; no IDs, proof groups, metadata, intervals, values,
contexts or gates may change. The graph must retain the captured whole-value
influence. It includes the event snapshot gate when needed for empty input/output.
The helper compares this expected graph to the Commit before invoking the existing
completion helper. It does not trust artifact attribution attachments to establish
this binding. Graph computation was performed by the installed trusted host using
Engine; this narrow helper validates binding/current authority, not a certificate
that arbitrary caller-authored data was produced by a compiler.

All equality is canonical **typed Rust** serialization after strict bounded
parsing, duplicate rejection and native validation. Use separate domains for host
registration, retained body and observed receipt digests. Objects/maps follow the
existing canonical identity helper; integers remain i64/u64, with no binary64 or JS
Number conversion. Test changed key order as equivalent and
9007199254740992 versus 9007199254740993 as different. Preserve original SDK bytes
in a separate immutable BLOB with their own raw-byte SHA256; canonical identity
never replaces that original response. Journal quota counts BLOB plus typed body
before insertion. Existing handler request hashes remain unchanged.

Within BEGIN IMMEDIATE, ownership/kind/store/event checks, bounded retained-body
validation, reconstructed exact closure/current guards and result authorization
all precede the existing handler receipt fast path. Use one operation clock,
read budget and proof traversal context. The closure starts from the genuine event
source and every semantic record gate/metadata dependency actually retained, and
loads pinned records to verify existence/integrity/current authority. Never accept
a trimmed journal closure Vec or an empty QueryResult as a substitute. Only then
call existing `complete_handler_in_transaction`; rollback all new output/receipt/
checkpoint state on validation, budget, CAS or commit failure. Historical receipt
return uses identical current checks, but does not re-execute/rebase the cluster.
