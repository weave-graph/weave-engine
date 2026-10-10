# Recorded actor lifecycle verification

Mandatory engine4.1–4.4; R12/R13/R14/R36/R38, E05/E06/E14. Native store27 preserves
protocol0.21/capsule0.4 and every existing language/vendor contract.

`root_actor_lifecycle` exercises compatible state/artifact transfer, actual prior
pair rollback, default observation fences, terminal effect provenance, nested
rollback before intermediate observation, exact retired completion retries,
current input/state CAS, paused/drained requirements, incompatible opaque ABI,
stale nonce/host authority, unavailable history, schema loss/downgrade, actual
journal/fence corruption, panic-safe rollback, private checkpoint noninterference
and current hidden primary denial. The earlier ten actor checks remain required.

`scripts/check_actor_lifecycle.py` executes separate processes using two actual
Rust tool artifact versions. Both sample nondeterministically and commit their
actual results to an independent native journal. An independent idempotent sink
records real physical actions. Its genuine store26 baseline has already completed
an actor with actual tool/effect/output/state/receipt data. Every original schema
and row, tool journal and physical receipt survives pre/postcommit store27 deaths;
the old binary refuses the newer marker.

The controller kills a host after its tool journal commit, around both upgrade
and rollback commits, after a physical destination commit with lost acknowledgment,
around actor completion, and around observation. Independent SQL snapshots prove
precommit rollback and postcommit persistence. Unknown physical outcomes block
transfer until the actual destination receipt is reconciled. Rollback refuses new
tool work/intents; observation leaves the tool journal, physical sink and output
head unchanged. Nested rollback reaches the actual ancestor without an intermediate
observation. Old retries cannot rewind later work. A future occurrence executes
the installed restored version. Collection validates the actual typed lineage.

Raw requests/responses and all source/sidecar SQLite databases are retained with
`--evidence-dir`. Ubuntu/macOS/Windows run the same controller. Genuine store26
also joins every earlier populated inventory and retained SDK journal upgrade.
Frozen clean-source full workspace checks, strict lint/format, all native hosts,
earlier source/recovery controllers and exact publication hosted checks are
required before merging. Local draft results do not establish hosted acceptance.

Trust remains native host computation and compatible host-declared opaque state.
Artifact hashes are not execution attestation; local sink reconciliation is not
arbitrary remote exactly-once. Full source/portable and application integration,
effect-aware cancellation, graph-output reconstruction, receipt expiry, isolation
and all original formal/cryptographic/quality/platform requirements remain open.
