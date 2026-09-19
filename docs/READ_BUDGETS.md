# Cumulative native read work

Nested graph reads in one program, query, join, signed admission, capsule operation,
structural/assertion resolver or live-view read/refresh share limits of 4,096 load attempts and 128 MiB of stored
graph/manifest text. Missing references count as attempts. Repeated references
count again: this is a work limit, not a cache or a unique-object allowance.

Stored graph and manifest extraction uses the smaller of the remaining operation
budget and the 16 MiB record bound. Bytes are charged before parsing and content
digest verification. Exhaustion returns `E_BUDGET`; a containing program or signed
publication rolls back prior writes and receipts. A scope guard releases counters
on success or failure, so the next independent call on the same engine receives
a fresh budget. Nested calls cannot reset their parent's allowance. The engine
remains movable between host actor threads.

This closes repeated-provenance load amplification that previously received a
fresh per-edge recursion budget. The existing output/visible metadata budgets,
recursion limits, admission ancestry limits and pure evaluator limits still apply.
Schema initialization can scan the host's whole legacy database and is not treated
as a bounded client query; each loaded record still has its integrity/size checks.

These counters measure stored text work, not process RSS, CPU instructions,
all SQLite I/O, hashing of incoming capsule records, or timing noninterference.
Snapshots are stored as whole JSON records, so physical read costs can depend on
restricted content even though hidden objects do not enter the visible traversal
queue. A denial at a resource boundary is not an activity-hiding guarantee. A
visibility-partitioned store, session cache, and deployment-wide quotas remain
future work; no output authority is widened to avoid a limit.

`cargo test -p weave-engine --test root_read_budget` verifies repeated large
premises hit the cumulative byte limit with prior-command rollback, thousands of
small repeated premises and source-assertion resolution hit the load-count limit, bounded requests succeed, the
same engine remains usable after an error, and moving it to a host thread works.
