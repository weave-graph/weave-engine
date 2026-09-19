# Contract 0.14.0: typed context definitions and persisted influence

This additive profile interprets exact context snapshots with a complete nominal axis schema. It does not equate independently authored contexts or grant authority from axis labels. Existing Default/Pinned selection remains an untyped exact-context profile.

`GraphExpression::TypedContext { input, reference, expected_schema }` uses JSON kind `typed_context`. The expected schema is `{reference:{id,revision},axes:{name:axis_type}}`. Axis types are tagged objects with `kind`: `boolean`, `integer`, `string`, `decimal`, `quantity` (with exact `unit` descriptor), or `enum` (with distinct `members`). Assignments are total; missing axes, extra axes, null, Float conversion and wildcards are unsupported. Signed-i64 and exact Decimal/Quantity semantics remain unchanged. At most 32 axes and 128 members per enum; a descriptor is bounded to 64 KiB, with duplicate-free decoding even when embedded in a larger plan.

The pinned source must contain a visible explicit assertion `definition` on a self-edge with predicate `weave:context:definition`. It is positive, context-free and valid from i64::MIN with no end. Its properties contain `weave.context: {schema,values}`. Unsupported additional definition assertions, missing/malformed/denied definitions and mismatched expected schemas fail generically with `E_CONTEXT_UNAVAILABLE`. This fixed immutable-descriptor convention is an implementation choice, not prescribed source-paper syntax or an endorsement of the assignment. The complete expected schema is compared canonically. Queries never install a global schema authority; compilation and composition reject conflicting use of one schema label.

GraphData gains optional `context_typing`, omitted in earlier graph encodings:

```json
{
  "selected": {"graph_id":"World","revision":"pin"},
  "witnesses": [{
    "context":{"graph_id":"World","revision":"pin"},
    "schema":{"reference":{"id":"WorldSchema","revision":"1"},"axes":{"scenario":{"kind":"string"}}},
    "definition":{"graph_id":"World","revision":"pin","assertion_id":"definition"},
    "anchor_nodes":[{"graph_id":"World","revision":"pin","node_id":"anchor"}]
  }]
}
```

At most 32 witnesses and 16 anchor references per witness are accepted structurally; this descriptor profile resolves exactly its single self-edge endpoint. `selected` is nullable and may clear during mixed composition while all witness influences remain. Every witness is re-resolved under current host authority before consumption. A raw serialized carrier is not evidence of successful validation. Selected carriers require compatible record qualifiers; same-context witness conflicts and same-label schema conflicts reject. An unresolved or forged carrier on a stored graph is withheld with generic partial coverage, exposing no carrier, selected type, schema labels or witness count.

The carrier is a conservative AND authorization boundary on the whole graph value, including empty values. Query, persistence, algebra and missing metadata results preserve that influence. Mixed union cannot turn an empty private typed context into public unknown support. Generated Support/Explain/Rules/Join/Geometry/cluster/identity records additionally retain descriptor assertion and anchor-node gates. These gates remain effective after the carrier and reader lists are removed from a copied scalar value. Original source records retain their original identities; navigation through a typed host does not relabel an independently read target.

The complete carrier participates in result identity, capsule/reference closure, current-authority view/stream checks and signed-read scope/retry checks. Broad whole-value influence can cause conservative denial when a mixed result includes a private typed input; no per-alternative release optimization is claimed. Bare pins do not implicitly become typed in the native services. Explicit typing and compatible source qualifiers remain required.

All protocol 0.1–0.13 features retain their previous meaning. Earlier versions reject typed-context expressions recursively and graph carriers before writes. Native tests cover generic unavailable behavior, private empty values, mixed union, missing metadata, support/explanation/geometry nodes copied without carrier/readers, saved empty typed values, forged witnesses, qualifier rejection, capsule closure, current-policy revocation without head movement, signed scope narrowing and atomic old-profile rollback. Portable tests cover exact assignments, canonicalization, embedded duplicate rejection and carrier composition.

Still open: compatibility/broadcast mappings, governed world crossing, hypothetical assumptions and branch masks, object-reference axes, richer context/function types and complete-scope absence. This profile does not complete the language or runtime roadmap.
