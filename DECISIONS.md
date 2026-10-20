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

### 2026-10-20 — #37 A7 — A minimal hand-written MCP subset, not an SDK

`weave-mcp` implements the part of MCP a tools-only stdio server needs:
newline-delimited JSON-RPC 2.0 with `initialize`, `ping`, `tools/list`,
`tools/call` and ignored notifications, in a few hundred lines over `serde_json`.
The official Rust SDK (`rmcp`, on crates.io, MIT/Apache-2.0) is mature
enough, but it is async, needs `tokio` and brings a large dependency tree into
a workspace that has none of it, to serve four synchronous methods. A small
subset is also easier to bound: every message, argument and result has an
explicit limit and a stable code. B4 can move to `rmcp` if it needs the HTTP
transport or authorization flows; the tool layer (`Tool`, `expression`,
`Server::call_tool`) does not depend on the transport.

### 2026-10-20 — #37 A7 — Each tool is one read-only `evaluate` through `ScienceSession`

A tool call parses its arguments into one contract `GraphExpression` and runs
`{"version": "0.23.0", "commands": [{"op": "evaluate", "value": …}]}` through
the same `ScienceSession::handle` as the science `execute` operation. The
`result` field of a tool result is that operation's result, and engine errors
pass through with their code and message. The criterion "every tool result
matches the equivalent JSON API result" therefore holds by construction, and
`tests/mcp.rs` checks it on every tool for two readers, errors included. The
preview has no semantics of its own to drift.

### 2026-10-20 — #37 A7 — Tool arguments are the contract JSON

`query` takes a `QueryPlan`, `expand` an `ExpandPlan`, `what_changed` a
`ChangesPlan`, `as_of` `{unix_millis, query}` and `explain` `{input}` with a
`query`, `as_of` or `expand` expression. They are parsed with the contract's
`deny_unknown_fields` types, so an argument can express nothing the tool does
not offer, and the guides for those expressions apply unchanged. The preview
adds only envelope bounds (1 MiB messages, 32 levels of nesting, 4,096-byte
strings, `E_MCP_INPUT`). Seeds, hops and limits are left to the engine, so a
bound fails with the same code (`E_BUDGET`, `E_EXPAND`) as in the JSON API.

### 2026-10-20 — #37 A7 — `what_changed` is a sixth tool

Exit question 2 asks "what changed for unit U between K1 and K2", which only
the `changes` expression answers. None of the five listed tools can carry it
without becoming a general expression evaluator, so the preview adds
`what_changed` with exactly the `ChangesPlan` arguments. The name is the one
B4 uses for the same tool. `explain` does not take a change set, because the
engine rejects one as an operand (`E_CHANGES_OPERAND`).

### 2026-10-20 — #37 A7 — `hybrid_search` is listed and fails with `E_UNSUPPORTED`

The issue asks that a tool whose operator has not landed reports itself
unavailable rather than approximating. The preview lists `hybrid_search`, so
an agent can see the full intended surface, says so in its description and
fails every call with `E_UNSUPPORTED`. It is not approximated by exact kNN plus
`expand`. The A4 part replaces it with the A4 operator.

### 2026-10-20 — #37 A7 — The reader is required, and the server cannot write or create

`--actor` is required and has no default. It must be nonempty, at most 256
bytes, without control characters and outside the reserved `weave:`
namespace. The engine has no all-seeing principal, but a default would make
the preview silently read as whoever the default is. There is no `--write`
flag: the session's `HostContext` has no write grants, so even a write
command, which no tool can produce, would fail with `E_FORBIDDEN`. `--db` must
name an existing file; a read-only preview never creates a store.

### 2026-10-20 — #37 A7 — The data/code boundary of the preview

Tool names, descriptions, schemas and the `initialize` instructions are
static strings; nothing from the store is placed in them. Store content is
returned only as JSON under `result` (or in `error.message`) in an envelope
with `"trust": "untrusted_data"` and a fixed notice, both as
`structuredContent` and as the same JSON in one text block for older clients.
Tool failures are `isError` results so an agent can correct its arguments;
an unknown tool is a JSON-RPC `-32602` error with `data.code:
E_MCP_UNKNOWN_TOOL`. B4 extends this boundary to writes: there, injected text
must also be unable to trigger writes or change permissions.

### 2026-10-20 — #37 A7 — Strict JSON shared with the science CLI

Messages are parsed with the science interface's strict parser, now exported
as `weave_science::parse_strict_json`, which rejects duplicate object keys.
Otherwise one message could mean different things to the client, which might
keep the first key, and to the server, which would keep the last.

### 2026-10-20 — #37 A7 — "No tool writes" compares store rows

Opening a store bumps the SQLite file change counter, so the file bytes
differ after any session even when nothing is written. The no-write test
therefore compares every row of every table, with the table definitions,
before and after a scripted session of the real binary that calls every
listed tool, including calls whose arguments try to smuggle write commands.

### 2026-10-20 — #37 A7 — Integration checks pin weave-language `bb5e559`

At the coordinator's request this PR moves both weave-language checkouts in
`integration.yml` to `bb5e559` (weave-language#17, LA1 part 1: time-travel,
`expand` and `changes` syntax), and adds `check_traversal.py --compiler
--engine` to the `compiler-runtime` job after `check_temporal.py`, which takes
the same arguments.

### 2026-10-20 — #13 A4 — Delivery in two parts; protocol 0.24.0 and store 31

Part 1 activates the ADR-0006 records on the wire (fact kinds, `GraphData`
fields, `nearest`, `similar_candidates`, `ExpandPlan.seeds_from`,
`EvidenceRef::Embedding`, the `neighbors`, `candidates` and `exactness`
result fields), the engine operators, the migration from the `vector`
property, golden fixtures, the spec, paired hidden-input tests and exit
questions 3 and 4 on the fleet. Part 2 measures 100k-vector upserts at 1,536
dimensions and adds the Python helpers and tutorial, which wait for the A3
part 2 Python work to merge. Every wire addition is optional and omitted when
empty, so this is the additive minor bump 0.24.0. The store moves to 31
although no table changes: the fact log now holds kinds a store-30 binary
cannot parse, and the marker makes that binary refuse the store with
`E_STORAGE_VERSION` instead of failing later with `E_INTEGRITY` on one graph.
The migration only moves the marker; a genuine store-30 binary checks it in
`scripts/check_embedding_store_migration.py`.

