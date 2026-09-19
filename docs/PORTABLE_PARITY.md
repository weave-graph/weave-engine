# Executed portable semantic parity

`weave-conformance` is a fixed-input test harness, not a storage engine, authentication boundary or general browser API. It compiles the same portable contract, spaces and clustering functions to a native executable and a `wasm32-unknown-unknown` library. The WebAssembly module exports only a pointer and length for immutable precomputed test output; callers cannot submit arbitrary plans. The Node host requires zero WebAssembly imports and reads the bounded exported result from module memory.

```sh
rustup target add wasm32-unknown-unknown
python3 scripts/root_portable.py
```

The native fixture asserts expected four-valued support across half-open temporal boundaries, independent-source union and idempotence, exact default-context reselection rejection, Decimal 0.1 + 0.2 = 0.3 and beyond-binary64 integer precision, nonterminating division rejection, nominal quantity conversion and mismatch rejection, a 3–4–5 physical distance with denied/expired access, and isolated-leaf-preserving lazy clustering. Both executions serialize complete result graphs, identities, origins, pins and coverage; the Python driver compares every output byte, not only selected scalar answers. Native assertions also execute in WebAssembly and trap on failure.

The [recorded local result](measurements/2026-09-19-portable-parity.json) has seven case groups and 19,639 identical JSON bytes. It identifies the source commit, Rust/Node versions, output digest and actual compiled WebAssembly digest. The binary digest describes this build and is not promised reproducible across compiler/path changes. CI runs the same check on Linux, macOS and Windows with Node 24; native/WASM compilation checks for the other portable crates remain in place.

These are bounded fixed host-authorized fixtures, not arbitrary-input fuzz coverage or proof of universal semantic equivalence. A known exact 3–4–5 floating fixture agreeing across targets does not establish bitwise portability for every floating operation. There is no SQLite/WASM persistent host, IndexedDB adapter, browser lifecycle or storage-quota acceptance implied by Node's WebAssembly execution. The module makes no network/storage/clock calls and cannot acquire external authority; that is a property of this fixed test artifact, not a production adapter sandbox. E10 remains open for the full supported-host scenarios.

Protocol 0.14 adds an eighth case group: strict typed context definition decoding, schema/assignment fingerprint stability under enum ordering, duplicate rejection, and unknown-support output retaining descriptor influence after an empty typed input. The [0.14 report](measurements/2026-09-19-portable-parity-0.14.json) records 22,005 identical native/WebAssembly output bytes. This exercises portable carrier semantics; SQLite descriptor authorization remains covered by native integration tests.
