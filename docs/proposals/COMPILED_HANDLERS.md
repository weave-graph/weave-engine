# Compiled handler registration and durable preparation

Design for review, based on native `1eb33c0` and the language `REACTOR_INTERFACE.md` / `REACTOR_LOWERING_PREPARATION.md` proposals. No handler DTO, protocol version, storage change or executable behavior is introduced by this document. The proposed next boundary is protocol **0.18**, store marker **16**, after the paired 0.17 snapshot carrier publishes. Root has reviewed the seams below; the complete interface still requires approval before shared edits.

## Canonical interface proposed for freeze

New `weave-contract::handler_registration`, using existing `GraphExpression`, `SourceRevision` and diagnostic types:

```rust
struct CompiledHandlerTemplate {
    format: String,              // weave-handler-registration/1
    protocol: String,            // 0.18.0 for this initial artifact profile
    name: String,
    revision: String,
    input: HandlerInput,
    event_types: Vec<HandlerEventType>,
    recipe: HandlerRecipe,
    output_slot: String,
    source_revisions: Vec<SourceRevision>,
    definition_digest: String,
}
struct HandlerInput {
    graph_id: String,
    branch_id: String,
    metadata_depth: u32,         // 0 means no expansion; 1..=8 means bounded preload
}
enum HandlerEventType { GraphAccepted, GraphCommitted }
struct HandlerRecipe { bindings: Vec<HandlerBinding>, output: String }
struct HandlerBinding { name: String, value: GraphExpression }
```

Enums serialize exactly `graph.accepted` and `graph.committed`. The canonical vector is exactly `[GraphAccepted, GraphCommitted]`; no selective matching or silent dropping is introduced. Strict unknown-field rejection applies throughout. New Program expression variants are unnecessary. The compiler emits an inert artifact; raw Programs never install a handler.

Public helpers: `seal_handler_template(CompiledHandlerTemplate)`, `validate_handler_template(&CompiledHandlerTemplate)`, `handler_definition_digest(&CompiledHandlerTemplate)`, `handler_source_revision(&CompiledHandlerTemplate)`, and `validate_handler_recipe(&HandlerRecipe)`, each returning the sealed value, digest, source revision or `Result<(), Diagnostic>` as appropriate. Own source identity uses a framed handler-specific name namespace and a separate recipe/configuration hash domain. Definition identity covers the complete normalized artifact and canonical conflict-checked source manifest, excluding only `definition_digest`. No old view/Program fingerprints change. Fixed vectors establish framing and alias/whitespace versus imported-byte identity semantics.

The sole predefined binding is `$event`; authored identifiers cannot name it. Each binding name is unique, references only `$event` or prior bindings, and the output names an available graph binding. Validate **all** bindings, even unused ones, and every nested operand. Allow Reference, Filter, Join, Union, Diff, Project, Support, Context, Metadata, Reason, Explain, Counterparts and Geometry. Reject Query, TypedContext, AcceptedGraph, CurrentView, ResolveIdentity and Cluster; future variants remain rejected until reviewed. Metadata only selects already materialized input. Rule/function/schema restrictions remain unchanged.

Bounds before cloning: artifact1MiB, 1,000 source revisions, 256 bindings, 1,000 total expression nodes across the recipe, expression depth32, identifier512 bytes, metadata depth8. The compiler additionally caps16 combined view/handler artifacts and4MiB aggregate output. Native validation does not rely on compiler validation. Native recipe evaluation uses one shared expression work budget and cumulative32MiB materialized binding budget, charging known reference copies before retaining cloned values; no resetting budget per binding.

## Native host API and immutable installation

```rust
struct HandlerOutputBinding { slot: String, graph_id: String, branch_id: String }
struct PreparedHandlerReceipt {
    duplicate: bool,
    preparation_id: String,
    definition_digest: String,
}
Engine::install_compiled_handler(&self, manifest: &AdapterManifest,
    template: &CompiledHandlerTemplate, output: &HandlerOutputBinding,
    authority: &HostContext) -> Result<()>;
Engine::prepare_compiled_handler(&mut self, adapter: &str, event: &str,
    lease: &str) -> Result<PreparedHandlerReceipt>;
Engine::complete_prepared_handler(&mut self, adapter: &str, event: &str,
    lease: &str, preparation_id: &str) -> Result<HandlerReceipt>;
```

`HandlerOutputBinding` is trusted installation configuration, not a plan authority object. Receipts expose no graph payload, dependency inventory, private count or global checkpoint. These are local embedding-host APIs, not remote principal authentication endpoints.

