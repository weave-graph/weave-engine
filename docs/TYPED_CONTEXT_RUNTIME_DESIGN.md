# Typed context runtime profile B — proposed boundary

Status: design for independent review after protocol 0.13. No protocol number or implemented capability is claimed. Companion portable profile A is language commit ed0125f. This preserves the mandatory richer context/governance scope in the implementation plans.

## Exact source interpretation

`GraphExpression::TypedContext { input, reference: GraphRef, expected_schema: ContextSchema }` reads an explicit exact context snapshot under the current host. The schema is the complete canonical descriptor, never a policy installation or an authority token. Reads do not write a global schema registry. Compilation and composition reject one schema label associated with different complete descriptors; interpreting an arbitrary property does not reserve a global name.

The first profile requires an explicit-profile canonical assertion `definition` on one self-edge whose endpoint is the context anchor, with predicate `weave:context:definition`. It must be positive, context-free, and have an unbounded valid interval. Its assertion properties contain `weave.context: ContextDefinition`. Other assertions using the definition predicate are unsupported, including negative alternatives. This avoids silently resolving a contested descriptor or giving empty support results an undeclared descriptor clock. These are implementation choices, not paper-prescribed syntax. Ordinary readers, structural restrictions, node/assertion dependencies and current policy apply before interpretation. Missing, denied, malformed or unsupported definitions return the same typed-context-unavailable diagnostic.

The actual definition must validate and match the complete expected canonical schema. Equal axis assignments at different GraphRefs remain different worlds. Schema labels and axis values grant no access or real-world endorsement. Existing Default/Pinned selection remains an explicit untyped profile.

## Proposed persistable carrier

A result-envelope-only witness is insufficient: `Commit { data: result.graph }` would lose type meaning, and an empty typed value could lose its private descriptor dependency through union. Propose this additive field on GraphData, omitted when absent:

```rust
context_typing: Option<ContextTyping>

struct ContextTyping {
    selected: Option<GraphRef>,
    witnesses: Vec<TypedContextWitness>,
}
struct TypedContextWitness {
    context: GraphRef,
    schema: ContextSchema,
    definition: AssertionRef,
    anchor_nodes: Vec<NodeRef>,
}
```

The full schema permits independent roundtrip validation and composition conflict detection. Its fingerprint is derived canonically, not separately caller-authoritative. At most 32 unique context witnesses; each descriptor is at most 64 KiB; all bytes count toward existing graph/result budgets before cloning. Witness ordering/dedup is canonical, and one context pin cannot acquire conflicting schema or definition bindings. The canonical descriptor assertion and its endpoint pins must match the witness on resolution. `selected` must reference one retained witness. GraphData carries the persisted marker; QueryResult.selected_context remains the existing active exact scope and must agree when the type marker is selected. No duplicate independently mutable result witness field is needed.

The carrier is a conservative AND influence boundary over the whole graph value, including an empty value. Its contents are ordinary restricted data, not public graph-level schema metadata. Query must authorize and validate all witnesses before returning any carrier or content. If a witness cannot be resolved under current authority, withhold the value and carrier with generic partial coverage; do not expose schema labels, counts or private pins. Capsule export and accepted import require full authorization, as with other proof dependencies. Raw plans may serialize references but cannot manufacture a validated runtime witness; validation of the descriptor and gates is repeated at consumption. A trusted local host can still author arbitrary source claims, as in the existing provenance model.

This whole-value restriction is intentionally conservative. Union with one private typed input can restrict an otherwise public output. Future per-alternative influence optimization must prove equivalent privacy before relaxing it. It is not implicit context conjunction or evidence endorsement.

## Propagation rules

- TypedContext validates once in the operation snapshot, applies existing exact selection, and adds the witness. Empty results retain it. It injects descriptor assertion and anchor-node gates into scoped derived records; original source identities remain unchanged.
- Query, Filter, Project and persistence retain the carrier. A query of a persisted selected typed value restores the active scope after validating that materialized claims/attachments have compatible qualifiers. Unqualified representation nodes remain allowed.
- Union retains the canonical union of all witnesses even when selected markers differ and clear. Same-label/different-descriptor conflict rejects. Diff and Join retain both input influence sets; existing exact context compatibility remains mandatory. An untyped value is never advertised as typed merely by sharing a pin.
- Support, Explain, Rules, Geometry and future aggregates must include every retained witness in generated node proof gates. Every generated assertion alternative carries the relevant descriptor premise; node endpoints carry anchor dependencies. Unknown support with zero source nodes/edges remains dependent on its descriptor.
- Metadata traversal retains the originating witness influence but does not transfer its selected type to independently qualified target claims. Typed selection of that target is explicit. Separate direct reads remain independently authorized.
- Accepted-identity and clustering services read validated source values and retain their witness sets in reusable outputs. They do not infer a typed descriptor from a bare context label.
- Cache identities include the complete carrier. Current read/view/transition/dispatch/cached-admission checks resolve its proof closure; stale data does not mean stale authorization. Signed traversal scope checks, capsule closure and missing-reference budgets include witness assertion/anchor references.

## Required review and acceptance

Before code: root and language owners approve the whole-value privacy/false-denial tradeoff and exact DTO. Shared wire is a new explicit version; prior profiles reject any carrier/operator recursively before writes. Existing graph bytes remain unchanged when field absent.

Tests must cover strict descriptor validation, label conflicts, equal assignments under distinct pins, private and missing descriptors, empty typed selection -> mixed union -> unknown support, saved reader-cleared output, query of saved empty typed graph, contextual metadata, geometry/identity/cluster reuse, current-policy revocation in views/streams/receipts, exact snapshot pinning, capsule closure and old-profile atomic rollback. Native end-to-end source examples and independent root tests are required. Portable validation/WASM compilation does not prove a browser runtime.

Still open: compatibility mappings, explicit broadcast, governed world crossing, hypothetical assumptions, branch masks, reference axes, richer function typing, complete-scope absence and general aggregation. This profile does not close L18 or E11.
