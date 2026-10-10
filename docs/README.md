# Weave Engine documentation

The current supported delivery is a local native engine for data-science
experiments. Start with the guides below. The wider decentralized architecture,
application work and research proposals have separate completion boundaries.

## Start and interpret an experiment

| Document | Use it for |
|---|---|
| [Repository quickstart](../README.md#run-your-first-experiment) | Build, install and run the temporal/vector example |
| [Experiment concepts](SCIENCE_CONCEPTS.md) | Identity, revisions, valid versus recorded time, authorization, coverage and replay |
| [Python tutorial and reference](PYTHON_SCIENCE.md) | Scripts/notebooks, import, corrections, joins, analysis, CSV, export and saved experiments |
| [Native JSON and Rust interface](SCIENCE_INTERFACE.md) | Python-free requests, responses, algorithms, defaults, errors and budgets |
| [Scientific validation](SCIENCE_VALIDATION.md) | Run independent checks and reproduce measurements |
| [Current status](STATUS.md) | Supported versions, verified evidence and remaining work |

The [example source](../examples/science/temporal_vectors.py) is an executable
end-to-end experiment. It supplies toy vectors; bring your own model outputs for
an embedding experiment. The [raw measurement index](benchmarks/README.md)
identifies current science reports and older baselines.

## Runtime and contract references

The science interface calls the existing engine. Its complete graph-expression
schema is defined by the [canonical Rust contract](../crates/weave-contract/src/lib.rs)
and related modules. [Contract history](contract/README.md) explains how to read
the versioned documents, which describe additions rather than a single combined
schema.

| Topic | References |
|---|---|
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
