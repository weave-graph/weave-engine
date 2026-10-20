# Principals, groups, agents and graph ACLs

The principal registry is how a store shares knowledge between people and
teams (B1, [#38](https://github.com/weave-graph/weave-engine/issues/38),
[ADR-0007](architecture/ADR-0007-identity-groups-and-tenancy.md)). It holds
users, groups (a team is a group) and agents, group membership, agent
delegation, graph ACLs and protected branches. Records name groups through `reader_groups`, and
membership is resolved when each request runs, so a revocation takes effect on
the next request and nothing stored is rewritten.


## Local stores and registry stores

A store starts **without a registry**. It keeps the local trusted-host
profile exactly as before:

- `HostContext.principal` is matched against literal `readers` (empty means
  public);
- `HostContext.writable_graphs` grants writes;
- every graph is discoverable and readable;
- any `reader_groups` value is rejected on write with `E_UNAVAILABLE`, and a
  record that has one (for example from a capsule) is visible only through
  its `readers`.

`Engine::enable_registry(host)` turns the registry on. It is one-way and
recorded: the host's principal becomes the first user and a member of the
built-in group `weave:administrators` (the tenant administrators). From then
on:

- `HostContext.principal` must be an **active user or agent** of the
  registry. An unregistered, suspended, retired or expired principal, or a
  group, can read only
  public records of graphs it can discover, which is none, so every request
  fails as if the graphs did not exist.
- Graph ACLs decide every graph operation, and `writable_graphs` is no longer
  consulted.
- `reader_groups` resolve through group membership.

Existing literal readers keep working: register a principal whose ID equals
the string the records already use (for example a signed-admission public
key). Nothing is rewritten.

From the command line:

```sh
weave-engine registry enable --db store.db --actor root
weave-engine registry apply  --db store.db --actor root changes.json
weave-engine graphs          --db store.db --actor alice
```

## Changes

`Engine::apply_registry(changes, host)` applies a list of `RegistryChange`
values atomically, as one fact revision of the registry, written by
`host.principal`. Changes are checked in order, each against the state the
earlier ones produced. At most 1,000 changes per call.

| `change` | Fields | Who may make it |
|---|---|---|
| `add_principal` | `id`, `kind` (`user` or `group`), `display_name?` | tenant administrators |
| `add_agent` | `id`, `owner`, `delegation`, `display_name?` | the owner, a principal above it in its ownership chain, or a tenant administrator; an agent owner needs `may_delegate` |
| `set_delegation` | `agent`, `delegation` | a principal in the agent's ownership chain (never the agent itself) or a tenant administrator |
| `set_status` | `id`, `status` (`active`, `suspended`, `retired`) | tenant administrators; for an agent, also any principal in its ownership chain |
| `add_member`, `remove_member` | `group`, `member` | tenant administrators and the group's administrators |
| `set_group_administrators` | `group`, `administrators` | tenant administrators |
| `grant` | `graph_id`, `subject`, `permission` | holders of Administer on the graph |
| `revoke` | `graph_id`, `subject` | holders of Administer on the graph |
| `protect_branch`, `unprotect_branch` | `graph_id`, `branch_id` | holders of Administer on the graph |
| `grant_branch_write`, `revoke_branch_write` | `graph_id`, `branch_id`, `subject` | holders of Administer on the graph |

```json
[
  {"change": "add_principal", "id": "alice", "kind": "user"},
  {"change": "add_principal", "id": "maintenance", "kind": "group", "display_name": "Maintenance team"},
  {"change": "add_member", "group": "maintenance", "member": "alice"},
  {"change": "grant", "graph_id": "fleet", "subject": "maintenance", "permission": "write"}
]
```

Rules:

- **IDs are never reused.** A principal is never deleted; it is retired, and
  retirement is final. Adding an existing ID, even a retired one, fails with
  `E_CONFLICT`. IDs in the `weave:` namespace are reserved
  (`E_RESERVED_NAMESPACE`).
- **Groups are bounded.** Membership is acyclic, groups nest at most 4 deep,
  and a principal is in at most 1,024 groups transitively. A change that
  would break a bound fails with `E_BUDGET` and changes nothing, so reads
  never fail on registry size.
- **Status.** A suspended or retired principal cannot act and is removed
  from every closure. Suspending a group removes it, and every group reached
  only through it, from its members' closures.
- **At least one administrator.** The last active tenant administrator
  cannot be removed or suspended (`E_CONFLICT`).
- Additive changes (`add_principal`, `add_agent`, `add_member`,
  `grant_branch_write`, `unprotect_branch`) leave the signed-admission policy epoch alone. Every
  other change advances it to `registry:<revision>`, so pending signed
  requests and cached signed retries need a fresh request.

## Agents

An agent is a principal with exactly one owner, a user or another agent,
and a **delegation**. Ownership never changes, and chains end at a user
within 8 links (the `weave-policy` chain bound; a longer chain fails with
`E_BUDGET`). An authenticating host maps an agent's credential to the
agent's ID and passes it as `HostContext.principal`; an agent can never
authenticate as its owner.

```json
{"change": "add_agent", "id": "alice-planner", "owner": "alice", "delegation": {
  "scopes": [{"graph_id": "fleet", "permission": "write", "branches": ["draft"]},
             {"graph_id": "manuals", "permission": "read"}],
  "exercise": {"kind": "only", "identities": ["maintenance"]},
  "expires_at_ms": 1767225600000,
  "may_delegate": false}}
```

| Field | Meaning |
|---|---|
| `scopes` | The graphs the agent may reach, each with a permission (at most Write) and optional `branches` to which its Write is limited. A graph not listed does not exist for the agent |
| `exercise` | Which of the owner's identities the agent reads with: `{"kind": "owner_all"}`, or `{"kind": "only", "identities": [...]}` (the default, with none). Exercised identities count only while the owner still holds them |
| `expires_at_ms` | Optional; from this trusted-clock time the agent has no authority |
| `may_delegate` | Whether the agent may register sub-agents |

On every request the agent's authority is

```
can(agent, p, X) = can(owner, p, X) ∧ scope(agent) grants p on X's graph
                   ∧ (p is not a record read, or X matches S_local(agent))
```

where `S_local(agent)` is the agent itself, its own groups and its exercised
identities. The owner's authority is evaluated **now**, up the whole chain,
so:

- an agent never exceeds its owner: when the owner loses a grant, a group or
  its active status, the agent loses it on its next request, whatever its
  delegation says;
- Administer is not delegable: an agent holds at most Write on any graph,
  cannot write a protected branch through its owner's Administer (it needs
  its owner's branch entry), cannot read or edit ACLs, and cannot change
  memberships, statuses or group administrators;
- a broad scope only lets the agent reach a graph. Records stay restricted:
  the agent sees a record only if both its owner and its own `S_local`
  match it. An agent added to a group its owner is not in gains nothing
  from that group;
- an agent may name in `reader_groups` only groups in every clause of its
  identity, which means groups its owner holds and it exercises or is in.

**Escalation is refused at write time too.** `add_agent` and
`set_delegation` reject Administer in a scope, exercised identities the owner
does not hold, and, under an agent owner, any scope, branch list or expiry
outside the owner's own delegation (`E_FORBIDDEN`). An agent may register
sub-agents only with `may_delegate`, and may change only agents below it in
its ownership chain, never itself or its owner.

**Revocation.** The owner (or anyone above it, or a tenant administrator)
suspends, narrows or retires an agent with `set_status` or
`set_delegation`; it takes effect on the agent's next request. Narrowing an
agent also narrows every sub-agent below it at once, because the ceiling is
re-evaluated per request.

**Owner audit.** An owner's identity includes its active agents, so the
owner reads every record its agents can read. `registry_principal(agent)` and
`registry_agents(owner)` are readable by the agent itself, by every principal
in its ownership chain and by tenant administrators.

**Delegated writes.** A fact write by an agent records `writer` (the agent)
and `on_behalf_of` (its ownership chain, owner first, ending at the root
user), both engine-recorded and outside content. Results of 1.1.0 programs
list `on_behalf_of` in `fact_provenance`; older programs never see it.
Agents write through fact-level `write` only: a snapshot `commit` by an
agent fails with `E_FORBIDDEN`, because snapshot facts carry no per-fact
delegation.

## Reading records: `readers` and `reader_groups`

A record is visible to a request when:

1. `readers` and `reader_groups` are both empty (public), or
2. some `readers` entry is the request's principal (for a user, also one of
   its active agents), or