Installation is atomic with the existing adapter row. Require exact manifest digest = sealed template digest, principal = authority principal, exactly one subscription matching the template graph/branch, exactly one output graph matching the explicitly mapped slot, output write authority, no effect destinations, and `projection_replay=true`. Reject subscribing to the configured output graph/branch. A template string never chooses a native destination or grants write access. Input existence/read permission is checked at real event time, so deployment before the first input revision remains possible.

Reinstallation is exact-byte/config idempotence. A preexisting legacy adapter identity cannot be adopted as compiled, even if its manifest happens to match: its checkpoints/receipts may describe caller-built Programs. Public `complete_handler` must reject compiled IDs before receipt reuse or execution. The compiled bridge alone calls a private shared completion core inside its already established transaction. Raw `install_adapter` cannot alter an existing compiled binding. Removed identities cannot restart; paused adapters cannot prepare/complete/replay. Draining permits only the already pending occurrence. Effect APIs remain unavailable through the empty destination list and projection flag.

## Preparation captures one immutable occurrence

Start `BEGIN IMMEDIATE`, reserve the writer, and capture one trusted operation clock/read/proof scope. Validate installation, lifecycle and the real current lease using existing dispatcher state. Re-read event identity/type/graph/branch from the durable event table and verify current whole-source authorization. Do not trust a caller DispatchEnvelope. Current event type derivation is the existing `accept:` occurrence classification; the compiled bridge uses the same authenticated local event row, not a caller event string. Both supported types are accepted; no MetaGraphRebound alias is fabricated.

For a fresh preparation, query the exact event revision without predicate/time filtering and optionally preload metadata to the declared depth. Require complete authorized coverage and independently apply whole-snapshot authorization to every resolved pin before evaluation; ordinary Query coverage alone does not prove absence of reader-filtered records. Reject missing/denied/depth-truncated closure and all live graph handles in the raw event/resolved records. No latest head or equal replacement graph may substitute for a pinned dependency. A query may transiently resolve a live handle while assembling input, but preparation rejects before persisting or publishing any result; the eventual implementation should reject detected live handles as early as practical.

Build the whole-input gate from the actual event pin and exact `ResolvedGraph.reference` values produced by the engine's preload. These are authenticated operation data, not a caller/descriptive `input_snapshots` vector. Preloaded closure is conservatively consumed as a whole input, including empty snapshots and inputs unused by the final recipe branch. Preserve all preexisting carriers too. Snapshot gates require current whole visibility; they do not invent an ACL on ordinary empty public snapshots. Initialize `$event` with that protected graph value and canonical artifact source manifests; merge source identities before any identity hash or stored body is finalized.

Evaluate only the validated pure recipe. Intermediate failures or source-label conflicts abort preparation without changing dispatch checkpoint/lease state. A partial or empty derived result is allowed if the input closure itself was complete; it retains the whole-input gate. Capture the configured output head once in the same transaction. Existing output must be currently wholly authorized before its revision can enter the captured CAS; empty/nonexistent output is allowed. A stale captured CAS is never refreshed during retry.

## Owned output and descriptive attribution

Output is a new owned graph snapshot, not a QueryResult pretending changed payloads have original origins. Deterministically namespace local node, edge, structural-edge, assertion and attachment IDs using a domain-separated artifact+event digest and original local ID/kind. Remap only local endpoints, `Assertion.edge_id` and Node/Edge/Assertion attachment hosts. Preserve entity IDs, space IDs, exact historical proof references, contexts, artifact URIs and property strings. Validate that remapping is collision-free and bounded; never treat arbitrary property strings as identifiers. This prevents immutable output structural registries colliding when successive event results change derived wrapper endpoints.

Apply installed-principal readers and all existing plus whole-input gates to every generated record and the whole graph, including empty output. The canonical helper currently protects the materialized legacy profile; explicit inputs are materialized before recipe execution, and the bridge rejects an unexpected unmaterialized profile rather than silently dropping records. Each record/carrier must remain within999 combined references before a subsequent materialization self-pin; attachment origins count too. Preserve OR groups rather than flattening their meaning.

`GraphData` has no source manifest field. Persist a generated graph-host literal attachment with explicit format `weave-handler-attribution/1`, template digest and canonical source revisions. Give it a collision-checked generated ID, installed readers, the output's explicit context where present, and all three influence kinds. It counts toward byte/reference limits. This is ordinary descriptive metadata: it does not authenticate arbitrary same-key payloads and does not manufacture `QueryResult.source_revisions` when later queried. The trusted installed template and preparation remain the execution binding.

