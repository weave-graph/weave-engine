# Optional review of shared knowledge

Teams choose, per graph, whether facts written into it become shared at once
or only after review (B5, [#12](https://github.com/weave-graph/weave-engine/issues/12)).
A personal graph publishes immediately. A team graph can require that
agents' and people's writes are *proposals* that reviewers accept or reject
before anyone reads them.

Review is a trusted engine API in registry stores
([REGISTRY.md](REGISTRY.md)): every principal is a registry user, group or
agent, and every check uses graph ACLs, delegations and group membership as
they are when the request runs. The B4 agent interface and the service
endpoints for proposing and reviewing are the next part of the epic.

## Review modes

| Mode | Writes | Who changes it |
|---|---|---|
| `direct` (default) | publish immediately, as before | a holder of Administer on the graph |
| `reviewed` | are refused; principals with Write propose instead | the current reviewers' quorum (see [Policy changes](#policy-changes)) |

```json
{"kind": "reviewed", "reviewers": ["maintenance-reviewers"], "threshold": 2}
```

- `reviewers` names 1 to 32 users or groups. A group admits every active
  user in it, directly or through nested groups. Agents never review, and an
  agent ID is refused (`E_REVIEW_POLICY`). An unknown principal fails with
  `E_UNAVAILABLE`, as for a graph grant.
- `threshold` (1 to 32) is the number of distinct reviewers who must approve.
- A graph with no policy is `direct` at revision 0. Each change adds a
  revision; earlier revisions stay recorded.

In a reviewed graph every direct write path is refused: `write` and `commit`
fail with `E_REVIEW_REQUIRED` for principals who hold write authority (others
keep getting `E_FORBIDDEN` or the nonexistent-graph error), and peripheral
writers (adapters, compiled handlers, capsule receipt, integration, branch
acceptance and retirement, checkpoints) have no write authority on it.

## Proposing

`Engine::propose_review(graph, branch, changes, host)` takes the same
`FactChange` list as a `write` command. The proposer needs write authority
on the branch: graph Write, a branch entry on a protected branch, or an
agent's delegated Write. The changes are validated as the proposer's write
at that moment, with every `write` rule of the current protocol (missing or
hidden records, conflicts, dependents, `E_READERS_WIDENED`,
`E_DERIVED_READERS`, reader groups the proposer may name), and then rolled
back: nothing reaches the graph. A proposal that changes nothing fails with
`E_FACT_WRITE`. A direct graph answers `E_REVIEW_NOT_REQUIRED`: write the
change instead.

The receipt names the proposal ID (`proposal:` and 32 random hex digits,
minted by the engine), its version and its digest. The proposer can:

- **amend** it (`Engine::amend_review`) while it is pending. An amendment is
  a new version with a new digest; approvals of earlier versions do not
  count for it. At most 64 versions.
- **withdraw** it (`Engine::withdraw_review`); an agent's owner chain can
  withdraw the agent's proposals too.

A proposer holds at most 1,000 undecided proposals per graph, and a version
has at most 1,000 changes and 4 MiB.

## Reviewing

`Engine::vote_review(&ReviewVote { proposal_id, digest, verdict, note }, host)`
approves or rejects one exact version: `digest` must be the digest of the
current version (`E_REVIEW_CHANGED` otherwise), so an approval never applies
to content the reviewer did not see.

A voter must be an **eligible reviewer**: an active user named by the
current policy, directly or through a group, with Read on the graph, who is
not the proposer. The owner of an agent may review the agent's proposals;
that is the intended use. The voter must also be within the proposal's
**read scope**: it must be able to read every record the proposal writes,
the live versions it corrects or retracts, and the records those facts
depend on for visibility (an edge's endpoints, an attachment's host, an
embedding's subject). A proposal outside the reviewer's read scope behaves
exactly like a missing one (`E_UNAVAILABLE` "proposal unavailable").

- A **rejection** decides the proposal at once. Its note (up to 2,048 bytes)
  is kept.
- An **approval** decides it once `threshold` distinct reviewers have
  approved the current version under the current policy revision, counting
  only reviewers who are still eligible and within scope when the last
  approval arrives. The proposal's changes are then written **as the
  proposer**, in one fact revision, in the same savepoint as the vote and
  the decision. If they no longer apply (another write conflicts, the
  proposer lost Write), the vote fails with `E_REVIEW_STALE` "the proposal
  no longer applies to the graph" and nothing is recorded; the reviewer can
  reject it instead.

A decided proposal takes no further votes or amendments (`E_REVIEW_DECIDED`).
A second vote with the same verdict is a no-op; a different verdict on the
same version fails with `E_CONFLICT`.

## Provenance of accepted facts

An accepted fact keeps the proposer's provenance and adds the review's:

```json
{"writer": "alice-bot", "on_behalf_of": ["alice"], "recorded_at_ms": 1757808060000,
 "method": {"kind": "model", "model": "extractor", "version": "2"},
 "review": {"proposal_id": "proposal:5f0c…", "decision_id": "decision:0a1b…",
            "reviewers": ["bob", "carol"], "proposed_at_ms": 1757808000000,
            "decided_at_ms": 1757808060000}}
```

`review` is engine-recorded, like `writer`, and appears in `fact_provenance`
of protocol 1.3.0 programs ([1.3 note](contract/v1.3/README.md)). The fact's
recorded time is the decision time: before it, `as_of` reads and change sets
do not contain the fact. Reviewer identities are disclosed to the fact's
readers, as the writer is.

## Who sees proposals

Proposals live outside every graph, in their own append-only tables. No
graph read (query, `expand`, `as_of`, `changes`, `nearest`,
`similar_candidates`, explanations, counts, rankings, views) ever reads
them, so pending, rejected and withdrawn proposals are invisible to every
reader of the published graph. `Engine::review_proposal(id, host)` and
`Engine::list_review_proposals(filter, host)` show a proposal to:

- its proposer, and the principals in an agent proposer's ownership chain;
- eligible reviewers within its read scope;
- for a policy proposal, the graph's Administer holders.

Each also needs Read on the graph. A listing returns proposals of one
graph oldest first, at most 100 per page (`after` continues from a proposal
the caller may see), optionally by status, and never reports a total.

Decided proposals stay in history with their decision: accepted (with the
published revision and the approving reviewers), rejected (with the
rejecting reviewer and note) or withdrawn. They are never deleted.

## Policy changes

`Engine::set_review_policy(graph, mode, expected_revision, host)` needs
Administer on the graph and the current policy revision (`E_CONFLICT`
otherwise).

- On a **direct** graph the change applies at once: making a graph stricter
  needs no one else's consent.
- On a **reviewed** graph the change becomes a **policy proposal**, decided
  by the reviewers of the policy it replaces, under the same rules as fact
  proposals (exact digest, threshold, never the proposer). The proposed
  policy's reviewers have no say in its acceptance, so a policy change
  cannot authorize itself. A policy proposal records the revision it
  replaces; if another change lands first, it fails with `E_REVIEW_STALE`.

Pending fact proposals are judged by the policy current at each vote. Votes
cast under an earlier policy revision do not count after a policy change.
When a graph returns to `direct`, its pending proposals can only be
withdrawn.

## From the command line

`weave-engine review --db PATH --actor PRINCIPAL REQUEST.json` runs one
`ReviewRequest`:

```json
{"action": "set_policy", "graph_id": "kb", "expected_revision": 0,
 "mode": {"kind": "reviewed", "reviewers": ["team"], "threshold": 1}}
{"action": "propose", "graph_id": "kb", "branch_id": "main", "changes": [...]}
{"action": "vote", "proposal_id": "proposal:…", "digest": "…", "verdict": "approve"}
{"action": "list", "graph_id": "kb", "status": "pending"}
```

The other actions are `policy`, `amend`, `withdraw` and `show`.

## How this reuses governance

The accepted-view governance ([GOVERNANCE.md](GOVERNANCE.md)) supplies the
rules; registry authority replaces its signed member keys.

| Governance | Review |
|---|---|
| Immutable approval subject: an ed25519 approval binds the proposal digest | A vote binds the exact version digest; amendments are new versions |
| M of N distinct policy members | `threshold` distinct eligible reviewers, never the proposer |
| A replacement policy is approved by its predecessor's quorum | A reviewed graph's policy change is a proposal for the current reviewers |
| Expected-head compare-and-swap; one atomic decision, receipt and event | Expected policy revision; vote, decision and published revision in one savepoint |
| Members are public keys listed in the policy | Reviewers are registry users and groups, resolved per request |
| Collectors and readers named in the policy | Proposers need graph write authority; visibility follows graph Read and record readers |

The signed governance views keep working as before, in stores without a
registry.

## Limits and non-guarantees

- Review needs the registry: in a store without one every review call fails
  with `E_REGISTRY_DISABLED`. `HostContext::writable_graphs` is never
  consulted.
- Reviewer groups are registry groups. Whoever manages a reviewer group's
  membership (its group administrators and the tenant administrators)
  decides who reviews. A policy whose reviewers are only individual users
  cannot change if those users cannot reach the threshold; name a group.
- The read scope is computed from the head when a version is submitted, and
  checked under current membership when it is shown or voted on.
  Entity-hosted attachments and embeddings are scoped by their own readers.
- `E_REVIEW_STALE` is one uniform error: a reviewer learns that a proposal
  no longer applies, not why. A proposer learns from its own proposal's
  validation what any write by it would learn.
- Listing scans at most 100,000 proposals of a graph per call
  (`E_BUDGET`), and reads count against the operation read budget, which
  counts proposals the caller cannot see (the declared budget channel of
  SECURITY.md).
- Timing and response size are not protected.
- There is no expiry for proposals and no retention for decided ones.
