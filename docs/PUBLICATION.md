# Public project and acceptance tracking

The experimental source repository is public under the MIT license: [weave-graph/weave-engine](https://github.com/weave-graph/weave-engine). It is not a claim of full white-paper conformance.

- [Implementation board](https://github.com/orgs/weave-graph/projects/2)
- [Dependency graph](WORKFLOW.md)
- [Implementation plan](IMPLEMENTATION_PLAN.md)
- [Current implementation evidence and limits](STATUS.md)
- [Hosted CI](https://github.com/weave-graph/weave-engine/actions)

Original v0.1 papers were supplied by the project owner on 2026-09-19. Preserved source artifacts and hashes are in `docs/source/`; the original papers describe an architecture proposal and distinguish requirements, recommendations, and open research. Private conversation exports are excluded from Git.

Each gate has a public issue. Partial implementation keeps its gate open. Closing a gate requires evidence covering its acceptance criteria, including exact commits, checks, supported hosts, and limitations.

| Gate | Public acceptance issue |
| --- | --- |
| E00 | [E00](https://github.com/weave-graph/weave-engine/issues/1) |
| E01 | [E01](https://github.com/weave-graph/weave-engine/issues/2) |
| E02 | [E02](https://github.com/weave-graph/weave-engine/issues/3) |
| E03 | [E03](https://github.com/weave-graph/weave-engine/issues/4) |
| E04 | [E04](https://github.com/weave-graph/weave-engine/issues/5) |
| E05 | [E05](https://github.com/weave-graph/weave-engine/issues/6) |
| E06 | [E06](https://github.com/weave-graph/weave-engine/issues/7) |
| E07 | [E07](https://github.com/weave-graph/weave-engine/issues/8) |
| E08 | [E08](https://github.com/weave-graph/weave-engine/issues/9) |
| E09 | [E09](https://github.com/weave-graph/weave-engine/issues/10) |
| E10 | [E10](https://github.com/weave-graph/weave-engine/issues/11) |
| E11 | [E11](https://github.com/weave-graph/weave-engine/issues/12) |
| E12 | [E12](https://github.com/weave-graph/weave-engine/issues/13) |
| E13 | [E13](https://github.com/weave-graph/weave-engine/issues/14) |
| E14 | [E14](https://github.com/weave-graph/weave-engine/issues/15) |
| E15 | [E15](https://github.com/weave-graph/weave-engine/issues/16) |

## Independent integration evidence

The orchestrator cloned both public repositories without credentials and ran three separate compiler-to-runtime acceptance suites on language `bbaa5f3` / engine `938cde7` (protocol 0.3). All passed: persistence and revision pins across processes, temporal joins and permission filtering, reusable graph composition and atomic rollback. The engine repository contains these executable suites as `scripts/root_integration.py`, `scripts/root_join.py`, and `scripts/root_values.py`; its Language integration workflow runs against an explicitly pinned compiler commit.

This evidence covers those behaviors only. It does not establish federation, full capability security, clustering, portable mobile execution, or complete language semantics.
