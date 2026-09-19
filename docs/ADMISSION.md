# Signed per-operation admission

The native host exposes `admit_query`, `admit_publish`, and `admit_proposal` as a bounded bridge to `weave-policy`. The local CLI and `HostContext` remain trusted administration APIs. There is no network server, signed arbitrary `Program`, remote policy installation, or signed proposal promotion in this profile.

## Trust and request boundary

A trusted host installs `AdmissionContext` roots, audience, revocations and epoch using `install_admission_policy`. Signing identities in tests are fixed public fixtures, never deployment keys. A principal is the request subject public key, not an unverified actor label. Claimed assertion source labels remain distinct from authenticated request senders.

The signed body is `serde_json::to_vec` of the exact typed `QueryPlan`, `SnapshotCommit`, or `Capsule`; defaults and map ordering follow the shared Rust wire model. The signature binds operation, graph, branch, subject, audience, body digest, nonce and request validity interval. Hosts must supply trusted admission time. Generic arbitrary JSON serialization is not claimed to be equivalent canonicalization.

Each operation starts a SQLite immediate transaction, loads the current installed policy, verifies the attenuated chain and request, then checks the body/operation boundary again. Receipt, nonce binding, publication and graph events commit together. Rejected requests leave no nonce receipt. Root/revocation changes require a fresh never-used epoch; old epochs cannot be reactivated. Explicit host nonce revocations are retained. Trust roots and these management methods must not be exposed as signed plan actions.

## Read and traversal scopes

Queries require both Read and Traverse on their requested graph/branch. A supplied revision must be reachable from that branch's accepted head, following immutable parent links; its creation branch alone does not establish permission. Accepted fork history is valid, while an unrelated private-branch revision is not.

Before native authorization follows provenance, admission first applies primitive object reader and endpoint/host filtering without following dependencies. Hidden objects do not enter traversal queues or cumulative visible-data budgets. It then preflights the remaining metadata, context, structural-origin and derivation dependency closure. Every referenced revision must belong to the accepted history of a Read+Traverse scope. Live metadata targets require that exact branch scope and are pinned under the same transaction. This conservative profile rejects the entire request when a remaining visible candidate object has an out-of-scope dependency. Denial carries no target identifier, count or payload. Selective scope-aware evaluation that can withhold individual candidate conclusions with inaccessible evidence is future work. Revision pins still identify whole immutable snapshots; this is not an activity-hiding or timing/RSS noninterference guarantee.

Bounds are 1,000 dependency revisions, 32 MiB cumulative dependency serialization, 1,000 ancestry depth per branch walk and 10,000 ancestry steps across closure checks. Native principal readers still apply after capability admission. Capability scope never clears object restrictions.

## Retry and durable response limits

A successful query stores its original pinned response plus the exact admitted dependency closure, including resolved live targets and provenance that may not appear in the requested output. A response-lost retry returns that same response only after verifying a currently valid proof, the same body/operation/subject/nonce, the same installed policy epoch, and sufficient current scopes for the cached dependency closure. A narrower proof cannot retrieve a broader cached result. Expired or revoked proofs do not retrieve cached results. Epoch changes require a fresh request and nonce.

Publication and proposal retries return their original immutable receipt without repeating mutation. Each subject is limited to 10,000 durable receipts and 64 MiB of serialized receipt data; admission returns `E_BACKPRESSURE` atomically at capacity. There is no automatic nonce eviction or retention promise. Hosts must currently provision storage and manage lifecycle out of band; cross-subject quotas and deployment rate limits remain open.

## Publication and proposal isolation

Remote publication accepts one snapshot on one authorized Publish branch. Every output node, materialized edge, structural edge, assertion and attachment must retain exactly the subject reader. Clearing restrictions, including copying private scalar data into a public object, rejects with `E_EGRESS`. Embedded schemas must exactly match an already host-installed descriptor; new descriptors cannot smuggle private data through the currently public schema boundary. Live metadata publication is unsupported until an explicit pinned export context exists. Referenced evidence must also pass Read+Traverse admission.

Propose does not imply Read, Publish or acceptance. Every capsule record needs a Propose scope for its graph/creation branch. Integrity is checked in an isolated temporary engine, then the capsule is stored in a separate proposal table, with a 64 MiB per-subject quota. It does not populate the accepted runtime's revision, structure, schema, head or event tables. Conflicting proposals therefore cannot preempt accepted identities. The envelope signature authenticates its sender and exact bytes; it does not authenticate every embedded claimed author. Promotion, authenticated sync, confidential schema policies, selective proofs and release/declassification policies are not implemented.

## Evidence

`cargo test -p weave-engine --test admission` covers durable pinned read replay across reopen, nonce/body conflict, pinned branch escape and accepted fork history, metadata/provenance scope checks, narrower retry denial, epoch ABA/revocation, subject-scoped publication and deduplication, exact schema installation, isolated proposal poisoning prevention, receipt-capacity transaction rollback, unsupported live publication, expired retry rejection, hidden dependency noninterference for visible payload/coverage, and original live dependency retention after head changes. These are host API tests, not a production network or process-crash acceptance claim. The independent policy verifier has its own signature/delegation adversarial tests and WASM build.

`python3 scripts/root_admission.py` separately terminates a fixed-key test host
after publication/receipt SQL but before COMMIT (exit80), and after COMMIT before
response (exit81). New processes verify rollback or exact durable retry, stable
head/event/receipt counts, same-nonce body conflict rejection and private-egress
rollback. The before-COMMIT observer only exists under `recovery-testing`; no
environment variable activates it. This proves these local SQLite process-death
boundaries, not a production transport or arbitrary network exactly-once guarantee.
