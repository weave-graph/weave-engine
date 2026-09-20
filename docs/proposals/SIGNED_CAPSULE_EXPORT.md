# Signed exact capsule export: proposed native profile

Status: design for joint review after the existing-API trace. No implementation, new Program operator, shared contract version or network deployment is included. The prior trace and comparison fix remain frozen. This proposal makes the serving-side authority and response authenticity concrete; it does not turn receipt into acceptance or attribute every embedded assertion to the serving peer.

## Request and response identity

Proposed engine-local, strictly decoded request (all fields required, no unknown fields):

```rust
struct CapsuleExportRequest {
    format: String, // exactly "weave-capsule-export-request-0.1"
    root: GraphRef, // exact graph and revision, never a live head
    branch_id: String,
    server_key: String, // expected paired ed25519 public key
    response_audience: String, // recipient endpoint identity
}
```

Canonical request bytes are compact `serde_json::to_vec` of this typed structure in the declared field order, not the submitted JSON text. The existing signed Request uses Action::Read, root.graph_id, branch_id, and the existing body_digest of these bytes. Both Read and Traverse grants are required. The format tag prevents confusion with QueryPlan under the same action. IDs, canonical public key and total request size are checked before serialization/signature work; use the existing 512-byte identity bound and a 16 KiB request bound. No request-supplied budget is accepted.

A response has an immutable typed header, capsule and detached signature:

```rust
struct CapsuleExportBinding {
    format: String, // exactly "weave-capsule-export-response-0.1"
    server_key: String,
    server_audience: String,
    recipient_subject: String,
    recipient_audience: String,
    request_nonce: String,
    request_body_digest: String,
    policy_epoch: String,
    root: GraphRef,
    branch_id: String,
    contract_version: String, // currently "0.16.0"
    capsule_format: String, // exact capsule.format, supported profile only
    capsule_digest: String, // existing SHA256 digest of compact typed Capsule JSON
    served_at_ms: i64, // engine operation clock at first admission
}
struct SignedCapsuleExport {
    binding: CapsuleExportBinding,
    capsule: Capsule,
    signature: String, // canonical lowercase Ed25519 signature encoding
}
```

Use strict Ed25519 verification and weak-key rejection consistent with existing policy primitives. Signature bytes are canonical serialization of `(domain, binding)`, with the new exact domain `weave-capsule-export-response-signature-v0.1`. The capsule digest binds the complete typed capsule including every revision, manifest and list order. This is not generic JSON canonicalization and must have fixed test vectors. The key is pinned by a trusted host pairing; the included public key is never a trust root. The complete header is bounded to 16 KiB before signing/verification and the capsule to the existing 16 MiB limit. Verify exact header/root/format/digest bindings in addition to signature validity.

The stable request identity binds subject, serving audience, response audience, nonce and body digest. It deliberately does not bind capability ID, request timestamps, chain bytes or the full SignedRequest signature. A replacement currently valid proof may retry the same logical identity after current authority and all original disclosure obligations are revalidated, including after the original proof expired. A narrower grant does not inherit the earlier broad authorization. The response remains the original historical bytes with its original served_at; it does not assert that the original grant is still valid or that the signature records a fresh policy check. A changed root/body/audience/key requires a fresh nonce. This is immutable response replay under fresh authorization, not re-execution.

The response signature authenticates the server's bytes and their request binding. It does not certify truth, origin-author identity, recipient write permission, current recipient policy or accepted governance. Confidential transport is separate; the local-process acceptance fixture uses isolated pipes/files.

## Host API and keys

Proposed native-only surface:

```rust
fn admit_capsule_export(
    &mut self,
    proof: &AdmissionProof,
    request: &CapsuleExportRequest,
    signer: &CapsuleExportSigner, // host-created, not Deserialize
) -> Result<Admitted<SignedCapsuleExport>>;

fn verify_capsule_export_response(
    &self,
    response: &SignedCapsuleExport,
    expectation: &CapsuleExportExpectation, // host-created, not Deserialize
) -> Result<VerifiedCapsuleExport>;
```

The signer owns a host-provided SigningKey, configured serving audience and a bounded installed pairing map from requester subject to permitted response audiences. The method requires its key to equal request.server_key, its audience to equal the installed admission audience, and the verified subject/response audience pair to be installed. No request may add a pairing. Serialized requests never install a key. The host reloads its key from its own key-management boundary after restart; no private key is serialized to SQLite or receipts. After rotation, an old-key cached response is denied rather than re-signed. To finish an old-key retry the host would have to explicitly retain that key and pairing; no implicit key fallback is provided.

The expectation contains the explicitly paired server key/audience, intended recipient endpoint/subject, exact request and its currently outstanding signed request identity/time window. It is built by trusted requester code for an outstanding operation, not from response fields. Verification captures the engine-owned clock once inside an outer read transaction, checks that the outstanding request is currently within its validity window, and that served_at is nonnegative and not in the receiver's future. A replayed historical response may predate the freshly issued retry request; that is allowed. No implicit clock-skew allowance. Response binding must match the expectation exactly and its capsule digest/format/root must match its capsule. An expired outstanding request, wrong peer, wrong recipient, wrong nonce, substitution and unsupported versions fail before any proposal mutation. The response has no independent renewable authority lease: receiver verification proves historical bytes bound to the currently expected logical request, not remote current policy. Already disclosed signed bytes cannot be recalled; fresh serving calls must still reauthorize before returning cached bytes.

