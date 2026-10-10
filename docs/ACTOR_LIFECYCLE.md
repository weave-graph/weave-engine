# Native recorded actor transfer and historical observation

Protocol0.21/capsule0.4 are unchanged. Store27 adds three typed native tables:
`recorded_actor_migrations`, `recorded_actor_replay_fences`, and
`recorded_actor_observations`. Modern missing tables and marker downgrades fail
closed. All store26 definitions omit the default state ABI when serialized,
preserving their original bytes and hashes.

Use `recorded_actor_migration_inputs_for` on an initialized installed/paused
actor with drained deliveries and resolved effect outcomes. Its semantic binding
contains the actual definition, state, policy epoch and current whole
primary/metadata inputs. It exposes no global checkpoint coordinate. Unrelated
private scans can advance the internal pair without changing that public binding.
`migrate_recorded_actor_for` checks this actual CAS in the same transaction.

An upgrade installs actual destination artifact bytes into a fresh paused
namespace and copies the actual opaque state/checkpoint. Source removal,
initialized state and immutable migration receipt commit together. Event/state
protocols, owner, subscription/output/destination scopes and metadata depth must
match. `state_protocol` declares a host-compatible opaque ABI; it does not attest
arbitrary executable semantics. Pending deliveries or pending/unknown effects
prevent transfer. Incompatible artifacts or expired archived state require an
explicit different reconstruction profile.

Rollback selects an actual archived prior definition/state/checkpoint pair from
the same recorded migration lineage. It installs that exact artifact into another
fresh paused namespace, retaining every prior output revision and receipt. Its
private fence includes the source's actual checkpoint and any inherited known
observation frontier. It never captures an unseen global future event.

After starting delivery, `recorded_actor_delivery_mode_for` returns `observe` for
an event within that fence and `compute` for new work beyond it. Within the fence,
new run inputs, new effect intents, dispatch and ordinary actor completion reject
new computation. `observe_recorded_actor_for` resolves the actual historical
completion through at most16 cycle-checked lineage links. A missing actual outcome
returns `E_REPLAY_UNAVAILABLE`; it does not imply that an action never happened.

The kernel copies the actual compatible recorded state/tool artifacts, preserves
the original artifact/receipt/effect provenance, and commits the observer's state,
private checkpoint, empty handler acknowledgment and immutable observation audit
atomically. It creates no new effect intent, tool/model call, graph command or
physical action. Latest output branch heads stay in place; observation is state
replay, not graph-output reconstruction. Original effect intents remain in their
original namespace. A later new occurrence executes the installed restored
artifact and records new work normally.

Exact completion, migration and observation retries remain historical reads,
including after retirement. Renewable lease IDs do not identify a different
computation. Actual registrations, original handler/effect journals, event
identity, whole primary/metadata/state/result authority and archived pairs are
validated; a retry never replaces a later state/checkpoint/output. Observation
also checks current primary heads, including empty inputs. Retention validates
the typed records and actual lineage before collection and preserves their roots.

Limits are128 actor registrations,16 lineage links,128 migration receipts and
64MiB per principal,128 observations and64MiB per observer,3MiB transfer requests,
4KiB observation requests and16MiB stored migration/observation records. Existing
actor tool/state/event/pin/read budgets still apply. These serialized limits do
not provide CPU/RSS isolation.

This is a trusted native host profile with destination-specific reconciliation.
The acceptance sink uses durable local SQLite idempotency; it does not establish
exactly-once arbitrary remote I/O or model truth. [Effect-aware cancellation](ACTOR_DISPOSITION.md) has a store28 native profile.
Source/portable bindings, full graph-output reconstruction, receipt expiry,
larger collection, causal/resource controls and every original assurance remain
mandatory. E01–E14 remain in progress; E15 remains open.
