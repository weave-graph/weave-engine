# Protocol 0.20.0: replica-local recorded query selection

Implementation stage. This coordinated compiler/runtime boundary introduces
`RecordedQuery { query, selection }` and the query result's optional
`recorded_observations` manifest. Native store20 prevents older store19 runtimes
from interpreting cached values without these semantics; capsule0.4 is unchanged.

`LocalTime { unix_millis }` is a read criterion against the actual local replica,
never a client-installed authority clock. The runtime samples its trusted clock
once inside the captured SQLite operation snapshot and rejects a future criterion.
`Checkpoint { observer, checkpoint }` selects an exact retained graph/branch
observation from that replica. The query's explicit revision selector must be
absent. Valid time, branch observation time and genuine governance acceptance
remain distinct axes.

The runtime resolves the selected immutable revision, requires current whole
snapshot authority, and returns its actual observation witness. Empty/private,
missing, foreign and intervening lost observations fail closed. Pure composition
preserves all distinct witnesses, while cached reuse verifies them against the
actual local registry and exact input revisions under current authority. Witness
labels never install policy or grant remote authority. A time criterion can resolve
new equal-millisecond appends on a later execution; retain its returned checkpoint
for exact replay.

New selectors require protocol0.20 before any program writes. Absent result fields
are omitted so prior cached responses retain their bytes. Store19-to20 migration
preserves original recorded history; it never adds another baseline. Earlier stores
receive the store19 current-time baseline semantics. Serialized/read work limits
remain explicit; these selectors do not implement GC, expiration, indexed incremental
history, globally atomic cuts or hardware rollback protection.

The language declares `recorded_handle` with explicit graph/branch and `known_at`
or `checkpoint` plus `observer`. Declaration performs no read. Existing `pin`
executes one recorded read at its ordered position; its `at` selects valid time.
Pure functions consume pinned values and cannot declare or perform store reads.
Compiled view artifacts preserve the selector; hidden reads remain forbidden in
pure handler recipes. Native range APIs remain available separately. Full source
range/accepted-view history and broader paper gates remain mandatory follow-up.
