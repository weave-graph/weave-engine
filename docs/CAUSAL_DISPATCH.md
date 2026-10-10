# Local causal dispatch and owner-scoped lag

Native store29 preserves protocol0.21, capsule0.4 and the language/vendor
contract. This implements a bounded local graph-handler profile for mandatory
engine3.2–3.4/4.4/11, R10/R12/R13/R38 and E05/E06/E14.

Every actual local graph event has a kernel-written causal header. Fresh native
commits and local capsule integration start a local root. Genuine older events
receive an explicit `legacy_boundary` root during atomic migration: the kernel
does not invent earlier ancestry. Handler completion binds only events newly
created by its actual command results to the actual source event, immutable
adapter registration, root and depth. Outputs, receipts, headers and private
checkpoint commit together. A no-op or a historical logical batch emits no new
event and cannot relabel an existing occurrence. Exact cached completions remain
read-only. Trusted graph writes outside handler completion remain roots.

The kernel validates actual event ordering, parent/root/depth, registration,
subscription, output scope, principal and corresponding handler receipt. A
consumer defaults to depth16; fixed owners may choose1–64. Poll checks the next
currently authorized subscribed occurrence before leasing it. At the limit it
atomically records a circuit and pauses that consumer without acknowledging the
blocked source. Two adapters feeding each other share actual ancestry and reach
this limit. Fresh handler completion, compiled preparation, actor inputs and
ordinary effect-intent/dispatch entry points recheck causal work. Existing
Unknown outcomes still require actual destination reconciliation.

`set_causal_dispatch_policy_for` requires the fixed owner and current output
authority, installed/paused lifecycle, no pending delivery and no Pending/Unknown
effect intent. It clears the current suspension, preserves ancestry/checkpoints
and leaves the consumer paused. The owner explicitly resumes it. Raising a limit
permits only another bounded segment; this API does not skip a source, dispose an
effect or erase historical computation. The current circuit is an operational
record, not a complete immutable failure-history taxonomy.

`adapter_lag_status_for` rechecks the same owner/output authority and current
whole event visibility. It reports a visible backlog lower bound, truncation,
visible pending attempts/retry/dead-letter state, visible Unknown effect count,
checkpoint expiry and visible current suspension. SQL first filters declared
graph/branch subscriptions. Hidden occurrences never contribute to returned
counts; unsubscribed events do not consume the scan. No global sequence, causal
root/parent, event payload or lease identity is exposed. An empty/hidden pending
source returns no pending status. These native owner diagnostics do not cover
separate governed effect adapters or remote subscriptions.

Bounds are256 returned visible backlog occurrences,4096 subscribed candidates,
128 Unknown intents,16KiB causal cells,64 ancestry edges,256 new events per
completion and4096 installed adapters. Migration accepts at most100000 genuine
old events and4096 adapters; overflow rolls back. Existing128MiB operation reads
and4096 read requests apply. A hidden subscribed occurrence can cause a documented
resource/availability failure when these bounds are exceeded. This profile does
not claim full availability noninterference or CPU/RSS isolation.

Collection validates causal headers, policies and current circuits against actual
journals. Headers retain metadata without making every historical plaintext
payload a root; actual handler receipts and unresolved circuits retain their
dependencies. Missing modern cells, corrupt bindings and downgrade schemas fail
closed. Hashes are integrity witnesses, not execution attestation against a native
administrator who rewrites all journals consistently.

See [verification](VERIFICATION_029.md). Full event/schema taxonomy, cross-peer
causal identity and ordering, governance/mount/remote integration, source/portable
commands, resource isolation and every original formal, cryptographic, quality,
platform and public-release requirement remain mandatory.
