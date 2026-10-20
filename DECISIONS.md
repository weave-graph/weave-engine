# Decisions log

Running log of non-trivial design and process decisions made during the
autonomous Phase A → B → C roadmap run ([docs/ROADMAP.md](docs/ROADMAP.md)).
Architectural decisions named in the roadmap also get an ADR in
`docs/architecture/`; this log records the judgement calls around them.

Format: `### YYYY-MM-DD — <issue> — <title>`, then the decision and the reasoning.

### 2026-10-19 — process — Autonomous merge authority
The owner told the orchestrator to implement Phases A–C without pausing for
approval. This overrides the AGENTS.md rule "do not merge your own PR without
owner approval" and "do not close issues unless the owner asked" for this run.
Each PR is reviewed by the orchestrator against the issue's acceptance criteria,
must have green CI, and is squash-merged. No force-pushes to `main`.

### 2026-10-19 — process — Epics delivered as several PRs
Phase A epics such as A2 are too large for one reviewable PR. An epic may land
as a sequence of PRs that each reference the issue ("Part of #N"). Only the
final PR, which meets every acceptance criterion, uses "Closes #N". Progress
notes go in issue comments, per AGENTS.md.

### 2026-10-19 — process — Execution order
Order follows the ROADMAP dependency graph, with at most 3 parallel streams:
A1 → {A2, LA0} → {A3, A5} → A4 → {A7, LA1} → A6, then B1 → {B2, B3} → B4 → B5 → B6,
then {C1, C2, C5} → {C3, C4} → C6. Deferred epics (E08, E09, E10, E13, L5, L6)
and the ongoing LT epic are not scheduled. LT stays open by design.

### 2026-10-19 — process — Progress reporting is local
A cron job runs a headless, read-only Claude Code review every 12 hours. It
writes the summary to `~/work/weave/reviews/` on the owner's machine and does not
post to GitHub, per the owner's instruction.

### 2026-10-19 — #2 A1 — No wire change; protocol stays 0.21.0

Core contract v1 is a named profile (`core-v1`) over protocol 0.21.0, plus
reserved names. A1 changes no bytes that any program, result or store holds.
`VERSION`, the crate version and `STORAGE_VERSION` therefore stay as they are.
The new Rust items (`wire`, `provenance`, `embedding`) cannot be reached from
`Program`. A bump with no wire change (for example 0.21.1) would have made
every emitter and the engine's accepted-version list change for nothing.

### 2026-10-19 — #2 A1 — Protocol 1.0.0 is cut at the Phase A release (A6)

Before 1.0.0, every wire change is an additive MINOR bump. A2 and A4 activate
the reserved provenance and embedding fields this way, and each PR records its
exact number. 1.0.0 freezes core v1 once those fields are live. Freezing now
would put 1.0 on a contract that cannot yet express provenance or embeddings.

### 2026-10-19 — #2 A1 — ADR-0005 and ADR-0006 written as Accepted, with records defined now

Pending ADRs would leave A2 and A4 to design the fact model under schedule
pressure. Defining the records now (`FactProvenance`, `ClaimedProvenance`,
`EmbeddingSpace`, `EmbeddingFact`, `CandidateRelation`) lets golden fixtures
test them today. It also makes the reserved fields concrete instead of just
names. A2 and A4 may still add fields additively. Any change to the accepted
records needs an ADR amendment.

### 2026-10-19 — #2 A1 — Engine-recorded writer and time sit outside fact content

ADR-0001 revision identity is content-derived. A recorder inside the content
would give the same fact different identities on different replicas, and a
copied fact could carry a forged writer. Clients send only `ClaimedProvenance`,
which has no writer field and so fails with `E_UNKNOWN_FIELD`. The engine
returns `FactProvenance` in a side list keyed by `FactRef`. An omitted claim
becomes `method: unspecified`, never `human`.

### 2026-10-19 — #2 A1 — Embeddings: binary32, encoder_version, exact-equality spaces

Vectors are stored as binary32, which is half the storage for the A4
100k × 1536 target. The rounding happens once, at decode, and is documented.
`encoder_version` is a separate field because A4 must reject version mismatches
at equal dimension. Spaces compare by full equality. Candidates are result
values with predicate `weave:similar_to`, never asserted facts. The legacy
binary64 `vector` property is never reinterpreted.

### 2026-10-19 — #2 A1 — Stable decode error codes

`E_VERSION` is reused because the engine already returns it for unsupported
versions. New codes: `E_UNKNOWN_FIELD`, `E_UNKNOWN_VARIANT` and `E_DECODE`.
The version is checked before the structure, so a newer program never
half-decodes. Duplicate JSON keys are rejected, as the science envelope already
does. The codes come from serde's derive messages, and the harness pins that
mapping. This avoids a new dependency (such as `serde_path_to_error`) that the
vendored crate would also have to carry.

### 2026-10-19 — #2 A1 — `weave-engine run` reports the stable codes

The engine CLI now decodes through `decode_program`, so it reports
`E_VERSION` and `E_UNKNOWN_FIELD` instead of a blanket `E_INPUT`. The science
envelope (API 0.1.0) and the C ABI keep their documented codes. Changing them
would break their own stable-code promises.