### 2026-10-20 — #13 A4 — Spaces are immutable facts of the graph, keyed `id@revision`

A space must be part of the revision (as-of reads, change sets and exports
see which spaces existed), and `write` cannot change the line header. So a
space is a fact of kind `embedding_space` in its own key namespace. Its key
is `id@revision`; a space revision may not contain `@`, which makes the key
unambiguous within the 512-byte bound. Tightening `EmbeddingSpace::validate`
is allowed because no published version could carry the record.
`Fact::key_id` returns the key; `Fact::id` keeps returning the record's `id`
field so existing callers do not change. A space is never corrected to a
different descriptor or retracted (`E_CONFLICT`). There is deliberately no
cross-graph registry like the schema registry: a conflict there would tell a
writer that a graph it cannot read declares that space. Comparisons across
graphs still need equal descriptors (`E_SPACE`).

### 2026-10-20 — #13 A4 — Embeddings only through `write`; ordinary reads never return them

Snapshot commits that carry spaces or embeddings fail with `E_EMBEDDING`.
Fact-revision materialization (`query`, algebra, checkpoints) selects only
graph kinds, and closures never pull embeddings in, so a graph with many
vectors costs ordinary reads nothing and their results do not change. The
`nearest`, `similar_candidates` and `changes` operators read embeddings
through the fact index: the space key is the index `predicate`, the subject
is `subject` (or `entity` for entity subjects) and the valid time is
`valid_from`/`valid_to`. Values that another operator takes as input lose
their embeddings, ranking and candidates: the input is the graph records.

### 2026-10-20 — #13 A4 — `valid_at` applies to the embedding, not the subject's node valid time

An embedding's valid time is when that representation holds. A maintenance
note's node is valid while the work happens, but its text stays true
afterwards; requiring the node to be valid at the incident time would make
exit question 3 return nothing. So `nearest` selects on the embedding's valid
time and requires the subject to be visible, not valid at `valid_at`. A
candidate set (`expand` or `filter` with `valid_at`) restricts subjects by
time when that is wanted.

### 2026-10-20 — #13 A4 — Normative distance and total rank order

Scores are computed in binary64 from the binary32 components, summing in
component order (`EmbeddingMetric::distance`, in the contract so the
language and any oracle share it). Equal vectors therefore give bit-equal
scores, and the order (score, subject kind and ID, embedding ID) is total, so
ties and near-duplicates rank deterministically. The oracle test writes the
formula independently and compares scores bit for bit. A cosine space
rejects the zero vector because its distance is undefined.

### 2026-10-20 — #13 A4 — The ranking boundary for C5

Ranking goes through one internal trait (`NeighborSearch`) that receives
the already-selected, already-authorized embeddings. The exact scan is the
only implementation and results always say `exactness: exact`. An
approximate index (C5) implements the same trait, so it can never see or
rank a hidden vector, and falls back to the exact scan; the request carries
no mode yet, so adding one later is additive.

### 2026-10-20 — #13 A4 — Hidden input and errors

Embeddings are authorized before ranking with their subject and gates, the
`k` and `limit` bounds count visible output, and the comparison bound of
`similar_candidates` counts visible embeddings. A missing or hidden query
fact both give `E_UNAVAILABLE`, and an undeclared space gives `E_SPACE` for
every reader, since spaces are public descriptors. `exclude_linked` only
considers visible edges, so a hidden edge never removes a candidate. As in
A3, two things remain outside the claim and are documented: the read budget
counts the stored records an operator reads, hidden ones included, and
revision IDs are content-derived over every fact. The paired tests compare
all output, errors included, with revision IDs normalized.

### 2026-10-20 — #13 A4 — Candidates: unordered pairs with stable IDs, never written

A metric is symmetric, so a pair is reported once from the lower subject to
the higher, with the intersection of the two valid times; targets whose
valid time does not overlap the source's are not eligible. The ID hashes the
graph, the space and both embedding IDs, not the revision, so a client can
track a candidate across revisions while `origins` pin the exact facts.
Candidates are never written: persisting one is an ordinary edge with a
predicate of the caller's own and `EvidenceRef::Embedding` evidence, so the
engine-owned `weave:similar_to` never appears in stored data. `expand` has no
option to follow candidates; "explicitly requested" means evaluating
`similar_candidates`.

### 2026-10-20 — #13 A4 — Vector-to-graph composition through `ExpandPlan.seeds_from`

`expand` took only literal seed IDs, so "nearest chunks, then their units,
then their sites" could not be one plan. `seeds_from` adds the nodes of any
value (by origin in the expand's graph) to the seeds; `seeds` may then be
empty, and an empty seed value gives an empty, complete neighborhood unless
the value itself was partial. `ExpandPlan` loses `Eq` because it may now hold
an expression; nothing in either repository compares plans with `Eq`.
The candidate set of `nearest` and the sources and targets of
`similar_candidates` use the same origin rule: nodes, their entities and
edges whose origin is the operator's graph.

### 2026-10-20 — #13 A4 — Explicit, idempotent migration from the `vector` property

`Engine::vector_property_changes` plans, and never writes, one embedding per
visible node with the property: binary32 rounding is counted and reported,
the node's readers are copied, and the provenance is `method: import` with
the node as evidence at the revision that recorded its current version (not
the head), so running it again after the write plans nothing. Any malformed
value fails the whole plan with `E_SPACE`. The property and science
`nearest_vectors` are untouched, as ADR-0006 requires.

### 2026-10-20 — #13 A4 — Exit questions 3 and 4 in a sibling test with a hashing embedder

`fleet_scenario.rs` is being extended by the A3 part 2 work, so questions 3
and 4 live in `tests/fleet_hybrid.rs` on the same generated fleet. The test
embedder is signed feature hashing of words into 64 dimensions: deterministic
on every platform, no network and no model, and similar wording still gives
small cosine distances. Notes are embedded on their maintenance or incident
node with the row's readers. Question 3's oracle ranks independently over the
vectors the test wrote, within the reader's `expand` region (A3 checks
`expand` against a BFS oracle); question 4's oracle recomputes every unit's
nearest neighbor. Many units have identical profiles, so the top candidates
are often exact ties, which the deterministic order resolves. The test runs
at 150 units in the main suite and at 5,000 in the `hardware-fleet-5k` job.

