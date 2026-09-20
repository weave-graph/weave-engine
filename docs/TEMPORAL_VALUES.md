# Portable temporal values and conditional proof carriers

The protocol 0.19 portable implementation computes over already-authorized
`QueryResult` values. Declared references restrict visibility; pure code does not
resolve a reference or grant authority. Native authorization, persistence and
transport validation must enforce these fields before exposing results.

`Window` clips assertion occurrences to a nonempty half-open interval. Untimed
original nodes remain structure. Modified edges and metadata receive new IDs and
source proofs; original immutable payloads are not rewritten. After full input,
context and provenance validation, a window that changes no edge or attachment
interval retains all record IDs and payloads. Reapplying the same window is
therefore idempotent. This identity case keeps whole-value restrictions (including
an empty Union operand) and promoted declared attachment gates; subsequent generated
scalars inherit them. Like Filter/Project, it does not rewrite an unchanged public
original record merely to copy the value envelope onto that record. Newly clipped
wrappers carry their derived restrictions when detached. `Sequence` matches
positive occurrences through equal entity/space at the left target and right
source. It tests original intervals before clipping: `before` is strict,
`meets` is endpoint equality, `overlaps` is symmetric nonempty intersection and
`within` is inclusion (including equal intervals). Both occurrences must intersect
the window. Outputs retain two separate clipped occurrences; there is no hull fact
covering their temporal gap. Selection details and role remain in derivation
parameters. Metadata remains directly navigable using returned wrapper IDs;
derived proof references do not create aliases for original local IDs.

Record and whole-value carriers mean flat assertion/node/snapshot gates AND an OR
of nonempty derivation groups. Each group has explicit assertion, node and whole
snapshot premises. `input_snapshots` remains descriptive. Empty group lists retain
the old no-additional-gate meaning; new individual empty groups are invalid. Old
Edge/Assertion groups without snapshot premises keep their historical validators.
Node and attachment flat assertion fields remain global AND gates; an edge's
`derived_from` is a descriptive flat index when derivations exist.

`carrier_algebra::RefSet` is nonrecursive. `from_parts` precharges borrowed inputs;
`conjunction` performs a checked bounded product and preserves explanations.
`into_influence` supplies the canonical value envelope. One `Budget` spans a
composition. Limits are 128 alternatives and 1,000 raw combined authorization
references per carrier, plus shared work and byte ceilings (32 MiB maximum).
Canonicalization only changes newly constructed results. Same-value restricted
`disjunction` retains both traces. Unconditional plus restricted disjunction
returns `E_INFLUENCE_DISJUNCTION_PROFILE`; it does not silently drop a trace.
Ordinary graph Union uses conjunction, including on empty graph envelopes.

New node/attachment alternatives are not globally promoted by `input_influence`.
Existing declared flat attachment gates and whole snapshot gates retain their
conservative meaning even when a semantic filter removes every record. An actual
metadata path must explicitly conjoin the selected attachment's exact origin,
record carrier and host restrictions into the target envelope, including an empty
target. Generated scalar/graph records copy that envelope. Record-local groups
survive detached repersistence. Support, finite rules and Explain preserve explicit
snapshot alternatives; Explain renders snapshot premises as typed graph-pin
records. Different snapshot-only rule supports have distinct proof keys.

Portable evidence includes an independent 1,808-case Boolean truth table, temporal
finite-set/boundary checks, pair-conditioned detached records, all metadata host
kinds, projection/union composition, schema validation, empty OR-to-scalar flow,
rule idempotence, typed explanation snapshot records and failed-protection rollback.
Native and actual compiler/host acceptance are separate gates; passing these pure
tests is not a completed native protocol release or full recorded-time support.
