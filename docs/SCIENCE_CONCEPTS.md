# Concepts for temporal graph experiments

Weave experiments combine two choices: **which recorded graph revision to read**
and **which valid-time facts to analyze**. Keep those choices, identity, authority
and completeness in the experiment record so that the numeric result has a clear
meaning. The [Python tutorial](PYTHON_SCIENCE.md) and
[native JSON guide](SCIENCE_INTERFACE.md) turn these concepts into runnable code.

## Entities, manifestations and spaces

| Identifier | Meaning | Example |
|---|---|---|
| `entity_id` | Stable identity of the thing represented | `document-A` |
| Node `id` | A particular manifestation used as a graph endpoint | `document-A-vector-v1` |
| `space_id` | The space in which that manifestation lives | `encoder-v1-normalized` |
| Edge `id` | Identity of a directed assertion | `citation-A-B` |
| `graph_id` | Dataset or graph with revision history | `documents` |
| `branch_id` | Named mutable head of that graph | `main` |
| Revision | Immutable recorded snapshot identity | The digest in a commit receipt |

One entity can have different manifestations in different spaces. Topology
analyses use manifestation IDs, so they do not collapse two nodes because their
entity IDs match. Derived graph operators can generate new manifestation IDs
while preserving entity identity. Vector results retain manifestation, entity
and space identity. Similarity measures distance; it does not establish identity.

For vector experiments, choose a space that identifies compatible model outputs.
Record encoder/version, dimension, normalization and preprocessing in your
experiment parameters. Exact nearest-neighbor search compares numeric properties
in one requested space. It does not generate embeddings, validate your model
provenance or infer a mapping between spaces.

## Immutable revisions and compare-and-swap corrections

An import supplies a complete graph snapshot. It is not an append of arbitrary
rows: an updated snapshot must include the records you want to retain. A
successful change advances the branch head and returns a new immutable revision.
The earlier revision can still be selected if the database retains it.

An absent/null `expected_head` is create-only. To replace an existing branch,
send its current revision as `expected_head`. A stale head rejects instead of
overwriting another writer's work. Reimporting the same content with the correct
head can return an unchanged receipt. Save the returned revision; do not compute
one yourself or substitute a wall-clock timestamp.

A query without an explicit revision follows the selected branch's current head.
An explicit revision selects recorded content. That pin fixes the graph snapshot,
but current read authority and any live metadata dependencies still apply.

## Valid time and recorded knowledge

Valid time describes when an assertion applies in the dataset. The engine
contract represents it as signed 64-bit **Unix-epoch milliseconds** in an interval
`[start, end)`: the start is included and the end is excluded. `end=null` is
unbounded. The engine compares integers and does not convert date strings or
infer units. The examples use small toy millisecond values near the epoch.
An application can model an abstract integer time axis, but must document that
convention and convert consistently before combining it with calendar-time or
geometry profiles. Recorded-time selectors always use their stated milliseconds.

Recorded time describes when the local runtime observed a graph head. The engine
assigns its local recorded timestamp and observation checkpoint. A recorded-time
query selects what this replica knew at a cut, or at an exact checkpoint. It is
separate from the edge's valid time and is not a globally synchronized distributed
clock.

Consider the supplied example:

| Recorded revision | Edge `ab` valid interval | Is `ab` valid at `8`? |
|---|---|---|
| Original import | `[0, 10)` | Yes |
| Later correction | `[0, 6)` | No |

Both revisions can answer a valid-time question at `8`. They represent different
recorded knowledge about the same fact time. The correction does not rewrite the
old revision. A query of the current head and a query pinned to the original
revision therefore have different meanings even when both use `valid_at=8`.

Temporal joins require matching entity/space endpoints and compatible intervals
and context. The joined assertion uses the overlapping valid interval. Touching
intervals such as `[0, 5)` and `[5, 10)` have no simultaneous overlap. A topology
path across an interval union is a different operation: its edges need not have
coexisted at one time.

## Choose the node universe before analysis

