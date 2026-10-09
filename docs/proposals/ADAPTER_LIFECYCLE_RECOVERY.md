# Explicit native adapter cancellation and state migration

Required source: engine sections3.4 and4.1–4.4; R12/R13/R36/R38 and E05/E06/E14.
Store22 retention/rebuild is the starting profile. This proposal describes the
implementation direction. The bounded store23 native implementation is documented
in [ADAPTER_LIFECYCLE.md](../ADAPTER_LIFECYCLE.md); this proposal does not close full gates.

A stale compiled output CAS must have an explicit owner cleanup path. Cancellation
will compare the actual pending event and opaque lease, retain the exact immutable
preparation/occurrence and audit record, advance the private delivery coordinate,
and delete the pending lease in one transaction. A changed lease, completed
receipt, foreign owner or effect-enabled/governed adapter fails closed. No payload
is returned, so owner cleanup can remain possible after source read authority
expires. Unknown effects are never canceled or reissued through this operation.
Historical cancellation retries are exact; no fresh completion can reuse a canceled
occurrence. Stateful native projections require an explicit actual-state rebuild
before further polling, while sealed stateless recipes can continue with later
occurrences. The kernel does not certify unbound opaque host state.

Pure native state migration will use a new adapter/version namespace, an immutable
pinned destination manifest and an explicit compatible state/checkpoint pair.
The source must be paused/drained without pending delivery or unresolved effects.
Migration binds the actual prior state digest, artifact/config identity, epoch,
input snapshots, private checkpoint and declared event-schema compatibility. The
new state and manifest/checkpoint commit together; the old namespace remains
retired with immutable output provenance. Rollback must restore a compatible pair
explicitly under another namespace, never silently reset or reissue an old effect.
Newly requested input scopes require current whole-input authority or a fresh
rebuild; an incompatible checkpoint cannot be reused.

A new store marker must fence old runtimes from ignoring cancellation or required
rebuild records. Root tracing must know all new tables and retain their immutable
references. Migration initializes no fictitious lifecycle history, preserves all
prior rows and rolls back before any partial marker change. Existing default
retention migration checks remain independent.

Acceptance will use actual stale-CAS compiled preparations, owner cleanup after
policy/source denial, lease renewal races, duplicate requests, changed bodies,
pre/postcommit process deaths, state rebuild fencing, explicit migration/rollback,
and genuine populated store22 upgrade/refusal. The existing full source/peer/effect
trace and historical migration profiles remain required regressions. Source,
portable, effectful actor and resource isolation profiles remain mandatory work.
