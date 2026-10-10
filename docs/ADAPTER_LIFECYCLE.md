# Native cancellation, state migration and rollback

This native profile advances R12/R13/R36/R38 and E05/E06/E14. It does not close
those gates. Protocol0.21 and capsule0.4 stay unchanged; store23 fences older
runtimes from ignoring cancellation, rebuild requests or migration history.
Source/portable lifecycle bindings, compiled state upgrades, effectful actor
reconstruction, causal budgets, resource isolation and broader observability
remain mandatory work.

## Explicit cancellation

`cancel_handler_delivery_for` requires the fixed principal and every original
output scope. The request compares the actual pending occurrence and opaque lease,
including an expired lease. A renewed worker changes the lease and fences obsolete
cleanup. The cancellation reason and nonce become an immutable audit record.
Existing preparations are retained, the actual private checkpoint advances, and
the pending lease disappears in one transaction. No source payload, local offset
or historical command result is returned, so owner cleanup can remain available
after current source read authority expires. An exact retry returns the same
receipt; a changed body or reused nonce fails. Canceled occurrences cannot prepare
or complete through native, compiled or projection completion paths.

Completed work cannot be canceled. Effect-enabled and governed effect adapters
are excluded; pending or unknown effects must use their destination/reconciliation
protocol. Cancellation executes no commands and rewrites no graph head. A native
projection with bound state is marked for explicit rebuild before any new poll or
state read. Rebuild commits actual state, pins and matching checkpoint atomically.
Stateless compiled recipes can continue with later occurrences.

## Upgrade and rollback

`projection_migration_inputs_for` binds actual native projection state, immutable
manifest, retention epoch and authorized current scoped inputs. Public inputs
exclude the private event coordinate; unrelated private scan advances leave the
binding unchanged. The commit reads and transfers the actual private checkpoint.
Every old state pin and current input must still be wholly readable.

`migrate_projection_for` requires a paused or drained source with no pending
delivery, unknown effect, or required rebuild. Only pure native projections are
supported. Event schema0.21, principal, subscription scopes and output scopes must
match. A changed scope or incompatible schema requires a separately authorized
fresh rebuild. The destination identity must be new, and registration pins the
new artifact/configuration. Actual transformed host state, its retained pins,
private checkpoint, new paused namespace, old removed namespace and immutable
migration receipt commit together. The kernel binds actual opaque state and
authorized inputs; it does not prove the trusted host's transformation.

Rollback explicitly names a prior migration's retired source. The requested
artifact, configuration, state revision and state must exactly match the recorded
prior pair. It restores that pair and its actual checkpoint into another fresh
namespace. It never reactivates an old identity or automatically changes any
materialized graph output. Later polling can replay pure events in the new
namespace. An expired epoch cannot reuse the old checkpoint and requires a fresh
rebuild. Historical migration retries validate current owner/output authority,
actual manifests and state bindings and retained input authority; they never
restore an older state over later work.

## Storage and bounds

Store23 adds `delivery_cancellations`, `projection_rebuild_requests` and
`projection_migrations`. Initialization under an older marker rejects these
modern tables; a current marker requires all three. Upgrading an actual store22
preserves every original table, row, retention policy/epoch, erased payload anchor,
projection state, immutable receipt and private checkpoint. No fictitious
cancellation or migration is created. The retention inventory knows all three
tables, validates stored bindings, and retains immutable source/state references.

Cancellation requests are at most4KiB; each stored record is at most2MiB, with
10,000 records and64MiB per adapter. Migration requests are at most2MiB, state
at most1MiB including its binding, and stored records at most4MiB, with1,000
records and64MiB per principal. All reads also share the existing operation budget.
These bounds reject oversized work before commit. Receipt expiry and larger
incremental collection remain separate requirements.

