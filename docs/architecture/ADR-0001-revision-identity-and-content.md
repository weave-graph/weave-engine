# ADR 0001: logical revision identity and content integrity

Status: current limitation recorded; future replacement design proposed, not implemented.

## Current checkpoint

Contracts 0.1–0.3 use a revision ID derived from the SHA-256 digest of typed graph data plus graph, branch and parent context. Entity and manifestation IDs remain logical IDs. Semantic edges can form cycles because their endpoints are stable node IDs.

Pinned metadata references contain revision IDs inside the hashed data. This supports references to existing revisions and shared metadata, but it **cannot construct a self-reference or mutually recursive metadata references at the same newly created revision** without an infeasible cryptographic fixed point. A bounded resolver or missing-reference diagnostic does not demonstrate support for constructible cyclic metadata. This part of R02 remains incomplete.

## Proposed next revision model

Allocate stable logical revision IDs before constructing an atomic snapshot manifest. Hash immutable blocks separately, and record the mapping from logical revision IDs to content digests in that manifest. Graph-level references target logical IDs; integrity verification checks the manifest/block mapping rather than requiring every semantic reference to be a content hash. The manifest itself has an independent digest and, when authenticating peers, a signature.

Same-transaction references can then resolve through the enclosing manifest. Cross-transaction pinned references target an earlier accepted manifest. Cycles remain in the logical graph while the storage dependency structure is acyclic. A malicious peer must not be able to bind the same logical revision ID to different content unnoticed; equivocation detection and acceptance policy are necessary.

## Compatibility and open decisions

Do not reinterpret existing `sha256:` IDs silently. Preserve their current decoding and validation. A new version must define logical revision namespaces, manifest ownership/signing, collision/equivocation handling, canonical encoding, atomic publication, and migration. Capsules built on the current checkpoint must explicitly call their revision ID a content-bound checkpoint ID and must not claim metadata-cycle support or signer authenticity from a hash alone.

The language/runtime contract change needs both project owners and golden cycle fixtures. Full paper reconciliation may refine this design before adoption. Implementing a casual revision alias without these integrity rules would introduce an identity-spoofing gap, so this ADR does not claim that the replacement is finished.
