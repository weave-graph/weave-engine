# Recorded native actor state and effect recovery

Mandatory engine4.1–4.4 requirements; R12/R13/R14/R36/R38, E05/E06/E14.
Pure native/compiled reconstruction and compatible version transfer exist. A
nondeterministic or effectful actor still needs explicit actual output/artifact
recording, paired state/checkpoint completion, and safe effect-aware lifecycle.

A new trusted native actor registration pins its real manifest, compatible event
schema and recorded execution mode. It receives scoped deliveries and submits an
explicit bounded state transition, actual model/tool output artifacts, authorized
input pins and graph commands. The kernel binds the real event/lease, actual prior
state and manifest, current authority, actual resulting output references and all
actual effect-intent outcomes. State, artifacts, graph commands/events, immutable
receipt and private checkpoint commit together. It cannot complete while a
pending/unknown effect remains unresolved. An exact duplicate returns the recorded
outcome; it does not run the model/tool or issue an external action again.

The standard completion path must not bypass recorded actor state pairing.
Nonmatching private scans update only the paired private coordinate. Stored state
access and historical receipt reuse recheck current authority and all retained
input/artifact/output pins; public bindings exclude global event coordinates.

Effectful upgrade requires paused/drained delivery, no unresolved effects and an
explicit compatible artifact/state/checkpoint transfer into a fresh namespace.
Earlier model/tool output, graph provenance and external receipts stay historical.
Rollback restores an actual recorded prior pair into another namespace. Historical
replay observes actual prior effects and disables reissuing them through the broker
until its explicit replay boundary is caught up. A new model run is recorded as a
new computation. Unknown external outcomes always need explicit destination
reconciliation; this design does not promise exactly-once arbitrary external I/O.

Acceptance must cover genuine random/native tool output recording, an independent
idempotent local reference sink, lost intent/dispatch/completion acknowledgments,
unknown reconciliation, state/output/checkpoint atomicity, authority and lease/CAS,
actual upgrade/rollback and default observation replay, privacy, storage corruption,
old-store preservation and process deaths. Source/portable actor commands, untrusted
runner isolation, causal/resource controls, receipt expiry and all remaining
original requirements stay required. This proposal does not claim implementation
or passing acceptance.