Construct exactly one stored Commit Program targeting the installed graph/branch and captured expected head. Preserve its exact canonical bytes and source manifest. There is no caller Program or caller QueryResult parameter.

## Durable binding and current reauthorization

Proposed private tables: `compiled_handlers(adapter PRIMARY KEY REFERENCES dispatch_adapters, principal, template, output_binding, binding_digest)` and `handler_preparations(adapter, event_id, principal, preparation_id UNIQUE, body, body_digest, PRIMARY KEY(adapter,event_id))`. Preparation IDs are opaque durable random occurrence IDs, not hashes of hidden inputs. The internal canonical body contains a format tag, adapter/event/source identity, template and config binding, output mapping/CAS, canonical manifests, exact input closure, prepared-at time and exact Program. Prepared-at time is descriptive only; it never supplies current authority.

Validate canonical body digest and cross-field consistency on every load. Reconstruct the event/preload closure from immutable raw references at its declared depth and compare it to the stored closure. Check that every input pin remains in the output whole carrier and all generated record gates. A trimmed stored vector is not an authority shortcut. Revalidate every current input snapshot and the complete stored output proof closure, including protected governance callbacks, under the current operation clock. Ordinary trusted SQLite-administrator rewrites of all data/checksums are outside this integrity boundary; source graphs/plans cannot write the private tables.

Before completion, verify registration/lifecycle, opaque preparation binding, current event+input+output authority, and immutable Program integrity. If no handler receipt exists, require the current lease, then execute the exact stored single Commit through the private completion core. Output graph/events, existing handler receipt/checkpoint and pending-delivery removal commit atomically. A CAS conflict leaves all of them untouched; the preparation remains available with its original CAS.

After successful completion, duplicate calls may return the historical receipt without a live lease only after all current closure/lifecycle checks. They never recompute a recipe, rebase output, or promise source freshness. A renewed lease for the same pending occurrence reuses the same preparation bytes and ID. `prepare` on an existing unfinished preparation still requires a current lease; after a verified completion receipt exists it may recover the original preparation receipt under current authorization. Its `duplicate` flag means already prepared, not necessarily completed.

Existing explicit failure/dead-letter handling applies; neither replay nor failure changes the prepared CAS. A stale CAS can therefore remain a terminal dead letter requiring an explicit new adapter/deployment policy. Automatic rebasing, skipping, dead-letter cancellation and general retention/garbage collection are outside this bounded stage and must not be implied by retry support.

## Bounds, migration and executable acceptance

Before SQL extraction/JSON decoding, charge bounded row bytes and read calls to the existing operation budget. Manifest reads used by this path must gain the same bounded extraction; current `dispatch_manifest` reads unbounded text and cannot be used unchanged. Proposed limits:128 installed compiled handlers and16MiB total registration bytes per principal;256 retained preparations and64MiB total preparation bytes per principal;20MiB per complete preparation body, of which Program is at most16MiB. Existing32MiB cumulative materialization and128MiB/4,096-read operation limits remain. Count/byte quotas are atomic, include repeated installs/preparations, and cannot be evaded by new adapter IDs. Completed preparations remain retained for exact reauthorization/replay; at quota the host receives explicit backpressure, not silent deletion.

Store marker16 is required because older marker15 runtimes could otherwise complete a compiled adapter through their unbound caller-Program API while ignoring the new registry. Migration creates both tables and marker atomically, preserves old snapshots/templates/adapters/receipts, and old binaries refuse the upgraded store. No legacy row is automatically backfilled as compiled. Source artifact protocol0.18 is proposed; snapshot carriers keep their0.17 semantics and old supported Programs/capsules stay byte-compatible.

Tests must cover both event types, actual compiler Metadata→Reason output, identity/empty input and empty metadata, partial/constant outputs, source attribution and all3 movable gates, stripped readers/carrier persistence, hidden/missing/live inputs, current policy expiry between steps and on duplicate replay, recursive banned operators, forged artifact/event/lease/preparation, raw-completion bypass attempts, immutable renewed-lease bytes, stale CAS atomicity, per-record overflow and stored quota/corruption, lifecycle changes, and declaration/module identity. Controlled deaths before/after preparation commit and before/after completion commit prove restart/replay; observer unwinds must roll back same-Engine state. Migration uses a real populated0.17 baseline. The source fixture owns actual emitted artifacts, not handwritten approximations.

Native implementation remains one module plus narrow dispatcher factoring, private schema initialization, and canonical helper exports. Language owns parser/function/module/complete-artifact lowering and source acceptance; parent owns independent adversarial tests and publication. No external effects, sandboxing, declassification, semantic merge, automatic graph release or complete reactive-paper conformance is claimed.
