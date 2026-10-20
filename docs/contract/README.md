# Engine contract reference

The current science client emits **contract 1.2.0** Programs. 1.0.0 froze core v1 with the 0.24.0 feature set ([note](v1.0/README.md)), 1.1.0 adds group readers ([note](v1.1/README.md)), and 1.2.0 adds write-time propagation ([note](v1.2/README.md)); every older version stays accepted with its published semantics. The canonical
serializable schema is [`weave-contract`](../../crates/weave-contract/src/lib.rs)
and its modules. Native requests and Python expressions use these types; unknown
or incompatible input fails at the runtime boundary.

**Start with [core contract v1](core-v1.md).** It is the consolidated,
normative specification of the core subset, and it holds the golden fixtures
(`crates/weave-contract/fixtures/core-v1/`), the stable error codes, the
compatibility policy and a table that classifies every feature of 0.1–0.21 as
core v1, extension or deferred.

Versioned documents describe the base contract and successive additions. They
are not standalone copies of the whole current schema. Use the current types
for exact fields and these documents for semantics and compatibility. The science
request envelope is separate: see [SCIENCE_INTERFACE.md](../SCIENCE_INTERFACE.md).

## Base and extensions

| Version | Topic |
|---|---|
| [0.1](v0.1/README.md), [0.2](v0.2/README.md), [0.3](v0.3/README.md) | Base identities, graph data, commands and early graph algebra |
| [0.4](v0.4/README.md) | Schemas and named metadata snapshots |
| [0.5](v0.5/README.md) | Bounded graph algebra and alternative derivations |
| [0.6](v0.6/README.md) | Source assertions and computation identity |
| [0.7](v0.7/README.md) | Finite graph rules |
| [0.8](v0.8/README.md) | Exact context selection |
| [0.9](v0.9/README.md) | Geometry from graph assertions |
| [0.10](v0.10/README.md) | Declared counterpart bridges |
| [0.11](v0.11/README.md) | Exact node influence references |
| [0.12](v0.12/README.md) | Exact decimals and nominal quantities |
| [0.13](v0.13/README.md) | Pinned native graph services |
| [0.14](v0.14/README.md) | Typed context definitions and persisted influence |
| [0.15](v0.15/README.md) | Persistent graph influence and node proof alternatives |
| [0.16](v0.16/README.md) | Exact governed reads and host view artifacts |
| [0.17](v0.17/README.md) | Exact snapshot influence |
| [0.18](v0.18/README.md) | Sealed pure handler artifacts |
| [0.19](v0.19/README.md) | Alternative carriers and temporal graph values |
| [0.20](v0.20/README.md) | Replica-local recorded selection |
| [0.21](v0.21/README.md) | Accepted history and history ranges |
| [0.22](v0.22/README.md) | Fact writes, node valid time and per-fact provenance |
| [0.23](v0.23/README.md) | Traversal (`expand`), as-of selection and change sets (`changes`) |
| [0.24](v0.24/README.md) | Embedding facts and spaces, exact `nearest`, `similar_candidates` and `seeds_from` |
| [1.0](v1.0/README.md) | Core v1 frozen at the Phase A release; one version table for feature gates |
| [1.1](v1.1/README.md) | `reader_groups` on fact records and `FactProvenance.on_behalf_of` (B1) |
| [1.2](v1.2/README.md) | `widen_readers`, widening and declared-derivation rules, hidden live versions on the write path (B2) |

Each version states its finite support and remaining work. Contract version,
store schema version, capsule format and host request version identify different
boundaries and do not advance together automatically. The current native profile
is listed in [STATUS.md](../STATUS.md).

The engine owns the canonical contract. The companion
[language repository](https://github.com/weave-graph/weave-language) vendors an
exact copy for independent compiler builds. Changes require coordinated fixtures
and compatibility review; see [CONTRIBUTING.md](../../CONTRIBUTING.md).
