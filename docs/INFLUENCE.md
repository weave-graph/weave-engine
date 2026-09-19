# Influence implementation and verification

Protocol [0.15](contract/v0.15/README.md) separates graph membership from the dependencies that restrict reading a graph value. An empty result can still depend on a private context, node, attachment path or assertion. The serialized carrier adds restrictions; the host independently resolves every reference.

| Boundary | Implementation | Acceptance evidence |
| --- | --- | --- |
| Carrier and node-only groups | `weave-contract/src/influence.rs`, `algebra.rs`, `rules.rs`, `identity.rs` | `weave-contract/tests/influence.rs`: stable rule closure, distinct OR node sets, support/explanation gates, exact byte/profile checks |
| Query and direct lookup | `weave-engine/src/influence.rs`, `lib.rs`, `assertions.rs` | `tests/root_influence.rs`: denied empty carrier, structural/assertion reads, global AND plus OR alternatives, bounded logical cycles |
| Persistence and old profiles | program preflight and store marker 11 | `tests/root_influence.rs`: atomic old-profile rejection; `tests/root_storage.rs`: migration/marker compatibility |
| Capsule and peer read | shared dependency snapshots and current authorization | `tests/root_influence.rs`: empty carrier closure, receive without acceptance, peer reauthorization |
| Generated scalar/edge | Support/Explain/Geometry protection | `tests/geometry.rs`: both node-only copies and edges with public replacement endpoints after carrier/readers removal; `tests/influence_composition.rs`: node-only cluster membership with replaced public endpoints |
| Relationship membership | join endpoint wrappers | `tests/identity_acceptance.rs`: endpoint-only persistence, then current relationship policy revocation |
| Empty metadata | selected attachment path carried before target lookup | `tests/influence_composition.rs`: unavailable private path through unknown Support, persisted without carrier/readers |
| Cached current authority | common stored-result revalidation | `tests/identity_acceptance.rs`: empty influenced cached view/changes invalidated without head movement |

Conservative AND gates can deny a complete summary when one supporting alternative becomes unavailable. This is intentional until a release policy and more precise value-level alternatives are defined. The model does not prevent a trusted author from manually reauthoring facts; it protects engine-generated proof-carrying values and rejects serialized attempts to mint authority. Whole JSON storage also exposes the already documented physical cost of reading private content, without a timing/RSS noninterference claim.

Generated output IDs and proof origins distinguish wrapper values from unchanged source records. Node-source gates preserve exact graph-local manifestation identity; node references never masquerade as assertion references. The generic carrier does not use context descriptors as a substitute for governance decisions.

Local verification before independent freeze: 279 all-feature workspace tests passed on the merged public base, followed by the additional passing metadata-wrapper union regression; strict all-target/all-feature lint passed. The eight existing native/WASM fixtures executed identically (21,161 bytes). Actual old 0.14 binary refusal of a schema-11 store, all three migration process-death checks and valid fuzz seed execution passed. These are local checks, not hosted publication evidence.

Compatibility review required before publication: the older root metadata CLI check asserts the original local ID `C` after path extraction. Version 0.15 deliberately returns a path-qualified wrapper ID with the original exact NodeRef retained as a dependency; direct target queries still return `C`. The new native wrapper test proves original/wrapped union and repeated union stability. The independent historical assertion has not been edited here.
