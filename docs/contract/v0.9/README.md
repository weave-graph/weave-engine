# Contract 0.9.0: geometry from graph assertions

This additive profile introduces `Geometry`, `Explain`, finite `Float` schema properties and conservative node dependencies. Versions 0.1–0.8 remain accepted, but these operators, Float schemas and nonempty node dependency fields require 0.9, including nested expressions and atomic batches.

## Authorized geometry operands

A `GeometryOperand` selects `assertion_id` from an evaluated graph expression. The assertion must be visible, positive and valid at the explicit Unix-millisecond `valid_at`. Its `assertion_properties["weave.geometry"]` contains a tagged domain value. This is ordinary claimed source data, never caller-supplied execution authority. The runtime constructs the portable geometry library's Evidence wrapper from authorized assertion origins, intervals and the trusted principal.

Coordinate payloads use `kind: "coordinates"`, an exact `space` descriptor, `role` (position, direction or embedding) and finite numeric `values`. A space has an `id`, immutable `revision` and `geometry`: physical3d with exact frame/unit, or embedding with encoder, preprocessing, dimensions and metric. Coordinate `space.id` must equal the assertion's FROM manifestation's `space_id`. Rigid-transform payloads use `kind: "rigid_transform"`, `from`, `to`, a 3×3 rotation and translation. Their descriptor IDs must match the assertion's FROM and TO manifestation spaces respectively. Descriptor and encoder claims retain their source lineage; no independent attestation is implied.

`GeometryOperation::Distance` takes left/right operands, `Transform` takes input/mapping, and `ProjectAxes` takes input, three axes and a pinned projection revision. Physical unit/frame mismatches and incompatible embedding encoders or preprocessing reject. Transforms are explicit geometry mappings, not entity counterpart assertions. Display projections have a distinct `navigation_projection` kind and cannot be reused as metric coordinates.

## Reusable results

Each operation returns a typed graph with node `result` and assertion edge `value`. The fixed `weave:geometry-result` schema revision 1 declares the node's kind, approximate flag and nullable finite Float value. The typed domain payload remains on `value.assertion_properties["weave.geometry"]`. A transformed coordinate result can be another operand. The result entity ID identifies a computation value; it does not assert identity equivalence with a source entity.

Results preserve exact interval intersections, input revision pins, source manifests, independent derivation alternatives and partial coverage. Operands require compatible explicit context selection, which remains on the result's claim and scoped node. All output objects are principal restricted. Numeric calculations use finite IEEE 754 binary64 with documented tolerance; they do not promise exact decimal arithmetic or bitwise cross-platform identity. Only navigation projection is flagged approximate in the domain sense.

`Explain { input }` exposes the existing bounded portable graph-valued explanation, retaining distinct alternative and joint-premise groups. It cannot fetch unauthorized evidence. Geometry and Explain are pure: no graph commit, event or external effect is hidden in evaluation. Live views clock geometry explicitly; an unavailable operand rejects refresh and leaves the previous view stale under the existing view failure policy.

## Persisted node dependencies

`Node.derived_from` is a bounded list of exact AssertionRefs. All dependencies must remain visible before the node is returned, independently of its mutable reader list. Geometry, support and explanation populate it for derived scalar/status nodes. Queries prune edges and attachments whose nodes are withheld. Recursive premise checks also enforce source endpoint-node dependencies, preventing an incident edge from bypassing a protected value. Missing, cyclic or budget-exhausted dependencies withhold the result with generic partial coverage. Capsule and signed-admission closures include these pinned dependencies.

This node gate is deliberately conservative AND semantics: a node depending on alternative evidence may be withheld when only one alternative is readable. It does not silently grant on an incomplete alternative. Rich node derivation groups and explicit release policies remain future work. A trusted writer can fabricate new public claims; removing an entire proof is not something a local principal selector can authenticate. Signed publication continues to require every output object to retain its subject restriction.

## Acceptance and limits

Native signed-admission tests additionally require the proof scope for node-only scalar reads and cached retries. Capsule tests reject unauthorized node-only export and preserve the dependency through authorized round trips. Native geometry tests cover typed results, exact time, context, source pins, private premises, descriptor anchoring, malformed authority wrappers, old-profile rejection, rollback and repersistence. The companion language's `examples/geometry.weave` and `scripts/check_geometry.py` execute physical distance, transformed result reuse, embedding distance, display-only projection rejection, explanation and graph-function composition against the actual CLI.

Typed assertion-property schemas, full counterpart mapping, typed context axes, geometry services beyond this bounded profile, clustering integration and portable runtime storage remain open. Payload validation occurs when a geometry operand is consumed; merely storing a JSON property is not a geometry validation certificate.
