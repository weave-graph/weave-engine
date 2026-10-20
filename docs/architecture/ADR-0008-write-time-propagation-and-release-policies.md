# ADR 0008: write-time propagation and release policies

Status: **accepted** on 2026-10-20, when B2
([#5](https://github.com/weave-graph/weave-engine/issues/5)) implementation
started. B2 lands in parts. Part 1 adds this ADR, the noninterference harness
(section 7) and the fixes the harness found. Write-time propagation (sections
2 to 4) follows in part 2. Persisted analytics and release policies (sections
5 and 6) follow in part 3. Each section says which part implements it, and
"Implementation" at the end records what has shipped.

## Context

Read-time protection is strong. Every read authorizes each record against its
`readers` and `reader_groups` clause (ADR-0007 section 2) and against its
influence carriers (`derived_from`, `derived_nodes`, `derived_snapshots` and
the OR groups in `derivations`), which are resolved under current authority
([INFLUENCE.md](../INFLUENCE.md)). Engine-generated values (joins, rules,
supports, windows and the rest) carry those carriers, so a reader who cannot
see an input cannot see the value.

Three gaps remain ([audit](../AUDIT_2026-10-17.md); issue #5):

1. **Writes trust the writer.** A fact write may declare `derived_from` or
   provenance `evidence` naming restricted records and still give itself
   broader `readers`, or declare nothing. Nothing compares a written record's
   readers with those of its declared inputs.
2. **Results leave the engine without their restrictions.** Analytics
   (degree, components, paths, PageRank), `nearest` hits, candidate relations
   and embeddings computed from restricted text are values in a response. A
   caller that persists them writes new records with whatever readers it
   chooses.
3. **Noninterference is shown by a finite set of paired tests.** Each A3, A4
   and B1 operator has hand-written pairs. Nothing covers every operator
   with generated inputs.

ADR-0007 section 2 also leaves this ADR the write-time form of a conjunction
of group-based clauses. Until now `visibility::intersect` encodes it as a
clause-wise meet, which is safe but narrower than the conjunction (ADR-0007
amendment 8).

## Decision

### 1. One restriction model

A record's **restriction** is the AND of:

- its own clause: `readers` OR `reader_groups` (both empty means public);
- each influence carrier, resolved at read time: every `derived_from`
  assertion, every `derived_nodes` node and every `derived_snapshots` revision
  must be visible, and at least one alternative of `derivations` must be.

A reader *satisfies* a record when every part admits it under current
authority. This is exactly what reads enforce today. It is the only model:
write-time checks (sections 3 and 4) and persisted results (section 5) are
stated in it, and none of them resolves group membership at write time.

### 2. Engine-computed derivations carry an exact conjunction (part 2)

When the engine builds a record from input records (an explicit assertion on
its structural edge, a join, a rule conclusion, a support or window wrapper,
an identity member), the derived record's restriction is the conjunction of
its inputs' restrictions:

- **No input names a group.** The own clause is the literal intersection of
  the restricted inputs' `readers`, byte for byte as today, except that an
  empty intersection of two restricted inputs becomes `weave:no-reader`
  instead of the empty (public) list. Today such a record is never shown to
  anyone, because a reader must be listed on both inputs, but a copy of it
  would be public.
- **Some input names a group.** The own clause is the first restricted
  input's clause, exactly. Every other restricted input enters as an
  influence carrier on the record (`derived_from` for assertions and records
  that have an assertion reference, `derived_nodes` for nodes), so the read
  check enforces that input's clause exactly and under current membership.
  This replaces the clause-wise meet of ADR-0007 amendment 8. A meet is
  narrower than the conjunction: an agent whose two clauses each admit one
  input saw nothing.
- The carrier references are bounded by the existing 1,000-reference carrier
  limit (`E_BUDGET` beyond it).

### 3. Declared derivations may not widen their inputs (part 2)

A fact write declares inputs in two places: its influence carriers and its
provenance `evidence` (ADR-0005), which names assertions, nodes, graphs or
embeddings. For each declared input that is restricted and visible to the
writer, the written record must be **no broader** than the input. It is no
broader when either holds:

- the record carries the input as an influence carrier (the read check then
  enforces it), or
- the record's own clause implies the input's clause syntactically: the
  record is restricted, its `readers` are a subset of the input's `readers`
  and its `reader_groups` a subset of the input's `reader_groups`.

Otherwise the write fails with `E_DERIVED_READERS` and changes nothing,
unless the change names a release policy that allows it (section 6). Like
every rule that rejects input a published version accepted, it applies to
programs of protocol 1.2.0 and later (core v1 section 11.4). The check is
syntactic and conservative. It never resolves membership, so its
outcome does not depend on registry state the writer cannot see. A declared
input the writer cannot see fails the write with `E_EVIDENCE_UNAVAILABLE`, as
it does today.

**What this does not cover.** A principal who can read a fact can always copy
its content into a new fact without declaring it. No engine can detect an
undeclared copy. That is a disclosure by an authorized reader, and it is
listed in SECURITY.md. Section 3 makes the declared path safe, and section 5
makes the engine's own results carry their restrictions, so the safe way is
also the easy way for agents (B4).

**Corrections must not silently widen (issue #5 risk 2).** A `correct` change
whose clause is broader than the clause of the version it replaces fails with
`E_READERS_WIDENED`, unless the change sets `widen_readers: true`. "Broader"
uses the same syntactic rule: the new clause is public while the old one is
not, or it lists a principal or group the old one does not. The flag and the
check are new in protocol 1.2.0. Saved programs of older versions keep their
published meaning (core v1 section 11.4), so a 1.1.0 correction still
replaces the readers it omits. The language compiler emits the version it
vendors, so source corrections (the weave-language#19 risk) are protected
from the 1.2.0 vendor update on, and until LB1 lets source set `readers`, a
source correction of a restricted fact fails instead of publishing it.

Section 3 and the widening check guard against mistakes by writers who may
read the inputs: such a writer could copy the content anyway. They are input
rules, so they follow the version policy. Section 4 is different. It is
authorization, it protects records from writers who may *not* read them, and
a version gate would let any writer bypass it by declaring an older version.
It applies to programs of every version, as earlier authorization fixes did
(for example the cached-receipt fix of E04).

### 4. The write path never reveals hidden records (part 2)

The A2 write path compares a change with the live version of its key without
checking whether the writer may read that version. The harness design review
for this ADR found that this reveals more than the ID occupancy that ADR-0007
open question 1 declared:

- asserting a fact identical to a hidden live fact is a no-op, while a
  different one fails `E_CONFLICT`, which is an equality oracle on hidden
  content;
- "a live fact with this ID has another kind" reveals the hidden fact's kind;
- `correct` and `retract` act on hidden facts, so a writer could overwrite or
  delete a record it cannot read, and `expected: unchanged_since` tests a
  hidden fact for change. B3 (#70) fixed this in registry stores, where such
  a fact is now treated as absent; local stores still do it;
- references to hidden records (an edge endpoint, an attachment host, an
  embedding subject) validate when the hidden record exists and fail when it
  does not, which is an existence oracle on any guessed ID;
- `Engine::reconcile_facts` plans `correct` or nothing for hidden keys, the
  same equality oracle.

The rule from part 2: a live version the writer cannot read is treated as
absent for every purpose except occupancy. `correct`, `retract` and
`expected: unchanged_since` fail exactly as for an absent key. A reference to
a hidden record fails exactly as a reference to a missing one. An `assert` of
a hidden-occupied key, and `expected: absent` on one, fail with `E_CONFLICT`
"fact ID is not available", whatever the hidden fact's kind or content.
`reconcile_facts` plans `assert` for a hidden key, so the following write
fails the same way.

A writer *can read* a live version when its clause admits the writer, or when
the writer recorded that exact version (ADR-0005 `writer`). The second case
keeps A5 importers working: they write customer-restricted units that they
cannot read, and must be able to reconcile and correct what they wrote. It
reveals nothing the writer did not author.

**Risk 1 decision (record-ID occupancy).** Occupancy stays a declared
channel. It is one bit per guessed ID, reachable only by a principal with
Write on the graph, and only for IDs a hidden live record holds. Closing it
would need per-writer ID namespaces, which would change every fact identity.
The B3 service already closes it for its clients: a non-administrator may
assert only IDs that `POST /v1/ids` minted for it, and B4 tools use those. The
engine adds no minting of its own. A local host is trusted and chooses IDs,
and an embedding host with a registry can mint IDs as the service does.
The engine's part is the uniform conflict above, so occupancy is the only
bit. Reusing a structural identity a hidden record once had
(`E_ASSERTION_IDENTITY`) is the same bit.

**Hidden dependents refuse a retraction.** Retracting a record that a live
record the writer cannot read depends on fails with `E_DEPENDENT_FACT`, with
the same message as for a visible dependent and no detail. Cascading would
let a writer delete records it cannot see or name. Orphaning would leave a
live record with a dangling reference, against the invariant that the
references of live records are live. Refusing keeps every record intact
and costs one bit, "something depends on this record", visible only to a
principal with Write that can read the record. This holds in local and
registry stores from part 1, and it is declared in SECURITY.md.

### 5. Persisted analytics, rankings and embeddings carry their inputs (part 3)

Results that leave the engine as values are not records, and a value in a
response has no readers. When one is persisted it becomes a record, and its
restriction must cover the authorized input it was computed from:

- **Analytics** (science `degree`, `components`, `shortest_paths`,
  `pagerank`, `nearest_vectors`): a new science operation, `persist`,
  writes an analysis result as attachment facts on the analysed nodes (or on
  the graph), computed by the engine. Each persisted record carries the
  conjunction of every input record that the value depends on, by section 2.
  A score or component label depends on the whole analysed input, so it
  carries the input's restricted records.
- **`nearest` hits and candidate relations**: persisting a candidate as an
  edge with `EvidenceRef::Embedding` evidence (the A4 path) is a declared
  derivation under section 3. Its readers must be no broader than both
  embeddings and both subjects.
- **Embeddings of restricted text**: an embedding whose provenance evidence
  names the source text is a declared derivation under section 3. An
  embedding that cites nothing is the "undeclared copy" of section 3.
- **Indexes**: no index holds per-principal visibility (ADR-0004 section 11).
  An approximate index (C5) ranks only already-authorized embeddings (A4
  ranking boundary), so it needs no restriction of its own.

### 6. Release policies (part 3)

A **release policy** is a named, versioned, audited declassification. It is
the only way a derived value may be readable by principals who cannot read
all of its inputs. B2 defines one kind, and the record leaves room for more:

- `id` and `revision`;
- `kind: aggregate_count` with `min_k`: the released value is the number of
  distinct matching subjects in an input, released only when it is at least
  `min_k`;
- `source`: the graphs and selector the count may range over;
- `audience`: the principals and groups that may read the released value.
  The released record's readers are the audience;
- `evaluator`: the principal whose authority evaluates the input. It must be
  able to read the source, and it is the authority being declassified.

Policies are facts in the reserved graph `weave:release`. Only tenant
administrators change them, through a trusted engine API, as with the
registry (ADR-0007 amendment 2). Evaluating a policy appends an audit fact to
the same graph: the policy revision, the requesting principal, the time, and
whether the value was released or withheld. A below-threshold result is
withheld with one fixed outcome, so the requester learns only "below `k`",
which is exactly what the policy allows.

### 7. The noninterference harness (part 1)

**Property.** For every operator, every generated pair of stores that differ
only in facts hidden from reader R, and every generated plan, R gets
identical outcomes from both stores: values, counts, ranks, explanations,
coverage, diagnostics, `fact_provenance` and errors (code and message).

**Generator** (`crates/weave-engine/tests/support/noninterference.rs`).
Each seed gives one pair. Both stores receive the same visible facts at the
same recorded times. The second also receives hidden facts, as their own
revisions, after each visible step. The hidden facts are placed where they
can interfere: edges between visible nodes and to hidden nodes, attachments
and embeddings on visible nodes and entities with the visible keys,
assertions on visible structural edges (relations and geometry positions),
nodes of visible entities in visible spaces, exact duplicates of visible
vectors (ties), corrections over time, a hidden descriptor graph and a
hidden identity member. Half of the seeds are registry stores (B1), which
add group-restricted records (`reader_groups`), a group nested inside
another, hidden principals and memberships, and a graph only the hidden
reader can discover (ADR-0007 section 10, items 1 to 3). The readers under
test are `alice`, who can read some restricted visible records, and
`carol`, who reads only public ones.

**Plans.** Every `GraphExpression` kind has a generator, checked against the
contract source by a test, and generated plans compose leaf reads (`query`,
`as_of`, `expand`, `nearest`, `filter`, `window`) under each operator. They
name hidden, visible and missing IDs alike. The science harness
(`crates/weave-science/tests/noninterference.rs`) runs every analysis over
generated authorized inputs on the same pairs.

**Sensitivity.** `mallory` can read every record of both stores. For every
operator, some cases must give `mallory` different outcomes from the two
stores. That shows the hidden facts reach every operator, so the equality is
not vacuous.

**Normalization.** Outcomes are compared after removing what differs by
design between two stores:

- *revision identity*: revision IDs are content-derived over every fact
  (ADR-0001, ADR-0004), so each revision is replaced by a label for the
  visible state it represents; head-observation IDs and the store's random
  replica identity are blanked;
- *generated IDs*: IDs the engine derives by digesting a value (for example
  `temporal:edge:sha256:…`) digest its origins, revision IDs included, so they
  are compared up to a consistent renaming;
- *order that follows those IDs*: arrays are compared as multisets, a list
  of pins of one visible state as a set, and nested proof conjunctions as one
  set of conjuncts. Rank orders (`neighbors`, `candidates`, change entries)
  are compared exactly.

**Declared channels.** The harness does not close these, and SECURITY.md
lists them:

1. *Whole-revision operators.* `recorded_query`, `accepted_graph` and
   `accepted_history` serve a revision only to a reader who can see all of
   it (ADR-0002; accepted views publish whole revisions). They answer one bit
   about hidden records in the selected revision. The harness allows exactly
   their single "unavailable" error in the store with hidden facts and
   counts those outcomes.
2. *Revision identity.* A revision ID changes with every write, so it
   reveals that something changed, never what. Consequences: a plan that
   combines a pinned or `as_of` read with a head read of the same visible
   state sees two revisions if a hidden write came between them (for
   example `union` keeps both copies of a record), and a compiled view is
   stale after a hidden write. The harness writes hidden facts at the same
   recorded time as the visible step they follow, so every time and label
   selects one revision, and covers revision identity through the
   normalization above.
3. *Resource and work budgets.* The read budget, the `nearest` scan budget
   and the 10,000-step proof budget count the records an operation reads,
   hidden ones included. A request near a limit can fail with `E_BUDGET`
   only because hidden records exist. **Risk 3 decision:** they stay
   declared. They bound real work, and charging only visible records would
   leave the work unbounded. The harness keeps generated stores far below
   every budget.
4. *Timing and response size* (side channels, not outputs).
5. *Write-path occupancy* (section 4): record IDs, retraction dependents
   and structural identity reuse, for principals with Write.
6. *Disclosed identities*: a fact's `writer` and `on_behalf_of` are shown to
   its readers (ADR-0005, ADR-0007 section 8).
7. *Graph existence in the local profile*: a store without a registry has no
   Discover, so graph IDs and revision IDs are visible to every trusted
   local caller. Registry stores close this (the harness checks it).

**Counts.** `WEAVE_NI_CASES` sets the cases per operator: 40 by default in the
debug workspace suite, 500 in the PR job and 10,000 in the scheduled
full-count job, both optimized. The run prints one
`NONINTERFERENCE {json}` line with per-operator cases, outcomes, declared
outcomes, sensitivity and error codes. Measured counts are recorded in
[docs/measurements](../measurements/).

## Alternatives considered

1. **Flatten group clauses into literal readers at write time.** Rejected by
   ADR-0007: it freezes membership and breaks revocation.
2. **A new "conjunction" field on every record.** It would carry the same
   information as influence carriers, need a contract and store change on
   every fact kind, and give reads a second restriction mechanism. Rejected
   in favor of the carriers that reads already enforce.
3. **Semantic implication for section 3** (resolve membership and compare the
   admitted sets). Its outcome would depend on registry state the writer
   may not see, which is itself a channel, and it would freeze membership
   into a write-time decision. Rejected.
4. **Server-generated record IDs now** (risk 1). This would change every
   client that chooses IDs today, including the A5 importers, whose row keys
   are their idempotency keys. B4 tools generate IDs instead, and occupancy is
   declared.
5. **Budgets that count only visible records** (risk 3). Rejected: work on
   hidden records is real work.
6. **An external property-testing library for the harness.** The generators
   need engine-specific shapes and pairing, the repository already uses
   seeded SplitMix64 generators (A5), and a failing case is replayed by seed
   (`WEAVE_NI_SEED`). Shrinking was not needed to diagnose the cases found.

## Consequences

- Part 2 is a contract change (`widen_readers`, the new error codes) and an
  additive minor protocol bump. Release policies (part 3) add a reserved
  graph and its records.
- Writers that today rely on writing over records they cannot read, or on
  publishing a restricted fact by an empty correction, get explicit errors.
- Every operator has a generated noninterference check, and a new operator
  cannot be added without a generator (the kind-coverage test fails).

## Implementation

**Part 1** (this PR): the ADR; the harness for all 28 `GraphExpression`
kinds and all 5 science analyses, in registry and local stores; a PR CI job
at 500 cases per operator and a scheduled job at 10,000. The harness found
two interferences, each fixed with a regression test that fails without the
fix. Both came from orders that followed revision IDs, and neither exposed
hidden content:

- Rule explanations (`reason`) chose among equivalent witnesses in the order
  of a digest over revision IDs, so hidden facts in a revision changed which
  visible witness a reader was shown. Saturation now visits a fact's proofs
  in the order of their records without revision IDs.
- Proof conjunctions were nested pairs, folded in record order. The nesting
  depth, and so whether the carrier depth budget (32) was exceeded, depended
  on hidden facts: one store answered, the other failed with
  `E_INFLUENCE_BUDGET`. A conjunction now lists its conjuncts flat, without
  duplicates, in an order of the conjuncts without revision IDs. The gates
  and the operator label are unchanged, and a stored nested pair reads as the
  same conjunction.

Both are output canonicalization fixes in shared contract code: the meaning
of every result is unchanged, so they apply to every protocol version, and
the contract files change without a version bump.

A write-outcome harness runs generated writes in registry stores (new,
visible, hidden and missing records corrected, retracted and asserted). It
covers the B3 rule that a fact the writer cannot read is absent for
`correct` and `retract`, and it allows only the uniform dependent refusal.
The other write-path findings of section 4 are fixed in part 2, which also
extends this harness to hidden IDs, hidden references and local stores.
