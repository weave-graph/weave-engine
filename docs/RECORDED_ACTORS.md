# Recorded native actors

Original engine4.1–4.4 require recorded nondeterministic actors and safe external
effects. Native store26 supplies a bounded trusted host profile. Protocol0.21,
capsule0.4 and source language contracts are unchanged.

`install_recorded_actor_for` reserves a fresh immutable actor identity with a
fixed owner, subscriptions, writable output scopes, permitted destinations,
event protocol and metadata depth. Its actual stored artifact bytes must match
the manifest SHA-256. Installation executes nothing. Native computation, opaque
state and submitted tool results are trusted host assertions; hashing them does
not attest execution or certify model output.

`recorded_actor_inputs_for` captures every current primary branch head separately
from its canonical whole authorized metadata closure. It includes the replay
epoch, actual registration digest and current semantic state CAS. It excludes
global event offsets. Live metadata, unavailable/hidden closure and partial input
fail explicitly. `bootstrap_recorded_actor_for` requires installed/paused,
drained state with no pending or unknown effects. It records explicitly supplied
initial/reconstructed opaque state and the actual current private checkpoint
together. Reusing stale initialization cannot replace newer state. This operation
does not reconstruct graph outputs or issue effects.

Before a new host tool computation, `recorded_actor_run_inputs_for` checks the
actual leased occurrence, current state/epoch and current whole input authority.
An independent durable host journal can recover a tool result already computed
before engine acknowledgment. Tools never execute inside a pure graph Program.
The reference native fixture stores actual seeded-random output in its own SQLite
journal before completing the corresponding engine occurrence.

`complete_recorded_actor_for` compares the actual prior semantic state and lease.
The kernel reads all actual broker intents for that occurrence: every one must be
confirmed/failed with a recorded response. Pending/unknown outcomes prevent
completion. The kernel hashes submitted tool values, executes the authorized
Program and collects actual query/output/provenance pins. Output, artifact/state,
immutable completion receipt, paired private checkpoint and delivery acknowledgment
commit in one transaction. Failed output CAS, exceeded budgets and host panic roll
back the entire graph/state completion; already recorded physical effect outcomes
remain durable and are not silently reissued. Raw handler completion cannot skip
this state pairing.

Exact retries compare the original computation excluding its renewable lease,
recheck current owner/output/source/result authority and read actual terminal
effect and handler records. They return their original artifact/results and
receipt identity, leaving newer state/output/checkpoint intact. Current state is
also checked for empty results. Private or unsubscribed scan positions update the
internal state/checkpoint hash without changing public state or binding identity.

The broker persists Unknown before host I/O. An attempted intent cannot be
automatically dispatched again. The reference sink uses its own durable
idempotency key and payload binding; lost acknowledgment is reconciled using its
actual stored receipt. Its absence evidence is specific to a stopped local worker
and this destination protocol. It is not an inference from a remote timeout and
does not establish arbitrary remote exactly-once effects.

The profile bounds registration artifacts to256 KiB, registration JSON to2 MiB,
128 actor registrations, bootstrap JSON to1 MiB, completion JSON to4 MiB,
32 tool artifacts/2 MiB per completion, state to4 MiB, 1000 input pins,
128 receipts/64 MiB per actor and8 MiB per receipt. New actor intents are capped
at32 per occurrence,4096 per actor and64 MiB accumulated payload/response bytes
before insertion; existing immutable key retries remain available. Existing
shared read, graph work and materialization budgets still apply. Serialization
limits do not establish CPU/RSS isolation for trusted native code.

Collection recognizes and validates typed registrations, paired states and
receipts against actual handler/effect/event records. Initialization records a
durable initialized flag, so deleting a previously initialized current state
cannot look like a fresh actor. Unsupported, missing modern or downgraded schema
fails closed. All earlier populated schemas/rows remain unchanged on store25
upgrade; default migration performs no erasure.

Complete effectful actor upgrade/recorded-pair rollback and default observation
of prior physical effects remain the next required lifecycle work. Current pure
handler cancellation refuses recorded actors. Full source/portable actor commands,
host applications, reconstruction with graph output, receipt expiry, broader
collection, causal feedback/resource isolation and original assurance remain
required. This profile does not close R01–R40 or E15.