### 2026-10-19 — #2 A1 — Fixtures live inside the contract crate and are language-neutral JSON

The language vendors `crates/weave-contract/` byte for byte. Putting the
fixtures and the harness in that directory means LA0 only needs to copy the
files, add their hashes and run one cargo command. The fixtures are JSON with
a SHA-256 manifest, so a non-Rust consumer can use them too. Positive inputs
are canonical, which catches renamed fields and changed encodings. A
`canonical` key pins defaults. An optional `engine` block gives golden runtime
outcomes, including content-derived revision IDs.

### 2026-10-19 — #2 A1 — Classification boundaries

Classified *core v1*: schemas, attachments, `commit_batch`, union, diff,
project, support, `explain`, `window` and `sequence`, the explicit assertion
profile, every restriction carrier (influence, `derived_*`, alternative
derivations), `recorded_query` and `recorded_range`. Phase A builds on all of
them.

Classified *extension*: rules, context selection, typed contexts, decimals and
quantities, counterparts, `resolve_identity`, accepted and governed reads, views
and geometry. They stay supported, but Phase A does not depend on them.

Classified *deferred*: `cluster` and handler artifacts, which follow the
roadmap's deferred epics. No semantics change.

### 2026-10-19 — #2 A1 — AGENTS.md left unchanged

AGENTS.md still says "the next number is ADR-0004". That remains true, because
ADR-0004 (A2) is unwritten. A pointer to core-v1.md would help, but AGENTS.md
holds agent instructions, so the PR proposes the edit for the owner to make
rather than changing it from an agent branch.

### 2026-10-19 — weave-language#1 LA0 — Contract classification behind `describe --contract`
The core-v1 / extension / deferred report is opt-in via `weave describe --contract`; plain
`describe` keeps its existing output because tests and scripts depend on it. The report
classifies the emitted plan (not source syntax) with an exhaustive match, so a future vendored
variant fails to build until it is classified. Locations are plan paths, not source spans
(documented limit). Typed embedding vectors are reported as the 0.9 geometry extension because
they still lower to legacy vector properties.

### 2026-10-19 — weave-language#1 LA0 — Vendored harness runs in place
The engine golden fixtures run in language CI from `vendor/weave-contract` with a separate
target dir. `verify_contract.py` ignores exactly `Cargo.lock` and `target/` produced by that run;
every vendored file is still hash-checked.

### 2026-10-19 — #2 A1 — A1 closed across two repositories
A1's "fixtures run in language CI" criterion was delivered by LA0 (weave-language#13). A1 closed
when both #48 and weave-language#13 merged. Suggested AGENTS.md updates from both agents
(test counts, harness command) are deferred to the owner, since AGENTS.md is agent instructions.

### 2026-10-19 — #3 A2 — Delivery in three parts, one store bump

Part 1 adds ADR-0004, the `FactBackend` trait, the SQLite and in-memory
backends, their conformance suite and the CI SQL-boundary check. The engine
does not install or use the fact tables yet, so part 1 changes no store or
wire bytes. Part 2 wires fact writes, provenance and node valid time into the
engine with the protocol bump and the single 29→30 store migration. Part 3
adds the indexed read path, cap removal, the 1M-fact tests, benchmarks,
incremental science import and the remaining docs. One migration instead of
one per part keeps the crash-tested upgrade surface (and the compatibility
scripts that pin the store marker) to a single change.

### 2026-10-19 — #3 A2 — Snapshot revisions stay authoritative; fact revisions are new

Existing and future full-snapshot revisions keep their stored payload and
ADR-0001 IDs. Fact-level writes create `fact:sha256:…` revisions whose content
is the parent plus the logged operations. Converting snapshots into log
entries was rejected: retention may have erased old payloads, and snapshot
array order is client-defined, so exact bytes could not be re-derived. Leaving
every pre-A2 row untouched makes "pinned reads are byte-identical after
migration" true by construction. The distinct prefix means no code can verify
one kind as the other.

### 2026-10-19 — #3 A2 — Lines copy their base instead of sharing ancestry

A line is a linear chain of fact revisions. The first write after a snapshot
or a fork copies the parent state once as base entries. Every read is then one
range scan on one line, rather than a walk over ancestor lines. Snapshots are
bounded at 16 MiB, and forks of large fact graphs are rare (E09 is deferred).
C4 can replace this behind the trait.

### 2026-10-19 — #3 A2 — The backend is semantics-free

The trait stores opaque record bodies plus index keys that the engine
extracts. Validation, authorization, provenance merging and revision IDs stay
in the engine. A C4 backend therefore implements storage guarantees only and
cannot drift semantically from SQLite. The conformance suite runs every check
against both backends and adds a differential random-history test.

### 2026-10-19 — #3 A2 — Checkpoints are explicit, never written by reads

A read that wrote a cache would need a write lock and could fail for reasons
unrelated to the read. Checkpoints are written only by an explicit
maintenance call, and dropping them changes no result.

### 2026-10-19 — #3 A2 — Time-varying node properties are attachment facts

