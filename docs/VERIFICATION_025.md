# Compiled snapshot reconstruction verification

Mandatory engine3.4/4.1/4.4; R12/R13/R36, E05/E06/E14. Native store25 retains
event protocol0.21 and capsule0.4. This bounded native profile does not close the
original full project gates.

`root_compiled_rebuild` checks actual recipe output and actual SQL checkpoints,
rather than accepting a caller-supplied result. Expired replay stays fenced until
an explicit paused/drained reconstruction commits its output, immutable receipt
and current replay state together. Later event completion and unrelated private
scans update the paired private checkpoint. Historical retries return their prior
receipt/output without overwriting later output or rewinding a checkpoint.
Compatible version transfer preserves actual readiness and a new-version event
then completes. Current input/output CAS, owner, pending work, nonce conflicts,
actual stale prepared-output cancellation, panic rollback, state corruption and
schema downgrade are covered independently. Two copies of the same real store
with different private events have equal public inputs, receipt identity and
materialized graph, but different actual private checkpoints.

`scripts/check_compiled_rebuild.py` runs66 compiler/native processes and8 deaths.
Two actual0.21 `.weave` versions are compiled. The genuine old24 host creates
completed output/preparation/receipt history, newer input, and actual compiled
upgrade and recorded rollback. Atomic store25 initialization preserves every
prior schema and row and adds two empty tables; an old reader then refuses it.
Real compaction erases an orphan payload, and compiled polling reports expiration.

The kernel reconstructs current input2. A private nonmatching event advances only
the private coordinate. A real later preparation loses output CAS, owner
cancellation records it and fences delivery, and a second reconstruction computes
input3. Its historical first retry never overwrites input3. Compatible source
version transfer keeps replay ready and completes input4. Original registrations,
migrations, preparations and receipts remain byte-for-byte historical. Both
reconstructions, state-bound cancellation and schema initialization exercise
pre/postcommit process-death pairs. Raw responses, source/artifacts, databases and
independent SQL snapshots can be retained with `--evidence-dir`.

Strict all-target/all-feature lint/format and the full workspace remain required.
Hosted Rust jobs run source reconstruction crashes on Ubuntu, macOS and Windows.
All earlier source/peer/effect, retained journal, native lifecycle and retention
profiles remain regressions; genuine store24 snapshot/handler/effect/accepted
history and retained SDK journal upgrade join the historical matrix. Source
contracts and original source hashes remain unchanged. Portable/browser/mobile
semantics are separate profiles and do not attest lifecycle applications.

Opaque state, effectful actors and external retained host journals are excluded
from stateless reconstruction. Complete source/portable lifecycle commands,
causal/resource controls, receipt expiry/larger collection and every remaining
original application/formal/cryptographic/performance requirement stay mandatory.
