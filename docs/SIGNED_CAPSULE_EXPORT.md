# Native signed exact capsule export

`Engine::admit_capsule_export` is a bounded signed read endpoint for an exact whole capsule. `Engine::verify_capsule_export_response` verifies the paired server's historical response against a currently outstanding request. Both use the engine-owned operation clock. This is a native embedding API, not a deployed transport service, automatic synchronization loop or recipient installation authority.

## Request, pairing and response

The engine-local request is `CapsuleExportRequest { format, root, branch_id, server_key, response_audience }`. Its required format is `weave-capsule-export-request-0.1`. The existing policy Request signs the digest of compact typed request JSON under Action::Read; Read and Traverse scopes are both required. No new Program operator, capability action, shared protocol version or capsule version is introduced. Fields are strict and bounded; canonicalization is typed serialization, not preservation of arbitrary incoming JSON spelling.

The embedding host constructs `CapsuleExportSigner::new(SigningKey, server_audience, pairings)`. Its private fields retain a bounded subject-to-recipient-audience pairing map. Request fields select an already installed key/pair; they never install trust. The key must match the request and the serving audience must match the installed admission policy. Keys are never stored in SQLite or serialized receipts.

The signed response binds its format (`weave-capsule-export-response-0.1`), paired server key/audience, recipient subject/audience, request nonce/body digest, admission epoch, exact root/branch, protocol/capsule versions, capsule digest and first admission's `served_at_ms`. The signature uses strict Ed25519 with weak-key rejection and the separate domain `weave-capsule-export-response-signature-v0.1`. It signs compact typed JSON for `(domain, binding)`. The capsule digest covers compact typed Capsule JSON including all revision, manifest and list contents. An immutable fixed signature vector and cross-domain negative test pin this encoding.

A trusted requester constructs `CapsuleExportExpectation::new(paired_key, server_audience, recipient_subject, recipient_audience, request, outstanding_policy_request)`. It must use independently configured peer/endpoint identity and its actual outstanding request, not echo the response's included key. Verification captures the receiver's trusted time once, requires a currently valid expectation, verifies all bindings and content digests, and rejects future `served_at_ms` without implicit clock skew. `VerifiedCapsuleExport::capsule()` / `into_capsule()` expose authenticated bytes only. Propose admission and explicit integration/acceptance remain separate operations with their own authority.

## Complete closure and current authority

The root must be reachable from its requested accepted branch. Every emitted revision must be reachable from some granted Read+Traverse branch; creation branch and received-only quarantine are insufficient. The export walk includes all parent ancestry, whole logical manifests and every semantic dependency: metadata, origins, structural references, context witnesses, whole-value influences, node and assertion gates, alternative premises, and input snapshot indexes. The signed profile rejects partial/external dependency inventory and live handles. Reserved local identity/governance records do not become remotely installable authority.

Each record passes current protected-reference guards and whole-snapshot authorization. Flat compatibility index permutations use the narrowly defined comparison in [whole-snapshot authorization](WHOLE_SNAPSHOT_AUTHORIZATION.md); no other proof group/content change is normalized. Primitive visibility is checked before following object dependencies. Missing, denied and out-of-history graph-dependent closure failures use the same unavailable response. Invalid format/key/signature and missing root-operation grant remain admission errors. Resource failures remain explicit; no timing/RSS noninterference claim is made.

An immediate SQL transaction contains current proof verification, export, receipt and nonce reservation. The exact response and admitted closure are stored in the existing admission receipt row. An error or process death before COMMIT leaves no consumed nonce. No source head, graph event or recipient proposal changes during export.

On retry the engine first verifies the current proof, audience, epoch, revocation and time. It reconstructs closure from the signed capsule and requires the stored dependency vector to match it exactly. Every emitted record is compared against current integrity-verified stored content and reauthorized under current scope and guards. Trimmed cached metadata cannot remove a disclosure obligation. The response is not regenerated or re-signed.

Logical replay identity excludes capability IDs, signatures and proof time windows. A refreshed valid proof for the same subject/audience/nonce/body can retrieve the same historical response after the original proof expires, provided its current scopes still cover the full disclosure. Narrowing, changed epoch or revoked authority denies replay. `served_at_ms` stays historical; response verification does not claim a fresh remote-policy attestation. The receiver must have a freshly valid expectation. Already disclosed signed bytes cannot be recalled. Rotation denies an old-key receipt unless the host explicitly retains that original signer/pair; a new key/body needs a new nonce.

## Limits and storage compatibility

The request/header limit is 16 KiB each; capsule 16 MiB; final stored response 32 MiB. Closure is bounded to 1000 revisions, depth 32 and a shared 10000 dependency/ancestry-work count. Repeated reference occurrences are charged before retaining queue entries, the queue deduplicates globally, and bounded deserializers cap stored/wire revision, manifest and dependency lists. Existing cumulative 128 MiB/4096-read operation limits and per-subject 10000-receipt/64 MiB quotas apply. Receipt SQL bounds both response and operation text before allocation and charges them before decode. These are serialized/work bounds, not measured process-memory isolation or delta-only I/O.

SQLite marker14 is retained: no table, immutable graph representation or shared DTO changes. New responses occupy the existing opaque receipt field, under a distinctly tagged body digest and the existing subject/audience replay identity. Root built the actual old `8d359df` native admission API in the existing cache and verified an old ordinary query with the export nonce fails E_REPLAY without changes, while a fresh nonce returns an ordinary QueryResult. Old execution leaves heads and original export receipt bytes unchanged. [Compatibility evidence](measurements/2026-09-20-schema14-export-compatibility.json) records the old source and probe hashes. This is tested API compatibility, not a claim that an old binary understands signed export.

## Verification

Native tests cover empty whole-value influence, logical siblings, structural-only references, ancestry, quarantined history, hidden/missing generic denial, cached closure tampering, refreshed proofs, subject/pair/key rotation, expiry, receiver tampering, bounded receipt reads, backpressure and no event mutation. The response signature vector and streaming-work test cover domain and repeated-input boundaries.

Run the existing process trace through the new endpoint:

```sh
CARGO_BUILD_JOBS=2 cargo build --locked -p weave-engine --example three_peer_trace --features recovery-testing
python3 scripts/check_three_peer_trace.py --signed-export --report /tmp/weave-signed-three-peer-report.json
```

The signed trace retains all prior offline evidence, warning/cluster, conflict, acceptance and unknown-effect checks. It additionally kills before and after export receipt COMMIT, restarts for exact replay, and verifies paired responses/tamper denial without recipient mutation. The host still installs test keys, runs recipes and chooses explicit acceptance. Selective proofs, remote trust-root installation, cross-user release, compiled reactors, reliable network queues and confidential transport remain outside this profile.

At this checkpoint, 258 engine test/doctest checks passed with all features, including the existing admission suite; strict all-target/all-feature Clippy and formatting passed. Both the original 88-process trace and signed 110-process trace passed on the final rebuilt fixture. The signed trace retained P/W/T event counts 8/9/2 and 52,585 serialized capsule bytes; its [local report](measurements/2026-09-20-signed-three-peer-trace.json) is fixture evidence, not a performance guarantee.

Independent orchestrator verification at `4b06f72` passed all eight focused export/security tests and a fresh signed 110-process trace with 52,585 capsule bytes and 8/9/2 final event counts. Root separately executed the exact old-source admission compatibility check described above, reviewed paired bindings/closure/receipt accounting, and confirmed temporary scenario stores were removed. Hosted integration pins public formatter compiler `e2ac5d5`; protocol remains 0.16 and schema14. Hosted success is recorded after execution.
