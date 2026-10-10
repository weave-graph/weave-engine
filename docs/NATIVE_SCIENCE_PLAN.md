# Native data-science completion

The user changed the active delivery scope on 10 October 2026: use an agent team,
finish a functional engine for data-science experiments, and defer browser and
mobile applications. This document is the completion contract for that delivery.
It supersedes the application prerequisites in the original roadmap for this
release; it does not claim completion of the full decentralized white paper.

The published protocol0.21/store29 Rust engine remains the semantic and storage
authority. The experiment interface uses its real commits, reads, graph values,
authorization, temporal selections and provenance. The language remains an
optional way to compile the existing JSON plans. No new compiler protocol,
actor framework, application, database or network service is needed for this
delivery.

## Acceptance

| ID | Required observable behavior | Evidence |
|---|---|---|
| DS01 | Import node/edge tables and vectors into durable immutable graph revisions; stale writes fail | Native and Python checks |
| DS02 | Select valid time and exact historical revisions; reopen the database and reproduce a pinned result after later corrections | Independent scientific acceptance |
| DS03 | Execute existing parameterized JSON plans, temporal joins, graph metadata and graph-valued queries | Existing core suite and experiment interface checks |
| DS04 | Analyze only engine-authorized results; denied topology never enters statistics, paths, clustering or vector candidates | Policy noninterference checks |
| DS05 | Deterministic degrees, weak/strong components, shortest paths and PageRank with declared multigraph/self-loop semantics | Independent reference answers |
| DS06 | Exact cosine/Euclidean vector neighbors retain stable manifestation/entity/space identity and deterministic ties | Independent numerical answers and invalid-input checks |
| DS07 | Python experiments work without mandatory scientific dependencies; export tables/results and record parameters, versions and exact snapshots | Installed client tests and runnable example |
| DS08 | Invalid inputs, missing snapshots, partial knowledge and exhausted resource limits are explicit; mutations cannot hide in analysis plans | Negative and bounded-work checks |
| DS09 | Publish build/install instructions, measured deterministic workloads, and a supported native validation profile | Native CI, benchmark report and clean installation |

Clustering, geometry, reusable graph values, recorded/accepted history and
provenance remain available through the existing engine plan operators. Exact
numeric analytics are an experiment API over selected query results, rather than
new evidence assertions or identity merges. A partial result must retain its
coverage and diagnostics; analysis must require explicit consent to partial input.

## Team ownership

- Native science agent: new Rust experiment crate, API, algorithms and focused checks.
- Python agent: installable client, table/CSV helpers, experiment records and examples.
- Acceptance agent: independent semantic oracles and reproducible benchmarks.
- Integration owner: shared build configuration, scope, documentation, CI, review and publication.

Agents edit separate file areas in one integration checkout. Heavy Rust builds
are coordinated by the integration owner and reuse one target cache. Development
runs focused checks; the complete current native suite runs at integration.
Historical migration and compiler matrices run when their implementation files
change or when explicitly requested for release review. Browser persistence stays
available through manual CI. Branch pushes do not duplicate pull-request CI.

## Deferred work

Browser/mobile applications and device acceptance, portable source actors,
distributed networking and key lifecycle, remote resource isolation, approximate
vector indexes and the full formal/cryptographic white-paper assurance program
remain future work. Existing application and paused source-actor code is preserved.
The native experiment release does not need these projects to pass DS01–DS09.

Measured workloads establish a baseline on the reported host, not an unmeasured
production capacity promise. Exact search and selected-graph analytics are bounded
in-memory operations. Large datasets can be filtered by the core plan before
analysis; distributed or approximate analysis requires a later explicit design.
