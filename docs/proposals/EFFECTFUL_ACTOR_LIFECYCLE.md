# Effect-aware recorded actor lifecycle and observation replay

Mandatory engine4.1–4.4; R12/R13/R14/R36/R38, E05/E06/E14. This is an
implementation design with draft native acceptance; frozen/publication acceptance remains pending. Store26 already records actual
native state/tool artifacts/effect outcomes; this work completes their version
transfer and default historical effect observation.

Upgrade captures the actual current actor definition, opaque state, private
checkpoint, semantic state CAS, policy epoch and current whole primary/metadata
inputs. Source and destination must have compatible event/state protocols,
owner, subscriptions, output scopes, destinations and metadata depth. The native
host declares a compatible opaque state ABI; the kernel does not attest arbitrary
code semantics. Both source delivery queues and pending/unknown effects must be
drained; source is installed/paused. A fresh destination namespace, actual stored
artifact bytes, copied state/checkpoint, source removal and immutable audit commit
together. Old records and physical effects remain historical. Exact retries never
rewind a later state or checkpoint.

Rollback selects an actual archived prior definition/state/checkpoint pair from
the same recorded lineage, requires the current epoch and current whole authority,
and installs it into another fresh namespace. It records an internal observation
fence through the current source's actual checkpoint, not a public global offset
and not a silently assumed distributed cut. Unseen later events remain new work.
Prior output revisions remain immutable and the latest output branch is not
silently overwritten by observation.

For a leased subscribed event within this rollback fence, starting a new tool
computation and ordinary broker request/dispatch must fail closed. Explicit
observation reads the source namespace's actual immutable completion or prior
observation witness under current whole authority. It copies the actual recorded
compatible state/tool results and pins, preserving their original artifact/effect
provenance; it executes no model/tool, graph command or physical effect. New
observer state, private checkpoint, ordinary empty acknowledgment receipt and an
immutable typed observation audit commit together. Original effect intents remain
bound to their original namespace. The observation reports the actual source
receipt; it never fabricates new terminal intents for the observing namespace.

A source event skipped under earlier authority can lack a recorded outcome.
Observation must report unavailable rather than automatically recomputing or
pretending that no effect happened. Nested upgrades/rollback use a bounded,
cycle-checked actual observation lineage (maximum16 links) to reach the original
recorded completion. Source/state/result/metadata authority is rechecked even
for empty output. Exact historical observation retries return the original audit
without overwriting newer state/checkpoint/output.

After the observing checkpoint crosses its fence, a later source event can run a
new computation under the installed artifact and current lease/authority. Its
results are recorded as new work. Existing unknown external outcomes always
require destination-specific reconciliation; this design does not establish
exactly-once arbitrary remote I/O.

Acceptance must exercise actual nondeterministic native tools, physical sink
receipts and distinct real artifact versions; upgrade, later event execution,
rollback, historical observation, current input/output/state CAS and denied
authority; compatible lineage and incompatible definition rejection; pending/
unknown fences; missing observations; stale retries without rewind; collection
validating actual registrations/journals/lineage; genuine store26 row preservation;
and pre/postcommit process deaths for installation, transfer and observation.
Source/portable bindings, full output reconstruction, receipt expiry, broader
collection, causal/resource isolation and original assurance remain mandatory.
