# Security boundary

The engine and science CLI are trusted local interfaces. A caller controlling
CLI flags, the SQLite database or the embedding `HostContext` controls host
authority. `--actor` selects a principal; it does not authenticate a remote user.
The Python SDK inherits the same boundary. Do not expose these interfaces as
untrusted multi-user endpoints without a separate host authentication and
admission design.

## What the runtime enforces

Request plans cannot grant themselves host authority. Reads apply object readers,
endpoint visibility and transitive evidence restrictions. Metadata traversal is
bounded and uses structured unavailable diagnostics. Native science analyses
consume the resulting authorized graph, including its provenance, coverage and
selected revisions. Old revision reads and saved experiment replay recheck
current authority; an artifact cannot restore a revoked grant.

The `readers` field is a principal allowlist. It is separate from the portable
[signed capability verifier](docs/CAPABILITIES.md) and bounded
[per-operation admission bridge](docs/ADMISSION.md). Signed query, publish and
proposal APIs require host-installed roots and policy. The science request
protocol does not expose those APIs or authenticate arbitrary JSON Programs.
The admission profile's limitations and scope checks are documented separately.

## Trust and limits

Graph writers must be trusted to classify authored content correctly. The
runtime's proof restrictions protect engine-derived values; they do not prevent
an authorized writer from manually reauthoring a fact. Stored digests detect
corruption relative to retained anchors; an administrator who can replace all
anchors controls that trust boundary. See [storage integrity](docs/STORAGE_RECOVERY.md).

Snapshots can reveal graph existence and revision identity to trusted local
callers. Whole-snapshot storage and resource rejection do not provide timing,
RSS or activity-hiding noninterference. Algorithm and read budgets bound declared
work; they are not process isolation. See [read budgets](docs/READ_BUDGETS.md)
and [science limits](docs/SCIENCE_INTERFACE.md).

Saved experiments contain authorized data, parameters and provenance. Their
integrity hash detects edits; it is not an authorship signature or encryption.
CSV and JSON exports are ordinary local files. Protect databases, backups and
experiment artifacts according to the evidence they contain. Use SQLite's
consistent backup facilities when the store may have live WAL state.

Native adapters and the effect broker have finite lifecycle and destination
protocol profiles. They do not promise exactly-once arbitrary external effects
or sandbox arbitrary code. Unknown outcomes require reconciliation. Science
analysis is read-only; the general `execute` operation can make authorized
commits. After a mutation timeout or after-execution output failure, inspect
durable state before retrying.

## Report a vulnerability

Avoid posting exploit details, private data or credentials in a public issue.
Until the public repository has a private reporting channel, use a minimal issue
requesting one, with no sensitive details. A local or hosted test pass is evidence
for its named profile, not a claim of production hardening or independent
cryptographic review.