### 2026-10-20 — #13 A4 — Older change sets omit the new kinds

A `changes` expression of a 0.23.0 program on a graph that now holds
embeddings would otherwise return `embedding` and `embedding_space` entries,
enum values a 0.23.0 consumer must reject (core v1 §9). Saved plans keep the
semantics of their version, so results of programs older than 0.24.0 drop
those entries. The change-set `limit` is applied before that, so such a
program can still fail with `E_BUDGET` because of embedding changes; this is
documented in the 0.24 note rather than computing a second, version-specific
change set.

### 2026-10-20 — #37 A7 — `hybrid_search` takes a `nearest` or `similar_candidates` expression

`hybrid_search` is one tool whose arguments are one A4 expression with its
`kind`: `nearest` (optionally restricted to a neighborhood through
`candidates`, valid at `valid_at`, as known at `as_of`) or
`similar_candidates`. Both are parsed with the contract types, so the tool
adds no semantics and its result equals the science `execute` result, like
the other tools. Vector-to-graph composition stays on `expand`, through
`seeds_from`. One tool keeps the surface the issue names, and the `kind`
makes the mode explicit to the agent. `E_UNSUPPORTED` is gone, since no
listed tool lacks its operator.

### 2026-10-20 — #37 A7 — Nested expressions are limited to the preview's read operators

A4 added `ExpandPlan.seeds_from`, a nested `GraphExpression`. Without a check,
the `expand` tool would have accepted any expression there, including ones
the preview does not offer (references, rules, views). Every nested
expression (`seeds_from`, `candidates`, `sources`, `targets` and the `explain`
input) must now be `query`, `as_of`, `expand`, `nearest` or
`similar_candidates`, at most 4 levels deep, or the call fails with
`E_MCP_INPUT`. `changes` stays out because no operator takes a change set.

### 2026-10-20 — #37 A7 — The preview has no encoder

Weave does not generate embeddings (EMBEDDINGS.md), and neither does the
preview. An agent passes a query vector from the space's encoder, or names a
stored embedding with `{kind: "embedding", embedding_id}`. A text-to-vector
tool would put a model choice inside a read-only adapter, which belongs to
the host or to B4.

### 2026-10-20 — #37 A7 — The exit-scenario test drives the real binary

`fleet_exit_questions.rs` spawns `weave-mcp` and speaks JSON-RPC to it, as an
agent's MCP client would, rather than calling the server in-process. Scores
read back from the wire are compared with a tolerance of 1e-12: `serde_json`
without `float_roundtrip` can parse a printed `f64` one unit in the last place
off, as the first run showed (`1.1102230246251563e-16` against
`…565e-16`). IDs and their order are compared exactly. In-process tests keep
comparing scores exactly.

### 2026-10-20 — #16 A6 — The exit scenario is one script over the three existing scenario tests

`fleet_scenario.rs`, `fleet_hybrid.rs` and `fleet_exit_questions.rs` already
hold independent oracles for every exit question. A new single test would
have copied those oracles, or moved them into a shared support crate that
the A4 part 2 work is also editing. Instead, `scripts/check_phase_a_exit.py`
runs the three tests optimized at the exit size, and each test prints one
`PHASE_A_EXIT {json}` line only after all of its assertions have passed. The
script fails unless every scenario clause is reported by the tests listed for
it, and unless the three tests report the same dataset digest and the same
content-derived head revision after the 1,000 updates. The second condition
shows that they answered over one fleet state, not three similar ones. The
result is one pass or fail with a JSON report, and no oracle exists twice.
The MCP test was fixed at 60 units; it now takes `WEAVE_FLEET_UNITS` and
`WEAVE_FLEET_UPDATES` like the other two and still defaults to 60.

### 2026-10-20 — #16 A6 — `phase-a-exit` replaces `hardware-fleet-5k` and runs on three OSes

Issue #16 asks for the scenario to pass in CI on Linux, macOS and Windows.
The new `phase-a-exit` job runs the script on all three, with a 90-minute
timeout. It stays a separate job so that a slow runner cannot hide failures
in the main matrix. It already runs the two tests that `hardware-fleet-5k`
ran, at the same size, so that job was removed rather than running them
twice on Linux.

### 2026-10-20 — #16 A6 — Scenario gaps closed in the tests, not in the script

A review of the three tests against the scenario text found three gaps. The
MCP `what_changed` answers were not checked for source facts. Restricted
units were not checked as hidden in MCP change sets. `fleet_scenario.rs` did
not reproduce "what changed for U" after reopening the store. All three are
now assertions in the tests. None needed an engine change: every check passed
on first run at 5,000 units.

### 2026-10-20 — #16 A6 — Fuzz targets share a seeded graph with a hidden-word leak oracle

The four new targets (`fact_write`, `traversal_plans`, `vector_plans`,
`mcp_jsonrpc`) run over one small seeded fact graph with three writes at
recorded times 10, 20 and 30. Every fact readable only by `admin` contains
the word `secret` in its ID or content. For the analyst, no output, result or
error may contain that word unless the fuzz input contains it. This is a
cheap noninterference check that covers error messages too. `check_seeds`
proves the check is not vacuous: the admin does see `secret` through the
seeds of each read target. The read targets also require evaluation to be
deterministic and to leave the head and event count unchanged. The plan
targets decode whole programs with `decode_program`, so version-first
decoding is fuzzed too, and they keep only programs whose top-level commands
are the target's operators. Nested expressions are left free, so
compositions like `nearest` over `expand` are reached. The write target uses
a fresh store per input and checks atomicity (a failed write changes nothing
and the line stays writable), readability after success, and determinism
across two stores. The MCP target writes the seeded store to a temporary
file, because the server owns its session. A second engine on that file
checks that no message changes the store.

### 2026-10-20 — #13 A4 — Exact `nearest` streams under its own scan budget

