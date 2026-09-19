# Weave contract 0.3.0

The engine accepts programs explicitly labelled 0.1.0, 0.2.0 and 0.3.0. `bind` and `evaluate` require 0.3.0, while legacy operations preserve their semantics. Results identify the current output contract version. This version adds pure reusable graph values without hidden persistence.

`Command::Bind { name, value }` evaluates a graph expression, returns a queried result and binds an immutable program-local value. Names must be nonempty and unique. `Command::Evaluate { value }` returns a queried result without a binding. References resolve only previously bound values in the same program. No graph commit, branch, event or adapter effect is created by these commands.

`GraphExpression` uses the JSON tag `kind`:

- `query`: `query: QueryPlan` reads a pinned stored graph.
- `reference`: `name: string` retrieves an earlier immutable value.
- `join`: `left`, `right`, `output_predicate`, `match_on` compose graph expressions using the 0.2 identity-space path semantics.
- `filter`: `input`, optional `predicate` and `valid_at` select assertions and their endpoint nodes. With no filters it preserves the graph. Metadata results and coverage are inherited; this operator does not fetch new metadata.

Expression evaluation is bounded to depth 32 and 1000 visited expression nodes per command. Named values are bounded to 200000 total nodes and edges. A cumulative 32 MiB serialized-byte budget counts bound values and returned command results, including properties and metadata; repeated references cannot amplify retained output without bound. Direct query and join construction also apply incremental byte accounting to metadata, nodes, edges and provenance. These serialized-data ceilings are not an exact process-memory quota. Individual input snapshots and CLI plans are limited to 16 MiB, and identifiers to 512 UTF-8 bytes. Existing join pair/output and input graph limits remain enforced. Budget failures are explicit and roll back the program.

`QueryResult.edge_origins` maps each output edge ID to its exact original stored assertion references. Queries begin with their stored assertion identity; composed joins flatten leaf origins instead of fabricating revisions for intermediate values. `derived_from`, provenance and snapshot vectors retain these references through later operations. Filtering discards origins of removed edges. Visible input restrictions and partial coverage propagate; derived output stays principal-scoped until a release-policy mechanism exists.

A program transaction sees its prior explicit commits, while bound values retain their original snapshots if later commits change a branch. Pure graph values are scoped to one execution; they do not claim a persistent object identity. Output IDs are deterministic for a given current output contract, plan and pinned inputs, not a guarantee of cross-version identity stability.
