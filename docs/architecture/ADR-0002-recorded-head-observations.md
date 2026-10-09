# ADR 0002: replica-local recorded head observations

Status: implementing. Requirements R04/R08/R09/R18/R20/R36; E02/E03/E09/E14.

Record branch-head selection separately from immutable revision creation/receipt.
Accepting an old or received revision changes what a branch knows at that local
observation; its original revision timestamp cannot stand in for acceptance time.
Author clocks and imported observation claims are never local recording authority.

Store19 appends a checksummed, replica-bound head observation in the same transaction
as each head advance. Ordinary commits, atomic logical batches, handler/view writes,
forks and explicit acceptance use one shared boundary. No-op writes and replayed
receipts create no observation. Schema migration records current old heads as a
baseline at migration time; it does not invent historical acceptance times. The
old revisions/events remain unchanged. Pre-baseline time selection is unavailable,
while exact old revision reads remain supported. Marker18 binaries refuse store19.

An opaque checkpoint names one graph/branch observation in one runtime source.
Checkpoint selection is immutable. Time selection returns the last locally
recorded head at or before the requested millisecond, with append order breaking
ties, and exposes its checkpoint for exact replay. A range describes the state
at its start plus changes in a half-open recording interval. Every returned
snapshot is integrity checked and wholly authorized under current policy in one
operation snapshot. Denied/missing entries fail closed rather than being skipped.
No global offset, parent checkpoint, actor or private event inventory is exposed.
Date/range selection follows the authenticated predecessor path from the current
head instead of skipping rows with a SQL date filter. A missing/corrupt intervening
record is unavailable history. This walk consumes the existing read budget;
long histories can require a narrower request or fail with a work-budget error.
Range entry overflow uses the same unavailable denial as a denied snapshot, and
never returns a partial range or the count of hidden entries. Indexed incremental
history maintenance and retention remain subsequent work.

Recording time must not move backwards within a branch, including after reopen;
a regressed trusted clock rejects the head mutation atomically. This does not add
hardware clock/rollback resistance. Checkpoints, not wall-clock equality across
replicas, supply exact reproducibility. Date cuts are resolved against the captured
local log; selected checkpoint/revision pins must be retained for later replay.
The log does not advertise a globally atomic distributed cut or treat quarantine
receipt as accepted branch knowledge. Governance acceptance times remain their
separate genuine occurrence axis.

Bound observations to 100,000 per store and range results to 1,000 entries, with
existing cumulative read/materialization budgets. No implicit eviction is allowed.
Retention/replay expiration must subsequently preserve declared horizons and
anchors, with explicit expired-checkpoint/rebase behavior. This first storage step
does not implement GC or falsely close the retention gate.

Native typed checkpoint/query/range APIs come first, followed by coordinated
language/contract query selection and source acceptance. Pure results still pin
the actual selected immutable graph revisions; local observation metadata never
authenticates remote policy, compiler origin or data truth. Existing trusted host
APIs and authorization rules remain in force.