The 128 MiB read budget capped one whole-space ranking at roughly 7,000
vectors of 1,536 dimensions, too low for hybrid RAG and for the issue's
100k × 1,536 target. `nearest` now streams the selected embeddings through
`FactBackend::scan` (a new trait method: the SQLite backend iterates rows,
the reference backend materializes), authorizes them in chunks of 256 with
their subjects through the one authorization path, and keeps only the best
`k`, so memory is O(k) plus one chunk. Embedding bodies it streams are not
charged to the read budget; instead every stored embedding it reads counts
its dimensions against `MAX_SCAN_COMPONENTS` = 2^28 components (about
174,000 vectors of 1,536 dimensions, `E_BUDGET` beyond), a contract constant
so clients can see the ceiling. Subject records and gates are still charged
to the read budget. The scan budget counts hidden embeddings too, the same
documented exception as the read budget; nothing else about hidden vectors
reaches the output, because each chunk is authorized before ranking and the
merged top-k uses the same total order, which a chunk boundary cannot change
(`streaming_nearest_spans_chunks_and_ignores_hidden_vectors` checks it across
chunk boundaries with hidden duplicates of every visible vector). The
measured whole-space top-10 at 100k × 1,536 is about 7 s on one host, mostly
JSON decoding of stored vectors; a binary vector encoding or an index is a
C4/C5 decision, not this epic's. `similar_candidates` keeps the read budget:
pairwise ranking needs every target in memory anyway.

### 2026-10-20 — #13 A4 — Python helpers name the typed operator `nearest_embeddings`

`Engine.nearest` already means the science analysis over the untyped
`vector` property, and changing it would reinterpret existing notebooks. The
typed, in-plan operator is `Engine.nearest_embeddings` (builder
`nearest_embeddings`), next to `similar_candidates`, `embedding_space`,
`embedding`, `asserts` and `expand(seeds_from=...)`. Exact replay pins the
revision of embedding rankings and of their nested inputs. The tutorial is a
runnable example with a toy hashing encoder standing in for the caller's
model, run by the native Python tests.

### 2026-10-20 — #16 A6 — Store-29 evidence: a third job, covering both routes

`store29-fact-store-migration` already upgrades a store 29, populated by the
exact store-29 binary, to the current store. It checks kills before and after
the one schema transaction, refusal by the store-29 binary, and byte-identical
pinned queries. It does not cover the route through store 30, where a user ran
the store-30 binary before upgrading again. It does not check that the
store-30 binary refuses store 31 built from store-29 data. And it does not show
the Phase A operators working on store-29 lines. The new `store29-to-current`
job (`check_store29_to_current.py`) adds all three. It upgrades one store-29
population directly and stepwise. It kills each step before and after its
commit and checks every older binary's refusal. It reruns the pinned queries
with the version of the binary that pinned them, then runs write, `nearest`,
`as_of`, `expand` and `changes` over the migrated lines with the store-29
reader restrictions. The two existing jobs stay as they are, because their
scripts are the per-step evidence for ADR-0004 and ADR-0006. No engine change
was needed: every check passed on the first run.

### 2026-10-20 — #16 A6 — One Phase A benchmark example, recorded after streaming nearest

