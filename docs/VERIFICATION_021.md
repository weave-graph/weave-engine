# Accepted source selection and range verification (0.21 / store21)

This extension advances R04/E02, R08/E03 and E11 without completing either paper.

- Independent native oracles cover exact/date cuts, receipt versus acceptance,
  equal-time/restart ordering, finite half-open ranges, current authority, corrupt
  intermediates and old-profile preflight rollback. A real persisted empty result
  rejects changed observer/time/source/occurrence fields and missing pins, and
  rechecks current policy expiry. Historical quorum validation excludes approvals
  already expired at acceptance while preserving their authentic immutable bytes.
- Development engine:512 tests and strict lint/format, including seven native
  accepted-history oracles. Development compiler:198 tests and strict lint/format. Actual SDK/SystemClock
  accepted-history/range controller:8 compiler processes and15 runtime processes;
  existing recorded pin/sealed-view controller:8/12. Complete original SDK response
  bytes and independent SQL timestamps identify each selected protected/source pin.
- Actual source offline/diagnostic/cluster/signed P/W/T/acceptance/effect trace:
  214 runtime processes,12 compiler processes and22 controlled deaths. The one
  reference sink action remains unknown until explicit destination reconciliation.
- Four real populated store20 upgrades preserve every original table/schema/row,
  signed response, old template identity, completed/pending preparation and unknown
  effect. Precommit/postcommit deaths, exact replay and old-runtime refusal pass.
  The accepted-history controller also follows equal-time genuine ancestry.
- Real0.20 compiler/native host journal upgrades mid-trace to21:114 runtime
  processes,12 compiler processes and16 controlled deaths; original empty cluster
  completion and every journal row replay unchanged.
- Eleven native/WASM semantic groups produce53,651 identical bytes. Acceptance
  witness composition is descriptive and does not install governance authority.
-32 arbitrary-source native Rust/native ABI/zero-import WASM SDK requests produce
  562,198 identical response bytes, including accepted selectors, both ranges and
  pure hidden-read denial. Valid fuzz seeds execute history/range atomic rollback.

Publication/archive checks and hosted results are recorded against exact frozen
heads, independently of these development checkpoints. No speedup, CPU/RSS sandbox,
global recording clock or physical-device acceptance is inferred from these small
fixtures. Retention/GC/expired replay/rebase, source reactors/effects and lifecycle,
full incremental operators/clustering, authenticated network/selective transport,
complete portable applications and formal/cryptographic/quality/platform assurance
remain required. Accepted-history source values do not expand the current sealed
live/recorded-view recipe profile.
