# Metadata selector review of c51393b (protocol 0.15 candidate)

This is an independent read-only implementation review. No runtime, shared contract or language source changes are included. `scripts/root_metadata_selectors.py` is a failing compatibility regression against this candidate.

## Finding

The distinct wrapper identity is justified: metadata navigation changes node readers and proof gates, so it must not claim the changed payload is the unchanged pinned node through `node_origins`. The new wrapper IDs bind path-qualified payloads and let distinct paths coexist in union. Original NodeRefs remain dependency gates; they are not identity aliases.

However, the new node-ID rewrite also changes every node-host attachment selector. `crates/weave-engine/src/metadata.rs:357` hashes the node to `metadata-node:...`, updates edge endpoints and node attachment hosts, then clears immutable node origins. Candidate selection at `metadata.rs:43` still compares the requested host with the current attachment host exactly.

The source parser's `metadata_host` emits the literal `MetadataHost::Node` string, and lowering passes it directly into `GraphExpression::Metadata`. There is no dynamic returned-node-ID binding in source. Thus an unchanged source program that extracts B through A and then says `metadata CValue from BValue on node "b" key "next";` changes from Complete C on the 0.14 runtime to empty Partial `E_METADATA_UNAVAILABLE` on c51393b. The returned wrapper ID also depends on principal/path proof state, so hardcoding a hash is not a meaningful compatibility solution.

The reproduced source transaction has A/node a/attachment ab → B/node b/attachment bc → C/node c. The first metadata extraction succeeds on both engines; only the second source selector fails on 0.15. `scripts/root_metadata_selectors.py` reproduces this through the actual language CLI and engine process.

## Current compatibility boundary

- Direct node-host selection from an ordinary graph query still uses the original local ID and works.
- Current language examples `live_handles.weave` use first-hop node hosts; `metadata_cycle.weave` and `contexts.weave` use edge hosts. These source forms are not themselves affected by the node rewrite.
- Graph-host cycle navigation and edge/assertion-host nested selection do not depend on rewritten node IDs. Entity-host attachment selection remains exact entity-host matching; it is not a substitute for selecting a node-host attachment.
- Nested node-host metadata selection fails unless the caller first inspects and supplies the returned wrapper ID. This is usable through a host API but currently unavailable as a general source expression.
- Projection using original node IDs over a metadata result also changes behavior. The proposed narrow metadata selector compatibility fix does not claim a general original-ID projection alias. Algebra already namespaces outputs, and arbitrary derived-value projection remains keyed by returned local IDs.
- `root_metadata.py`'s graph-host cycle reaches the correct C entity and retains C's original NodeRef, but its literal output-node-ID assertion now fails. Updating that one assertion is justified only together with the nested-node source regression and stronger wrapper/proof checks; it does not establish compatibility by itself.

## Smallest coherent fix recommendation

Keep path-qualified wrapper IDs and empty immutable `node_origins`. For a Node selector that has no exact match, optionally resolve the original local ID only through an already-authorized exact target snapshot:

1. Require exactly one current target entry in `input.snapshots` and its exact matching `ResolvedGraph` in the already materialized metadata graphs. Do not read storage or infer an identity mapping.
2. In that snapshot, find original node-host attachment records whose host is the requested local node ID and whose key matches.
3. Match a current attachment by the same unchanged attachment ID and the engine-derived exact `AssertionRef { target graph, target revision, attachment ID }` in its attachment-origin envelope. Require its current host to name a present path-qualified wrapper with empty immutable node origins. Do not match arbitrary node proof references.
4. Exactly one candidate may resolve. Missing, duplicate or conflicted candidates return the existing generic partial diagnostic. Exact returned-ID selection retains precedence. Do not fall back when an exact selector is already ambiguous.

This is a compatibility shorthand for direct metadata materialization, not a new identity rule. It does not alias union-remapped attachment IDs, independently saved wrappers or arbitrary source proof nodes. Those values still use their returned local IDs. If broader source-addressability is required, it needs an explicit distinct source-object selector contract with an authoritative source role, rather than treating a generic proof list as aliases.

## Regression evidence required

The new source regression includes original-ID two-hop navigation, exact C source dependency, both path assertion gates, empty immutable origins, different path wrappers, repeated-path stability, self-union idempotence, graph-host cycle closure, and persistence of only a private wrapper node with reader lists removed. The last case must remain invisible to an outsider.

Add native negative tests for duplicate/ambiguous original-host candidates, no fallback from arbitrary `derived_nodes`, wrong target revision, and saved/union-remapped wrappers. Also cover nested edge/assertion hosts so the fallback does not change their semantics. The current script fails at its first nested-navigation assertion on c51393b, as expected; it must pass only after a coherent fix, not after removing the assertion.

## Extended projection/selector matrix

`scripts/root_metadata_compatibility.py --baseline PATH_TO_014 --candidate PATH_TO_015` executes the same 0.14 input shape on both runtimes. It includes explicit assertions and node-, structural-edge-, assertion-, graph- and entity-host metadata attachments. On c51393b, all baseline cases and all candidate graph/edge/assertion/entity cases pass; only the candidate's second-hop original node-ID selector fails as reported above.

Projection is **not silently empty**: both versions reject unavailable member IDs with `E_PROJECT_MEMBER`. On 0.14, metadata retained `b`, so projecting `b` succeeds. On 0.15, projection must name the returned wrapper ID; original `b` is unavailable and produces `E_PROJECT_MEMBER`, rolling back a preceding marker commit. Direct target queries still expose and project `b`. Projecting an unchanged assertion ID retains the correct wrapper endpoints. Entity/space identity remains unchanged, but neither the entity label nor a dependency NodeRef implicitly aliases a current member ID.

This matrix treats current-result ID projection as the explicit established operation semantics and documents the changed metadata output IDs. It does not silently add source-ID aliases to projection, which would be a wider contract decision than the narrowly witnessed metadata-host fallback. A source function with hard-coded node IDs after metadata must be migrated or use stable edge-ID projection/current-result IDs; source currently has no general dynamic node-ID binding. This limitation must be described with the 0.15 wrapper change, even after the compatibility fallback repairs nested metadata navigation.

## Fix verification

The selector fix preserves wrapper payload identity and adds only the narrow original-host lookup described above. The current attachment origin envelope must contain the exact target attachment ref. Legacy authorized `ResolvedGraph` entries can omit their remapping envelope; when such an envelope exists it must agree with that ref. No storage read, authored origin override, or node dependency alias is introduced.

Both unchanged independent scripts now pass locally: the actual source two-hop/cycle/privacy regression and the 0.14-versus-0.15 host/projection matrix. Native selector tests additionally reject ambiguous original/current/target candidates, wrong revisions or target values, missing current provenance, saved/remapped attachments, and arbitrary dependency refs. Exact current-host selection takes precedence, including its conflict behavior. Full workspace/lint evidence is recorded in the checkpoint handoff; the scripts remain unchanged.
