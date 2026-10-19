# ADR 0007: identity, groups, agent delegation and tenancy

Status: **proposed** on 2026-10-19 for B1
([#38](https://github.com/weave-graph/weave-engine/issues/38)) and B3
([#39](https://github.com/weave-graph/weave-engine/issues/39)). It becomes
accepted when B1 implementation starts, after Phase A. It is written now,
during A2, so the fact model can reserve what this design needs (see
[section 13](#13-what-phase-a-must-reserve)). No code implements it yet.

## Context

Today a reader is a literal string. `HostContext` carries one `principal` and
a set of `writable_graphs`. A record is visible when its `readers` list is
empty (public) or contains that principal byte for byte
(`crates/weave-engine/src/lib.rs`, `allowed`). Derived records intersect
literal reader lists (`assertions.rs`), and influence carriers
(`derived_from`, `derived_nodes`, `derived_snapshots`) add AND restrictions
that are resolved at read time ([INFLUENCE.md](../INFLUENCE.md)). Missing and
denied objects already share one code, `E_UNAVAILABLE` (core v1 section 10).

`crates/weave-policy` verifies ed25519 capability chains with the actions
Discover, Read, Traverse, Propose, Publish and Delegate over exact
graph/branch scopes. `admission.rs` bridges it to bounded query, publish and
proposal operations ([CAPABILITIES.md](../CAPABILITIES.md),
[ADMISSION.md](../ADMISSION.md)). There the principal is the request
subject's public key, and trust roots, revocations and the policy epoch are
installed by a trusted host. `identity_acceptance.rs` shows the pattern for a
trusted registry held in a reserved `weave:` namespace, with current policy
rechecked on historical pins.

Phase B needs people, teams and agents sharing one service
([VISION.md](../../VISION.md) G2). That needs four things the engine does not
have:

1. a registry of principals;
2. groups that a record can name, with membership resolved when a request
   runs, so that revocation takes effect on the next request;
3. agents whose authority can never exceed their owner's;
4. an authenticated service in which each tenant is isolated in its own store.

Constraints carried from earlier decisions:

- A `readers` entry stays a literal principal ID forever. The field name
  `reader_groups` is reserved on every fact record for group references
  (ADR-0004 section 11, core v1 section 11.6).
- The fact index holds no per-principal visibility (ADR-0004 section 11).
  Authorization is evaluated at read time.
- Revision IDs are content-derived (ADR-0001). The writer and the recorded
  time are engine-recorded and sit outside content (ADR-0005).
- Stored data and saved plans are never reinterpreted.

## Decision

### 1. The principal registry

Each tenant store holds one **registry**. It is the only source of identity
and authorization state for that tenant.

A **principal** has an ID, a kind, a status and kind-specific fields:

| Kind | Can authenticate | Fields |
|---|---|---|
| `user` | yes | display name, credential bindings (section 7) |
| `group` | **no** | display name, member list (users, agents, groups), administrators |
| `agent` | yes | `owner` (a user or an agent), delegation (section 3), credential bindings |

A *team* is a group. The registry does not add a separate kind for teams,
because teams and groups have the same semantics. A team's graphs are ordinary
graphs whose ACLs name the team's group.

Rules:

- **Opaque, never reused IDs.** A principal ID is a core v1 identifier (a
  nonempty UTF-8 string of at most 512 bytes). It carries no prefix
  semantics. IDs are unique across all kinds within a tenant. A principal is
  never deleted. It is *retired*, and its ID can never be reissued. A new
  principal that reused an old ID would silently gain every record that names
  the old one.
- **Existing literal readers keep working.** To keep access to existing
  records, an administrator registers a principal whose ID equals the string
  those records already use. This includes the hex public keys that signed
  admission uses as principals today. Nothing is rewritten.
- **Groups never authenticate.** A group ID inside `readers` is still a
  literal and matches no one, because no request ever carries a group as its
  principal. Groups are named only through `reader_groups` and in ACLs. This
  keeps `readers` from changing meaning.
- **Nested groups are allowed but bounded.** Membership is acyclic, nesting
  depth is at most 4, and the transitive group closure of any principal is
  at most 1,024 groups. A registry change that would break a bound is
  rejected with `E_BUDGET`. Reads therefore never fail on registry size.
- **Status.** A principal is `active`, `suspended` or `retired`. A suspended
  or retired principal cannot authenticate, and it is removed from every
  closure. Suspending an agent's owner suspends the agent's effective
  authority (section 3).

**The registry is a reserved graph.** The registry is stored as fact records
in the reserved graph `weave:registry` of the tenant store. The `weave:`
namespace is already reserved for the engine (core v1 section 11.6). Raw
commits, fact writes, capsule receipt and branch acceptance reject that
graph, as they reject `weave:identity:` today. Only the registry API writes
it. Each registry change is therefore a fact write with an engine-recorded
writer and recorded time (ADR-0005). Registry history, audit and "who was in
group G on date D" come from the A2 fact log and A3 as-of queries with no
second history mechanism. The registry graph is readable by tenant
administrators. A principal can read its own record and its own memberships.
A group's administrators can read that group's members.

### 2. Identity sets and record visibility

For one request, the engine resolves the authenticated principal into an
**identity set** once, from the registry head pinned at request start:

- user `u`: `S(u) = {u} ∪ G*(u) ∪ A*(u)`, where `G*(u)` is the transitive
  closure of active groups that contain `u`, and `A*(u)` is the set of
  active agents that `u` owns, directly or through other agents. An owner can
  therefore see everything its agents can see, which keeps owner audit of
  agents possible.
- agent `a`: section 3.

A record matches an identity set `S` when one of these holds:

1. `readers` and `reader_groups` are **both** empty (the record is public);
2. some entry in `readers` is in `S` (literal principal match, unchanged);
3. some entry in `reader_groups` is in `S` (a group in the closure).

`reader_groups` widens access only alongside `readers` (a disjunction, like
`readers` itself). One rule changes: a record with empty `readers` but
nonempty `reader_groups` is **not** public. Records written before B1 have no
`reader_groups`, so for them rules 1 and 2 are exactly today's `allowed`
function. Their results do not change.

Group references fail closed. A `reader_groups` entry that names an unknown,
retired or non-group ID matches no one. Writes validate the field: every
named group must exist and be discoverable by the writer (section 4). Any
other value is rejected with `E_UNAVAILABLE`, the same code as a nonexistent
group, so a write cannot probe for hidden groups.

**Derived records keep conjunctions.** Today the engine intersects literal
reader lists when it combines records (for example an assertion with its
edge). An intersection of two group-based reader sets cannot be flattened
into one reader list without resolving membership at write time. That would
freeze membership and break revocation. The rule is:

- if no input has `reader_groups`, the engine keeps today's literal
  intersection, byte for byte;
- otherwise the derived record carries the restriction as an AND of its
  inputs' reader clauses, through the existing influence carriers, and
  resolves each clause at read time.

ADR-0008 (B2) defines the write-time form of this conjunction. ADR-0007
fixes only that group-based restrictions are never flattened.

### 3. Agents and delegation

An agent is a principal with exactly one `owner`, which is a user or another
agent. Ownership chains end at a user and have at most 8 links, matching the
8-token bound on `weave-policy` chains. An agent's **delegation** states:

- `scopes`: graph/branch pairs with permissions (section 4);
- `exercise`: which of the owner's identities the agent may act with. It is
  a subset of `S(owner)`, or the symbolic `owner_all`, which means all of
  `S(owner)` as resolved for each request;
- `expires_at_ms` (optional), checked against the trusted operation clock;
- `may_delegate` and the remaining depth for sub-agents.

The agent's own identity set is
`S_local(a) = {a} ∪ G*(a) ∪ exercise(a)`. The rule that keeps an agent under
its owner is:

```
can(a, p, X) = can(owner(a), p, X)
             ∧ scope(a) grants p on X's graph and branch
             ∧ (p is not a record read, or X matches S_local(a))
```

Here `p` is a permission and `X` a graph, branch or record. The definition is
recursive up the ownership chain. By construction, `can(a)` is a subset of
`can(owner(a))`, and the owner's authority is evaluated **now**, for every
request. A later loss of access by the owner removes the agent's access on
the agent's next request. An agent added to a group that its owner is not in
gains nothing from that group.

Escalation rules, each with a negative test in B1:

- An agent cannot create a sub-agent whose scope or `exercise` exceeds its
  own. The registry rejects it, and the next request also re-evaluates it.
- An agent cannot change its own or its owner's registry entries,
  memberships or ACLs. Administer is not delegable to agents in B1
  (open question 3).
- An agent cannot name a group in `reader_groups` that it cannot discover.
- Delegation never clears record restrictions. A broad graph scope only
  allows a request to reach a graph. It does not reveal any record there.

### 4. Permissions, graph ACLs and Discover

Permissions form an ordered set: **Discover ≤ Read ≤ Write ≤ Administer**.
Each permission implies those below it.

| Permission | Graph | Branch | Record |
|---|---|---|---|
| Discover | the graph appears in listings, and its ID resolves | the branch appears | a record is never listed apart from Read |
| Read | query, traverse, as-of, change sets, explain | read that branch | `readers` and `reader_groups` (section 2) |
| Write | fact writes and commits on unprotected branches | write a protected branch | correct or retract it only when Read also holds |
| Administer | edit the graph ACL, protect branches, create branches | edit a branch ACL | none (no record ACLs in B1) |

**Graph ACLs.** Each graph has an ACL of entries `(principal or group,
permission)`. An entry for a group applies to the group's closure. The ACL
is registry state, so it is versioned and audited (section 1), not part of
the graph's revisions. A graph with no ACL entries is visible only to tenant
administrators. Graphs are closed by default.

**Branch ACLs.** An administrator can *protect* a branch. A protected branch
has its own write ACL, and graph Write does not apply to it. Writing it
needs a branch entry (or graph Administer), plus graph Read. An unprotected
branch takes graph Write. Branch ACLs grant only Write. Read is per graph.

**Discover.**

- Without Discover on a graph, every operation that names it fails
  `E_UNAVAILABLE` with the same message as a nonexistent graph. The graph is
  absent from listings and counts, and no listing reports a total.
- With Discover but not the needed permission, the operation fails
  `E_FORBIDDEN`. That holder already knows the graph exists.
- Records have no separate Discover. A hidden record is always reported as
  `E_UNAVAILABLE` or omitted with `partial` coverage, as it is today.
- **Graph creation cannot probe IDs.** In the B3 service, a created graph
  gets an engine-assigned opaque ID by default. Choosing a graph ID needs
  tenant Administer, because a collision would otherwise reveal a hidden
  graph's existence.
- References from a record to a graph the reader cannot discover, such as
  `GraphRef` evidence or derivation snapshots, behave like references to an
  unavailable revision, which the engine already handles.

**Mapping to `weave-policy` actions.** Capability scopes keep their existing
actions. B1 maps them as follows, and adds no new action:

| B1 permission | `weave-policy` action(s) |
|---|---|
| Discover | `discover` |
| Read | `read` and `traverse` (admission already requires both) |
| Write | `publish`. `propose` stays the B5 review path and implies no Read |
| Administer | none. Registry and ACL management stay trusted host operations and are never signed plan actions, as [ADMISSION.md](../ADMISSION.md) already requires |
| delegation to sub-agents | `delegate` |

**Local trusted-host profile.** A store with no registry keeps today's
behavior exactly: `HostContext.principal` matching against literal
`readers`, with `writable_graphs`. Graph ACLs, Discover and groups apply
only once the registry is enabled for a store. Enabling it is an explicit,
one-way, audited administration step.

### 5. Revocation, history and as-of queries

**Revocation takes effect on the next request.** A request starts by pinning
the registry head inside its read transaction and resolves identity sets
from that head. A request that starts after a registry change commits sees
the change. A request already running finishes on its pinned head.
Anything that outlives a request rechecks authority:

- stored views, admission retry caches, change-feed cursors and A7/B4 tool
  sessions revalidate current authority on every delivery, as
  `identity_acceptance` and the cached-receipt fix already do (`AllowStale`
  never grants stale authority);
- every authority-reducing registry change advances the tenant's admission
  policy epoch. Examples are a membership removal, a suspension, an ACL
  narrowing, a delegation narrowing and a credential revocation. Pending
  verified requests and cached signed retries under the old epoch then need
  a fresh request, as [ADMISSION.md](../ADMISSION.md) specifies. Additive
  changes do not advance the epoch.

**No rewrite.** Revocation changes registry facts only. It rewrites no
revision, index row or checkpoint, because records name groups and the index
holds no visibility (ADR-0004 section 11).

**History is read under current authority.** An as-of query, a recorded-time
selection, a change set between K1 and K2, an explanation and a pinned
revision read are all authorized with the registry as it is **now**, never
as it was at the selected time:

- a principal removed from group G today cannot read G-restricted facts as
  of last year, even though it could read them then;
- a principal added to G today can read G-restricted history. Joining a
  group grants the group's history, which is what group membership means
  for a shared knowledge base;
- the registry's own history answers the audit question "who could read F at
  time T". It is reconstructed from registry facts as of T, read by an
  auditor, and never used to authorize a request.

This matches the existing rule that historical pins do not bypass revocation
([IDENTITY_ACCEPTANCE_DESIGN.md](../IDENTITY_ACCEPTANCE_DESIGN.md)).
Recorded history stays immutable. Only the set of principals who may read it
changes.

### 6. Tenancy: one store per tenant (B3)

- **A tenant is one store.** Each tenant has its own database, opened through
  the A2 `FactBackend` abstraction, with its own registry, graphs, admission
  state and audit log. A second backend (C4) changes nothing here.
- **Tenant from the credential.** A request's tenant comes from its
  authenticated credential, never from the request body or path. A request
  that names another tenant's graph or store fails `E_UNAVAILABLE`, and the
  service writes an audit record of it in the credential's own tenant.
- **No cross-tenant references.** Principal and graph IDs are unique only
  within a tenant. A reference that would resolve in another tenant is
  unresolvable, and fails closed with partial coverage. One person in two
  tenants has two principals.
- **Control plane.** The service keeps a separate control store for tenant
  definitions, the store locations, OIDC issuer settings and hashed API
  tokens with their tenant binding. It holds no graph data. A tenant's
  registry and data live together in the tenant store, so a backup restores
  data and authority together, and a revocation commits atomically with the
  data it protects.
- **Audience per tenant.** Each tenant has its own `weave-policy` audience,
  so a capability issued for one tenant is never valid in another.
- Row-level tenant keys inside a shared store are not used. They are added
  only if a later need is shown (roadmap amendment of 2026-10-18).

### 7. Authentication mapped to principals (B3)

Every endpoint authenticates the request to exactly one active, non-group
principal of one tenant before any engine call. Unauthenticated requests are
rejected on every endpoint, and the error carries no graph or tenant
information.

| Credential | Bound to | Notes |
|---|---|---|
| API token | one user or agent and one tenant | 256 random bits, shown once, stored only as a SHA-256 hash in the control store, with expiry, revocable, never logged |
| OIDC ID or access token | a user, through the registry binding `(issuer, subject)` | signature and claims checked against the configured issuer. The IdP's group claims are **not** live Weave groups (section 11, alternative 6) |
| Agent token | one agent | an API token whose principal is the agent. It can never authenticate as the owner |
| ed25519 key | a user or an agent, through a registry credential binding | used by signed admission (section 8) |

The authenticated principal ID, and never a key, token or display name, is
what the engine receives as `HostContext.principal`. It is recorded as the
ADR-0005 `writer` and in the B3 audit log.

**Transport.** B3 serves HTTP/JSON over TLS first. The request and response
bodies are the existing core v1 JSON contract. gRPC is optional and deferred
until a measured need exists. It would carry the same bodies and the same
authentication.

### 8. Relation to signed admission and capability chains

The registry is authoritative. Capability chains are the wire form of
delegation for agents that hold keys. A signed request is admitted only when
both of these hold:

1. **The chain is valid under `weave-policy`.** Signatures, audience,
   expiry, attenuation, nonce and epoch are checked exactly as today. In the
   service, the `RootAuthority` set is derived from the registry and is no
   longer installed by hand. Each active user's bound key is a root whose
   scopes are that user's current ACL grants, mapped through section 4. The
   tenant's revoked keys and capabilities come from registry credential
   revocations.
2. **The registry agrees.** The leaf subject key is bound to an active
   principal. Every key along the chain belongs to that principal's
   ownership chain. The request's operation is allowed by `can(...)` under
   the current registry (section 3).

So a capability can only narrow what the registry allows, never widen it.
Capability scope never clears record restrictions, which ADMISSION.md
already states. Bearer-token requests skip step 1 and use step 2 only.
Nonce receipts, body binding, retry caches and the per-subject quotas stay
as they are, with the policy epoch advanced by registry changes (section 5).

**Provenance of delegated writes.** The writer recorded for a fact is the
acting principal: the agent, not its owner. B1 adds `on_behalf_of` to the
engine-recorded part of `FactProvenance`. It holds the ownership chain from
the agent's owner to the root user at write time. Like `writer`, it is
outside content, so it never enters a revision ID. It is disclosed to the
fact's readers, as ADR-0005 already does for `writer`.

### 9. Audit

- Registry changes are facts in `weave:registry` (section 1).
- B3 keeps an append-only read and write audit log per tenant store. Each
  entry holds the principal, `on_behalf_of`, the operation, the graph, the
  revision, the fact references returned or written, a result digest and the
  recorded time. It answers "who read fact F" and "who wrote fact F". The
  log is a separate table and not fact-log data, because read volume would
  dominate the fact log. Only tenant auditors can read it.
- Denied requests are audited without the denied topology. An entry records
  that a request was denied, never which hidden object caused it.

### 10. Noninterference obligations for B2

B2's property harness (ADR-0008) must treat the registry as an input as
restricted as any fact. It must show that:

1. **Group-based readers.** Generated pairs differ in facts hidden through
   `reader_groups`, nested groups and agent ceilings, as well as through
   literal `readers`. Outputs are identical: values, counts, ranks,
   explanations, errors, coverage and `fact_provenance`.
2. **Hidden registry state.** Membership of groups the reader cannot
   discover, and the existence of hidden principals, change no output,
   including write validation errors for `reader_groups`.
3. **Discover.** Pairs that differ only in an undiscoverable graph give
   identical listings and errors for every operation that names a graph.
4. **Conjunctions survive.** No derivation path, including analytics, kNN,
   candidate relations, embeddings and persisted results, flattens a
   group-based restriction into literal readers or loses one clause.
5. **Agent ceiling.** For every operator, an agent's output equals its output
   under `can(owner) ∧ scope ∧ S_local`. Pairs where only the owner's access
   differs change the agent's output only by removing data.
6. **Revocation.** After an authority-reducing change, no cache, view,
   change feed, embedding index or C5 approximate index returns anything
   computed for the old authority. Permission filters for indexes evaluate
   the identity set at query time.
7. **Declared channels.** The harness lists the channels it does not close,
   and SECURITY.md documents them. Examples: timing and size; the occupancy
   of client-chosen record IDs by hidden records (open question 1); writer
   and `on_behalf_of` identities disclosed to a fact's readers.

### 11. Alternatives considered

1. **A `group:` prefix inside `readers`.** Rejected in A2. Pre-0.22 data
   could already use such strings as literal principal IDs, so it would
   reinterpret stored data.
2. **Resolve groups into `readers` at write time.** This needs no read-time
   resolution, but it freezes membership. Revocation would then need a
   rewrite of history, which breaks immutability and ADR-0001 identities.
   Rejected.
3. **Authorize history with the authority of its time.** "You could read it
   then, so you can read it as of then." Rejected: a removed member would keep
   access to everything up to the removal, which is the leak revocation exists
   to stop.
4. **Capabilities only, with no registry.** `weave-policy` chains alone cannot
   name a group on a record, and removing a member would mean reissuing every
   affected token. Kept as the delegation wire form, under the registry.
5. **An external authorization service** (a Zanzibar-style system such as
   OpenFGA or SpiceDB). This would be a second source of truth outside the
   tenant store, with no atomicity with data writes and no as-of audit
   through the fact log. Rejected for B1. The registry's
   `(subject, relation, object)` shape keeps a later adapter possible.
6. **Live IdP groups from OIDC claims.** Their revocation depends on token
   lifetime and on the IdP, and they are not audited in Weave. Rejected as the
   authority. A later sync job may import IdP groups into the registry, where
   the normal rules apply.
7. **Agents as plain users with their own grants.** This is simpler, but
   nothing stops an agent from exceeding its owner. Rejected.
8. **Row-level tenant keys in one shared store.** One missed predicate leaks
   across tenants. Rejected by the owner's roadmap amendment.
9. **Group-owned agents.** This would make team bots convenient, but a group
   cannot authenticate or answer for an agent, and the ceiling would need a
   group's "scope". Deferred (open question 2).

### 12. Consequences

- `allowed` becomes a function of an identity set, and the public rule
  includes `reader_groups`. Every literal-reader check must go through one
  predicate. This includes the policy reader checks in `governance.rs`,
  `governance_graph.rs` and `accepted_history.rs`.
- Each request does one registry resolution, which is bounded by the closure
  limit. The cost of a group change is one registry write, and no data is
  touched.
- Existing stores and the local trusted-host APIs keep their behavior. The
  registry is opt-in for local stores and mandatory in the B3 service.
- A contract change is needed. It activates `reader_groups` on every fact
  record, the registry API and `on_behalf_of` in `FactProvenance`, with a
  minor protocol bump, golden fixtures and the language vendor update.
- A store migration is needed. It installs the registry tables or graph
  indexes, adds `on_behalf_of` storage if A2 did not reserve it, and adds the
  audit log tables in B3.
- Joining a group grants the group's history. Deployments that need
  date-bounded access must use separate graphs or groups per period, until a
  release policy (B2) or open question 4 adds more.

### 13. What Phase A must reserve

A2 should keep the following true. None of it needs B1 code now:

1. **`reader_groups` encodes as absent when empty.** The field is omitted
   when empty on every fact kind, including A4 embedding facts. Records
   without groups then keep the exact canonical bytes, revision IDs and
   `fact:sha256:` IDs they have before B1.
2. **`reader_groups` is fact content.** It is inside the record body and the
   fact revision hash, like `readers`. It is never an index column.
3. **Writer is a principal ID, and delegation has a place to go.** The
   `writer` values in `fact_log` and `fact_revisions` hold the
   `HostContext.principal` string, never a key or a display name. Either
   `on_behalf_of` gets a reserved nullable place in the engine-recorded
   provenance (preferred), or B1's migration adds it. It is outside content.
4. **One visibility predicate.** New A2 to A4 read paths (indexed `query`,
   `expand`, change sets, `nearest`) call one shared visibility function and
   never re-implement `readers.is_empty() || contains`. B1 can then extend
   one place.
5. **The `weave:registry` graph ID.** It stays inside the reserved `weave:`
   namespace. Fact writes to `weave:` graphs are rejected, as raw commits to
   `weave:identity:` are.
6. **Reader intersections stay in one place.** Any new A2 to A4 code that
   intersects readers for a derived record goes through one helper, so B1
   can switch it to a conjunction when groups are present (section 2).

### 14. Open questions

1. **Occupancy of record IDs.** A write that asserts a client-chosen ID that
   a hidden live record already holds fails `E_CONFLICT`, which reveals that
   the ID is taken. The lean is that B4 tools generate record IDs
   server-side, and the remaining channel is declared to principals with
   Write (section 10, item 7). B2 decides.
2. **Service principals and group-owned agents.** Should a team own a shared
   agent through a designated responsible user instead?
3. **Delegable Administer.** Should an agent ever manage memberships, for
   example for onboarding automation, and under what extra review?
4. **Time-bounded membership.** Expiry is supported (it acts as a scheduled
   revocation). Should membership also carry a valid-time window that limits
   which history a member may read?
5. **Record-level write ACLs.** Is "only the original writer may correct
   this fact" needed beyond B5 review?
6. **Registry caching.** Per-request resolution is the baseline. If B3's
   50-client load test shows it is too expensive, a cache keyed by registry
   revision is the fallback. That cache could never outlive a change.
7. **Cross-tenant sharing.** Capsule export between tenants stays deferred
   (E08). Is a federated read ever in scope?