`Node.properties` stays the time-invariant map. A value that changes over
valid time (such as firmware) is a node-hosted metadata attachment with a
literal value, which already has valid time, readers and derivation carriers.
Each value is one fact, so a change is one O(1) write. A separate property
record type would duplicate the attachment's authorization and influence
rules.

### 2026-10-19 — #3 A2 — Room for B1 group readers

The index stores no per-principal visibility, so group resolution can happen
at read time without reindexing. `readers` entries stay literal principals
forever, so no record changes meaning. The field name `reader_groups` is
reserved in core v1 §11.6 for B1 group references. A `group:` reader prefix
was rejected: pre-0.22 data could already use that string as a literal
principal ID.

### 2026-10-19 — #3 A2 — First recording is listed

The provenance listed for a fact at revision R is the recording of the version
live at R. An identical re-assert is a no-op and records nothing. A changed
claim is a new version. Facts of snapshot revisions are attributed to the
snapshot's committer, because a snapshot commit restates the whole graph.
This needs no data migration.

### 2026-10-19 — #38 B1 — ADR-0007 written as Proposed during A2

The roadmap asks for B1 design during A2 so the fact model leaves room for
group readers. Writing ADR-0007 as Proposed fixes the semantics A2 must not
foreclose (section 13: empty `reader_groups` omitted from canonical bytes,
`writer` as a principal ID, a place for `on_behalf_of`, one visibility
predicate) without committing B1 code before Phase A closes. It becomes
Accepted when B1 implementation starts, and may be amended then.

### 2026-10-19 — #38 B1 — Groups resolved at read time; history read under current authority

Membership is resolved from the registry head pinned at request start, so
revocation takes effect on the next request and rewrites nothing. As-of and
recorded-time queries are authorized with the current registry, not the one
at the selected time. Authorizing history with past membership would let a
removed member keep reading everything up to the removal, which is what
revocation must stop. The consequence, that a new member sees the group's
history, is the intended meaning of joining a shared knowledge base. The
registry's own history still answers the audit question "who could read F at
time T".

### 2026-10-19 — #38 B1 — `reader_groups` is a disjunction alongside `readers`; groups never authenticate

A record is public only when both `readers` and `reader_groups` are empty. A
group ID inside `readers` stays a literal and matches no one, because a group
can never be a request principal. This keeps the A2 promise that `readers`
never changes meaning. Principal and group IDs are never reused, because a
reused ID would inherit every record naming the old one. Unknown or
undiscoverable groups fail closed and are rejected on write with the same
code as a nonexistent group.

### 2026-10-19 — #38 B1 — Group-based restrictions are never flattened

The engine intersects literal reader lists for derived records today. An
intersection involving groups cannot be flattened without resolving
membership at write time, which would freeze it. Derived records with any
group input keep an AND of reader clauses through the influence carriers.
Records without groups keep today's literal intersection byte for byte.
ADR-0008 owns the write-time form.

### 2026-10-19 — #38 B1 — Agent ceiling as a recursive predicate

`can(agent) = can(owner) ∧ scope ∧ match(S_local)`, evaluated per request up
an ownership chain of at most 8 links (the `weave-policy` chain bound). An
agent therefore never exceeds its owner, even after the owner loses access.
Owners see what their agents can see (agents are in the owner's identity
set), so owners can audit their agents. Administer is not delegable to agents
in B1, and group-owned agents are deferred. Both are open questions, not
rejections.

### 2026-10-19 — #38 B1 — Registry stored as facts in `weave:registry`; audit log separate

Storing the registry as facts in a reserved graph of the tenant store gives
registry history, audit and as-of reconstruction through the A2 fact log with
no second mechanism. It also commits revocation atomically with the data it
protects. The read and write audit log (B3) is a separate table, because read
volume would dominate the fact log.

### 2026-10-19 — #38 B1 — Permissions are ordered; branches are protected, not separately readable

Discover ≤ Read ≤ Write ≤ Administer, mapped onto the existing
`weave-policy` actions with no new action (Read = `read` + `traverse`,
Write = `publish`). Branch ACLs only grant Write, on branches an administrator
protects. Read stays per graph, because record restrictions already give
finer read control. A missing Discover gives `E_UNAVAILABLE`, which is
identical to a nonexistent graph and reuses the existing missing-or-denied
code. Discover without the needed permission gives `E_FORBIDDEN`. Graph IDs
in the service are engine-assigned unless the caller is a tenant
administrator, so creation cannot probe for hidden graphs.

### 2026-10-19 — #39 B3 — Registry authoritative over capability chains; HTTP/JSON transport

A signed request must pass `weave-policy` and agree with the current
registry, so a capability can only narrow registry authority. In the service,
admission roots come from registry key bindings, and authority-reducing
registry changes advance the admission epoch. IdP group claims are not live
Weave groups, because their revocation depends on token lifetime and the IdP.
They may be imported into the registry. B3 serves HTTP/JSON over TLS with
core v1 bodies. gRPC is deferred until measured. Tenancy is one store per
tenant, with the tenant taken from the credential, a per-tenant audience and
a separate control store that holds no graph data.

### 2026-10-19 — #3 A2 — One `write` command with assert, correct and retract changes

