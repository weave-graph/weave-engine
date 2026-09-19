# Snapshot capsules and local branches

The public Rust API provides `export_capsule`, `receive_capsule`, `accept_revision` and `fork_branch`. These are trusted-host operations; they are not network endpoints or language effects.

A `weave-capsule-0.1` contains a root reference, immutable whole-snapshot revision records and an explicit list of external dependencies. Export follows permitted ancestry, metadata graphs and derivation dependencies within depth/size budgets. It never exports a filtered graph under an unchanged content digest. A root that cannot be exported intact fails with a generic unavailable error; unavailable/denied dependencies stay external.

Receive validates the format, graph structure, each revision digest, root membership and dependency manifest before an atomic insert. Identical reception is idempotent. The host must grant storage authority for each graph. Reception does not update branch heads or publish events: a received revision is queryable only by an explicit pinned reference until acceptance. This is a local quarantine distinction, not a complete distributed trust protocol.

Acceptance requires an explicit graph write grant and compare-and-swap against the destination branch head. It records a durable `graph.accepted` event with the head update. Fork creates a new branch at an existing revision and rejects if the branch already exists. Subsequent offline commits use the fork head as their expected parent; the original branch remains unchanged. Acceptance chooses a complete revision; it does not perform domain conflict resolution or collective governance. Competing immutable revisions remain retained.

Capsule hashes provide content integrity, **not signer identity, provenance truth or peer authenticity**. Capsules are currently unsigned. Source system timestamps are not imported as trusted local recording time: receipt records local host time, while source-clock reconstruction is not yet implemented. Logical batch snapshots separate revision names from content digests, so cyclic metadata can be transported with a complete manifest; see [ADR 0001](architecture/ADR-0001-revision-identity-and-content.md).

Limits: 1000 included revisions, ancestry/dependency traversal depth 32, 16 MiB serialized capsule. Tests cover persisted offline edits, acceptance separation, repeated receipt, altered content, omitted dependency manifests and denied acceptance. Transport, encrypted replicas, signed capabilities, actual mobile storage, attachment mounts and governed synchronization remain open.

## Logical snapshots: capsule 0.2

An export containing logical snapshots uses `weave-capsule-0.2` and includes the complete canonical `SnapshotManifest` for each batch. Every member must be included, authorized, and bound to its content digest. This permits atomic metadata cycles without requiring cyclic content hashes. Legacy content-addressed snapshots remain valid and use capsule 0.1 when no logical manifest is present.

Receipt verifies manifest membership, ordering, content bindings, dependency declarations and included ancestry before inserting anything. Existing batch/revision identities cannot be rebound to different data. Manifest records, integrity bindings, structural registries and revisions are inserted in one transaction. Reception is idempotent and leaves every branch head and event stream unchanged. Acceptance additionally checks required metadata availability and authorization.

Whole manifests may disclose membership, so export refuses a logical root if any batch member is unavailable to the caller. It does not send a partial manifest or hidden-member hashes. This conservative rule trades export availability for privacy. Selective proofs are not implemented. Live metadata handles also require a future pinned export context and currently fail explicitly.

The independent `root_capsule` tests cover a two-graph cycle transferred between engines, exact pinned-query equivalence, quarantine and explicit acceptance, duplicate receipt, hidden batch membership, tampered content, incomplete membership, and immutable identity conflicts. Hash integrity remains unsigned; this does not establish peer authenticity, network delivery, encryption, or governed acceptance.
