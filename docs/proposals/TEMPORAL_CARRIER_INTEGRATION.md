# Temporal proof carrier integration review

Review of language checkpoint `9832d70`, `TEMPORAL_ATTACHMENT_PROOFS.md` and its
vendor-only `carrier_algebra.rs`, after facade A `c11694a`. This is an integration
proposal, not a reserved contract or an implemented native profile. Facade A stays
protocol0.18/store17; merge public pairing changes before a future canonical freeze.

The bounded Boolean algebra is suitable groundwork: flat gates remain AND,
nonempty alternatives are OR-of-AND, conjunction checks the product, and
same-value-only disjunction distributes each branch's flat gates. The independent
truth table is meaningful pure evidence, not evidence of native authorization or
metadata composition. No algebra correctness blocker was found in this review.
Unconditional-plus-restricted disjunction remains an explicit unsupported case;
do not implement it by dropping explanations or inventing an empty proof group.

## Proposed exact wire and compatibility boundary

Recommend jointly reserving **protocol0.19 / store18 / capsule0.4** for a coherent
restriction profile, followed by Window/Sequence in the same unpublished stage.
The store marker must advance even without a new table: old binaries must not read
objects while ignoring new authority-bearing groups.

All fields default empty and omit on serialization:

- `GraphInfluence.derivations: Vec<Derivation>`: additional whole-value alternatives.
- `Node.derivations: Vec<Derivation>`: record-local alternatives, including scalar
  and isolated generated nodes.
- `MetadataAttachment.derivations: Vec<Derivation>`: record-local alternatives.
- `Derivation.snapshot_premises: Vec<GraphRef>`: explicit whole-snapshot premises
  within that branch, including existing Edge/Assertion derivations.

The last field is necessary, not an optional optimization: OR between two
snapshot-only restrictions on the same empty value cannot be expressed with
AssertionRef/NodeRef leaves without fabricating an object. Descriptive
`input_snapshots` never substitute for it.

The prototype's `Carrier.flat: GraphInfluence` must become a nonrecursive internal
reference-set type when GraphInfluence itself gains groups. Canonical helpers must
consume the full carrier so a nested group cannot be accidentally ignored. Do not
add a second executable serialized Carrier alongside the canonical fields.

Preserve existing flat semantics. Node/attachment derived_from is global AND;
Edge/Assertion derived_from remains its existing legacy/global-or-descriptive-index
meaning depending on derivations. Their derived_nodes/derived_snapshots remain
existing global gates. A generic conversion cannot treat every derived_from field
identically. Attachment.origin and actual host/context visibility also remain
required outside its new branch groups.

New carriers use max128 raw groups and max1,000 aggregate gate references before
dedup, plus shared byte/work/depth budgets. For historical Edge/Assertion records
without new fields, retain their existing per-group/index validators. Apply the new
aggregate profile only to newly introduced carriers or explicit snapshot-premise
semantics; do not tighten old portable validators retroactively. New generated
output must be charged at its final serialized shape, including composed trace
parameters, rather than relying solely on a reference-name size approximation.

Recursive old-wire preflight must reject new operators/gates before any command
writes, using typed fields rather than scanning literal property keys. Capsule
format selection must examine actual new fields, not operator labels embedded in
ordinary provenance. Accept genuine old0.1/0.2/0.3 capsules; reject new gates when
claimed under old formats. Whole visibility comparison must not normalize away
pruned groups or changed parameters.

Explicit compatibility edits are necessary: view validation and signed-export
response validation currently allow `[VERSION, .17, .16]`, which would accidentally
drop .18 after the bump. Retain .16/.17/.18 explicitly. Handler validation currently
requires exactly .18; accept historical .18 artifacts with their original recipe
profile and digests, and use the new profile only for .19. Preserve old stored
Programs, preparations, receipts and signatures byte-for-byte; no backfill/reseal.

## Native authorization and propagation

A single checked branch evaluator should serve nodes, attachments, whole values
and existing edge/assertion groups under the existing operation-wide authorization
context. Preserve active-recursion versus completed-DAG behavior, protected
policy/source callbacks and shared read budgets. A failed branch cannot reset work
or manufacture support; a cyclic branch plus an independent valid branch can still
resolve within budget. A exhausted/denied nonempty group list must return a distinct
unavailable outcome, never an empty serialized list (which means no extra gate).