`examples/phase_a_benchmark.rs` measures every Phase A path on one graph of
each size, 100k and 1M facts. The paths are single-fact commit, 2-hop `expand`,
`as_of` `expand`, `changes` over 1 and 199 facts, and `nearest` over the whole
space and inside a 2-hop `expand`. The graph is two fifths nodes, two fifths
edges and one fifth 32-dimensional embeddings. The A2 and A3 benchmarks stay,
since they measure growth with history length, which this one does not. The
recorded report was run after A4 part 2 (#62, streaming `nearest`) merged.
A run before it gave the same `nearest` medians within 10%: with
32-dimensional vectors the cost is per row, not per vector component. #62
records its own 100k × 1,536 measurement. Exit-scenario timings come from
the `phase-a-exit` CI reports of the merge commit on all three OSes, plus one
local run. The CI times include the release build.

### 2026-10-20 — #16 A6 — The README quickstart uses the hardware CLI and a stdlib script

#16 asks for the hardware model plus hybrid retrieval in under 10 commands.
The quickstart is five: build, `weave-hardware generate`, `import` and
`apply-updates`, then `examples/hardware/hybrid_quickstart.py`. The script
embeds the notes with the tests' word-hashing scheme and writes them through
`weave-science import`. It then runs one `nearest`-over-`expand` plan and one
`similar_candidates` plan, printing each hit's source. It speaks the
`weave-science` JSON protocol directly and uses only the standard library, so
it does not depend on the Python helpers that A4 part 2 is adding. Weave does
not generate embeddings, so a model-free embedder is the only way to keep the
quickstart offline. The main CI matrix runs the same commands on Linux, macOS
and Windows at 60 units, as both readers, so the README cannot drift from a
working path.
### 2026-10-20 — weave-language#18 — The compiler vendors and emits 0.24.0

weave-language#18 vendored the 0.24.0 contract from engine `5e2a1d9`, the A4
embedding facts and operators. The vendored crate has 98 files, including 29
positive and 25 negative fixtures. The compiler now emits 0.24.0, and its
emitted commands and data are otherwise unchanged. `nearest`,
`similar_candidates` and the embedding fact kinds are classified core v1, as
§12 of the core contract says. `describe --contract` walks the nested
candidate values (`candidates`, `sources`, `targets`, `seeds_from`), so an
extension operator inside one is still reported.

### 2026-10-20 — weave-language#19 — LA1 complete; `check_phase_a.py` runs in engine CI

weave-language#19 (`4634485`) adds source syntax for fact `write`s, embedding
spaces, `nearest`, `similar_candidates` and `expand ... seeds from`. With it,
all five Phase A exit questions fit in at most 15 lines of `.weave` each.
`scripts/check_phase_a.py` runs them on a real engine and compares them with
JSON references and independent oracles. Engine `integration.yml` now pins
both weave-language checkouts to `4634485` and runs `check_phase_a.py` in
`compiler-runtime`. Before the bump, the language agent ran all 44
compiler-runtime lines, `check_phase_a.py` and the 4 Linux compiler-host lines
against engine `5e2a1d9`, and they passed. **Known risk:** source writes cannot
set `readers` until LB1. A source `correct` of a restricted fact therefore
writes it without its readers, which publishes it to every reader. This is
logged on engine #5 and language #5. Until LB1, writes of restricted facts
must go through the JSON or Rust interfaces.

### 2026-10-20 — #16 A6 — Scoped `nearest` follows the candidates: an index hint, not a new path

A6's baseline showed that a `nearest` restricted to a 2-hop `expand` costs
about a twentieth of a whole-space scan at every size (249 ms at 200k
vectors). The engine already looked up each visible candidate subject's
embeddings through `FactFilter.subject` and `entity`. The cause was SQLite's
plan. With no `ANALYZE` statistics, a filter with both a subject and a space
(`predicate`) used the `fact_index_predicate` index, so every per-subject
lookup walked the whole space. `EXPLAIN QUERY PLAN` confirms it: both
two-column indexes match equally, and the planner takes the predicate
index. Any selection that names both a subject (or an entity) and a
predicate has the same tie. With a subject and a kind only, the planner
already uses the subject index.

`filter_sql` now adds `INDEXED BY fact_index_subject` when a subject is set,
or `fact_index_entity` when only an entity is set. A subject or entity is one
record's anchor, so its index is the selective one in every engine use. A
node with very many facts costs in proportion to its own facts, never to the
whole graph. The hint changes the plan only: the WHERE clause, and so the
rows, the results and `E_BUDGET` at the selection limit, are the same. No
schema change and no `ANALYZE` are needed. Statistics would be data-dependent
and could change the plan again later. Measured at 32 dimensions, a top-10 in
a 2-hop `expand` drops from 23.9 to 2.65 ms at 20k vectors and from 251 to
2.74 ms at 200k, now flat with the size of the space. The whole-space scan is
unchanged.

**Path choice and hidden input.** A plan with `candidates` always uses the
lookups, and a plan without them always scans. There is no candidate-count
threshold, so nothing visibility-dependent picks the path. The lookups run
over the subjects of the already-authorized candidate value, so hidden
candidates add no lookups. Embeddings that the lookups read are authorized in
the same chunks and count against `MAX_SCAN_COMPONENTS`, hidden ones
included, as before. That is the documented exception and is now bounded by
the candidates' embeddings.

**Equivalence.** The reference for the lookups is a full scan that keeps the
rows whose subject or entity key names a candidate. That is the same row set
the lookups select, so coverage reflects only the candidates' embeddings. A
full scan that authorized every embedding in the space before filtering would
mark a result partial for a derivation-gated embedding outside the candidate
set. That contradicts the A3 rule that coverage is attributed to the output's
own region. A `cfg(test)` switch runs this reference inside `nearest`. The
unit test `restricted_nearest_index_lookups_equal_a_full_scan_on_random_stores`
compares the two paths byte for byte. It uses 12 random stores, on both the
SQLite and the in-memory backend, with node, entity and edge subjects, ties,
valid times, reader restrictions and derivation gates, and covers `expand` and
`query` candidates, vector and embedding queries, and both readers. Removing
the entity lookups makes it fail. The paired test
`hidden_candidates_never_change_a_scoped_nearest` covers hidden input. One
known difference remains outside both paths' tested range: a single candidate
subject with more embeddings than the selection limit fails with `E_BUDGET`
in the lookup. That is unchanged from before this PR.

### 2026-10-20 — #16 A6 — Protocol 1.0.0 is a freeze, with no wire change

As decided in A1, 1.0.0 freezes core v1 once the provenance (0.22),
traversal (0.23) and embedding (0.24) fields are live. The cut adds no wire
feature: a 1.0.0 program has exactly the 0.24.0 feature set, and every
0.1.0–0.24.0 program keeps its published semantics. Stored data, content
hashes and revision IDs are unchanged, so there is no store bump. Results
report the producing engine's version (1.0.0). New handler and view
artifacts record protocol 1.0.0 (`*_TEMPLATE_PROTOCOL` = `VERSION`, as
artifact versions advance with the protocol since A2), and older artifact
protocols stay accepted. The unknown-version fixtures move to 1.1.0 (the next
minor) and 2.0.0 (the next major, still checked before structure). A positive
`protocol-1-0-0` fixture runs a write, `nearest` and `expand` under 1.0.0
with golden outcomes. The Python client and `check_science.py` expect engine
contract 1.0.0. Exact replay of an experiment record saved against a 0.24.0
engine therefore reports a contract mismatch. The replay policy is unchanged:
the program still runs, but an exact replay requires the same engine
contract. This is listed in the release notes rather than relaxed here.

### 2026-10-20 — #16 A6 — One version table; gates name the version that introduced the feature

Every protocol bump used to add the previous version to about 50 list
literals (`[VERSION, "0.23.0", …].contains(v)`) and `== VERSION`
comparisons. `weave_contract::versions` now holds the one ordered table,
`SUPPORTED_VERSIONS`, which `wire` re-exports, and `version_at_least(v,
introduced)`, `version_rank`, `is_supported_version` and `is_current`.
Converting a gate was mechanical. Every list was a contiguous suffix of
publication order (checked by the conversion script), so it became
`version_at_least(v, <oldest listed>)`. The four `!= VERSION` gates became
`version_at_least(v, "0.24.0")`, the `== LEGACY_VERSION` join gate became
`version_at_least(v, "0.2.0")`, and the capsule check `== "0.16.0"` became
`!version_at_least(v, "0.17.0")`; the last is equivalent within the
condition, which already rejects anything older than 0.16.0.

`tests/version_gates.rs` keeps every gate's old list, copied from the 0.24.0
source, and requires the same accept or reject result for every version in
`SUPPORTED_VERSIONS`, with 1.0.0 accepted wherever the current version was.
A behavioral test runs each gated feature under every version against a real
engine:

- `evaluate`, `write`, `expand`, `changes`, `nearest`, embedding writes and
  `seeds_from` each run exactly from their introducing version;
- per-fact provenance appears from 0.22.0;
- change sets of older programs omit embedding kinds.

`version_at_least` panics on an introducing version that is not in the table,
which is a programming error, never an input error.

One gate keeps exact-version semantics: `OBSERVATION_PROGRAM_VERSION` =
0.21.0 in `actor_observation.rs`. It is a stored, hashed program format and
stays pinned so receipts remain verifiable; its existing comment explains
why. A future minor version touches only the table and `VERSION`.

### 2026-10-20 — #38 B1 — Delivery in two parts; protocol 1.1.0 and store 32

Part 1 is the registry with users and groups, `reader_groups`, graph ACLs,
protected branches, Discover and revocation. Part 2 is agents: ownership,
delegated scope, `on_behalf_of` on their writes, the escalation tests and the
full permission matrix. Every wire change of the epic is in part 1
(`reader_groups` and `FactProvenance.on_behalf_of`, which part 2 fills), so
the language vendors one contract version, 1.1.0. Store 32 changes no table:
the registry is ordinary facts and `on_behalf_of` already had a column. The
marker still moves, because a store-31 binary would read group-restricted
records under its literal-reader rule and ignore graph ACLs. That is the A4
store-31 pattern.

### 2026-10-20 — #38 B1 — ADR-0007 accepted with ten amendments

The ADR was accepted as B1 started, and the amendments are recorded in its
section 15. The two with the most effect are these. Identity sets have two
parts, principals matched against `readers` and groups matched against
`reader_groups`, because a single set would let a group ID in `readers`
match its members. The registry API is a trusted engine API
(`RegistryChange`), not a `Program` command, because the ADR keeps registry
management a trusted host operation and the language never emits it.

### 2026-10-20 — #38 B1 — A registry store ignores `writable_graphs`

Once the registry is enabled, graph ACLs are the only write authority. An
intersection with `writable_graphs` would let the host narrow further, but
it would make every B3 request carry a per-graph list computed from the same
ACL, and it would give two sources of truth for writes. Local stores keep
`writable_graphs` exactly. Internal contexts that grant themselves a graph
through `writable_graphs` therefore fail closed in a registry store. They
belong to peripheral subsystems (governance internals and dispatch), which
are documented as not extended for registry stores.

### 2026-10-20 — #38 B1 — Tenant administrators are a built-in group

`weave:administrators` is created when the registry is enabled, with the
enabling principal as its first member. A group, not a flag, keeps
administrator changes ordinary audited membership facts. Only users may
join it, because Administer is not delegable to agents. The last active
member cannot leave or be suspended, so a store can never lose its
administrators.

### 2026-10-20 — #38 B1 — Registry facts are nodes and edges with restricted readers

A principal is a node whose ID is the principal ID. A membership is an edge
`weave:member` from member to group. A graph ACL entry is an edge
`weave:grant` from the subject to a node for the graph, and a protected
branch is a node with `weave:branch-writer` edges. Edge and node IDs of
composite keys are SHA-256 hashes, so they stay within 512 bytes. Readers
implement ADR-0007 section 1. A user's node has readers `[user]`, and a
group's node has `reader_groups [group]`. A membership edge has readers
`[member]`. Graph and branch records are readable only by administrators.
Every registry record also has `reader_groups [weave:administrators]`. A
group administrator reads members through `registry_members`, not through
record readers, because a change of group administrators would otherwise
have to rewrite every membership edge.

### 2026-10-20 — #38 B1 — Bounds are checked on the affected subtree; redundant changes are no-ops

Adding member M to group G re-checks the depth and closure of M and of
everything below M, which are the only principals whose closures change.
A cycle counts as a depth violation (`E_BUDGET`), as the ADR groups it with
the bounds. Adding an existing member, removing an absent one, or
protecting a protected branch is a no-op, and a write whose changes are all
no-ops returns the current registry revision and records nothing. This
mirrors the identical re-assert of `write`. A `grant` replaces the subject's
entry, so it can also lower a permission.

### 2026-10-20 — #38 B1 — Where Discover is checked

Each operator that names a graph checks graph permission before touching
the graph: `query`, `expand`, `as_of`, `changes`, `nearest`,
`similar_candidates`, recorded history, `write`, `commit`, batches and
embedding imports. Without Discover, each one returns the
nonexistent-graph error (`E_UNAVAILABLE`, "graph unavailable"). Nested
references go through the shared `protected_reference_allowed` gate, which
now also requires graph Read, so a graph the reader cannot read behaves like
an unavailable revision. Input-only errors (`E_ID`, `k = 0`) stay first,
because they do not depend on the graph. Peripheral write paths answer
`E_FORBIDDEN` both for graphs that do not exist and for hidden ones, so they
reveal nothing either.

### 2026-10-20 — #38 B1 — Engine graphs are not under graph ACLs

`weave:registry` has a fixed rule: every active principal has Read, so it
can read its own records, and administrators hold Administer. ACL changes
naming any `weave:` graph fail with `E_RESERVED_NAMESPACE`. Governance and
identity graphs keep their own policy readers. Those policies have no
`reader_groups`, so they match the request's principal as before, through
the same shared predicate.

### 2026-10-20 — #38 B1 — The AND of group clauses is encoded as a meet until B2

`visibility::intersect` returns the literal intersection when no input names
a group. That keeps the materialized-assertion bytes exactly as before. When
an input names a group, it returns the conjunction of the restricted inputs'
clauses. One record field holds one clause, so the encoding of two or more
clauses is their clause-wise meet. The meet never admits a reader that an
input denies, and it never resolves membership. An empty meet becomes the
reader `weave:no-reader`, which no registry principal can carry. It is
never public. ADR-0008 (B2) replaces the meet with a carrier form.

### 2026-10-20 — #38 B1 — Discoverable groups for `reader_groups` writes

A writer may name a group it is in (under every clause of its identity), a
group it administers, or, as a tenant administrator, any active group. Every
other value fails with `E_UNAVAILABLE` and the message "reader group
unavailable". That covers unknown IDs, hidden, suspended and retired groups,
users, and every group in a store without a registry. A write therefore
cannot tell a hidden group from a missing one.

### 2026-10-20 — #38 B1 — Admission epochs named after the registry revision

An authority-reducing registry change installs the epoch
`registry:<registry revision>`, with the same policy body, in the same
transaction. Only `add_principal`, `add_member`, `grant_branch_write` and
`unprotect_branch` count as additive. A `grant` counts as reducing because it
may lower a permission. Being conservative there only costs a fresh signed
request. An epoch named after a revision can never be reactivated, which the
admission store already requires.

### 2026-10-20 — #38 B1 — Identity is resolved per check and cached per registry revision

`Engine::access(host)` reads the registry head on each call and reuses a
state and resolution cached for that exact revision. Requests run in one
SQLite read transaction, so the head is stable within a request. A cache
keyed by revision never outlives a change (ADR-0007 open question 6). The
incremental view-selection state, previously keyed by principal, is now
keyed by principal and registry revision, so a state built for an old
membership is never reused.

### 2026-10-20 — #38 B1 — Graph listing and registry CLI

`Engine::list_graphs` lists external graphs with a branch, plus graphs that
have ACL entries, filtered to those the caller can discover. It reports the
caller's permission and no total. The `weave-engine` CLI gains
`registry enable`, `registry apply CHANGES.json` and `graphs`, so the
compatibility job can drive a genuine store-31 upgrade into a registry
store. The registry error for a store without one is `E_REGISTRY_DISABLED`.

### 2026-10-20 — #38 B1 — The Python client and science checks speak 1.1.0

The science CLI reports `weave_contract::VERSION`. The client's strict
version check, its expression constant and `check_science.py` move to
1.1.0, as A2 did for 0.22.0. Its programs use no 1.1.0 feature.

### 2026-10-20 — #38 B1 — Agents extend the part 1 API additively

Agents are a new `PrincipalKind::Agent` with the new changes `add_agent` and
`set_delegation`. `set_status` gains agent revocation by the ownership chain,
and `Principal` gains optional `owner` and `delegation` fields. No part 1
change, field or error code changed meaning, so B3 can build on both parts
at once. An authenticating host passes the agent's principal ID as
`HostContext.principal`, as it does for a user. Agent tokens therefore need
no other engine API. `add_principal` refuses `kind: agent`, so an agent
always has an owner and a delegation.

### 2026-10-20 — #38 B1 — An agent's identity is the owner's clauses plus its own

An agent's `Access` is its owner's clauses (recursively up the chain) plus
one local clause: the agent, the agents below it, its own groups, and its
exercised identities limited to what the owner holds now. Requiring every
clause is the record half of `can(owner) ∧ S_local`. Graph permission is
the minimum of the owner's permission, the agent's scope entry and Write,
computed recursively. Everything is recomputed for each registry revision,
and the expiry is checked against the request's trusted clock outside the
cache. An owner that loses access therefore takes its agents' access with it
on their next request.

### 2026-10-20 — #38 B1 — Scopes limit Write by branch; Read stays per graph

A scope entry is `(graph, permission, branches)`. A nonempty `branches`
limits only Write, matching part 1's rule that branch ACLs grant only Write
and Read is per graph. An agent writes a protected branch only through its
owner's branch entry. Its owner's Administer would bypass the branch ACL,
and Administer is not delegable.

### 2026-10-20 — #38 B1 — Exercise defaults to none

An omitted `exercise` is `only` with no identities. Such an agent reads
public records and records that name the agent itself or its own groups.
`owner_all` must be asked for explicitly, so a forgotten field never grants
an agent its owner's whole identity. Exercised identities are checked
against the owner's identity when the agent is registered (`E_FORBIDDEN`)
and again on every request, where an identity the owner has lost is dropped.

### 2026-10-20 — #38 B1 — Escalation errors and who manages an agent

Delegations that exceed the owner fail with `E_FORBIDDEN`: Administer in a
scope, identities the owner does not hold, and under an agent owner a
graph, permission, branch set or expiry outside the parent, or a missing
`may_delegate`. That is the registry's authorization code. A chain of more
than 8 links fails with `E_BUDGET`. Only the agent's ownership chain and
tenant administrators may register, re-delegate, suspend or retire an agent;
the agent itself may not. Agents may make no other registry change, so they
never change memberships, statuses of others, group administrators or ACLs.

### 2026-10-20 — #38 B1 — Agents write through fact writes only

`on_behalf_of` lives on fact-log recordings. Snapshot commit facts take
their writer from the commit event and have no per-fact delegation. A
snapshot `commit` or batch by an agent therefore fails with `E_FORBIDDEN`,
rather than storing an agent write without its chain. The chain is the
JSON-array encoding of the reserved `on_behalf_of` column. The store's
bound for that column was raised from 512 bytes to fit 8 principal IDs. The
column already existed (store 30), so no migration is needed.

### 2026-10-20 — #39 B3 — Delivery in parts; agents wait for B1 part 2

Part 1 is the server, API tokens for users and agents, tenants, the audit
log, TLS, limits and logs. It covers the criteria on unauthenticated
endpoints, cross-tenant isolation, the audit log and SECURITY.md. Agent
tokens joined part 1 after B1 part 2 merged. Part 2 adds OIDC and signed
admission for agent capabilities. The 50-client load measurement closes the epic, because
it should measure the final authentication path. Each part says "Part of
#39"; the last says "Closes #39".

### 2026-10-20 — #39 B3 — axum on hyper with rustls and ring; no OpenSSL

The HTTP stack is axum 0.8 (default features off: http1, json,
matched-path, query, tokio) on hyper 1 and hyper-util. These are the
mature, MIT-licensed Tokio stack. TLS is tokio-rustls 0.26 with the `ring`
provider, not the default aws-lc-rs, which needs CMake and NASM on Windows
CI. ring also provides the HMAC and the system random source, so the
service adds no separate crypto crates. The accept loop is our own (about
40 lines in `tls.rs`), because `axum::serve` has no TLS. It bounds
connections, handshake time, header read time and header size. rcgen
generates certificates in tests only. There is no CLI-parsing or logging
dependency: flags are parsed by hand, and logs are JSON lines written with
serde_json.

### 2026-10-20 — #39 B3 — Every route authenticates, including unknown ones

Authentication is router middleware that runs before routing results are
reported. An unknown route or method therefore gets `401` without a
credential, and `404` or `405` only after authentication. Every failure
uses one byte-identical body: missing, duplicated or non-Bearer headers,
malformed, unknown, expired and revoked tokens, and principals that are not
active users. `health` also requires a token, so "rejected on every
endpoint" holds without exception; liveness probes can use the TLS
handshake or a probe token. The endpoint test is driven by the same route
table that builds the router, and it checks that the guide documents each
route.

### 2026-10-20 — #39 B3 — Tokens in the control store, checked with the registry per request

An API token is `wvs_` and 256 random bits in hex. The control store keeps
its SHA-256 digest (a fast hash is enough for a 256-bit random secret), a
public token ID, the tenant, principal, issuer and expiry. Each request
looks the digest up, then checks inside the tenant that the principal is an
active registry user. Nothing is cached, so revocation, expiry and
suspension apply on the next request. Tokens live in the control store, not
the registry, because a token must name its tenant before any tenant store
is opened. The registry stays the authority on the principal itself. Only
tenant administrators issue tokens over HTTP; any principal can list and
revoke its own.

### 2026-10-20 — #39 B3 — One connection and one request at a time per tenant

Every engine program reserves SQLite's write lock through the operation
clock, even a read-only one, and the engine sets no busy timeout. A pool of
connections per tenant would therefore fail with `SQLITE_BUSY` under
concurrency. Each tenant has one engine connection and one audit connection
behind a mutex, a bounded queue (256) and a queue timeout (10 s, then
`E_BACKPRESSURE`). Tenants run in parallel. The load measurement in a later
part decides whether reads need a separate path (ADR-0007 open question 6).

### 2026-10-20 — #39 B3 — The audit log is a separate database per tenant

ADR-0007 section 9 puts the audit log in a separate table, not in fact-log
data. It is a separate SQLite file, `audit.db`, in the tenant directory. A
table inside the engine store would change the engine schema (a store
version bump) for a service concern while B1 part 2 changes the engine in
parallel, and the engine owns its connection. The entry is appended before
the response is sent; if that fails, the response is withheld (`E_AUDIT`).
A crash between a committed write and its entry can lose the entry, but the
fact log still records the writer, so "who wrote F" survives. Entries are
hash-chained, and triggers reject UPDATE and DELETE. Reads are recorded from
the result's `fact_provenance`, which lists every stored source fact a
result cites, so the service requires protocol 1.0.0 or later, where it is
always present. Denied requests record the graph IDs the request named,
which are client input, never the hidden object. Tenant administrators are
the auditors.

### 2026-10-20 — #39 B3 — The service runs fact writes and reads only

`/v1/execute` accepts `write`, `query`, `evaluate` and `bind`, which cover
the A2 to A4 operations. Snapshot `commit` and `commit_batch`, `join`,
`recorded_range` and `accepted_range` fail with `E_UNSUPPORTED`. They are
legacy or peripheral, and `fact_provenance` does not describe their writes
and reads, so the audit log could not name their facts.

### 2026-10-20 — #39 B3 — Record IDs minted by the service for non-administrators

ADR-0007 open question 1 (B2 decides) leans towards server-generated record
IDs. The service decides it for its own clients now: a non-administrator
may assert only IDs that `POST /v1/ids` minted for it. A minted ID carries
an HMAC tag over the tenant, the principal and 128 random bits, under a
per-tenant key. A hidden record cannot hold such an ID, an ID seen on
another record cannot be reused, and the check is stateless. Rewriting
client IDs inside a program was rejected, because edges, assertions and
attachments reference IDs. Administrators can read every record, so they
may choose IDs and learn nothing. The key lives in the control store. It is
a service secret that authenticates no one, so it is not hashed.

### 2026-10-20 — #39 B3 — Graph creation through an engine hook

ADR-0007 section 4 gives created graphs engine-assigned IDs, and only
tenant administrators may choose one. Granting the creator Administer on a
new graph is a registry write that only an administrator could make, so the
engine gains `Engine::create_graph(entropy, host)` in a new child module of
the registry (`registry/create.rs`, plus one line in `registry.rs`). The
host supplies the randomness, because the engine has no random source; the
ID is `g-` and 32 hex digits of its SHA-256. Groups, and agents once they
exist, cannot create graphs, because Administer is not delegable.

### 2026-10-20 — #39 B3 — Correct and retract need Read on the fact

The paired HTTP hidden-input test found that B1 part 1 let a writer of a
graph retract or correct a record it could not read, and that the error for
a hidden record differed from the error for a missing one. ADR-0007
section 4 allows correction or retraction "only when Read also holds". In a
registry store the write path now treats a live fact the writer cannot read
(its own readers and groups, and an edge's endpoints) as absent, so the
write fails exactly as for a missing fact. Local stores and the registry's
own writes are unchanged. The fix is a few lines in `facts.rs`; B1 part 2
may move it when it threads `on_behalf_of` through the write path.
Retracting a visible fact with a hidden live dependent still fails
`E_DEPENDENT_FACT`. That is declared for B2, because allowing the
retraction would leave a dangling hidden fact.

### 2026-10-20 — #39 B3 — Agent tokens: the chain must be able to act, owners manage tokens

An agent token is an API token whose principal is the agent; the service
passes the agent's ID as `HostContext.principal`, as B1 specifies, and needs
no other engine API. Authentication requires the agent record to be
active and readable by the agent itself, which the registry allows only
while the whole ownership chain can act, and every delegation along the
chain to be unexpired at the trusted clock. A suspended owner or an expired
delegation therefore fails the agent's next request with the uniform `401`,
not with empty results, so a cut-off agent learns nothing new and the
denial is audited. A user may issue and revoke tokens for agents it owns,
directly or through other agents, because the registry already lets it
suspend them; any other non-administrator request gets one `E_FORBIDDEN`,
whether or not the principal exists. Agents cannot issue tokens, create
graphs or read the audit log, because Administer is not delegable. The
audit entry's `on_behalf_of` is the agent's chain, read record by record as
each principal itself.

### 2026-10-20 — #39 B3 — The service serves the LB1 visible manifest

weave-language LB1 compiles against a host-supplied `weave-visible/1`
manifest and expects B3 or B4 to serve it for the authenticated caller.
It fits B3, because it is one read of the caller's own standing:
`GET /v1/manifest` returns the graphs from `Engine::list_graphs` (with
permission and branches) and the groups the caller may name. The groups
come from a new `Engine::registry_nameable_groups`, which uses the same
predicate as write validation (now one `State::nameable_group` function),
so the manifest can never list a group a write would refuse, or omit one
it would accept; a test checks both directions. Embedding spaces and
schemas are omitted, which the format defines as unknown, because listing
them needs a per-graph read that B4's `describe_schema` will provide. The
manifest is advisory, as LB1 states: every program is authorized again
when it runs. The engine's language CI moves to weave-language
`5d2b9d2` and runs LB1's `check_permissions.py` and `check_agent_repair.py`.