A2's write API is one command op, `write`, whose `changes` are tagged
`assert`, `correct` or `retract`. One write is one revision with one recorded
time. An agent's typical change, such as decommissioning a unit (correct the
node and retract an edge), is therefore atomic in recorded time as well as in
the transaction. The core-v1 reservation of `assert`, `retract` and `correct`
as command ops is released and reused as change kinds. `assert` refuses a
live key; overwriting needs an explicit `correct`, so no write silently
replaces a fact.

### 2026-10-19 — #3 A2 — Concurrency: `expected_head` absent, null or string; per-fact `expected`

On `write`, `expected_head` absent means no head check. This keeps
single-fact appends cheap for agents. `null` keeps the `commit` meaning (new
branch), and a string must equal the head. Per-fact `expected` is `absent` or
`unchanged_since` a named revision, which the client already has from the
query that showed it the fact. Any mismatch fails the whole write with
`E_CONFLICT`.

### 2026-10-19 — #3 A2 — No cascading retraction; identity is immutable on correction

Retracting a fact that live facts still need fails with `E_DEPENDENT_FACT`;
the writer retracts the dependents in the same write. A cascade would delete
facts the writer never named. A correction cannot change a record's
structural identity (node entity and space, edge endpoints and predicate,
assertion edge, attachment host and key). This matches the existing per-graph
structural registry and keeps every index key of a fact stable.

### 2026-10-19 — #3 A2 — Provenance only in results of 0.22.0 programs

`fact_provenance` is filled only when the program declares 0.22.0. Programs
of earlier versions get their exact previous results, differing only in the
engine's `version` field. That is what lets pinned pre-migration queries stay
byte-identical. Native APIs that return a `QueryResult` directly (views,
accepted graphs, cluster navigation) do not attach it. Five existing tests
compared a 0.22 program's result with such a native result. They now clear
`fact_provenance` before comparing. No authorization or influence test
changed.

### 2026-10-19 — #3 A2 — Artifact and event-schema versions advance with the protocol

The view and handler artifact protocols move to 0.22.0 and still accept 0.21.0
artifacts. The actor and projection `event_schema` checks accept both 0.21.0
and 0.22.0. Stored definitions keep their declared version and are never
rewritten. The `negative/unknown-version.json` fixture now uses 0.23.0,
because 0.22.0 is real. `negative/reserved-node-provenance.json` is replaced
by `negative/node-provenance-version-gated.json`: a 0.21.0 program with node
provenance now decodes and fails with `E_VERSION` at execution. It adds
`negative/reserved-node-reader-groups.json`. The Python client and
`check_science.py` speak 0.22.0.

### 2026-10-19 — #3 A2 — ADR-0007 section 13 folded into store 30

`on_behalf_of` is a reserved nullable column in `fact_log` and
`fact_revisions`, so B1 needs no migration for it. `reader_groups` stays
rejected and is never an index column. The literal-reader rule now lives in
`visibility::readers_admit`. Three copies in governance and accepted history
were moved onto it, which was cheap and changes no behaviour. The assertion
reader intersection lives in `visibility::intersect_readers`, with its exact
previous semantics. Fact writes to `weave:` graphs fail with
`E_RESERVED_NAMESPACE`.

### 2026-10-19 — #3 A2 — Retention and capsules around fact revisions

Retention treats every fact revision as present and retained (a root). It
never erases them, so whatever they reference is also kept. It skips the
fact-store tables in its generic inventory; those tables belong to the
backend. Capsule export rejects fact revisions with `E_UNSUPPORTED`. Log
compaction, erasure and export formats are C4 and E08 decisions. A silent
partial capsule would be worse than a refusal.

### 2026-10-19 — #3 A2 — Head-observation capacity moves to part 3

The 100,000-observation store cap (ADR-0002) is in A2 scope, per the review of
part 1. Part 3 restructures it together with the indexed read path. Part 2's
migration therefore leaves `head_observations` unchanged. If part 3 needs a
schema change for the cap, it brings its own crash-tested 30→31 migration.

### 2026-10-19 — #3 A2 — The head-observation index already exists, so store 30 needs no extra schema

The review asked to put the `(graph, branch, recorded_at_ms)` head-observation
index into store 30 rather than adding a 30→31 migration later. That index
already exists. Store 19 created it as `head_observations_scope`
(`recorded_history.rs`), and every upgrade path creates it with
`IF NOT EXISTS`. A migrated store 30 holds it, and the query planner uses it
for "last observation at or before T". So #52 adds no schema for this. Part 3
changes only code: it removes the store-wide `count(*)` cap and selects
recorded heads through this index, keeping the predecessor integrity check on
the selected observation. Neither change needs a new store version.

### 2026-10-19 — #3 A2 — Filtered fact reads scope their coverage to what they select

A `query` on a fact revision with `predicate`, `from` or `to` reads only the
facts the filter can select, plus their authorization closure and every
influence carrier. The result is the same as a whole-snapshot read of the same
records in every field but one. On a snapshot, a derivation denied on any
record, even one the filter discards, makes the result `partial`. On a fact
revision, only denials on selected records count. Matching that would mean
reading the whole graph again. The answer is no less complete, and it reveals
less about records outside the filter. `tests/fact_reads.rs` checks that the
two reads otherwise match and that an indexed read is never less complete
than a snapshot read.

