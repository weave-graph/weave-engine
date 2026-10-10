# Native host lifecycle operations

Engine4.1–4.4/11, R10/R12/R13/R14/R36/R38 and E05/E06/E14 require usable
embedding operations in addition to kernel methods. The trusted native profile
uses `weave-host-request/2` through `HostSession::call` and `weave_host_call`.
Protocol0.21, store29, capsule0.4 and the exact shared contract are unchanged.
Request1 retains its original four operations; new kinds under request1 reject
before sampling the operation clock or touching storage. Response format1 and
the legacy C ABI remain unchanged.

```json
{"format":"weave-host-request/2","operation":{"kind":"capabilities"}}
```

Capabilities report31 kinds, accepted request formats, current protocol/store,
the byte/command limits, trusted native execution and the embedding durability
fence. They describe implementation support, not a grant. Every owner operation
delegates to the existing Engine `*_for` method with the session's immutable
principal and output grants. Reads and old receipts recheck current whole input,
output and actual registration authority; cached transport handles cannot widen
that authority. Missing/foreign registrations use `E_HOST_AUTH`. Explicit bounded
budget/integrity failures retain their kernel error codes.

| Kind | Fields after `kind` | Kernel operation |
|---|---|---|
| `execute`, `poll`, `prepare`, `complete` | Original request1 shapes | Original Program and sealed compiled delivery |
| `capabilities` | none | Static implementation support and limits |
| `lifecycle` | `adapter`, `state` | Actual durable owner lifecycle |
| `lag` | `adapter` | Scoped bounded lag/pending/Unknown/circuit status |
| `causal_policy` | `adapter`, `policy` | Paused/drained actual owner depth policy |
| `projection_inputs`, `projection_state`, `projection_migration_inputs` | `adapter` | Existing whole projection bindings/state |
| `compiled_migration_inputs`, `compiled_rebuild_inputs` | `adapter` | Actual sealed recipe/state/output inputs |
| `actor_inputs`, `actor_state`, `actor_migration_inputs` | `adapter` | Actual registered actor bindings/state |
| `cancel_handler` | `request: DeliveryCancellationRequest` | Event/lease CAS and state-bound reconstruction fence |
| `projection_rebase`, `projection_complete`, `projection_migrate` | `request`: corresponding existing Engine request | Pure projection state/checkpoint/output transactions |
| `compiled_migrate`, `compiled_rebuild` | `request: CompiledMigrationRequest` or `CompiledRebuildRequest` | Compatible sealed transfer or kernel-computed reconstruction |
| `actor_bootstrap`, `actor_complete`, `actor_migrate`, `actor_observe`, `actor_cancel` | `request`: corresponding existing Engine request | Actual actor state/artifact/output/checkpoint/receipt transactions |
| `actor_run_inputs`, `actor_delivery_mode` | `adapter`, `event`, `lease` | Fresh actual computation inputs or default Compute/Observe decision |
| `actor_receipt`, `actor_observation` | `adapter`, `event` | Actual historical immutable receipt under current authority |

Request structs are the public types in `weave_engine`; nested fields are strict,
with opaque state/tool payloads checked for recursively duplicated decoded keys.
The outer16MiB limit, 1,000,000 JSON values, depth120 and16 Program commands apply
before Engine invocation. Actor/projection completion Programs share the command
bound. Each delegated operation also retains its existing finer kernel limits.
Responses remain bounded to32MiB+4,096 bytes. These limits do not promise a total
CPU, transient-heap or process-RSS ceiling.

Initial installation remains a separate trusted operation. Rust offers
`install_recorded_actor(&RecordedActorDefinition)`; the C header adds
`weave_host_install_actor(token,len,definition,len)` with a2MiB strict definition
limit. Swift `installActor(Data)` passes original bytes. Existing handler
installation remains explicit and retains the complete SDK artifact response.
Compatible owner migration cannot widen the actual installed subscription/output
scope. No operational request installs a clock, principal, grants, registry or
signing key, and no request can fabricate broker reconciliation evidence.

Recorded completion consumes submitted native computation, actual tool artifacts
and terminal broker ledger outcomes. `E_ACTOR_EFFECT_PENDING` blocks completion
while an intent lacks a terminal recorded response; `E_EFFECT_UNKNOWN` blocks
actor disposition while physical acknowledgment is uncertain. The trusted
destination broker must reconcile actual external evidence before completion or
cleanup. Default historical Observe mode rejects fresh computation/effects with
`E_ACTOR_REPLAY`; observation reuses actual old outcomes and emits no new graph
output or physical action. Cancellation does not run pending actions.

Every invoked outcome, including an ordinary rejection, requires the embedding
durability fence. A response lost after a native SQLite transaction is recovered
by reopening and using actual immutable receipts/current state. Old exact retries
do not rewind later work. Initial actor bootstrap has current-state CAS; uncertain
bootstrap is inspected rather than blindly retried. Serialization/storage/unwind
uncertainty poisons the session and withholds its operational payload. See the
[facade byte/fence rules](HOST_FACADE.md).

The native C controller and two actual-source recipes have separate
[verification](VERIFICATION_HOST_LIFECYCLE.md). The Swift wrapper typecheck does
not extend the earlier simulator application's acceptance to these operations.
Actual browser image generation fences and complete mobile/source actor execution,
general output reconstruction, full remote effect/causal/resource isolation and
all original formal/cryptographic/quality/release requirements remain mandatory.
No full R01–R40 or E15 completion is claimed by this interface.
