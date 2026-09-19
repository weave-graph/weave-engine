# Weave contract 0.2.0

The canonical types are in `crates/weave-contract/src/lib.rs` (crate version 0.2.0). The engine explicitly accepts both 0.1.0 and 0.2.0 programs. Legacy programs cannot use `join`; unsupported combinations fail before mutation. Existing commit/query semantics remain as [0.1](../v0.1/README.md).

## Identity-key path joins

```json
{"op":"join","left":{"graph_id":"fleet"},"right":{"graph_id":"advisories"},"output_predicate":"affected","match_on":"entity_space_to_from"}
```

`left.to` and `right.from` match exactly when both endpoint nodes have equal stable `entity_id` **and** `space_id`. Local node IDs may differ. Equal labels, vector similarity and cross-space identity alone do not establish a join. This is an explicit identity-key join, not automatic identity alignment.

The two input views are pinned in one local database read transaction. Only authorized positive edges are premises. A negative edge supplies explicit negative support and does not produce a positive path. Derived edges run from the left source to the right target. Their valid interval is the intersection; empty intersections produce no edge. Unbounded ends stay unbounded unless constrained by the other premise. `valid_at` filters premises and does not change their stored interval.

Output node IDs are deterministic hashes of source graph/revision/node IDs, preventing accidental collision of graph-local names. Entity and space IDs remain unchanged. Output edge IDs hash the operation and source assertion references. Each edge retains both premise references, and every output is conservatively readable only by the evaluating principal until an explicit release policy is implemented. External effects are absent.

`QueryResult.input_snapshots` is the authoritative complete vector of distinct graph/revision pairs, including expanded metadata. The legacy `snapshots` map retains only unambiguous graph IDs for joins; two revisions of one graph remain in the vector and are omitted from the legacy map. Metadata snapshots remain explicit in `metadata_graphs` as well. Partial input coverage propagates to the result. Results cannot claim a globally atomic snapshot across external peers.

The current implementation bounds the candidate pair count at 1,000,000; exhaustion is an explicit error, never a silently truncated complete result. This is a basic reference execution strategy, not an optimized join planner. N-ary rules, alternative join patterns, generic graph-valued functions and recursive inference remain open.