### 2026-10-19 — #3 A2 — Store 30 amended before release: an index carrier flag and an append-order observation index

Store 30 had not been released, so part 3 amends its schema in place instead
of adding a 30→31 migration.

- **`fact_index.carrier`.** This new column, with a partial index, marks
  records whose declared snapshot gates or attachment influence enter
  whole-value influence even after filtering, so a filtered read can find them
  without a scan.
- **Pre-release stores.** A store created by the pre-release store-30 build
  (main `61eccfb`) has no `carrier` column. The engine refuses it with
  `E_STORAGE_VERSION` instead of reading it wrongly.
- **`head_observations_order`.** This new `(graph_id, branch_id)` index is
  created idempotently on every open. Tip and predecessor lookups sort by
  append order. Through the recorded-time index they needed a temporary
  B-tree, which made N writes cost O(N²). Measured on the same host: 20,000
  single-fact writes took 132 s before and 3.7 s after.

### 2026-10-19 — #3 A2 — Recorded-time selection is indexed and checks its boundary

The store-wide 100,000-observation cap is gone. A date cut selects the last
observation at or before T through the `(graph, branch, recorded_at_ms)`
index. Two checks keep the old guarantee that a deleted or corrupt
observation cannot be skipped at the selection point:

- the selected observation's digest is verified;
- the next observation in append order must name it as its immediate parent.

The linear walk from the tip was bounded by the 4,096-read budget, so it
failed for cuts more than about 4,000 writes back. Observations between the
boundary and the tip are no longer re-verified on every date read; reads of
those observations still verify them.

### 2026-10-19 — #3 A2 — 1M-fact evidence runs in its own release CI job

The `fact-scale-1m` job in `ci.yml` runs `tests/fact_scale.rs` optimized,
with 1,000,000 facts and 120,000 single-fact revisions, under a 60-minute
timeout. Because it is a separate job, a slow runner cannot hide failures in
the main matrix. The default debug suite runs the same test at 20,000 facts
and 2,000 writes. Locally the 1M run takes about 66 s and the 120k-write run
27 s.

### 2026-10-19 — #3 A2 — Swift host release-build timeout raised to 900 s

`scripts/root_native.py` gave the native library's release build 180 s. On a
cold macOS runner that timed out before any test ran (#52, first run). Both
release builds, host and simulator target, now get 900 s. The probes keep
their short timeouts.

### 2026-10-19 — weave-language#14 — `check_handlers.py` expects E_VERSION from a field table

The language handler check now expects `E_VERSION` only when the copied data
uses a field newer than the compiler's protocol. It looks the field up in a
table instead of comparing version labels, because a protocol bump with no
new graph field leaves old-profile data valid.

### 2026-10-19 — weave-language#15 — The compiler emits 0.22.0

The compiler's emitted protocol label follows its vendored contract (0.22.0),
since its commands are unchanged. `write` and fact provenance are classified
core v1. The script comparisons check provenance by presence and ignore
`recorded_at_ms`. There is no source syntax for fact writes yet.

### 2026-10-19 — #36 A5 — `SeriesRef` is a typed literal on an attachment, with no contract change

A `SeriesRef` is an `hw:series` metadata attachment on a sensor. Its value is
a `literal` tagged `type: "weave.hw.series_ref"`, `version: 1`, with
`store_uri`, `series_id`, `range` and `digest`. Its `valid_time` must equal
the range. A new `MetadataValue` variant would have needed a protocol bump,
golden fixtures and a vendor update in weave-language, while A3 is changing
the same crate in parallel. Making the valid time the range means the
engine's existing valid-time selection and indexes apply without new
operators. The decoder in `weave-hardware` is strict, so the tag is a typed
contract in practice. C1 (ADR-0009) can promote it to a contract variant
later, with a migration from this tag.

### 2026-10-19 — #36 A5 — `Engine::reconcile_facts` plans idempotent imports

Idempotent re-import needs to know which facts are absent, identical or
different. The engine's write path already decides that (an identical assert
is a no-op), but one differing fact fails a whole write. Reading the current
state through `query` would hide restricted facts from an importer that is
not their reader, and a whole-graph read hits the result budget at fleet
size. So the engine gets one public method, `reconcile_facts(graph, branch,
facts, host)`. It needs write authority and returns the head plus `assert`
and `correct` changes, omitting identical facts. It reveals no more than a
`write` would: absent, identical or different, never a stored version. It is
an engine API, not a contract command or query operator.

### 2026-10-19 — #36 A5 — The reference schema is an engine graph schema

The model is a `GraphSchema` (`weave.hardware`, revision 1) with twelve node
types and fourteen edge types. The importer commits it as the graph's first
(snapshot) revision, and every later fact write inherits and is validated
against it, so the engine rejects records that do not fit the model, whoever
writes them. Node and edge `type_id`s are the type names and predicates. All
nodes are in space `hw`, and the entity ID equals the node ID, because the
model has one manifestation per entity.

### 2026-10-19 — #36 A5 — Row keys are the idempotency keys; a row is one claim

