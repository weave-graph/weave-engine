# Native retention/rebuild verification (protocol0.21 / store22)

This native extension advances R04/E02, R08/E03, R12/E05 and R36/E14. It keeps
protocol0.21 and capsule0.4 unchanged. Full original acceptance remains open.

Independent native oracles cover shared/cyclic metadata, all atomic snapshot
members, genuine owned pins and current whole-input authority, retained-window
start states, explicit branch retirement, preview invalidation, payload/anchor
integrity and transaction rollback across reopen. Literal identifiers and native
audit payloads remain legal; malformed encoded pin records and mismatched native
projection state/receipt bindings stop erasure. Lowering
the storage marker cannot silently reconstruct/default existing retention state.

Projection-state oracles cover atomic state/output/receipt/checkpoint completion,
actual restart, raw-completion bypass denial, detached/missing state, current
authority, expired epochs, unresolved work and panic rollback. A genuine in-flight
lease can finish once after a frontier change; subsequent polling requires an
explicit rebuild. Historical duplicates never rewind a newer state. View rebuild
oracles cover all owned views, full cold evaluation, rollback/reopen and replay
epoch stability during collection under an unchanged policy.

The development workspace passes532 test/doctest checks with all features and
strict all-target/all-feature lint. The final focused closure/state/view checkpoint
has19 independent oracles. Reproducible commands include:

```sh
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo build --locked -p weave-engine --features recovery-testing --example retention_probe
python3 scripts/check_retention_recovery.py --host target/debug/examples/retention_probe
```

The actual fixed native worker controller uses16 fresh processes and6 controlled
deaths before/after completion, payload erasure and explicit rebuild commits. It
compares every SQLite table around precommit failure and duplicate retry. It
checks one erased orphan, retained original input payloads, durable output and
state, the raw bypass guard and a historical duplicate after a newer rebuild.
This is actual trusted host state; the kernel does not certify arbitrary worker
computation.

Populated store20/store21 migration controllers exercise original sealed views,
signed replies, compiled preparation/completion, unknown governed effects with
an independent destination, and genuine signed acceptance ordering. They compare
all original schema/rows, migration rollback, postcommit restart, original response
bytes and old-reader refusal. The independent SQL oracle verifies all eight new
tables, exact default policy/digest and copied owner replay epochs without erasure.
`check_retention_inventory.py` additionally invokes the native root planner over
actual populated fixtures, verifies unchanged rows/schema, and preserves their
fixture files and full plan responses. CI runs the current recovery profile on
Linux, macOS and Windows and the populated store21 inventory/upgrade profile.

Current source-history and signed offline/peer/acceptance/effect controllers remain
required regressions. Frozen archive commands, actual old SDK/host journal upgrades,
exact file hashes and hosted publication results are recorded against their exact
heads separately from these development checkpoints.

See [RETENTION.md](RETENTION.md) for the complete limits. Column erasure does not
shrink the database, erase backups or recall plaintext. Collection is conservative
and bounded; receipt expiry, larger incremental collection, compiled/effectful
actor reconstruction, lifecycle cancellation/upgrade, source/portable controls,
complete platform scenarios and original formal/cryptographic/quality/performance
assurance remain mandatory. No full project gate closes from these fixtures.
