# Source-compiled version transfer verification

Requirements R12/R13/R36/R38; gates E05/E06/E14. Native store24 keeps event
protocol0.21 and capsule0.4. Exact publication and archive receipts remain
separate evidence; the full original scope remains open.

`root_compiled_migration` checks actual sealed registrations, source/output
revisions, immutable preparations and receipts, registry rows and private
checkpoints. Upgrade transfers the current private checkpoint into a fresh
paused namespace. Actual completion then advances that namespace. Rollback
restores the recorded prior artifact/configuration/checkpoint pair into another
fresh namespace, where pure replay produces another real output. Earlier
outputs and preparation bytes remain historical. Historical migration retries
never rewind later work. Current owner/output authority, whole input/metadata
authority, current input CAS, compatible sealed scopes, unresolved work and
replay expiry are checked independently. Collection validates immutable audit
bindings against actual registered versions before erasing any payload.

`scripts/check_compiled_lifecycle.py` compiles two real `.weave` handler versions
with the exact historical0.21 compiler. An actual old23 host commits input,
prepares/completes version1, commits newer input, and pauses the source. The
controller upgrades that genuine store, installs version2, completes new work,
and restores the recorded version1 pair in version3. It also migrates separately
populated old23 databases containing real GC tombstones, opaque state, native
upgrade/rollback receipts, and canceled stale compiled preparations.

The controller runs61 compiler/native processes and12 controlled deaths: three
schema upgrade pairs, compiled upgrade, real completion and compiled rollback.
Before each interrupted transaction, the complete SQL schema/row snapshot must
remain exact. After commit/restart, every prior row/schema remains exact through
initialization; transfers preserve heads/events/history and atomically bind the
new registry, checkpoint, retirement and receipt. Old23 readers refuse the new
stores without writes. Historical records and exact duplicates survive reopen.
`--evidence-dir` preserves source files, emitted artifacts, actual databases,
responses and independent SQL snapshots before temporary cleanup.

Hosted Rust verification runs this source/process controller on Ubuntu, macOS
and Windows. Earlier real store20/21/22 migration and journal controllers remain
required; populated store23 snapshot/handler/effect/accepted history and actual
retained SDK journals are added. Full source/peer/governance/effect traces,
native retention/lifecycle recovery, strict lint/format, portable semantic parity
and browser persistence remain separate regressions.

Native opaque state, effectful actors and external retained journals are excluded
from stateless compiled transfer. Expired replay requires explicit reconstruction;
it cannot be relabeled compatible. Source/portable lifecycle command bindings,
complete actor lifecycle, causal/resource controls, receipt expiry and every
remaining original platform/formal/cryptographic/performance gate stay mandatory.