VerifiedCapsuleExport has private construction and exposes the authenticated capsule to a subsequent, independently authorized Propose request. It grants no installation/head/event authority. Duplicate verification of an outstanding immutable response is allowed; receiver proposal nonce/receipt rules deduplicate the actual mutation. The host owns outstanding-request lifecycle and cannot claim that this pure receipt verification implements a durable network acknowledgment protocol. Supplying a different outstanding expectation rejects a stale nonce. General key discovery, rotation distribution and pending-network queues are not bundled here.

## Transaction and closure

Admission starts an immediate SQL transaction and one trusted operation-clock scope. Current request signature, chain attenuation, audience, epoch, revocation, time and replay_id are checked before receipt lookup. The original requested root must be reachable from the explicitly requested authorized branch. Every other emitted or influencing revision must be reachable through some currently granted Read+Traverse branch; a revision's creation branch is not sufficient authority. Quarantined received-only revisions are not accepted branch history.

A complete signed-export walk must include:

- every emitted CapsuleRevision, its parent ancestry, and every complete logical manifest sibling;
- graph references, required metadata, attachment origins, assertion/node/global/whole-value proof gates and typed context witnesses;
- structural_ref and every derivation.input_snapshots pin even if a normal generated value also repeats it in another proof field.

The existing query dependency checker alone is insufficient, and the native capsule walk alone is insufficient. Each whole record must pass primitive reader/endpoint filtering, current protected-reference guards and whole_graph_visible before it contributes dependent references. Then all semantic dependencies are traversed with scope and current authority. No hidden member's reference is returned as inventory. Logical manifests are all-or-nothing, including siblings not selected by the query. This profile rejects live handles and any missing, denied, budget-truncated or otherwise external dependency; never return an external_dependencies list. It may conservatively refuse an otherwise queryable partial graph.

Use one bounded queue/set across the export walk and scope validation, with at most 1000 distinct revisions, depth32, existing bounded ancestry work, cumulative native reads, capsule16 MiB and final receipt32 MiB. Charge before cloning/retaining/serializing output. Required source, ancestry or logical member absence and current denial yield a single generic unavailable result after valid proof admission. Invalid format/signature/key and syntactically invalid or missing root-operation scope remain distinguishable admission errors; graph-dependent missing/out-of-history/hidden closure details do not become discovery errors. Resource exhaustion is an explicit bounded-resource failure, with no constant-time/RSS hiding claim.

Before publication, record the exact signed response and exact full admitted closure atomically with the existing admission nonce receipt. Cache all bytes needed to return that response; do not add a fallible response-size check after COMMIT. A recovery-only observer immediately before commit supports process-death tests. No graph heads, graph events, structural/schema registries or recipient proposals change on export.

Retry verifies the current proof first, then reads bounded stored receipt bytes. It compares original logical request/key bindings, requires the current signer identity to match, and revalidates every originally disclosed/influencing reference under current grants/readers/protected guards and accepted branch history. It checks stored response integrity and the current proof expiry without regenerating or re-signing. The exact prior response is returned only after successful revalidation. Epoch change deliberately denies old receipts. Same subject+nonce replay identity remains the existing policy replay_id; do not introduce an unscoped nonce key.

## Storage compatibility proposal

Prefer reusing admission_receipts, storing a private tagged response payload:

```rust
struct StoredCapsuleExport {
    format: String, // exactly "weave-stored-capsule-export-0.1"
    response: SignedCapsuleExport,
    dependencies: Vec<GraphRef>,
}
```

It counts against the existing per-subject admission receipt quotas (10000 records/64 MiB). Bounded SQL extraction must precede allocation and parsing; improve the common prior-admission read if necessary rather than introducing an unbounded export-specific read.

No new table, graph field or authority-bearing revision encoding is needed. Proposed SQLite marker remains14: older binaries cannot construct the new tagged typed body through their existing QueryPlan/Commit/Capsule APIs, so their prior receipt lookup rejects body mismatch before response deserialization. Existing rows remain byte-identical. This compatibility claim requires an actual marker14 baseline test: old operations remain usable, attempting nonce reuse through an old read gets E_REPLAY, and the old binary cannot release the stored export response. If that cannot be established, use marker15 with migration/death/old-binary refusal rather than a speculative compatibility claim. This choice remains subject to root review.

## Acceptance before implementation freeze

Independent tests must cover fixed cryptographic vectors; key/signature/domain/format tamper; canonical request equivalence; root branch escape; ancestry and manifest siblings outside grants; every proof-only dependency form; hidden/missing closure generic denial; complete-byte retry after restart; narrowed capability denial and refreshed-proof time-window acceptance with unchanged response bytes; expired outstanding requests/grants, rotated keys and changed epochs; quota failures without nonce use; and pre/postcommit death with exact response replay. Receiver tests use its engine clock and explicit pair expectation, reject tamper before Propose mutation, then prove that successful verification alone leaves heads/events unchanged.

Finally rerun the existing three-peer trace through this endpoint and response verifier, retaining trusted explicit integration/acceptance and the fake unknown-effect boundary. Selective Merkle disclosure, inventory sync, peer key discovery, cross-user release, semantic merging, relay/rendezvous, automatic acknowledgment/retry queues and compiled reactors remain separate work.
