# Compiled-handler verification checkpoint

Protocol 0.18 / SQLite store 16 is a verified implementation checkpoint. Full paper conformance and release gates remain open. Native implementation freeze: `9b444ffdaf183826008f07f55c5a0bc7a81de191`. Exact source pairing is `e18608b842ad6efc14f977e30ed428513c5900a5`, pinned by the language integration workflow. Hosted results must be checked separately from these local results.

## Local evidence

- Engine owner: 405 workspace all-feature tests/doctests, strict workspace all-target/all-feature Clippy and formatting passed on the coherent freeze.
- Independent orchestrator: three native regressions verify raw-completion refusal before and after a genuine compiled receipt; renewed lease reuse of exact preparation and stale output CAS; and genuine empty-input influence surviving detached output and historical receipt checks.
- Actual source-owned workflow: 25 compiler processes and 57 native processes, including four deliberate before/after preparation/completion deaths. The report lists 12 behaviors: pinned module identity, complete artifacts and legacy rejection, Metadata/Reason, committed/accepted events, immutable replay, private/detached record visibility, original object provenance, empty/scalar snapshot influence, crash boundaries, renewed leases, stale CAS rollback and raw completion rejection. Its 83 fixture files are not a test count. The orchestrator ran the actual compiler and native fixture independently.
- Both populated historical upgrades, protocol 0.16/store14 and 0.17/store15 to store16, passed independently: real old sealed view/cache and signed receipt; precommit death rollback; postcommit death/restart; unchanged historical rows; exact signed response replay; and old binary refusal. Owner reports are in `docs/measurements/2026-09-20-handler-migration-016.json` and `2026-09-20-handler-migration-017.json`.

The source owner verified 151 tests, strict lint/format, 8,723-byte executed native/WASM parity, 22 historical artifact compatibility cases, actual handler/view/module/schema/live/geometry integrations and an exact-commit locked offline source-archive build. The archive was removed afterward. The orchestrator independently compared all 28 vendored files and contract documentation against the exact native commit and their manifest hashes.

## Hosted reproduction

The Rust workflow builds exact public prior revisions for both migration paths. Language integration builds the actual source compiler and `compiled_handler_fixture`, then runs `check_handlers.py`. Existing platform, portable parity, signed three-peer recovery and fuzz jobs remain required.

The artifact is inert until explicit trusted host installation. The host owns principal, output binding and effective authority. Preparation stores a validated immutable command and original CAS; completion rechecks current input/output authority. External effect intent atomicity, release/declassification, sandbox isolation, retention/GC, full portable-host paper scenarios and the remaining language semantics are separate open scope.
