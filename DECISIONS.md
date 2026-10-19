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
