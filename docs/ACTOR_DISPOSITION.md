# Effect-aware recorded actor cancellation

Native store28 extends the trusted actor/lifecycle profiles without changing
protocol0.21, capsule0.4 or the language/vendor contract. Mandatory
R12/R13/R14/R36/R38 and E05/E06/E14 remain broader than this profile.

`cancel_recorded_actor_delivery_for` takes an actual adapter/event/lease, owner
nonce and reason. A fixed native owner with current output authority may dispose
that pending occurrence, including an expired lease or unreadable source. Cleanup
returns only an audit identity and reconstruction-required flag. It returns no
source/state/tool payload. Actual registration, initialized state/private
checkpoint, subscribed event and pending lease must remain valid. Foreign
authority, a changed lease, completed occurrence or reused nonce fails before
changing the store. Pure cancellation continues to reject recorded actors.

An Unknown effect anywhere in the actor prevents fresh cancellation: reconcile
it against actual destination evidence first. A Pending intent has never crossed
the broker's durable Unknown-before-I/O boundary. One immediate transaction marks
those intents Failed with a stable `not_dispatched` owner-disposition response,
preserving their original payload and identity. Confirmed and Failed outcomes
remain historical. Cancellation performs no physical action or compensation and
makes no claim about unsubmitted native computation.

The same transaction advances only through the disposed occurrence, preserves
actual opaque state paired with its private checkpoint, pauses the actor, removes
its pending lease and writes a reconstruction-required flag and typed immutable
audit. Panic or precommit death rolls everything back. New computation,
completion and effect dispatch for the canceled occurrence are fenced. Future
delivery requires explicit paused/drained initialization against current whole
inputs. This is an owner-chosen new state, not reconstruction of missing tool
history or graph outputs. Existing outputs remain historical.

An exact retry validates the stored request and actual definition/event/current
state pair and terminal effect ledger, then returns the original audit with
`duplicate=true`. It cannot replace later state/output/checkpoints. A later
Unknown effect does not prevent reading an already recorded cancellation.
Collection validates typed audits and actual ledger bindings and retains their
roots. Audits store compact before/after effect digests and state classes; checks
read the actual terminal ledger, reconstructing a formerly Pending intent only
by removing its exact cancellation response. Large payloads are not duplicated
in every audit. These bindings are not execution attestation or protection
against an administrator who consistently rewrites all native journals.

Limits are4KiB requests,8MiB records,32 effects per occurrence,2MiB combined
payload/response per effect,65MiB aggregate effect data,128 audits and64MiB audit
data per adapter. Existing128MiB operation read and actor state/artifact budgets
apply. Serialized limits do not establish CPU/RSS isolation. Receipt expiry,
larger incremental collection, source/portable commands, full output
reconstruction, causal/resource controls and all original assurance remain open.
Local destination-specific reconciliation is not arbitrary remote exactly-once.
