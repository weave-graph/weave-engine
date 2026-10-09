# Coordinated recorded selection verification (0.20 / store20)

This scoped implementation advances R04/E02 and R08/E03. It does not complete the
original paper requirements, E15, mobile services or the assurance program.

- Locked native workspace: 499 tests/doctests; strict all-target/all-feature lint
  and formatting pass with two build jobs and incremental compilation disabled.
- Actual SDK → durable SystemClock CLI: eight compiler processes and twelve runtime
  processes. Independent SQLite timestamps/checkpoints identify old/new knowledge
  after a source correction. Exact checkpoint replay, separate valid time, foreign
  observer denial and old-profile atomic rejection pass. An actual sealed view
  installs, ticks, reopens and registers a separate historical replay instance.
- Populated store19 →20 profiles preserve every original table/schema/row, the full
  observation chain, actual old source templates, signed response bytes, completed
  and pending handler preparations and one unknown governed effect. Precommit death,
  postcommit death, restart, exact replay and old-runtime refusal pass. Independent
  sink evidence reconciles once, without a second dispatch ticket.
- A real old0.19 SDK/store19 retained host journal upgrades mid-trace: 114
  runtime processes, 12 compiler processes, 16 controlled deaths.
  The completed empty cluster receipt and raw journal rows replay unchanged. Later
  source diagnostic and retained cluster recovery complete under the new runtime.
- Current actual source offline →diagnostic →cluster →signed P/W/T →governance
  →effect trace: 202 runtime processes, 12 compiler processes,
  22 controlled deaths, 17.946 seconds. Both fixtures retain scoped Partial
  navigation and current whole-input authority, with original complete SDK bytes.
- Portable semantic kernel: ten native/WASM groups produce 47,522 identical bytes.
  The new group covers descriptive witness preservation through empty composition
  and conflicting-checkpoint rejection; it does not install storage authority.
- Paired compiler SDK: 28 arbitrary-source native Rust/native C ABI/zero-import WASM
  requests produce 560,498 identical response bytes, including recorded pins,
  sealed recorded view artifacts and hidden-read denial.
- Separate locked fuzz workspace: valid semantic seeds include recorded-query
  transaction rollback. Coverage fuzzing remains part of hosted CI.

Canonical/source range selection, accepted-view recorded cuts, retention horizons,
GC/expired replay/rebase, lifecycle/state upgrades, broader incremental operators,
permission/effect source profiles, network transport and full portable application
bindings remain mandatory follow-up. Hardware antirollback, global atomic history
cuts and physical-device behavior are not claimed. Hosted results are verified
against publication heads separately; local results do not establish CI success.
