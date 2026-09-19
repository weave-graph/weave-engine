# Working in Weave Engine

- Read `docs/IMPLEMENTATION_PLAN.md`, `docs/WORKFLOW.md` and source provenance before changing scope.
- Do not claim that an early subset satisfies the full white-paper architecture.
- Keep stable entity identity, manifestations, immutable revisions, branches and replicas distinct.
- Metadata graph references belong to nodes and edges; enforce temporal, provenance, coverage and authorization semantics during traversal.
- Never hide effects inside pure plans or claim exactly-once external effects without a proven destination protocol.
- Receiving revisions does not imply governed acceptance. Missing offline knowledge does not imply falsity.
- Policy applies to discoverability, topology, explanations, indexes, clustering and layouts, not only payload reads.
- Record evidence against requirement and gate IDs. Changes to shared contracts require coordinated review and fixtures in both projects.
- Parent orchestration owns GitHub organization creation, repository publication and final release actions.