For topology at a fixed time, read an unfiltered authorized graph and pass
`valid_at` to the **analysis**. This filters edges while retaining the selected
nodes, including isolates. A core query filtered with `query.valid_at` instead
selects matching edges and their endpoints, which can remove isolated nodes.

That difference changes component counts, degree statistics and PageRank. Decide
whether your population includes nodes with no edge at the sampled time, and
record that decision. Without analysis time, topology uses the interval union
of selected positive edges; it does not establish simultaneous connectivity.

Degree, components, shortest paths and PageRank use a positive directed multigraph.
Parallel edge assertions retain multiplicity. Self-loops contribute one incoming
and one outgoing incidence. Negative assertions remain in the selected query
envelope but are excluded from positive topology, with their exclusion reported.
Shortest paths count unweighted hops. PageRank reports convergence and residual;
check convergence before treating the score as a converged result.

## Authorization, provenance and coverage

The local host supplies an actor and explicit graph write grants. Request JSON
cannot supply its own authority. All reads, including old revisions and analytics,
go through current runtime authorization. A graph may contain data outside an
actor's view; statistics describe the actor's selected authorized graph.

Nodes and edges may refer to graph-valued metadata. Their `metadata` lists hold
pinned `GraphRef` values with a graph ID and immutable revision. Named metadata
attachments can instead select `live_graph`, following a graph branch head.
Resolving either is a bounded authorized read, not a direct copy of everything
in the referenced graph. Missing, denied or otherwise unavailable dependencies
can produce partial coverage and diagnostics.

Keep the full query envelope with your experiment:

| Field | What it preserves |
|---|---|
| `graph` | The selected graph value |
| `snapshots` and `input_snapshots` | Selected source revisions, including composed inputs |
| `provenance` and origin maps | Evidence references used by the derived result |
| `metadata_graphs` | Resolved authorized metadata graphs |
| `coverage` and `diagnostics` | Completeness within the selection and unavailable dependencies |
| Observation/context fields | Recorded/accepted selectors and context when relevant |

`snapshots` maps graph IDs to revisions; use `input_snapshots` as well when a
composition selects multiple revisions of one graph. Complete coverage is relative
to the declared selection and authority; it does not establish that the whole
world is known. An authorized empty answer is not proof that unavailable facts
are false. Analysis rejects partial coverage unless you explicitly permit it.
Permitting partial input retains the partial envelope.

## What a saved experiment can reproduce

The Python client saves the request, parameters, full selected input, algorithm
semantics, versions, actor and executable hash. For supported read-only plans,
it fixes unambiguous observed revisions and recorded checkpoints for replay. It
then re-reads under current policy and compares the complete selected input.

Later corrections to a root branch can leave an old pinned experiment replayable.
Changed live metadata, changed selected authority, missing retained revisions or
an incompatible executable can prevent exact replay. Pin metadata revisions when
the experiment needs those dependencies to stay fixed. Artifacts carry integrity
hashes; they do not sign authorship or restore read/write grants.

Some runtime service/view expressions can execute and be recorded but are not
eligible for exact SDK replay. See the [Python replay reference](PYTHON_SCIENCE.md#results-exports-and-replay)
for that explicit profile. Saving a graph-only JSON or CSV export also loses some
of the query envelope; retain the full JSON and experiment record beside tables.

Results use deterministic ordering, including lexical vector tie-breaking.
Floating-point analyses should be compared with an appropriate tolerance when
deliberately comparing different builds or platforms. A revision pin and an
executable identity do not prove cross-platform bit-identical numerical output.

## Resource and mutation outcomes

Native analyses have explicit node, edge, vector-dimension, work and output limits.
Core reads have separate cumulative budgets. These are bounded experimentation
profiles; they are not a CPU/RSS sandbox or a demonstrated large-dataset capacity.
The [interface](SCIENCE_INTERFACE.md) lists limits and the
[validation guide](SCIENCE_VALIDATION.md) describes measured workloads.

A transport timeout, or an output failure reported after execution, can occur
after durable commits. Read the relevant branch heads before deciding whether
to retry. The SDK does not retry mutations automatically. A stale-write rejection,
an unchanged import and an unknown commit outcome are different states.
