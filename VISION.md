# Weave vision and goals

Status: approved by the owner on 2026-10-18. This file is the canonical
statement of what Weave is for. It applies to both
[weave-engine](https://github.com/weave-graph/weave-engine) and
[weave-language](https://github.com/weave-graph/weave-language). The phased plan
that implements it is [docs/ROADMAP.md](docs/ROADMAP.md).

## The problem

Organizations that build and operate hardware (fleets of machines, test benches,
vehicles, energy assets, labs and factories) hold their knowledge in disconnected
places. Telemetry sits in time-series stores. Hardware metadata such as parts,
serial numbers, firmware, configurations and maintenance lives in PLM, ERP and
spreadsheets. User and organizational metadata (owners, operators, customers,
teams and permissions) lives in other systems again. Three questions are hard to
answer:

- **What is related to what, and since when?** Which units run firmware 4.2,
  sit at customer X, and showed this vibration signature after the bearing swap?
- **What changed?** What did we know about this unit last Tuesday, what do we
  know now, and which correction changed the answer?
- **Who may see it?** Knowledge has to be shared across people, teams and now
  AI agents without leaking what a reader must not learn, including leaks
  through derived results.

AI agents make the problem sharper. Agents produce and consume facts constantly.
They need one place to write what they learned, query what others learned, and
retrieve relevant context. That context comes from explicit graph relations and
from semantic similarity, and both depend on the point in time being asked about.

## Who it is for

- **Engineering and operations teams in hardware-heavy industries** who need one
  queryable, time-travelling graph across telemetry, hardware metadata and user
  metadata.
- **Teams running AI agents** that must share a fact store across people and
  teams, with permissions and provenance on every fact.
- **Developers building retrieval (RAG) systems** who want vector search and
  graph traversal over the same versioned knowledge.

## Three goals, none sacrificed

### G1. Open-source temporal graph database for hardware-heavy industries

Relate telemetry, hardware metadata and user metadata in one graph. Every fact
has a **valid time** (when it was true in the world) and a **recorded time**
(when Weave learned it). Users can *time travel*: query the graph as it was
known at any recorded time, as it was valid at any world time, and ask
directly for **what changed** between two points.

Weave is not a time-series database. Raw high-rate signals stay in a series
store (native or external, decided in Phase C). The graph holds the
relationships, the metadata, references to series ranges and the facts derived
from them (events, anomalies, aggregates, states).

### G2. Shared fact store for AI agents, with permissions

People, teams and their agents **write and query knowledge directly**. Each fact
records who or what asserted it, from which evidence, when, and with what method
(human, rule, model and version). Visibility is controlled per fact and per
graph through principals, groups and agent identities.

**Derived facts must not leak hidden inputs.** A join, rule, aggregate, cluster,
analytic score or embedding computed from restricted data is restricted to the
readers allowed to see all of its inputs, unless an explicit release policy says
otherwise. Optional review lets a team accept agent-proposed facts before they
become shared truth.

### G3. Time-aware hybrid RAG

An embeddings layer is linked to the graph:

- **Vector search surfaces relations absent from the graph.** Semantic
  neighbors become explicitly marked *candidate* relations with provenance. They
  never silently become identity or fact.
- **Graph traversal yields vectors for embedding search.** A subgraph such as
  "all units in this fleet and their maintenance notes" becomes the seed or the
  filter for vector retrieval.
- **Both are time-aware.** Retrieval respects valid and recorded time and the
  same permissions as graph queries.

## One shared core

All three goals rest on one core, which is why it comes first:

| Core capability | G1 hardware DB | G2 agent fact store | G3 hybrid RAG |
|---|---|---|---|
| Bitemporal facts with immutable history | time travel, "what changed" | audit, retraction, correction | as-of retrieval |
| Per-fact provenance | trust in sensor-derived facts | who or which agent said it | cite sources |
| Incremental writes at fact granularity | continuous ingestion | agents add one fact at a time | embedding upserts |
| Traversal, joins and change queries | relate telemetry, hardware and users | agent queries | graph-to-vector seeding |
| Embeddings as first-class temporal facts | similar units and failures | semantic recall | vector-to-graph expansion |
| Restriction labels carried through derivation | customer data separation | permissions | no leakage through retrieval |

## Sequencing (agreed)

1. **Phase A: rock-solid shared core.** Temporal graph, provenance and the
   embeddings layer, usable locally through Rust, JSON and Python.
2. **Phase B: permissions and agent knowledge sharing.** Authenticated
   multi-user service, groups and agent identities, an agent interface,
   leakage-safe derivation and optional review.
3. **Phase C: scale for high-volume telemetry.** Ingestion, storage scale-out,
   retention, approximate vector indexes and incremental derivation.

Each phase ends with an integrated acceptance scenario. A phase is done when its
scenario passes, not when code exists.

## Principles

- **Facts are immutable; corrections are new facts.** History is never
  rewritten. Retention and erasure are explicit, audited operations.
- **Similarity is not identity.** Vector proximity yields candidates with
  provenance, never merged entities.
- **Unknown is not false.** Partial or unavailable inputs are reported as
  coverage, never as an empty complete answer.
- **Permissions apply to everything derived:** results, explanations, counts,
  clusters, embeddings and indexes, not just raw reads.
- **One engine, many interfaces.** Rust is the semantic authority. JSON, Python,
  the Weave language and the Phase B service are interfaces to the same
  semantics, never second implementations.
- **Measured, not claimed.** Performance and security statements cite a
  reproducible measurement or test.

## Explicit non-goals for Phases A to C

These came from the original white papers and are **deferred, not deleted**.
Existing code for them is kept and keeps its tests:

- Decentralized peer-to-peer sync, offline branches on phones, and capsule
  exchange as the primary sharing model. Phase B shares through an
  authenticated service.
- Browser and mobile applications (already deferred on 2026-10-10).
- Automatic recursive clustering and semantic zoom as a product feature.
- A general effect broker for external side effects.
- The full formal and cryptographic assurance program.

## Relationship between the two repositories

- **weave-engine** is the product: storage, temporal semantics, provenance,
  permissions, embeddings, retrieval and every service interface. It owns the
  canonical `weave-contract`.
- **weave-language** is an optional, AI-friendly language that compiles to the
  engine's JSON contract. It is not required to use Weave. Its roadmap follows
  the engine's phases and does not set them.