3. some `reader_groups` entry is an active group the principal is in,
   directly or through nested active groups.

A group ID inside `readers` is a literal that matches no one. A record with
empty `readers` and a nonempty `reader_groups` is **not** public.

A write (`write` or `commit`) may name only active groups the writer can
discover: groups it is in, groups it administers, or any group for a tenant
administrator. For an agent, see above. Anything else (an unknown ID, a hidden group, a retired group,
a user) fails with the same `E_UNAVAILABLE` error, so a write cannot probe for
groups. `reader_groups` needs protocol 1.1.0 (`E_VERSION` under older
programs).

**History is read under current membership.** A pinned revision, an
`as_of` query, a change set and an explanation are all authorized with the
registry as it is now. A member removed today cannot read group records as
of last year; a member added today can read the group's history. Who could
read a fact at time T is answered from the registry's own history (below),
never used to authorize a request.

**Derived records** keep an AND of their inputs' reader clauses. An
explicit assertion materialized with its structural edge keeps the literal
intersection when neither names a group, exactly as before. When one input
names a group, the edge carries that clause unchanged; when both are
restricted and one names a group, the edge carries their clause-wise meet
(common principals and common groups), which never admits a reader that
either input denies. B2 replaces the meet with the exact carrier form.

## Graph permissions and Discover

Permissions are ordered: **Discover ≤ Read ≤ Write ≤ Administer**.