Each table has a key column, and every fact ID the row produces derives from
it (`unit:<serial>`, `runs:<install_id>`, `note:<host>`). Claimed provenance
is part of fact content, so its fields are chosen to be stable. The source is
the row's `source` column, else the batch source, else a per-table default
(`erp`, `plm`, `fleet-ops`, ...), never a file name or run ID. The method is
`import` with importer `weave-hardware` and version equal to the mapping
revision, not the binary version. Re-sending a row with another source
re-attributes every fact of that row, because the row is one claim. Readers
are sorted and deduplicated, so their order in an export does not matter.

### 2026-10-19 — #36 A5 — Interval relations are their own tables; no deletion by omission

Placements, ownerships, configuration assignments, part installs, firmware
installs and sensor mounts each have their own table, with one row per
interval. That is how a firmware rollout, a move or a part replacement
becomes a correction (`valid_to` set on the current row) plus an assertion
(a new row), without rewriting history. Sensors started with an embedded
mount, but were split into `sensor_mounts` so a sensor can move between
units. The importer never retracts facts missing from an export: an export
can be partial, and ending a relation is a valid-time fact. Imports above
100,000 changes are written as several head-guarded revisions. Each is
atomic, and re-running completes an interrupted import.

### 2026-10-19 — #36 A5 — Restricted readers pin revisions instead of using recorded-time selection

`recorded_query` admits an observation only for a reader who can see the
whole selected revision (`authorize_observation`, ADR-0002). In a fleet with
customer-restricted units, `fleet-analyst` therefore gets
`E_HISTORY_UNAVAILABLE` for any "as known at K" read. A5 does not change that
rule, because time travel is A3 scope. The query helpers take
`At::Revision` as well as `At::Recorded`, and the scenario answers "as known
at K" for the analyst by pinning the revision the trusted importer wrote at
K. It checks that the admin gets identical answers both ways and that the
analyst's recorded read fails with that code. This is flagged for A3 and A6
exit check 5.

### 2026-10-19 — #36 A5 — A deterministic generator with a pinned fixture; datasets stay out of git

The generator uses its own SplitMix64, integer millisecond arithmetic and
ordered maps, and writes `\n` line endings, so output is byte-identical on
every platform. `fixtures/fleet-small` (seed 7, 12 units, 24 updates, about
100 KB) is committed and must equal a fresh run, and the default 5,000-unit
digest is pinned in a test. The default dataset (about 11 MB of CSV) is
regenerated on demand, not committed. Each update has a `reported_at`, which
is not recorded time. The scenario uses it to drive a `ManualClock`, so "as
known at K" has an exact oracle. Every fifth customer (C003, C008, ...) is
restricted to `fleet-admin` and its own principal, for A6 exit check 5.

### 2026-10-19 — #36 A5 — The full-size scenario runs in its own CI job

`tests/fleet_scenario.rs` runs at 150 units and 150 updates in the debug
matrix, and at 5,000 units and 1,000 updates optimized in the
`hardware-fleet-5k` job (about 90 s locally). That job is separate, as with
`fact-scale-1m`, so a slow runner there cannot hide failures in the main
matrix.

### 2026-10-20 — CI — Live governance fixture windows outlast the job, not seconds

The one-off Windows failure of `check_history_source.py` (`E_GOV_HISTORY_UNAVAILABLE`
on the `old` query, PR #54 run 39327688334 attempt 1) was wall-clock expiry of the
`source_view_fixture` policy, not clock ties, SQLite locking or test order. In
`live_*` modes the fixture installed a SystemClock policy valid for 10 s after
`live_seed`. Accepted-history reads check the current head policy and fail closed
once it expires, so a slow runner (the failed step took 14 s; the passing re-run
took 2 s) turned a correct query into "unavailable". A second, tighter window hid
behind it: `live_publish` proposed with expiry `own sample + 9 s`, which must not
exceed `seed sample + 10 s`, so publishing more than 1 s after seeding failed with
`E_GOV_POLICY`. Both reproduce locally by delaying one fixture process.

The fix is in the trusted harness only: live policies now last 60 minutes and
live proposals and approvals 30 minutes, both longer than the 20-minute job
timeout. ManualClock modes keep 10 s and 9 s, which the `expired` mode depends on.
The engine checks are unchanged, and two tests now pin them: history fails closed
exactly at policy expiry, and a proposal may not outlive its policy. No retries
were added. The script reports elapsed seconds on a failed invocation so a
recurrence shows wall-clock drift immediately.

### 2026-10-19 — #4 A3 — Delivery in two parts; protocol 0.23.0, store unchanged

Part 1 adds the contract and engine operators (`expand`, `as_of`, `changes`),
explanations, coverage, the paired hidden-input tests, the 100k-fact BFS
check, the change-set property test, the as-of benchmark, golden fixtures and
the spec. Part 2 adds the Python helpers, the science JSON operations and the
tutorial, and closes #4. The operators are new expression kinds and one new
optional result field (`QueryResult.changes`), so this is the additive minor
bump 0.23.0. Nothing new is stored: the change set is computed from the fact
log and indexes that store 30 already has, so `STORAGE_VERSION` stays 30.

### 2026-10-19 — #4 A3 — `expand` is a graph value; `changes` is a terminal change set

`expand` returns an ordinary `QueryResult` with exact origins, so it composes
with every operator (A4's "restricted to the 2-hop neighborhood" is a join or
filter over it) and `fact_provenance` explains it without new machinery. The
edge set is every matching edge that leaves a node fewer than `hops` hops
away, which is what an independent BFS produces and what "how was it related"
needs: edges between two nodes at the last hop are not included. A change set
is not a graph: a corrected fact has two values. It goes in a new optional
`QueryResult.changes` field with authorized `before` and `after` values, and
operators reject a change set as input (`E_CHANGES_OPERAND`) rather than
silently dropping the field. The reserved names `expand` and `changes` are
kept.

### 2026-10-19 — #4 A3 — A change set is the difference of two authorized states

`changes(K1, K2)` lists, fact by fact, what differs between the two states a
reader can see, not every intermediate log entry. This is the acceptance
criterion's definition, it is what a pull feed consumer needs (apply the
entries to reach the new state), and it makes permissions simple: both states
are authorized under current authority, exactly as two pinned reads. A fact
whose visibility changes because a dependency changed (an edge whose endpoint
became hidden) is reported as the reader sees it. The cursor is the `to`
revision ID, so a cursor works on any revision of the graph and needs no new
stored state.

