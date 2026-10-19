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
