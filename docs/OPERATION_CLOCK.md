# Trusted operation clock

The native runtime now obtains current authority time from a clock installed by the trusted embedding host at engine construction. `Engine::open` and `Engine::memory` use checked system UTC milliseconds. `open_with_clock` and `memory_with_clock` accept an `Arc<dyn TrustedClock>`; `ManualClock` supports explicit fixture/embedding-host control. No graph program, signature body, capsule, or serialized request can install that clock.

## Snapshot and nesting

Each outer authority operation establishes its SQLite transaction snapshot before sampling the clock. Deferred reads perform an actual bounded table read before sampling. Mutation boundaries first reserve the SQLite writer with a zero-row update, without changing graph records, then establish the snapshot and sample. A busy writer attempt fails before sampling; its top-level retry obtains a new snapshot/time. Programs reserve the writer because they can contain commits; standalone queries use a read transaction.

Nested query, join, source proof, metadata, direct resolver, capsule, native service, cached view, signed admission and dispatcher checks reuse the outer captured time and cumulative read budget. The owned scope cannot escape a public operation. Clock callbacks are called once per outer scope, not once per premise. Local revision/receipt timestamps use that same captured time. Assertion valid time, source context, view ticks and signed claimed timestamps remain separate data; they never select current authority time.

Authorization is evaluated at this snapshot/time boundary. Expiry or revocation occurring after that boundary takes effect at the next operation; already returned data cannot be recalled. A view tick does not advance or rewind the authority clock. Clock-free trusted administrative diagnostics such as raw head/count inspection are not reader-authorized payload APIs. The legacy audit-only adapter mock records no graph payload or external effect and remains a trusted administrative fixture; the scoped dispatcher uses the operation clock.

Negative or out-of-range system timestamps, clock errors, callback panics and backward steps relative to this engine instance's last successful sample fail with generic `E_CLOCK_UNAVAILABLE`. A failed clock cannot write a receipt, checkpoint, graph, or decision. Its private error details are not returned. A later valid sample can recover; a failed sample does not lower the remembered last time. Clock scope state is dropped on normal/error/unwind paths; a clock callback panic is caught before leaving the storage operation.

The default system clock trusts the host to maintain correct UTC. This profile does not provide a durable anti-rollback clock across engine restarts, OS-clock manipulation, database restore, or independent replicas. No remote time attestation or process-time/RSS isolation is claimed.

## Breaking native API migration

Authority-time arguments were removed from these method families; there are no wrappers that silently ignore an old `now` argument:

- `admit_query`, `admit_publish`, `admit_proposal`, and `integrate_proposal` (including recovery observers).
- Governance proposal, approval, acceptance, inspection, subscription, polling, acknowledgment and dead-letter replay.
- Adapter polling and handler failure/backoff. New handler completion and effect-intent requests now enforce the current lease's half-open expiry interval as well as its token. Effect dispatch also rechecks current event authority before writing its unknown-outcome fence.

Callers must deliberately migrate to constructor-installed clocks. Production callers normally use the default system clock. Tests must install and advance a host clock rather than pass authority time to a method:

```rust
use std::sync::Arc;
use weave_engine::{Engine, ManualClock};
let clock = Arc::new(ManualClock::new(200));
let engine = Engine::memory_with_clock(clock.clone()).unwrap();
// Configure the trusted host and signed request, then call admission without `now`.
clock.set(300);
```

Portable `weave-policy::AdmissionContext` still has a time for standalone verification by its own trusted host. Native policy installation extracts the policy configuration; native verification constructs its current context from the captured engine clock. A context value used to install roots cannot backdate a later native request. Native scheduling/fact APIs retain their explicitly data-oriented tick/valid-time parameters.

Existing regression and recovery fixtures install a test-host clock at construction. Their test-only controls are not production request fields. The dispatcher recovery probe explicitly requires `fixture_clock_ms` for every invocation; the Python driver preserves the fixture clock across process restarts. Other fixed-key probes install fixture clocks for their defined scenarios. These executables do not form remote authority endpoints.

No graph DTO, protocol version, schema marker, or immutable graph encoding changes in this stage. Previously stored policy and receipt records are preserved. This does not create reusable governance decision assertions or expose accepted graphs; that remains the next reviewed stage.

## Verification

- [Operation clock tests](../crates/weave-engine/tests/operation_clock.rs): one sample across nested queries, matching recorded timestamps, fail-closed negative/backward/error/panic clocks, no partial writes, and a real second-connection head change inside the clock callback proving the read snapshot was already established.
- [Signed admission](../crates/weave-engine/tests/admission.rs): a cached response expires under the installed clock independently of its fact-time filter; dependency traversal and the nested query sample once.
- [Dispatcher](../crates/weave-engine/tests/dispatch.rs): expired unrenewed leases cannot complete handlers or request effects; renewed processing and nested execution share one sample.
- [Governance delivery](../crates/weave-engine/tests/governance_delivery.rs): current policy expiry without head movement blocks inspection, pending delivery and already-acknowledged receipt replay. Historical admission evidence does not bypass current time.
- Existing process-death scripts exercise pre-commit rollback, post-commit lost responses, nonce conflicts, lease renewal and durable effect fences with migrated host clocks. The checkpoint handoff records their actual outcomes; no distributed exactly-once claim is made.

Candidate proposal submission also establishes a writer snapshot and samples the trusted clock before source validation or persistence. Feature-only recovery observers propagate their original panic after rolling back manual mutation boundaries; they do not leave reusable Engine instances inside uncommitted transactions. This applies to handler completion, identity acceptance, proposal integration, governance acceptance and governance acknowledgment. The process-exit hooks still exercise SQLite recovery independently.