Authorize primary readers/hosts before promoting declared influences; hidden
unreadable extra records must not suppress public siblings or leak their pins.
Prune unauthorized alternatives from delivered copies and mark generic partial
coverage as appropriate. Never rewrite stored signed payloads. Query/direct
node/direct assertion/attachment reads must enforce the same branch semantics.
Whole-snapshot gates remain intentionally stronger: any missing/redacted branch
means that exact whole snapshot cannot satisfy the whole-profile guard or export.

`metadata_value` needs actual composition, not removal of its current origin gate.
Resolve selected attachment authority to an explicit alternative path, conjoin its
original attachment/host/context/global requirements and the incoming value carrier,
and preserve it before target lookup, including empty/missing targets. Graph-valued
metadata cycles remain navigable where already supported; mixed proof cycles deny.
Both reusable empty values and generated scalar nodes must retain the composed path.
Do not reinterpret descriptive attachment_origins as one mandatory union of leaves.

Source/engine agreed selector boundary: changed temporal wrappers and attachments
receive distinct local IDs. Metadata selects **current returned IDs**, as Project
does; no alias inference from arbitrary derived_nodes, and no widening of the
existing narrow original-node shorthand. Native fixtures may inspect returned IDs
before a second plan. Single-source fixtures use supported unambiguous graph/entity
hosts or explicit host-driven second selection. Document that an original local
node ID does not automatically identify a derived copy.

All consumers must be audited, not only Window/Sequence: Filter/Project,
Union/Diff, Join, Support/Explain, Reason, geometry/counterparts, context typing,
metadata traversal, generated attachments, identity/cluster output and repersistence.
Whole-value Union uses conjunction, not proof disjunction. Multiple consumed
record groups require bounded AND-of-OR composition. Existing declared attachment
flat fields continue promotion even when the attachment is discarded. New ordinary
record groups are not globally promoted merely because a filter drops that record;
actual path/scalar consumption must capture them explicitly.

Semantic dependency walkers include every new branch premise and snapshot gate:
ordinary capsule traversal, signed complete closure, admission scope closure,
governed effect contexts, protected governance sources, cached handler/preparation
and view results, source snapshots and Explain indexes. A current-policy retry
must reconstruct/authenticate closure rather than trust stored cache vectors.
Query descriptive pins include only authorized retained branch dependencies; they
remain indexes, not permission grants. Signed whole-capsule services remain
conservative and may deny a partially visible record which ordinary query can use.

## Ownership and staged freeze

1. Engine owns the initial canonical field/enum/version checkpoint: contract
   lib.rs/assertions.rs/influence.rs definitions, strict bounded field decoders,
   old-profile preflight, handler/view version policy and package/lock versions.
   Release shared files explicitly after this small checkpoint. No concurrent edits.
2. Language owns new pure carrier_algebra.rs/temporal.rs and agreed portable
   propagation modules/tests after field handoff. Adapt temporary representations;
   retain parameters/provenance and add actual node/attachment/empty-path examples.
   Source AST/parser/modules/formatter preparation can proceed before reservation,
   but executable lowering must remain unavailable until the coherent contract exists.
3. Engine owns native authorizers, metadata selection/path composition, generated
   native records, storage marker18, all closure walkers/capsule0.4/services and
   historical migration/recovery. The facade remains a thin caller of those semantics.
4. Parent owns independent native adversarial cases and old-binary acceptance.
   Public vendor pairing waits for portable **and** native closure, not just enum
   compilation or truth-table tests. No need to start combined-scenario B meanwhile.

Before integration freeze, require: A OR B combined with C; one branch revoked;
both revoked; empty metadata target followed by Support/Explain; every generated
record copied with envelope/readers removed; original metadata still navigable;
flat gate noninterference; mixed cycles/diamonds/budget recovery; denied alternatives
absent from output; group-product bounds; exact whole-export denial; cached/accepted/
effect reauthorization; old-wire atomic rejection and inert similarly named literals.

Use preserved .18/store17 binaries for a populated migration fixture containing
compiled handlers (pending and completed), views, signed export receipts and governed
effects. Test pre/post-commit death, exact historical replay, unknown-effect state
and old-binary refusal. Keep earlier .16/.17 objects in the fixture or compose the
existing historical scripts. Browser image restore must migrate then fence the new
image, while failed/ambiguous migration leaves only a complete prior/new generation.
Actual native/WASM pure parity and source→native persisted temporal results remain
required before claiming the new temporal profile; complete mobile/browser temporal
scenario coverage is a later host execution gate, not supplied by this design.
