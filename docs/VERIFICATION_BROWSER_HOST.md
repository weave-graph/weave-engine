# Generic browser host verification

Local development evidence, 2026-10-10. Requirements R10/R12/R13/R14/R36/R38 and
gates E05/E06/E10/E14 remain part of the full original project; no full gate is
closed by this bounded profile. Frozen exact-source and hosted acceptance remain
required before publication.

The shared workspace passes 608 tests/doctests and strict all-target/all-feature
Clippy. Three actual image tests cover exact >2^53/i64-max restore, abandonment
after outer-fence failure, immutable authority preflight, explicit creation and
oversized/uninitialized restore. Existing native APIs still pass the actual
65-process/eight-death C lifecycle controller, including genuine old28/27/26
histories, ten native tool samples and five physical receipts. The two original
source C lifecycle cases pass after facade extraction.

The actual generic host WASM is 17,690,469 bytes in the local debug profile. The
checked-stack source build uses the same Rust/Emscripten/SQLite ABI as the first
image experiment. Chromium 151.0.7922.34 passes 28 check groups / 169 actual RPCs,
109 measured completed fences, 16 staged worker terminations and one actual
browser-process crash/profile reopen. Four actual SDK compilations produce two
independent source cases and their genuinely changed revisions. Evidence retains
raw requests/responses, source/SDK bytes, typed whole-generation metadata and
actual exported SQLite images. Initial failures are preserved separately.

Verified behavior:

- Arbitrary request2 Programs preserve exact integers above 2^53 and i64 min/max
  through real SQLite, worker reload and browser-process crash.
- Worker death after SQL/before IDB, during an actual live readwrite transaction,
  and after IDB/before acknowledgment yields a complete old or new generation.
  Concurrent access to a pending fence is rejected. A second actual tab cannot
  open an independent writable image.
- Actual IDB abort and separately labeled synthetic quota poison the worker.
  Chromium quota override is checked and its cached allowance expires before
  actual `QuotaExceededError` enforcement. Reopen restores the acknowledged
  graph and journals; no failed write payload is released.
- Above-capacity post-SQL state is withheld and abandoned; a near-cap image
  restores successfully. The 8 MiB bound includes the serialized journal.
- Whole SDK retention survives each storage boundary with its original UTF-8
  inventory. Malformed/incomplete SDKs cannot enter the journal. Both original
  and upgraded scalar/handler/view inventories survive lifecycle transfer.
- Actual source-compiled completion, reconstruction, upgrade and recorded
  rollback survive lost acknowledgments and exact receipt retries. Current
  foreign and narrowed owner/output grants reject historical requests.
- The independent Python inspector verifies exact content, entity identity,
  external premises and correct self-output revision attribution. Reconstruction
  creates new owned node/attachment occurrence IDs. It also inspects actual
  SQLite integrity, marker, heads, one completed handler receipt, one rebuild,
  two migrations, no pending delivery and no fabricated effect intent.
- Duplicate decoded request fields/unsupported legacy opcodes reject before a
  fence. Invoked ordinary errors are fenced before their outcomes are released.
- Image/journal hash corruption, unsupported generation format, u64 overflow,
  invalid journal record type, malformed/uninitialized SQLite, future storage
  marker and missing image fail closed without replacing persisted bytes.

The original fixed browser persistence/store17 migration controller remains a
separate hosted profile. Initial unknown-symbol dynamic-library linking was
resolved by extracting the shared Rust host. The 64 KiB default Emscripten stack
reproduced a preparation memory trap; the explicit checked 4 MiB stack resolves
it in both source cases. An initial 4 MiB payload fixture remained within image
capacity; the corrected 9 MiB overflow fixture and 7 MiB near-cap fixture verify
the actual boundary instead of treating payload size as database size.

Physical power loss, other browser families/mobile applications/devices, complete
portable recorded tools/effects, full source lifecycle compilation, broader graph
reconstruction/retention, CPU/RSS isolation, complete cross-peer causal/network
semantics and every original quality/formal/cryptographic/performance/release
obligation remain required. No model truth, remote exactly-once or full E15 claim
is made. [Semantics and reproduction](BROWSER_HOST.md).
