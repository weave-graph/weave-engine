# Weave Engine documentation

## Direction

| Document | Use it for |
|---|---|
| [Vision and goals](../VISION.md) | What Weave is for, who it serves, the three goals and the non-goals |
| [Phased roadmap](ROADMAP.md) | Phases A, B and C, the epics, dependencies and definition of done |
| [Proposed issue changes](ROADMAP_PROPOSAL.md) | Approved GitHub epic texts with acceptance criteria |
| [State audit, 2026-10-17](AUDIT_2026-10-17.md) | What the code does today versus the goals, with file references |
| [AGENTS.md](../AGENTS.md) | Architecture overview, build and test commands, conventions, definition of done |

The currently supported release is a local native engine for data-science
experiments. Start with the guides below. The original decentralized white-paper
architecture and its proposals are kept for reference; the roadmap decides what
is scheduled.

## Start and interpret an experiment

| Document | Use it for |
|---|---|
| [Repository quickstart](../README.md#run-your-first-experiment) | Build, install and run the temporal/vector example |
| [Experiment concepts](SCIENCE_CONCEPTS.md) | Identity, revisions, valid versus recorded time, authorization, coverage and replay |
| [Python tutorial and reference](PYTHON_SCIENCE.md) | Scripts/notebooks, import, corrections, joins, analysis, CSV, export and saved experiments |
| [Native JSON and Rust interface](SCIENCE_INTERFACE.md) | Python-free requests, responses, algorithms, defaults, errors and budgets |
| [Scientific validation](SCIENCE_VALIDATION.md) | Run independent checks and reproduce measurements |
| [Hardware reference model](HARDWARE_MODEL.md) | Fleet schema, `SeriesRef`, CSV/JSON importers and the seeded fleet generator |
| [Read-only MCP preview](MCP_PREVIEW.md) | Connect an MCP client (agent) to a local store: tools, trust boundary, errors |
| [Authenticated service](SERVICE.md) | Run `weave-service`: tenants, API tokens, the HTTP API, audit log, limits |
| [Agent interface](AGENT_INTERFACE.md) | Connect an MCP client to `weave-service`: the nine agent tools, identity and provenance, the data/code boundary, quickstart |
| [Principals, groups and graph ACLs](REGISTRY.md) | The registry a tenant's requests are authorized by |
| [Current status](STATUS.md) | Supported versions, verified evidence and remaining work |

The [example source](../examples/science/temporal_vectors.py) is an executable
end-to-end experiment. It supplies toy vectors; bring your own model outputs for
an embedding experiment. The [raw measurement index](benchmarks/README.md)
identifies current science reports and older baselines.

## Runtime and contract references

The science interface calls the existing engine. Its complete graph-expression
schema is defined by the [canonical Rust contract](../crates/weave-contract/src/lib.rs)
and related modules. The consolidated specification is
[core contract v1](contract/core-v1.md), with its golden fixtures, stable error
codes, compatibility policy and the classification of every version 0.1–0.21.
[Contract history](contract/README.md) explains how to read the versioned
documents, which describe additions rather than a single combined schema.

| Topic | References |
|---|---|
| Core contract and decisions | [Core contract v1](contract/core-v1.md), [1.0.0 release note](contract/v1.0/README.md) and [release notes](../CHANGELOG.md), [ADR-0004 fact log and storage backend](architecture/ADR-0004-fact-log-and-storage-backend.md), [ADR-0005 per-fact provenance](architecture/ADR-0005-per-fact-provenance.md), [ADR-0006 embedding facts and spaces](architecture/ADR-0006-embedding-facts-and-spaces.md), [ADR-0007 identity, groups and tenancy](architecture/ADR-0007-identity-groups-and-tenancy.md) (proposed, B1/B3) |
| Traversal, time travel and change sets | [Expand, as-of and changes](TRAVERSAL.md), [0.23 traversal and change sets](contract/v0.23/README.md) |
| Embeddings and hybrid retrieval | [Embedding facts, nearest and candidate relations](EMBEDDINGS.md), [0.24 embeddings](contract/v0.24/README.md), [ADR-0006](architecture/ADR-0006-embedding-facts-and-spaces.md) |
| Immutable revisions and recorded history | [Revision identity ADR](architecture/ADR-0001-revision-identity-and-content.md), [recorded observations ADR](architecture/ADR-0002-recorded-head-observations.md), [0.20 recorded selection](contract/v0.20/README.md), [0.21 recorded/accepted history](contract/v0.21/README.md) |
| Temporal graph values and evidence | [Temporal values](TEMPORAL_VALUES.md), [influence](INFLUENCE.md), [whole-snapshot authorization](WHOLE_SNAPSHOT_AUTHORIZATION.md) |
| Spaces, identity and geometry | [Spaces](SPACES.md), [exact context/metadata](contract/v0.8/README.md), [geometry](contract/v0.9/README.md), [counterpart bridges](contract/v0.10/README.md), [exact quantities](EXACT_QUANTITIES.md) |
| Clustering and views | [Clustering](CLUSTERING.md), [native cluster service](CLUSTER_SERVICE.md), [views](VIEWS.md), [incremental selection](INCREMENTAL_SELECTION.md), [view scheduling](VIEW_SCHEDULING.md) |
| Persistence and budgets | [Storage recovery](STORAGE_RECOVERY.md), [retention](RETENTION.md), [cumulative read budgets](READ_BUDGETS.md) |
| Capsules and governance | [Capsules](CAPSULES.md), [mounts/integration](MOUNTS_AND_INTEGRATION.md), [admission](ADMISSION.md), [capabilities](CAPABILITIES.md), [governance](GOVERNANCE.md), [governance graphs](GOVERNANCE_GRAPHS.md) |
| Dispatch and actors | [Dispatch](DISPATCH.md), [adapter lifecycle](ADAPTER_LIFECYCLE.md), [recorded actors](RECORDED_ACTORS.md), [actor lifecycle](ACTOR_LIFECYCLE.md), [cancellation](ACTOR_DISPOSITION.md), [causal dispatch](CAUSAL_DISPATCH.md) |
| Embedding the runtime | [Native C host](NATIVE_HOST.md), [host facade](HOST_FACADE.md), [host lifecycle](HOST_LIFECYCLE.md), [Swift host](SWIFT_HOST.md) |

These documents describe finite implementation profiles. Read each document's
version and remaining-work notes, and pair behavior claims with its verification
record. Current SDK replay has narrower guarantees than general engine execution;
see the [Python replay section](PYTHON_SCIENCE.md#results-exports-and-replay).

## Evidence, development and roadmap

[CONTRIBUTING.md](../CONTRIBUTING.md) describes focused development and integration
checks. [SECURITY.md](../SECURITY.md) describes the trusted-host boundary and
reporting process. [Scientific validation](SCIENCE_VALIDATION.md) is the current
data-science evidence entry point; older `VERIFICATION_*.md` documents retain
their named contract/store/host checkpoint evidence. The
[historical status ledger](STATUS_HISTORY.md) links those earlier milestones.

The [native completion contract](NATIVE_SCIENCE_PLAN.md) covers DS01–DS09. The
[implementation plan](IMPLEMENTATION_PLAN.md), [workflow](WORKFLOW.md) and
[source reconciliation](RECONCILIATION.md) track the broader original requirements.
The optional [language compiler](https://github.com/weave-graph/weave-language)
is maintained in a separate repository.

Files in `proposals/`, documents titled `*_DESIGN.md` or `*_PROPOSAL.md`, and
original paper syntax need to be read in their stated design context. A proposal
may have a later implementation document; its presence alone does not establish
support. [Source provenance](SOURCES.md) and the [unchanged original papers](source/README.md)
explain the architecture baseline.

Browser/mobile application delivery is deferred. The
[experimental browser host](BROWSER_HOST.md) and its
[historical verification](VERIFICATION_BROWSER_HOST.md) remain available for
reference; they are not prerequisites for native experiments.

## Historical records and implemented designs

These documents are accurate as dated records. They are not current
references. Paths are kept stable because GitHub issue comments link to them.

| Kind | Documents |
|---|---|
| Original plan | [Implementation plan](IMPLEMENTATION_PLAN.md), [workflow](WORKFLOW.md), [reconciliation](RECONCILIATION.md), [publication](PUBLICATION.md) |
| Verification checkpoints | [018](VERIFICATION_018.md), [019](VERIFICATION_019.md), [020](VERIFICATION_020.md), [021](VERIFICATION_021.md), [022](VERIFICATION_022.md), [023](VERIFICATION_023.md), [024](VERIFICATION_024.md), [025](VERIFICATION_025.md), [026](VERIFICATION_026.md), [027](VERIFICATION_027.md), [028](VERIFICATION_028.md), [029](VERIFICATION_029.md), [store 17](VERIFICATION_STORE17.md), [accepted history](VERIFICATION_ACCEPTED_HISTORY.md), [host lifecycle](VERIFICATION_HOST_LIFECYCLE.md) |
| Traces and measurements | [Three-peer trace](THREE_PEER_TRACE.md), [native compiled scenario](NATIVE_COMPILED_SCENARIO.md), [portable parity](PORTABLE_PARITY.md), [Phase A exit scenario check](PHASE_A_EXIT.md), [performance baseline](PERFORMANCE_BASELINE.md), [browser persistence prototype](BROWSER_PERSISTENCE_PROTOTYPE.md), [fuzzing](FUZZING.md) |
| Reviews | [Metadata selector review](METADATA_SELECTOR_REVIEW.md), [operation clock review](OPERATION_CLOCK_REVIEW.md) |
| Implemented designs | [Identity acceptance](IDENTITY_ACCEPTANCE_DESIGN.md), [typed context runtime](TYPED_CONTEXT_RUNTIME_DESIGN.md), [graph influence](GRAPH_INFLUENCE_PROPOSAL.md), [accepted view governance](ACCEPTED_VIEW_GOVERNANCE_DESIGN.md), [accepted governance graph exposure](ACCEPTED_GOVERNANCE_GRAPH_EXPOSURE.md), and `proposals/` |
| Other references | [Signed capsule export](SIGNED_CAPSULE_EXPORT.md), [governance delivery](GOVERNANCE_DELIVERY.md), [operation clock](OPERATION_CLOCK.md), [synthetic relations](SYNTHETIC_RELATIONS.md), [ADR-0003 accepted view history](architecture/ADR-0003-accepted-view-history.md) |
