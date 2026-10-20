# Security boundary

The engine and science CLI are trusted local interfaces. The authenticated
service (`weave-service`, [below](#the-authenticated-service-b3)) is the
interface meant for untrusted network clients. A caller controlling
CLI flags, the SQLite database or the embedding `HostContext` controls host
authority. `--actor` selects a principal; it does not authenticate a remote user.
The Python SDK inherits the same boundary. Do not expose these interfaces as
untrusted multi-user endpoints without a separate host authentication and
admission design.

The read-only MCP preview (`weave-mcp`, [guide](docs/MCP_PREVIEW.md)) has the
same boundary: whoever starts it or writes to its stdin reads as its
`--actor`. It has no write tools and no write grants, and it marks store
content in its results as untrusted data for the agent.

The write-capable agent interface (B4, [guide](docs/AGENT_INTERFACE.md)) is
served by `weave-service` at `POST /v1/mcp`, inside the service boundary
below: the agent's principal comes from its credential, writes go through
the service's audited, minted-ID write path, and no tool changes
permissions. Store content stays data: tool text is static, results are
marked untrusted, and tool arguments have no field for a writer, a grant
or record readers. Requests with an `Origin` header are refused, so a
browser page cannot drive the endpoint. Text in the store can still
mislead an agent into calling a tool with the agent's own authority; that
call is authorized and audited like any other.

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

A store with the [principal registry](docs/REGISTRY.md) enabled (B1) moves
authorization, but not authentication, into the store. The host still
asserts `HostContext.principal`; the registry then decides what that
principal may do: graph ACLs (Discover, Read, Write, Administer), protected
branches and group-resolved `reader_groups`, evaluated against current
membership on every request. A graph without Discover is indistinguishable
from a nonexistent graph in errors and listings. The registry itself lives in
the reserved graph `weave:registry`, writable only through the registry API.
Declared channels it does not close: timing and response size, the
occupancy of client-chosen record IDs by hidden records (`E_CONFLICT`, see
[#5](https://github.com/weave-graph/weave-engine/issues/5)), and writer and
`on_behalf_of` identities disclosed to a fact's readers. Agents act with at
most their owner's current authority, narrowed by their delegation, and
never with Administer. Authentication and tenant isolation
are B3. In a registry store a writer can correct or retract only facts it
can read; retracting a fact with a hidden live dependent fails
`E_DEPENDENT_FACT`, the same error as for a visible dependent: a declared
one-bit channel (B2 refuses such a retraction instead of cascading or
orphaning the hidden record).

## The authenticated service (B3)

`weave-service` ([guide](docs/SERVICE.md)) moves the trust boundary from the
local process to the network. Its trust model:

- **Trusted:** the operator who runs the binary and owns its data
  directory. The control store (`control.db`), each tenant's store and audit
  log, the TLS private key and the operator CLI (`tenant create`, `token
  issue`, `token revoke`) are host authority. Anyone who can write those
  files controls the service; backups must be protected like credentials.
- **Untrusted:** every network client. A client is authenticated by a bearer
  API token, and the token alone decides its tenant and principal; no path,
  header or body can name a tenant or a principal. The principal must be an
  active user, or an active agent with an active ownership chain and an
  unexpired delegation, of its tenant's registry on every request, and the registry
  then authorizes every operation exactly as in a local registry store.
  Authentication failures are one uniform `401` on every route, including
  unknown ones, so routes cannot be enumerated without a credential.
- **Secrets.** API tokens are 256 random bits, shown once and stored only as
  SHA-256 digests. Tokens, request bodies and headers are never logged and
  never written to the audit log (which stores a digest of each response).
  The per-tenant key that tags minted record IDs is stored in the control
  store; it authenticates no one.
- **Tenant isolation.** Each tenant is a separate SQLite store and audit
  database. A request is served only from its token's tenant: another
  tenant's graph IDs answer like IDs that exist nowhere, and the other
  tenant's store is never opened for the request. Row-level tenant keys are
  not used.
- **Transport.** TLS 1.2 or 1.3 through rustls with the ring provider.
  Plaintext is refused on any non-loopback address.
- **Audit.** Every authenticated request except `health` is recorded in its
  tenant's append-only, hash-chained audit log before the response is sent.
  Only tenant administrators read it. A crash between a committed write and
  its audit entry can lose that entry; the fact log still records the writer.
- **Declared channels:** timing and response size; revision IDs of a
  readable graph change when hidden records are added to it; retracting a
  fact with a hidden dependent (above); occupancy of chosen record IDs by
  hidden records for tenant administrators only (non-administrators assert
  only service-minted IDs).
- **OIDC.** ID tokens are verified against operator-configured key sets
  (the service never fetches keys), and an `(issuer, audience)` pair names
  one tenant. Only bound subjects authenticate, as registry users. IdP group
  claims are never authority; only an administrator's audited import
  changes memberships. A compromised IdP can sign in as any bound subject
  of its issuer, within that user's registry authority.
- **Signed agent capabilities.** A chain is admitted only when it verifies
  under `weave-policy` with the tenant's audience and a registry-derived
  root, and every key in it is bound to the acting agent's ownership chain;
  the program then runs under the registry ceiling. Key bindings are
  per-tenant and never reused after revocation. Nonces are reserved until
  the request expires.

## Noninterference and its declared channels

Hidden facts must not change what a reader sees. A generated harness
([ADR-0008](docs/architecture/ADR-0008-write-time-propagation-and-release-policies.md)
section 7) checks this for every query operator and science analysis: pairs
of stores that differ only in facts a reader cannot see must give that reader
identical values, counts, ranks, explanations, coverage, diagnostics,
provenance and errors. It runs 500 generated cases per operator on every
pull request and 10,000 in a scheduled job. These channels are **not**
closed, and nothing in this repository claims otherwise:

- **Whole-revision reads.** `recorded_query`, recorded ranges and
  checkpoints, `accepted_graph` and `accepted_history` serve a revision only
  to a reader who can see all of it. Their "unavailable" error tells a reader
  that the selected revision holds something it cannot see.
- **Revision identity.** Revision IDs are content-derived over every fact.
  A new revision ID, a stale compiled view, or two revisions of one visible
  state in one result reveal that a write happened, never what it wrote.
- **Work budgets.** The read budget, the `nearest` scan budget and the
  proof-traversal budget count the records an operation reads, hidden ones
  included. A request near a limit can fail with `E_BUDGET` because hidden
  records exist.
- **Timing, response size and resource use** (CPU, memory, I/O).
- **Write-path occupancy**, for principals with Write on a graph: an
  `assert` of a record ID that a hidden live record holds fails with
  `E_CONFLICT`; retracting a record that hidden records depend on fails with
  `E_DEPENDENT_FACT`; a structural identity a hidden record once had cannot
  be reused. B4 agent tools generate record IDs on the server.
- **Disclosed identities.** A fact's `writer` and `on_behalf_of` are shown
  to its readers.
- **Graph existence in local stores.** Without the registry there is no
  Discover permission, so graph and revision IDs are visible to every trusted
  local caller.
- **Copies by authorized readers.** A principal who can read a fact can write
  its content into a new fact without declaring where it came from. The
  engine restricts the derivations it computes and, from protocol 1.2.0, the
  derivations a writer declares in provenance evidence; it cannot detect an
  undeclared copy.
- **References in local stores.** A store without a registry resolves a
  written record's references against every live record, so a trusted local
  writer can learn that an ID is live by referencing it (the occupancy bit).
  Registry stores treat references to records the writer cannot read as
  missing.

The fact write path never compares a change with a live version the writer
cannot read (ADR-0008 section 4, every protocol version): such a version is
absent for `correct`, `retract` and expectations, and an `assert` of its ID
fails with one uniform `E_CONFLICT`.

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
