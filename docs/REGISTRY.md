# Principals, groups and graph ACLs

The principal registry is how a store shares knowledge between people and
teams (B1, [#38](https://github.com/weave-graph/weave-engine/issues/38),
[ADR-0007](architecture/ADR-0007-identity-groups-and-tenancy.md)). It holds
users and groups (a team is a group), group membership, graph ACLs and
protected branches. Records name groups through `reader_groups`, and
membership is resolved when each request runs, so a revocation takes effect on
the next request and nothing stored is rewritten.

Agents with an owner and a narrowed delegated scope are the second part of
B1 and are not available yet.

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

- `HostContext.principal` must be an **active user** of the registry. An
  unregistered, suspended or retired principal, or a group, can read only
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
| `set_status` | `id`, `status` (`active`, `suspended`, `retired`) | tenant administrators |
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
- Additive changes (`add_principal`, `add_member`, `grant_branch_write`,
  `unprotect_branch`) leave the signed-admission policy epoch alone. Every
  other change advances it to `registry:<revision>`, so pending signed
  requests and cached signed retries need a fresh request.

## Reading records: `readers` and `reader_groups`

A record is visible to a request when:

1. `readers` and `reader_groups` are both empty (public), or
2. some `readers` entry is the request's principal, or
3. some `reader_groups` entry is an active group the principal is in,
   directly or through nested active groups.

A group ID inside `readers` is a literal that matches no one. A record with
empty `readers` and a nonempty `reader_groups` is **not** public.

A write (`write` or `commit`) may name only active groups the writer can
discover: groups it is in, groups it administers, or any group for a tenant
administrator. Anything else (an unknown ID, a hidden group, a retired group,
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
- Record restrictions still apply inside a readable graph.

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
  registry and fail with `E_FORBIDDEN` otherwise. Governance and identity
  graphs (`weave:governance:`, `weave:identity:`) keep their own policy
  readers, which match the request's principal as before.
- Timing and response size are not protected channels. Writer identities
  are disclosed to a fact's readers, as before.
- Agents (owned principals with delegated scope, `on_behalf_of` on their
  writes) are the next B1 change. Authentication, tenants and the audit log
  are B3.
