# Native retention and explicit projection rebuild

Store22 adds a trusted storage-administration profile without changing protocol
0.21 or capsule 0.4. Language commands and remote capabilities cannot install a
retention policy or run collection. Existing store21 templates, receipts, signed
governance history and observations migrate without rewriting their original rows.
Default horizons retain all existing payloads and preserve ordinary delivery.

An administrator previews `RetentionPolicy` with a local observation-time horizon
and a private local replay coordinate. Frontiers advance monotonically within
actual observed history. `compact_retention` recomputes the complete bounded root
closure and compares the preview inside the same transaction as erasure. A new
pin, head, receipt or dependency invalidates the preview. Unsupported schemas,
opaque registry blocks, missing dependencies, malformed encoded cells, immutable
payload/policy/anchor corruption and budget exhaustion
stop collection before payload mutation.

Roots include active heads, the value at each active branch's history boundary,
later observations/events, mounts, immutable owned pins, views and their changes,
policies/proposals/approvals, pending deliveries, handler preparations, completed
receipts, projection states and unknown effects. References recurse through all
graph/proof carriers and valid nested receipt encodings. Shared references and
cycles are legal. One reachable member of an atomic logical snapshot retains all
its members. This conservative profile may retain more than minimally necessary.

An owner can bind up to 1,000 genuine whole-authorized snapshots to an immutable
named pin. Releasing a pin does not release foreign pins or dependent objects.
Explicit branch retirement checks current write and whole-input authority plus
the expected head. Its causal identity remains retired; reuse requires a new
branch identity. A retained exact snapshot can remain readable after retirement.
Files or external journals must register required snapshot pins before a host
promises their retention; the kernel cannot discover arbitrary external copies.

Collected payload columns become empty alongside their immutable content/payload
digests, graph/branch/parent identity, erasure time and policy generation. Revisions,
events, observations and snapshot manifests retain their ordering anchors. Empty
payloads without valid matching anchors are corruption. Collection reports active
payload-column bytes. It does not run `VACUUM`, shrink a SQLite file, erase backups
or provide forensic erasure or recall of copied plaintext.

Date/range requests below the observation horizon fail with `E_HISTORY_EXPIRED`
or `E_GOV_HISTORY_EXPIRED` after current whole-input authority is checked. Genuine
exact checkpoints/decisions can still resolve if other roots retain their payloads.
`recorded_availability_for` reports one authorized branch's local observation
horizon and trusted observation time. It exposes no global sequence or private
counts and does not certify remote history or query completeness.

Projection replay uses an opaque random policy epoch. Advancing a frontier expires
existing consumers conservatively; collecting additional private garbage under
the same policy does not change their epoch. New polling never silently jumps an
expired checkpoint. Existing genuine leased work can finish using its retained
state and inputs; subsequent polling requires the new epoch's explicit rebuild.
Historical duplicate receipts recheck current authority and never restore an old
projection state.

`rebase_projection_for` accepts an explicitly rebuilt native worker state, exact
current authorized subscription snapshots and the pinned manifest. It rejects
pending delivery and unresolved effects. The kernel commits state, input pins,
manifest, epoch and private checkpoint together. A completion atomically commits
the next state, all preceding/new/result input pins, graph output, durable outbox,
receipt and checkpoint. Raw handler completion cannot bypass that binding.
Skipped unrelated events update only the private checkpoint binding; they do not
change the public state digest. Missing or detached state fails closed.

The worker is trusted native code supplying its actual opaque computation state;
this profile does not certify an arbitrary JSON state as a correct computation.
Rebuild supports pure native projection manifests, excluding compiled handlers
and effect-enabled/governed adapters. These other reconstruction and lifecycle
profiles remain required future work. External effects are never implicitly
reissued, and their unknown outcomes and immutable nonce/receipt anchors remain.

Scheduled living views have a separate owned replay epoch. Explicit
`rebase_view_schedule_for` clears membership caches, checks a full computation
oracle for every owned scheduled view, and commits all complete results, processed
manifests and the matching private cursor together. Failure rolls back the entire
owner rebuild. Normal scanning, draining and duplicate enabling cannot acknowledge
an expired window. No foreign views or global offsets are returned.

Current administrative limits are 3,000 revisions, 100,000 registry rows/events,
1,000,000 nested values, 32 MiB per registry cell and the shared 128 MiB/4,096-read
operation budget. State requests are at most 1 MiB with 1,000 input pins; immutable
completion receipts are limited to 10,000/64 MiB per adapter. Owner view rebuilds
are limited to 256 views within the same shared budget. Larger incremental
collection, receipt expiry, broader actor reconstruction, source/portable control
bindings and complete platform assurance remain mandatory. This native profile
advances R04/R08/R12/R36 and does not close the original project gates.