### 2026-10-19 — #4 A3 — `as_of` needs no whole-graph visibility and returns no witness

`recorded_query` requires the selected revision to be wholly visible and
returns observation witnesses, because the witness chain describes the head
history. `as_of` only maps a recorded time to a revision through the
`(graph, branch, recorded_at_ms)` index and then authorizes that revision
record by record, like a read pinned to it, which any reader can already do
with the revision ID. Requiring whole visibility would make `as_of` fail for
every reader with any hidden record, so hidden facts would change the output
(an error instead of a result). The branch head must be readable, so missing
and denied graphs stay indistinguishable. `recorded_query` remains the way to
get a replayable checkpoint.

### 2026-10-19 — #4 A3 — Coverage is attributed to the output's own region

Authorization reports one "incomplete" flag per value. On a subset chosen by
traversal, that flag would depend on hidden facts: a hidden edge pulls its far
endpoint into the subset, and a denied attachment there would make the result
partial only in the store that holds the hidden edge. The operators therefore
authorize and attribute denials per record: a record is *denied* when its
readers admit the principal but authorization removes or narrows it, and a
record whose readers exclude the principal is simply absent. `expand` reports
partial coverage only for denied candidate edges at visible frontier nodes and
denied attachments on returned records; `changes` only for denied facts whose
stored content changed. Header-level gates still make any value partial.

### 2026-10-19 — #4 A3 — Paired tests normalize revision IDs

Revision IDs are content-derived over every fact (ADR-0001, ADR-0004), so a
store with extra hidden facts has different revision IDs and a hidden write
creates a new one. Hiding them would mean a second, visibility-dependent
identity scheme. The paired tests compare all output with revision IDs
normalized, as `tests/fact_reads.rs` already does, and the spec states the
exception: a revision ID reveals that something changed, never what.

### 2026-10-19 — #4 A3 — One read path for fact and snapshot revisions

`FactSource` gives the closure and traversal code one interface over a fact
revision (the backend's indexes) and a snapshot revision (its payload indexed
in memory by the same keys). Both operators and the existing filtered-query
closure use it, so the two revision kinds cannot drift; the BFS test checks
both kinds give identical results. Changed keys between two revisions of one
line come from a new `FactBackend::changed_keys`, which reads the logged
operations in the revisions' position range, so the change set costs O(changes
and their dependents) with no schema change. Any other pair falls back to
comparing both states whole, within the read budget.

### 2026-10-19 — #4 A3 — Key lookups probe one version

The as-of benchmark showed single-fact writes slowing linearly with the
number of versions of the corrected key: the SQLite backend's live-version
lookup scanned every version of the key at or before the read point. It now
takes the version with the greatest `from_seq` at or before the read point,
one index probe, because versions of a key on a line never overlap. Results
are unchanged (the conformance suite passes on both backends). Selections by
subject still scan every version recorded under that subject, so a node
corrected very many times makes reads of its hosted attachments slower; a
composite index needs a store bump and is left to C4.

### 2026-10-19 — #4 A3 — Bounds and defaults

`hops` 1 to 16, `seeds` 1 to 1,000 and `limit` (nodes plus edges, or change
entries) 1 to 200,000 with default 10,000, matching the bound-value budget of
`bind`. Each limit fails with `E_BUDGET`; `E_EXPAND` is for an empty seed list
or zero hops, which are malformed rather than too large. View and handler
artifact protocols advance to 0.23.0 with the protocol, as in A2, and still
accept 0.22.0 and older artifacts. Views and handler recipes reject `expand`,
`changes` and `as_of`: their freshness and egress models do not cover a
traversal frontier or a change set yet.

### 2026-10-19 — #4 A3 — 100k-fact acceptance runs in the release CI job

