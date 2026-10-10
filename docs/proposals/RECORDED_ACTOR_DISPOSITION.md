# Explicit effect-aware recorded actor disposition

Mandatory R12/R13/R14/R36/R38, E05/E06/E14. This design follows the recorded actor
and lifecycle profiles; it is not passing acceptance.

A fixed native owner can dispose an actual pending occurrence by exact event and
lease CAS, including an expired lease or a newly unreadable source. Cleanup returns
no source/state/tool payload. The actual definition and initialized state/private
checkpoint must remain internally valid; foreign output authority cannot clean up.
Ordinary pure cancellation continues to reject recorded actors.

Unknown external outcomes prevent disposition. They must first be resolved through
the destination's actual reconciliation protocol. A pending broker intent has never
passed the durable Unknown-before-I/O transition, so an immediate transaction may
mark it Failed with an immutable owner-disposition response before it can dispatch.
Terminal confirmed/failed outcomes remain unchanged and are bound into the audit.
No physical action or compensation is executed by cancellation.

The source actor becomes paused, the pending lease is removed, its actual opaque
state and private checkpoint remain paired, and an explicit reconstruction-required
flag plus typed immutable cancellation audit commit together. The checkpoint moves
only through the explicitly disposed occurrence, never a global future frontier.
Old model work, completion and effect dispatch are fenced. Explicit paused/drained
current-snapshot initialization is required before future actor delivery. This is
an owner-chosen new initialization, not reconstruction of missing tool history or
graph outputs; every old output and effect receipt remains historical.

Exact retries validate the actual definition, event, terminal ledger and stored
request and return the original audit without changing newer state/checkpoints.
Native host journals remain independent historical work records. The kernel does
not claim that unsubmitted computation was executed or stored. Retention validates
the typed cancellation and actual terminal effect bindings before collection.

Acceptance requires foreign/stale leases, policy-expired cleanup, pending intent
race fencing, unknown before I/O and actual lost physical acknowledgment, terminal
receipt preservation, explicit state initialization, stale retries after later
computation, modern schema loss/downgrade and corrupt actual records, real store27
row preservation, and pre/postcommit process deaths around cancellation. Trusted
native host semantics do not provide CPU/RSS isolation or arbitrary remote
exactly-once. Source/portable commands, full actor graph-output reconstruction,
receipt expiry, causal/resource controls and every original assurance remain open.