Independent tests cover actual stale compiled CAS and source-policy expiry,
lease renewal, immutable retries, state rebuild fencing, real unknown-effect
exclusion, state/artifact/schema compatibility, rollback of the recorded pair,
private scan noninterference and valid-JSON storage corruption. The process
controller uses a real old22 host, a populated compacted store and four pre/post
commit death pairs; it compares actual SQL rows and schemas and proves old-reader
refusal. See [verification](VERIFICATION_023.md).

## Source-compiled pure stateless version transfer

`compiled_migration_inputs_for` captures the actual registered sealed artifact,
explicit primary input revision and current authorized metadata closure, opaque replay epoch and semantic
checkpoint binding. Private unrelated scan coordinates never appear in public
inputs. `migrate_compiled_handler_for` requires current owner/output authority,
a paused/drained source without pending or unresolved effects, and a current
input CAS. Event protocol, input/metadata scope, output slot/destination and
principal/subscriptions must remain compatible. A new sealed pure artifact and
configuration are installed into a fresh paused namespace; actual private
checkpoint transfer, old-version retirement and immutable receipt commit together.
Earlier outputs, preparations and receipts remain historical.

Rollback names an actual prior migration source and restores its recorded
artifact/configuration/checkpoint pair into another fresh namespace. It never
reactivates the old identity or changes graph heads automatically. Later pure
replay can produce new output. Historical retries validate actual stored
registrations, retained inputs and current authority; they never rewind later
checkpoint advances. Native opaque projection state and external host journals
are not stateless compiled state. Effects, scope/schema incompatibility and
expired replay fail closed until their explicit reconstruction profile exists.

Store24 adds `compiled_migrations`. Older markers containing this table and
current markers missing it reject before schema commit. Genuine populated
store23 upgrades preserve every prior schema/row, including native state,
cancellation/migration audits, erased anchors and original compiled preparations.
Collection checks typed receipt hashes and cross-bindings against actual
registrations and retains their real input pins. Requests are bounded to3MiB,
records to8MiB, and128 records/64MiB per principal, alongside existing registration
and shared operation limits. These serialization bounds do not provide process
CPU/RSS isolation. See [source/process verification](VERIFICATION_024.md).

## Explicit compiled snapshot reconstruction

`compiled_rebuild_inputs_for` binds the actual sealed registration, explicit
primary revision, whole authorized metadata closure, current replay epoch and
expected output head. `rebuild_compiled_handler_for` requires the current owner
and output authority, a paused/drained source and no pending or unresolved effects.
It executes the registered pure recipe on those actual inputs. It accepts no
caller result or opaque state. Current output, coverage/diagnostics, immutable
receipt, replay-ready state and actual private checkpoint commit together. This
explicit reconstruction supersedes earlier delivery history; it is not silent
checkpoint advancement. Unavailable or restricted input cannot become a complete
empty materialization.

Historical retries recheck current authority, actual registration, retained inputs
and prior output and never overwrite later work. Subsequent event completion and
private nonmatching scans advance paired replay/checkpoint state atomically.
Compatible version transfer retains this actual proof; expired epochs still need
fresh reconstruction. Canceling an actual state-bound compiled occurrence records
the existing preparation, advances its private coordinate and fences delivery
until another real reconstruction. Original native23/24 cancellation body hashes
remain exact when their new compiled-state flag is absent.

Store25 adds `compiled_rebuild_receipts` and `compiled_replay_states`. Older markers
containing either and current markers missing either reject before schema commit.
Collection checks typed body hashes, actual registrations, original receipt,
epoch/input/output bindings and actual paired checkpoints before erasure. Real
store24 migration preserves every previous schema/row and creates no fabricated
receipt/state. Requests are at most2MiB, records/states at most4MiB, and each
adapter keeps at most128 reconstruction receipts/64MiB. Existing operation,
recipe, object and materialization bounds also apply. Native opaque state,
effectful actors and external journals remain separate requirements. See
[source/process and privacy evidence](VERIFICATION_025.md).