`tests/traversal_scale.rs` runs at 100,000 facts in the existing
`fact-scale-1m` release job (about 25 s) and at 20,000 in the debug suite,
where 100,000 takes over two minutes. Each check compares one `expand` with
an independent BFS over the generated edges.

### 2026-10-19 — #4 A3 — An `as_of` expression for every query; ADR-0002 rule kept for witness APIs

A5 found that "as known at K" (`recorded_query`) fails with
`E_HISTORY_UNAVAILABLE` for any reader who cannot see the whole selected
revision, such as a fleet analyst with customer-restricted units. The Phase A
exit scenario needs those answers. Protocol 0.23.0 therefore adds
`{kind: "as_of", query, unix_millis}`: any `QueryPlan` read from the revision
selected through the recorded-time index and authorized record by record, so
the result equals the same query pinned to that revision. A pinned read is
already available to every reader, so `as_of` reveals no more than it: no
witness chain, no observation times and no hidden records. The revision ID is
the one remaining signal and a head read exposes it too.

`recorded_query`, `recorded_range` and checkpoints keep ADR-0002's
whole-revision rule (amended there), because their observation witnesses
describe every head change, including hidden writes. Callers that need
replayable checkpoints still use them.

### 2026-10-19 — #4 A3 — Hidden records never make an as-of result partial

The request was to mark partially visible as-of reads `partial`. That would
make coverage depend on whether hidden records exist: a reader could tell
"some unit is restricted from me" from `complete` versus `partial`, a one-bit
existence oracle. Core v1 §9 already defines `complete` as describing the
authorized input, not the world, and a pinned read of the same revision is
`complete`. `as_of` keeps exactly that coverage. It is `partial` only for
derivations a reader is denied (`E_DERIVATION_UNAVAILABLE`), as every read.
The paired tests compare as-of output, coverage included, with and without
hidden facts.

### 2026-10-19 — #4 A3 — The JSON interface exposes the operators as Program expressions

The science envelope gets no new `operation`. `expand`, `as_of`, `changes`
and `explain` are graph expressions, so `execute` and `analyze` already carry
them in 0.23.0 Programs, and `capabilities` now lists them under
`query_operators` with their semantics. Separate operations would be a second
request schema for the same plans and would have to repeat the engine's
validation. `analyze` rejects a change set (`E_SCIENCE_RESULT`): it is not a
graph, and analyzing its empty `graph` would silently report nothing.

### 2026-10-19 — #4 A3 — Python helpers replay exactly through pinned programs

`Engine.expand`, `Engine.as_of`, `Engine.changes` and `Engine.explain` wrap
expression builders of the same names, which validate their arguments before
any process starts. `pinned_program` rewrites each new kind to the exact state
it read: an `as_of` read becomes the query pinned to the revision it selected
(the two give the same result by construction), an expansion pins its revision
in place of `as_of`, and a change set pins both ends as cursors. An end that
was the empty state keeps its `as_of` time, which selects the same empty state
on every replay. `QueryResult` gains `fact_provenance`, `changes` and `cursor`.

### 2026-10-19 — #4 A3 — The fleet scenario uses `At::AsOf`; `At::Recorded` stays

`weave-hardware`'s `At` gains `AsOf(k)`, which runs the A3 `as_of`
expression. The scenario now checks for both readers that "as known at K" by
recorded time equals the pinned-revision answer, coverage and provenance
included, so the analyst gets the visible subset. `At::Recorded` keeps using
`recorded_query`, and the scenario still checks that it returns witnesses to
the admin and `E_HISTORY_UNAVAILABLE` to the analyst, so ADR-0002's rule
stays covered.

### 2026-10-20 — weave-language#16 — The compiler emits 0.23.0

The compiler's emitted protocol label follows its vendored contract, now
0.23.0. Its commands are unchanged and the engine accepts every earlier
version, so emitting the vendored version keeps both repositories on one
label. The engine's integration checks pin weave-language `5729212`.

### 2026-10-20 — weave-language#16 — `expand`, `as_of` and `changes` are classified core v1

`weave describe --contract` classifies the three A3 expression kinds as
core v1, matching core-v1.md §12. The classification is an exhaustive match,
so the vendored variants had to be classified before the build passed.

### 2026-10-20 — weave-language#16 — No script tolerance added

The language scripts needed no version tolerance or special case for 0.23.0.
Every compiler-runtime and Linux compiler-host check passed unchanged against
engine `5976d47`.

### 2026-10-20 — weave-language#16 — Source syntax for the A3 operators is deferred to LA1

The vendor sync adds no source syntax for `expand`, `as_of` or `changes`.
Programs can carry them; the query language that lowers to them is LA1.

### 2026-10-20 — CI — Path-filtered workflows also trigger on their own file, scripts and the lockfile
#58 bumped the pinned weave-language ref in `integration.yml`, but the integration workflow did
not run because its path filter excluded the workflow file itself; the orchestrator had to dispatch
it manually before merging. `integration.yml`, `compatibility.yml` and `fuzz.yml` now also trigger on
their own workflow file and `Cargo.lock`, and integration/compatibility on `scripts/**` (they run
engine check scripts such as `check_history_source.py`, changed in #55 without triggering them).