| Permission | Allows |
|---|---|
| Discover | the graph is listed by `Engine::list_graphs`, and operations that name it get `E_FORBIDDEN` instead of the nonexistent-graph error |
| Read | `query`, `expand`, `as_of`, `changes`, `nearest`, `similar_candidates`, recorded history, and references from other graphs |
| Write | `write`, `commit` and embedding imports on unprotected branches |
| Administer | the graph's ACL and its protected branches; every branch |

- An ACL entry names a user or a group. A group entry applies to every
  member, transitively. The highest entry that applies wins.
- **Graphs are closed by default.** A graph with no ACL entry is visible
  only to tenant administrators, who hold Administer on every graph.
- **Without Discover**, every operation that names the graph fails with
  `E_UNAVAILABLE` and the exact message a nonexistent graph gives, and the
  graph is absent from listings, which report no total. References to it from
  other graphs (metadata, evidence, derivation pins) behave like unavailable
  revisions.
- **With Discover but not the permission** an operation needs, it fails with
  `E_FORBIDDEN`.
- **Protected branches.** A protected branch is written only by subjects
  named in its branch ACL (who also need graph Read) and by Administer
  holders. Graph Write applies to unprotected branches only. Branch ACLs
  grant only Write; Read is per graph.
- Writing a graph that does not exist yet needs an ACL entry for it, so an
  administrator grants first. A write to a graph the writer cannot discover
  fails like a write to a nonexistent one.
- `Engine::registry_nameable_groups(host)` lists exactly the groups a write
  by `host.principal` may name in `reader_groups`, for hosts that serve a
  visible manifest (weave-language `weave-visible/1`).
- `Engine::create_graph(entropy, host)` lets any active user create a graph
  without choosing its ID: the engine derives `g-` and 32 hex digits from 32
  bytes of host-supplied randomness and grants the creator Administer, in
  one registry revision written by the creator. The B3 service uses it, so
  graph creation cannot probe for hidden graph IDs.
- Record restrictions still apply inside a readable graph.
- **Correcting or retracting needs Read on the fact.** A `correct` or
  `retract` of a live fact the writer cannot read (its own `readers` and
  `reader_groups`, and for an edge its endpoint nodes') fails exactly as for
  a fact that does not exist, expectations included. An `assert` of an ID a
  hidden live record holds still fails `E_CONFLICT` (a declared channel; the
  B3 service closes it with minted record IDs).

## The registry graph

The registry is stored as facts in the reserved graph `weave:registry`
(branch `main`): one node per principal, one edge per membership
(`weave:member`, member to group), one node per graph or protected branch
with an ACL, and one edge per ACL entry (`weave:grant`,
`weave:branch-writer`). Raw commits, fact writes and every other write path
reject the graph with `E_RESERVED_NAMESPACE`; only the registry API writes
it.

Every active principal can query `weave:registry`. Its records are
restricted: a principal sees its own record, its memberships and the groups
it is in; tenant administrators see everything. Because registry changes
are ordinary fact revisions, audit questions are ordinary queries, for
example "who was in group G at time T":

```json
{"op": "evaluate", "value": {"kind": "as_of", "unix_millis": 1767225600000,
  "query": {"graph_id": "weave:registry", "predicate": "weave:member", "to": "G"}}}
```

The native read API applies the same rules:
`registry_principal(id)` and `registry_memberships(id)` (the principal
itself or a tenant administrator), `registry_members(group)` (the group's
administrators or a tenant administrator) and `registry_graph_acl(graph)`
(Administer on the graph). Anything else fails with `E_UNAVAILABLE`.

## Limits and non-guarantees

- A request resolves its identity from the registry head it first reads.
  Engine requests run in one SQLite read transaction, so a change committed
  before a request starts applies to it, and a request already running keeps
  its snapshot. Native calls outside a program that perform several separate
  reads can observe a concurrent change from another connection between
  them.
- Resolution is cached per registry revision and principal; no cache
  outlives a registry change.
- The registry holds at most 100,000 records; a graph ACL or a branch ACL
  holds at most 4,096 entries; a group has at most 256 administrators.
- Peripheral subsystems that write graphs (dispatch adapters, compiled
  handlers, capsules, integration, retention) check graph Write under the
  registry and fail with `E_FORBIDDEN` otherwise. A graph in `reviewed`
  mode grants no direct write authority to anyone: writes are proposals that
  reviewers accept ([REVIEW.md](REVIEW.md)), and `write` and `commit` fail
  with `E_REVIEW_REQUIRED` for principals with Write. Governance and identity
  graphs (`weave:governance:`, `weave:identity:`) keep their own policy
  readers, which match the request's principal as before.
- Timing and response size are not protected channels. Writer identities
  are disclosed to a fact's readers, as before.
- Delegation expiry is checked against the trusted operation clock of the
  request.
- Authentication (mapping tokens, OIDC or keys to principals), tenants and
  the read and write audit log are B3.
- Retracting a fact that a hidden live fact depends on (for example a node
  with a hidden edge) fails with `E_DEPENDENT_FACT`, which reveals that a
  dependent exists. It is a declared channel until B2's noninterference
  harness decides the rule.
