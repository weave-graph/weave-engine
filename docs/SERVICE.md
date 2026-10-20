# Authenticated Weave service

`weave-service` is a long-running, multi-tenant Weave server (B3,
[#39](https://github.com/weave-graph/weave-engine/issues/39),
[ADR-0007](architecture/ADR-0007-identity-groups-and-tenancy.md) sections
6 to 9). It serves the core v1 JSON contract over HTTP/1.1 and TLS, so a team
shares one authenticated endpoint instead of a trusted local process.

The service is a thin adapter over the engine. Every graph operation is one
engine call with the authenticated principal as `HostContext.principal`, and
the [principal registry](REGISTRY.md) of the caller's tenant authorizes it:
graph ACLs, Discover, protected branches, `readers` and `reader_groups`
apply exactly as they do locally.

**Status.** B3 is complete: API tokens for users and agents, OIDC sign-in,
signed admission for agent capabilities, tenants, the HTTP API, the audit
log, TLS, limits, structured logs and a published 50-client load
measurement ([benchmarks](benchmarks/README.md#authenticated-service-b3)).
The service also serves the B4 agent interface, MCP at `POST /v1/mcp`
([guide](AGENT_INTERFACE.md)).

## Run it

```sh
cargo build --release -p weave-service

# Trusted local administration (the only way to create a tenant).
target/release/weave-service tenant create --data /srv/weave --tenant acme --admin root
# -> {"tenant":"acme","token":{"token":"wvs_...","token_id":"tok-...",...}}

# Serve over TLS (PEM certificate chain and private key).
target/release/weave-service serve --data /srv/weave --listen 0.0.0.0:8443 \
  --tls-cert cert.pem --tls-key key.pem

# Local development only: plaintext, refused on any non-loopback address.
target/release/weave-service serve --data /srv/weave --listen 127.0.0.1:8080 --plaintext-loopback
```

`tenant create` creates the tenant's store, enables its registry with
`--admin` as the first tenant administrator, and prints a bootstrap token for
that administrator. The token is printed once and is never stored in clear.
`token issue` and `token revoke` do the same for any active user, from the
operator's shell.

The data directory holds:

| Path | Contents |
|---|---|
| `control.db` | the control store: tenant definitions and API token digests. No graph data |
| `tenants/<id>/store.db` | the tenant's engine store: its registry, graphs and admission state |
| `tenants/<id>/audit.db` | the tenant's append-only audit log |

## Authentication

Every request carries exactly one `Authorization` header: `Bearer <API
token or OIDC JWT>` on every route, or `Weave-Signed` on `POST /v1/admit`
only ([signed admission](#signed-admission-for-agent-capabilities)).
Every route requires it, including unknown routes: an unauthenticated
request gets `404` or `405` never, only `401`. Each failure, whatever the
reason (no header, two headers, another scheme, a malformed, unknown,
expired or revoked token, or a principal that is not an active user of its
tenant's registry), is the same response:

```json
{"error": {"code": "E_UNAUTHENTICATED", "message": "authentication required"}}
```

with status `401` and `WWW-Authenticate: Bearer`.

- **API tokens.** A token is `wvs_` and 64 hex digits (256 random bits). The
  control store keeps only its SHA-256 digest, its public token ID
  (`tok-...`), its tenant and principal, and its expiry (one minute to 365
  days, default 30 days). A principal holds at most 100 live tokens.
- **The tenant comes from the token.** No path, query parameter, header or
  body field names a tenant.
- **Checked on every request.** Each request looks the token up and checks
  its expiry against the trusted clock, then checks in the tenant's registry
  that its principal can act: an active user, or an active agent whose
  whole ownership chain is active and whose delegations have not expired. A
  revoked or expired token, a suspended or retired principal, an expired
  delegation, or a suspended owner anywhere up an agent's chain fails on the
  next request. Nothing is cached.
- **Agent tokens.** A token issued for an agent ([REGISTRY.md](REGISTRY.md#agents))
  authenticates the agent, never its owner: the agent's ID is the request's
  principal, so the registry gives it at most its owner's current authority,
  narrowed by its delegation, and never Administer. The agent's writes record
  `on_behalf_of`, and so do its audit entries. Agents cannot create graphs,
  issue tokens or read the audit log.
- **Groups never authenticate.**

### OIDC

An operator configures an identity provider for a tenant from the shell,
with the expected audience (the client ID) and the provider's JSON Web Key
Set:

```sh
weave-service oidc set --data /srv/weave --tenant acme \
  --issuer https://login.example.com --audience weave-acme --jwks jwks.json
weave-service oidc remove --data /srv/weave --tenant acme --issuer https://login.example.com
```

The service never fetches keys over the network; a key rotation is another
`oidc set`, effective on the next request. An `(issuer, audience)` pair
belongs to one tenant, and that pair, never the request, decides a token's
tenant. A tenant administrator binds IdP subjects to registry **users**
(`POST /v1/oidc/bindings`); a token's `(iss, sub)` must be bound.

A bearer JWT authenticates when its signature verifies under a key of the
set (RS256 with 2048 to 8192-bit keys, ES256, or EdDSA with Ed25519; `none`,
algorithm/key mismatches and `crit` headers are refused), its `iss` and
`aud` match the configuration, and `exp` (required) and `nbf` hold with 60 s
of leeway. Tokens are at most 8 KiB. Then the bound user must be active, as
for an API token. Every failure is the uniform `401`.

**Group claims are never live.** `groups`, `roles` and every claim other
than `iss`, `sub`, `aud`, `exp` and `nbf` are ignored when authenticating
(ADR-0007 section 11, alternative 6). They reach the registry only through
`POST /v1/oidc/groups/import`, a tenant administrator's import of an IdP
snapshot:

```json
{"issuer": "https://login.example.com",
 "mapping": {"eng": "maintenance"},
 "subjects": [{"subject": "00u1", "groups": ["eng"]}, {"subject": "00u2", "groups": []}]}
```

For each mapped registry group, the principals bound to subjects that carry
a mapped claim become direct members, and principals bound to this issuer
that no longer carry one are removed. The snapshot is complete for the
issuer: a bound subject that is missing loses its mapped memberships.
Members not bound to the issuer are never touched. The changes are one
ordinary registry write by the administrator, so they are audited and
in the registry history; revocation through the import takes effect on the
next request. At most 10,000 subjects, 1,024 mappings and 1,000 resulting
changes per import.

### Signed admission for agent capabilities

Agents that hold ed25519 keys can send requests signed with a
`weave-policy` capability chain ([CAPABILITIES.md](CAPABILITIES.md))
instead of a bearer token (ADR-0007 section 8). Keys are bound to
principals with `POST /v1/keys` (a principal binds its own, an owner binds
its agents', an administrator any); a key is bound once per tenant, ever.

`POST /v1/admit` with `Authorization: Weave-Signed` and the body

```json
{"proof": {"chain": [...], "request": {...}}, "program": "<program JSON text>"}
```

runs `program` as the principal bound to the request's subject key. The
signed body is the exact UTF-8 bytes of `program`. `whoami` returns the
tenant's `admission_audience` and `admission_policy_revision`, which every
capability and request must name. A request is admitted only when:

1. **the chain verifies under `weave-policy`**, with the tenant's audience,
   the trusted clock, the tenant's revoked keys and a root derived from the
   registry: the first issuer's key must be bound to an active user, and the
   root grants that user's current permissions (Discover; Read, Traverse and
   Delegate for Read; Publish for Write) on the graph branches its root
   capability names;
2. **the registry agrees**: the leaf key's principal can act now, every key
   in the chain is bound to that principal or a principal of its ownership
   chain, and the first issuer is the chain's root user.

The program then runs as the agent, so the registry authorizes it again
under the agent ceiling: a capability only narrows. A signed program is one
operation: all reads of one graph branch (`read`, which needs Read and
Traverse in the leaf scope) or all writes of one (`publish`); anything else
is `E_UNSUPPORTED`. Each request nonce is reserved until the request
expires, so a replay is `E_REPLAY` (`409`); a request that fails after its
nonce is reserved needs a new nonce. Revoking a key (`DELETE
/v1/keys/{public_key}`), suspending the agent or its owner, or the owner
losing the permission fails the next signed request. Every verification
failure is the uniform `401`.

## Tenants

Each tenant is one store with its own registry, graphs and audit log
(ADR-0007 section 6). Principal and graph IDs are unique only inside a
tenant: the same ID in two tenants is two unrelated principals or graphs. A
request reaches only its token's tenant, so naming another tenant's graph
fails exactly like naming a graph that exists nowhere (`E_UNAVAILABLE`,
"graph unavailable"), never touches the other tenant's store, and is
audited in the caller's own tenant as a denial.

Each tenant runs one request at a time on one store connection (engine
programs take SQLite's write lock), with a bounded queue. Tenants run in
parallel.

## API

All bodies are JSON. Errors have the shape above with the codes in
[Errors](#errors). Every response has an `x-request-id` header, which is
also in the audit entry and the log line of the request.

| Endpoint | Who | Does |
|---|---|---|
| `GET /v1/health` | any principal | `{"status": "ok", "service", "version", "protocol"}`. The only route that is not audited |
| `GET /v1/whoami` | any principal | the caller's tenant, principal, `administrator` and `agent` flags, `on_behalf_of`, credential ID and expiry |
| `GET /v1/manifest` | any principal | the caller's visible manifest, `weave-visible/1` of weave-language `docs/PERMISSIONS.md`: `{"format", "principal", "graphs": [{"graph_id", "permission", "branches"}], "reader_groups"}`. `graphs` are the graphs it can discover; `reader_groups` are exactly the groups a write by it may name. Embedding spaces and schemas are omitted (unknown to the compiler) |
| `POST /v1/execute` | any principal | run a core v1 `Program` (below) |
| `POST /v1/mcp` | any principal | one MCP JSON-RPC message: the agent interface's tools over the same program path, audited per message ([AGENT_INTERFACE.md](AGENT_INTERFACE.md)) |
| `GET /v1/graphs` | any principal | `Engine::list_graphs`: the graphs the caller can discover, with branches and its permission. No total |
| `POST /v1/graphs` | any active user (not agents) | create a graph (below) |
| `GET /v1/graphs/{graph_id}/acl` | graph Administer | the graph's ACL and protected branches |
| `POST /v1/ids` | any principal | `{"count": n}` (1 to 1,000): mint record IDs for the caller (below) |
| `GET /v1/quota` | any principal | the caller's own quota and its use in the current windows ([quotas](#quotas-and-rate-limits)) |
| `POST /v1/registry` | per change | `{"changes": [RegistryChange, ...]}`: `Engine::apply_registry`, see [REGISTRY.md](REGISTRY.md). Returns `{"revision"}` |
| `GET /v1/registry/principals/{id}` | the principal itself, tenant administrators | its registry record |
| `GET /v1/registry/principals/{id}/memberships` | the principal itself, tenant administrators | the groups it is directly in |
| `GET /v1/registry/groups/{id}/members` | the group's administrators, tenant administrators | its direct members |
| `GET /v1/tokens` | any principal | its own tokens; a tenant administrator sees every token of the tenant. Never the secret or its digest |
| `POST /v1/tokens` | tenant administrators; a user for the agents it owns | `{"principal", "expires_in_ms"?, "label"?}`: issue a token for an active user or agent. `201` with the token, shown once. A user naming anything but its own agent gets `E_FORBIDDEN`, whether or not the principal exists |
| `POST /v1/admit` | a signed request (`Authorization: Weave-Signed`), never a bearer token | run a signed program ([above](#signed-admission-for-agent-capabilities)) |
| `GET /v1/keys` | any principal | its own key bindings; a tenant administrator sees all |
| `POST /v1/keys` | the principal itself, the owners of its agent, tenant administrators | `{"public_key": "ed25519:<64 hex>", "principal"?}`: bind a key (at most 16 live keys per principal). `201` |
| `DELETE /v1/keys/{public_key}` | the key's principal, the owners of its agent, tenant administrators | revoke it, for good. Anyone else gets `E_UNAVAILABLE`, as for a key that is not bound |
| `GET /v1/oidc/bindings` | tenant administrators | every OIDC binding of the tenant |
| `POST /v1/oidc/bindings` | tenant administrators | `{"issuer", "subject", "principal"}`: bind an IdP subject of a configured issuer to an active user. `201` |
| `DELETE /v1/oidc/bindings` | tenant administrators | `?issuer=I&subject=S`: remove a binding. Effective on the next request |
| `POST /v1/oidc/groups/import` | tenant administrators | import an IdP group snapshot into registry memberships ([above](#oidc)) |
| `DELETE /v1/tokens/{token_id}` | the token's principal, the owners of its agent, tenant administrators | revoke it. Effective on the next request. Anyone else gets `E_UNAVAILABLE`, as for a token that does not exist |
| `GET /v1/audit` | tenant administrators | `?after=SEQ&limit=N` (1 to 1,000): audit entries in order |
| `GET /v1/audit/facts` | tenant administrators | `?graph_id=G&fact_id=F[&access=read\|write][&after][&limit]`: who read and who wrote fact F |

Registry reads that the caller may not make fail with `E_UNAVAILABLE`, the
same as for an ID that does not exist.

### Programs

`POST /v1/execute` takes one core v1 [`Program`](contract/core-v1.md) and
returns `{"results": [CommandResult, ...]}`. It runs as one atomic engine
call. The service accepts:

- protocol **1.0.0 or later**, so every result carries `fact_provenance`
  for the audit log (`E_VERSION` otherwise);
- 1 to 64 commands;
- the commands `write`, `query`, `evaluate` and `bind`, which cover A2 fact
  writes and every A3 and A4 read: `query`, `expand`, `as_of`, `changes`,
  `nearest`, `similar_candidates` and `explain`. Snapshot `commit`,
  `commit_batch`, `join`, `recorded_range` and `accepted_range` fail with
  `E_UNSUPPORTED`.

### Graph IDs are engine-assigned

`POST /v1/graphs` with `{}` creates a graph with an opaque engine-assigned
ID, `g-` and 32 hex digits, and grants the creator Administer on it. The
creator then grants others through `POST /v1/registry`. Choosing the ID,
with `{"graph_id": "..."}`, needs tenant administration; anyone else gets
`E_FORBIDDEN`, whether or not the ID is taken, so graph creation cannot
probe for hidden graphs (ADR-0007 section 4).

### Record IDs are minted by the service

A write that asserts a record ID a hidden live record already holds fails
`E_CONFLICT`, which would reveal the hidden record (ADR-0007 open question
1). The service closes that channel: **every principal, tenant
administrators included, may assert only record IDs that `POST /v1/ids`
minted for it**. Any other ID in an `assert`
change fails with `E_ID_NOT_MINTED` before the engine sees the program. A
minted ID is `r-`, 32 hex digits of randomness, `-` and a 16-digit tag that
binds the ID to the tenant and the principal, so no hidden record can hold
it and an ID seen on someone else's record cannot be reused. `correct` and
`retract` name existing facts and need no minting. Edges may reference any
node. Administrators get no exemption: record readers apply to them like
anyone else, so a chosen ID could reveal a hidden record they cannot read.
Embedding spaces (`embedding_space` facts) are named freely, because a space
has no readers and is never hidden from a reader of its graph. Operators
load fixtures with chosen IDs through trusted local administration, not the
HTTP API.

## Quotas and rate limits

Each principal of a tenant has two limits, counted in fixed windows of the
trusted clock:

- **Requests per minute.** Every audited request (every route except
  `health`, and every MCP message) counts once. A request over the limit
  fails `E_QUOTA` with status `429` and `Retry-After`, is audited, and is
  not counted.
- **Fact writes per hour.** Every change of a committed `write` (each
  `assert`, `correct` or `retract`, through `POST /v1/execute`,
  `POST /v1/admit` or an MCP write tool) counts once. A program whose
  changes would pass the limit fails `E_QUOTA` before the engine runs it,
  so none of its changes is written; over MCP the tool result has
  `isError` with that code.

**Who is limited.** A principal's own quota row applies first. For an
agent, the tenant's default for agents applies next, then the service
default: **600 requests per minute and 10,000 fact writes per hour**.
Users are unlimited unless they have a row of their own. Rows are read on
every request, so a change applies to the next one.

```sh
weave-service quota set --data /srv/weave --tenant acme --agents \
  --requests-per-minute 120 --writes-per-hour 2000
weave-service quota set --data /srv/weave --tenant acme --principal alice-bot \
  --requests-per-minute unlimited --writes-per-hour 50000
weave-service quota remove --data /srv/weave --tenant acme --principal alice-bot
weave-service quota list --data /srv/weave --tenant acme
```

`GET /v1/quota` returns the caller's `requests_per_minute`,
`writes_per_hour` (`null` is unlimited), `requests_this_minute`,
`writes_this_hour`, `minute_resets_at_ms` and `hour_resets_at_ms`.

A tenant runs one request at a time, so a request's quota check, its write
and its count are one step: concurrent requests at the boundary commit
exactly the permitted number. Usage is kept in memory, so a restart starts
fresh windows, and fixed windows let a principal use up to twice a limit
across a window boundary.

## Audit

Every authenticated request except `health` appends one entry to its
tenant's audit log before the response leaves the server. If the entry
cannot be written, the response is withheld and the request fails with
`E_AUDIT` (`503`).

An entry holds the sequence number, recorded time, request ID, principal,
`on_behalf_of` (an agent's ownership chain, owner first), credential ID, operation, outcome
(`ok`, `denied` or `error`) and error code, the graphs and revisions read or
written, the number of fact rows, and `result_digest`, the SHA-256 of the
exact response body. Its fact rows name:

- **reads:** every fact in the response's `fact_provenance`, which lists
  every stored source fact the result cites (ADR-0005), with the revision
  read and the kind (`node`, `assertion` or `embedding`);
- **writes:** every change of a committed `write` command, with the new
  revision, the fact kind and `assert`, `correct` or `retract`.

`GET /v1/audit/facts` therefore answers "who read fact F" and "who wrote
fact F". A read only ever cites facts the reader could see. A denied
request records its error code and the graph IDs the request itself named,
never the hidden object that caused the denial.

The log is append-only: triggers reject every SQL `UPDATE` and `DELETE`, and
each entry stores the digest of the previous one, so a rewrite by someone
with file access breaks the chain (`AuditLog::verify_chain`). Only tenant
administrators read it.

## Errors

Engine codes (`E_UNAVAILABLE`, `E_FORBIDDEN`, `E_BUDGET`, `E_CONFLICT`,
`E_VERSION`, `E_DECODE`, ...) pass through unchanged. The service adds:

| Code | Status | Meaning |
|---|---|---|
| `E_UNAUTHENTICATED` | 401 | every authentication failure |
| `E_INPUT` | 400 | malformed JSON, unknown fields, invalid path or query parameters |
| `E_BODY_LIMIT` | 413 | the request body exceeds 16 MiB |
| `E_UNSUPPORTED` | 400 | a program command the service does not run |
| `E_ID_NOT_MINTED` | 400 | a principal asserted a record ID not minted for it |
| `E_QUOTA` | 429 | the principal's request rate or fact-write quota is used up; `Retry-After` gives the seconds until its window resets ([quotas](#quotas-and-rate-limits)) |
| `E_BACKPRESSURE` | 503 | the tenant's queue is full; retry later (`Retry-After: 1`) |
| `E_AUDIT` | 503 | the audit entry could not be written; the response is withheld |
| `E_NOT_FOUND`, `E_METHOD` | 404, 405 | no such route or method, after authentication |
| `E_MCP_ORIGIN` | 403 | an MCP request carried an `Origin` header |
| `E_MCP_PROTOCOL_VERSION` | 400 | an MCP request named an unsupported `MCP-Protocol-Version` |
| `E_INTERNAL` | 500 | an internal failure; storage details are never returned |

Statuses: `E_UNAVAILABLE` 404, `E_FORBIDDEN` 403, `E_CONFLICT` 409,
`E_BUDGET` 422, and 400 for other request errors.

## Logs

The server writes one JSON object per line to standard error. A request
line has `request_id`, `method`, the matched route template (never the raw
path or query string), `status`, `code`, `tenant`, `principal` and
`duration_ms`. Headers, tokens and bodies are never logged, and neither are
graph or record IDs, so denied topology stays out of the log.

## Limits and non-guarantees

- Request bodies: 16 MiB. Request heads: 64 KiB, read within 10 s. TLS
  handshakes: 10 s. At most 1,024 open connections.
- Per tenant: at most 256 requests queued or running; a request waits at
  most 10 s for its turn, then fails `E_BACKPRESSURE`. Engine work inside a
  request is bounded by the engine's own budgets ([READ_BUDGETS.md](READ_BUDGETS.md)).
- Quotas: per principal in fixed windows, kept in memory ([above](#quotas-and-rate-limits)).
- Programs: 1 to 64 commands. IDs: 1 to 1,000 per request. Audit pages: 1 to
  1,000 entries.
- **Audit and data are two databases.** A response is never sent without its
  audit entry, but a crash after a write commits and before its audit entry
  is appended loses that entry. The fact log still records the writer and
  recorded time of every fact (ADR-0005), so "who wrote fact F" survives.
- **Timing and size** are not protected channels, as in the engine. Requests
  of one tenant never touch another tenant's store, so their timing does not
  depend on other tenants' data. A valid token of a suspended principal
  takes slightly longer to reject than a garbage token; its holder already
  knows the token was once issued.
- **Revision IDs** identify whole immutable snapshots, so hidden records in a
  readable graph change the revision IDs (and IDs derived from them, such as
  explanation records) that a reader sees, but not its facts, counts or
  errors.
- The ID key that tags minted record IDs is stored in the control store. It
  authenticates no one; if it leaked, the occupancy channel above would
  reopen for writers of a graph, nothing more.
- OIDC keys are configured, not discovered: there is no OIDC discovery or
  JWKS fetching, and a rotation is an operator `oidc set`.
- Signed admission does not keep retry receipts: a lost response needs a
  new request with a new nonce. Capability IDs cannot be revoked one by one;
  revoke the key. A capability's scope covers the graphs and branches a
  program names; references stored inside records (metadata, evidence) are
  authorized by the registry alone.
- **Throughput.** A tenant runs one request at a time. On the recorded host
  one tenant served about 710 requests/s (20% writes) to 50 clients, with
  p50 latency 70 ms and p99 about 90 ms; one client saw 1.2 ms per read and
  1.8 ms per write. Five tenants together served 2,300 requests/s. See the
  [load report](benchmarks/service-load-2026-10-20.json) and
  `examples/load.rs` to measure your own host.
- Not yet: gRPC (deferred until a measured need) and tenant deletion.
