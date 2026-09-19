# Storage integrity and recovery

Database schema version 8 is the current native storage format. Identity policies/receipts, guarded view transitions, mount routes/lifecycle and signed integration receipts are initialized in the same schema transaction. `Engine::open`
rejects future versions before creating tables or attempting migration. Schema
creation, legacy structural-identity backfill, version-marker advancement, and
dispatcher/view/admission table initialization share one SQLite immediate
transaction. A failed migration leaves the previous records and schema unchanged.
Backfill verifies each retained revision before installing its structural identities.
WAL mode selection occurs before the transaction as required by SQLite.

Graph loads verify canonical graph/branch/parent/content against content-addressed
revision IDs. Logical snapshot IDs additionally require their integrity record,
the content digest, a correctly hashed manifest, canonical unique membership, and
an exact member binding to graph, branch, parent and content. Malformed, oversized,
missing-integrity and mismatched stored objects fail with `E_INTEGRITY` instead of
being evaluated as evidence. This adds hashing work to reads. It does not turn a
claimed author into an authenticated source.
Bounded SQL extraction prevents oversized corrupt data/manifest cells from being
copied into Rust strings before the 16 MiB per-record format limit is checked.

These checks detect stored corruption relative to retained anchors. They do not
protect against a database administrator replacing every anchor, rollback to an
older internally valid database, altered wall-clock timestamps, or independent
corruption of arbitrary cached view/receipt tables. Deployment backup encryption,
retention, off-host anchors and full-store scrub are still host responsibilities.

## Backup acceptance

Use SQLite's online backup facility or `VACUUM INTO` to obtain a consistent backup
that includes committed WAL state. Copying the main database file alone while it
is open is not a supported backup procedure. Backups contain all stored private
evidence, installed trust policy and receipts; they are trusted host artifacts,
not graph-scoped capsules or a remote signed operation.

`cargo test -p weave-engine --test root_storage` independently covers:

- content and parent corruption for hashed and logical revisions;
- missing logical integrity records and modified manifests;
- refusal of an unknown future schema without creating tables;
- repeated failed legacy identity backfill with complete DDL/record rollback;
- corruption rejection before backfill, followed by a repaired version-5 fixture
  upgrade, and rejection of an oversized corrupt cell;
- a SQLite snapshot backup while the source is open, exact restored query and
  event equality, and an independent subsequent commit in the restored store.

Actual process termination around graph/receipt commits is tested separately by
`scripts/root_dispatch.py` and `scripts/root_admission.py`. `scripts/root_migration.py`
terminates a process after backfill/table creation but before schema COMMIT (exit82),
then verifies that identities, new tables and the version marker all rolled back.
A new process completes the upgrade while preserving evidence/events; another
reopen is idempotent. The callback exists only under `recovery-testing`. Broader
historical upgrade fixtures, fuzzing and production backup operations remain open
under E14.
