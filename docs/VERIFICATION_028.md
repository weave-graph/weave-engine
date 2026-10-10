# Recorded actor disposition verification

Mandatory engine4.1–4.4; R12/R13/R14/R36/R38, E05/E06/E14. Native store28 preserves
protocol0.21/capsule0.4 and all shared language/vendor contracts.

Seven independent `root_actor_disposition` checks cover undispatched intent
retirement, Unknown refusal and terminal reconciliation, expired lease/policy
cleanup, foreign/stale authority, state/checkpoint/rebuild pairing, later
initialization/computation, old retries including during later Unknown work,
actual ledger corruption, modern schema loss, panic rollback and compact audits
for multiple large legitimate payloads. Eleven lifecycle and ten actor checks
remain required.

`scripts/check_actor_disposition.py` executes the exact original store27
controller/binary with the actual store26 baseline. That88-process/12-death trace
generates genuine completed state, output, transfer, rollback, observation and
tool/effect receipts; no modern store is relabeled. Interrupted store28 migration
preserves every original table/row, and the old binary refuses the new marker.
Original independent store26 tool/sink sidecars remain unchanged.

The new controller runs44 processes and7 controlled deaths. One actual physical
action loses acknowledgment and blocks cancellation until sink reconciliation.
Pre/postcommit deaths prove atomic state/private checkpoint/paused/rebuild/audit
changes and preservation of the terminal receipt. A second Pending intent becomes
Failed/not-dispatched without a physical action; dispatch stays fenced after
explicit initialization. New native computation completes later. Both old
cancellation retries preserve newer state/output/checkpoints, and the original
observation survives typed collection. Lifecycle sidecars end with six tool runs
and four physical receipts, including their three original tool runs/receipts;
the separate original store26 sidecars are also preserved.

Raw requests/responses, driver output and SQLite stores/sidecars are retained
with `--evidence-dir`. Ubuntu/macOS/Windows run the same actual trace. Acceptance
also requires store20–27 populated inventories and actual retained SDK journals,
the current source/peer/effect trace and every prior native recovery controller.
Frozen clean-source workspace checks, strict lint/format, all native hosts and
exact publication hosted checks are required before merging. Draft results do
not establish hosted acceptance.

Trust remains native host computation and opaque submitted state. Audit digests
are not execution attestation; local sink evidence is not arbitrary remote
exactly-once. Source/portable and application integration, full actor output
reconstruction, receipt expiry, isolation and every original
formal/cryptographic/quality/platform requirement remain mandatory.
