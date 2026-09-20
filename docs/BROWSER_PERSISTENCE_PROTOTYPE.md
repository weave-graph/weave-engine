# Bounded browser persistence prototype

Status: locally verified experimental profile, 2026-09-20. Native base is public
protocol 0.18/store 17. This is bounded browser persistence evidence, not full E10
acceptance or a published general browser SDK.
The first experiment has an **8 MiB database-image cap**; this is a resource
limit of the experiment, not a product capacity claim.

## Decision and alternatives

Reuse the Rust engine, bundled SQLite and existing authorization/transaction
semantics in one Emscripten worker. Persist one complete SQLite image and its
generation in one IndexedDB transaction. The host releases a result only after
the persistence transaction completes. Keep the engine open between operations.

| Candidate | Benefit | Additional obligations | Decision |
| --- | --- | --- | --- |
| Emscripten + explicit whole-image IndexedDB generations | Same kernel; explicit atomic outer persistence boundary | O(database size) copy/write per mutation; exclusive owner; uncertainty recovery | First bounded experiment |
| Emscripten + IDBFS | Familiar synchronous filesystem surface | SQL commit is initially memory-only; synchronize all files; mtime-based reconciliation can miss same-timestamp changes; WAL and flush ordering need proof | Do not use stock syncfs as the durability claim |
| SQLite WASM OPFS VFS | Potential page-level persistence | Official JS SQLite is not a drop-in implementation beneath rusqlite; integrate one compatible SQLite/VFS ABI; worker/locking requirements vary by VFS | Later comparison if whole-image costs require it |
| Rust unknown-unknown/custom VFS or browser WASI | Alternative toolchains | New filesystem/libc/clock/random/VFS integration rather than reuse of the present synchronous host | Larger first change |
| Existing native C/Swift host | Already demonstrated simulator SQLite persistence | Expand trusted host bindings and actual complete scenario; test lifecycle and physical devices separately | Independent mobile continuation |

These choices follow the [Emscripten filesystem contract](https://emscripten.org/docs/api_reference/Filesystem-API.html),
the [IDBFS implementation](https://raw.githubusercontent.com/emscripten-core/emscripten/main/src/lib/libidbfs.js)
and [SQLite's browser persistence profiles](https://sqlite.org/wasm/doc/tip/persistence.md).
The IDBFS timestamp issue is a source-inspection finding, not a reproduced
failure in this repository. Pin upstream source before implementation tests.

## Smallest host and durability boundary

One dedicated worker owns one engine and one memory filesystem database. The
worker acquires a lifetime exclusive Web Lock keyed by the application storage
namespace before loading any image. A competing tab receives busy/unavailable;
it must not open an independent writable copy. Lack of the locking API is a
closed failure for this profile. A lock coordinates cooperating application
contexts; it does not defend against malicious same-origin script.

Add a narrowly reviewed native host storage profile selecting **DELETE journal**
before schema initialization, checking the actual resulting journal mode.
Native hosts keep their existing WAL behavior. This changes neither Program nor
the SQLite schema marker. Do not pretend the existing unconditional WAL request
proves the browser VFS implements shared-memory locking. SQLite documents that
[journal mode availability depends on the VFS](https://sqlite.org/wal.html).

The state machine is:

1. Acquire lock; distinguish an explicit first-create request from reopen.
   Retain a format/header sentinel and require a matching complete image on
   reopen; absence cannot silently initialize a fresh database. Load and validate
   the current generation's bytes. Restore only trusted local storage, never an
   arbitrary remote database image.
2. Open the same engine kernel. Persist initialization or migration before
   returning successful open. Keep this engine alive so operation-clock state
   is not reset after every command.
3. Run one operation with the existing SQL transaction and host authority.
   Admit no concurrent operation while a result or image is pending.
4. After SQL completion, require autocommit/quiescence and obtain the complete
   database image. The first feasibility proof must establish that copying the
   memory file in DELETE mode includes every committed change and requires no
   unexported WAL/journal state. If necessary use SQLite backup/serialize rather
   than closing and reopening the engine per call. Test fresh opens of exported
   images against the source engine. Never copy an active transaction.
5. In one IndexedDB readwrite transaction, atomically write a typed record
   containing storage-format version, generation and Uint8Array image. Enforce
   the image limit before copying/persisting. Retain no automatically authoritative
   older generation: falling back could revive revoked authority.
6. Wait for transaction completion before returning the operation result. A
   request-success callback is insufficient. A result which exposes a newly
   persisted event, receipt or future external-effect ticket has the same fence.

On quota failure, abort or uncertain completion, discard the pending response
and poison the worker. Reload durable state before any further operation. Do
not automatically retry arbitrary Programs. A commit followed by a lost response
is an unknown outcome; reconciliation uses exact heads and existing idempotent
receipts. Never acknowledge success from the in-memory SQLite commit alone.
Reads must not observe unflushed state after a failed write. Image corruption or
unexpected absence fails closed; it must not silently create a replacement store.

IndexedDB supplies the transaction boundary; strict durability is a requested
hint whose supported behavior is recorded, not an unconditional physical
power-loss guarantee. Browser storage eviction/user deletion remain outside
the kernel's durable-store promise. See [IndexedDB transactions](https://www.w3.org/TR/IndexedDB/),
[Web Locks](https://w3c.github.io/web-locks/) and [Storage](https://storage.spec.whatwg.org/).

## Authority, ABI and exact bytes

The current C ABI exports open/execute/close/free, not the complete native
governance, compiled-handler, signed-exchange and effect administration APIs.
The first prototype therefore proves the storage and execution foundation; it
does not claim the complete paper scenario. Later host bindings must preserve
the existing trusted-embedding boundary, current closure checks and effect
unknown-before-dispatch fence. A same-origin worker is not a remote authority.

Transfer Program and result JSON as untouched UTF-8 bytes. JavaScript
JSON.parse/stringify cannot safely roundtrip arbitrary engine i64 values or
signed/hash-bearing canonical payloads. Expose native u64 handles using the
chosen BigInt ABI; the existing open response also encodes a numeric handle in
JSON, so use an exact bounded handle decoder or a dedicated opaque handle
export rather than reading it through JSON.parse. Represent generation counters
as exact strings/BigInt.
Test 9007199254740993, i64 limits and exact decimal payloads explicitly.

Verify the actual target's SystemTime and cryptographic randomness adapters;
do not stub them, accept a plan-controlled clock, or replace randomness with
Math.random. Preserve one trusted-clock sample per outer operation and nested
reuse. A WASM trap/abort discards the worker; do not continue after a poisoned
engine or silently claim native catch-unwind behavior when using panic=abort.

## Pinned build feasibility and resources

Use the already installed nightly **2026-09-19**, rustc
`1.100.0-nightly (420ed2a0c3d7225b1744266fd884d431b4d8cfe0)`.
Install its exact rust-src component and rebuild std with `-Z build-std` for
`wasm32-unknown-emscripten`. This avoids guessing the SDK/settings used by a
prebuilt Rust std. The [Rust target documentation](https://doc.rust-lang.org/rustc/platform-support/wasm32-unknown-emscripten.html)
explicitly warns that Emscripten version and ABI flags must agree.

Pin Emscripten **6.0.9**, release
`f04ea239d533260dd1db760dd2d668d5f9a88d6b`, using emsdk commit
`c59d6e841da55c2c21af32004c4c173cbd1c0f10`. Record downloaded archive SHA256 and
actual compiler/linker settings before building. Start without pthreads,
Asyncify, side modules or ignored unresolved symbols. Use a consistent
panic-abort/BigInt configuration across std, Rust, SQLite and linker glue, then
verify the imports and runtime smoke test. A successful link alone proves no
durability behavior.

HTTP HEAD on 2026-09-20 reports the ARM64 SDK archive as **275,476,160 bytes**
(about 263 MiB). Rust-src is **5,933,532 bytes**, with manifest SHA256
`2e12707e817e74e76939ff701d379d7e0c4d3c8da3174aade4dfbbf831cecf02`.
Expected incremental disk is a provisional **2–3 GiB**, with a **4 GiB hard
stop** covering downloads, extracted SDK, std and target outputs. Inspect actual
sizes while installing; the unpacked estimate has not been measured. Reuse
installed Node, Python, existing browsers and the current Cargo target cache;
no LLVM source build, additional simulator, duplicated browser download or
parallel build. Use two Cargo jobs at nice 10 after the source agent releases
the shared slot. Remove temporary download archives after checksums and install
are recorded. Stop for review if ABI support requires exceeding this budget.

## Acceptance sequence

First demonstrate one linked worker executes the real kernel, obtains an image,
opens that image in a fresh worker and reproduces exact committed/query results.
Verify journal mode, autocommit, randomness/clock failures, memory ownership,
BigInt handles and byte preservation. No external-effect dispatch in this step.

Then test actual browser IndexedDB persistence with controlled worker/page death:

| Boundary | Required observation after real reload |
| --- | --- |
| SQL committed, IndexedDB transaction not started | Prior durable image, no released success |
| IndexedDB transaction aborted, including injected quota failure | Prior complete image; poisoned old worker cannot serve later operations |
| Death during IndexedDB transaction | One complete old or new image, never mixed generation/data |
| Transaction complete, response lost | New image retained; outcome reported unknown and reconciled without blind execution |
| Successful response then reload | Exact acknowledged heads, receipts and bytes retained |
| Concurrent tab or owner termination | No second writer; successor locks then loads the latest durable image |

Include rapid same-millisecond commits, multiple updates and rollback inside a
Program, empty/private influence and current revocation, migrations, malformed
images and exact scalar limits. Use actual worker termination/reload and browser
transaction abort injection; distinguish these from physical power loss.
Exercise a real quota failure where the test environment permits it and label
synthetic QuotaExceededError injection separately.

Measure image size, O(database size) copy/write work, WASM/artifact bytes, peak
memory and commit-to-ack latency for small and near-8-MiB stores. Whole-image
costs are expected; no universal throughput or browser-storage guarantee is
required. Stop rather than remove security checks to meet the experimental cap.

The next separately reviewed gate can expand trusted host bindings and run the
actual compiled evidence → governance → guarded reference-sink scenario across
browser restarts and native peers. Mobile extension reuses the C/Swift kernel
but still requires background/termination tests and broader device evidence.
Neither this experiment nor fixed-input portable WASM conformance closes E10.

## Toolchain preparation evidence

Approved preparation completed outside the repository at the host's reusable
`~/.cache/weave-toolchains` location. The SDK archive SHA256 is
`b60514308507f64f4138d3c55bdb6979f20222288700fde603dced23b65dd533`;
its tar entries total 1,490,083,691 unpacked bytes. The pinned emsdk source
archive SHA256 is
`ce1e21dc9447d77591f12f78bf82158bc8ef40de646150d6808d5a79555889ff`.
After extraction and archive removal, the toolchain directory occupies
1,508,052 KiB. `emcc --version` reports 6.0.9-git, source
`4e4223852a0835923411059a3929907d7df1232e`. Existing Node is v26.5.0.
Rust-src was installed from the pinned nightly manifest. These are preparation
measurements; they are not browser durability evidence.

## First link/image proof

The real engine and bundled SQLite linked in 24.24 seconds with two low-priority
jobs. The exported DELETE-mode image was 520,192 bytes. Both Node v26.5.0 and a
dedicated worker in existing Chromium 151.0.7922.34 passed image restore with
byte-identical full query results, i64 extrema and 9007199254740993, matching
head and event count. Reopen changes only SQLite header counter offsets 27/95
in this fixture, because initialization performs a transaction; the host must
fence successful open as well as later writes. The initial fixture's valid-at
query correctly pruned its isolated node; the proof uses an unfiltered query.

This uses an experimental opt-in feature and the new single-owner constructor;
default native opening still requests WAL. No arbitrary authority or source
objects are synthesized. The Rust clock remains alive while the file is copied.
The native helper alone does not enforce exclusive ownership or provide
IndexedDB durability. Those are explicit next host obligations.

Reproduce from the repository root, using the installed pinned SDK:

```sh
WEAVE_RUST_TOOLCHAIN=nightly WEAVE_EMSDK="$HOME/.cache/weave-toolchains/emsdk-c59d6e841da55c2c21af32004c4c173cbd1c0f10" sh scripts/build_browser_image_probe.sh
node -e 'require("./target/wasm32-unknown-emscripten/debug/examples/browser_image_probe.js")()'
# Set NODE_PATH to an existing installation containing Playwright if not local.
node scripts/check_browser_image_smoke.cjs
```

The Emscripten setting WASM_BIGINT emits a deprecation warning because this SDK
already always uses that ABI for WASM. No symbols are ignored or runtime
functions stubbed. Pinned SDK source connects `/dev/urandom` to `randomFill`,
which calls browser `crypto.getRandomValues`; failure behavior remains to be
fault-tested. Current toolchain, rust-src and target artifacts occupy about
1.75 GiB before any additional native test rebuild. No IndexedDB writes,
quota tests or persistent browser restart are claimed by this first proof.

## Bounded IndexedDB host checkpoint

The subsequent fixed-fixture host is
[`examples/browser-image/worker.js`](../examples/browser-image/worker.js). It uses
a lifetime Web Lock, explicit create/reopen intent and a retained header sentinel.
One IndexedDB transaction stores the complete image Blob, SHA256 and an exact
decimal generation string. Reopen checks Blob size before allocating bytes,
checks the hash, and invokes `open_restored_single_owner_image`. That constructor
refuses missing files and schema marker zero before initialization; supported
nonzero historical markers retain the existing migration path. This prevents a
valid but empty ordinary SQLite file from silently replacing an acknowledged
Weave store.

Native fixture replies use a singleton string handle and carry engine result
JSON as a string, avoiding JSON-number conversion. These are fixed trusted test
operations, not a new general host SDK, public wire protocol or source language.
The worker blocks concurrent operations while a write is awaiting durability,
withholds native results until transaction completion, and refuses every further
operation after a storage/trap/oversize failure until worker replacement.

The actual Chromium 151.0.7922.34 matrix passed all **12 named checks**:

- Explicit create, acknowledged commit, page reload and exact i64 preservation.
- Worker death after SQL and before IndexedDB retains the old image; pending
  writes expose no concurrent read.
- Worker termination during an actual readwrite transaction yields a complete
  old/new state, never mixed data and generation.
- Lost acknowledgment after IndexedDB completion retains the new state without
  automatically re-executing a Program.
- Explicit transaction abort and separately labeled synthetic quota failure
  poison the host and retain the previous acknowledged state.
- A real Chromium quota-enforced IndexedDB write fails and preserves that state.
- Cross-tab exclusion and owner termination/reacquisition load the latest image.
- Mixed-program authorization failure rolls back all graph writes.
- A committed in-memory image over 8 MiB releases no success and requires reload.
- A **7,864,320-byte** image acknowledges and reopens through actual IndexedDB.
- Malformed bytes, valid SQLite with marker zero, and missing image with retained
  sentinel fail closed rather than creating a new store.
- Actual `Browser.crash` followed by a new browser process using the same test
  profile preserves acknowledged state.

The quota case allows Chromium's 30-second cached space allowance to expire
before applying the enforcement assertion. The first immediate override did
not reject a write and was not counted as quota evidence. The failing browser
operation subsequently produced an actual QuotaExceededError. The test uses a
temporary persistent browser profile, bounded disconnect/close paths and forced
closure of owned HTTP sockets. Cleanup fallback verifies an exact test-profile
command before terminating an owned browser PID. No app, user profile, simulator
or external destination is touched; successful runs leave no fixture processes
or profiles.

Reproduce without rebuilding:

```sh
# Requires an existing Playwright installation and cached Chromium; no downloads.
node scripts/check_browser_persistence.cjs
```

The [owner report](measurements/2026-09-20-browser-image-prototype.json) records
artifact SHA256 hashes and per-operation samples. Its `persist_ms` measures only
IndexedDB transaction completion, excluding SQL, image copying and hashing:
520,192-byte images had 20 local samples spanning 2.6–8.7 ms; 7,864,320-byte
images had two spanning 6.8–7.3 ms. These are small acceptance-run observations,
not throughput promises or end-to-end performance claims. Peak resident memory
and complete operation timing remain unmeasured. WASM linear memory has a
256 MiB ceiling. The unoptimized WASM artifact is 11,992,841 bytes plus 180,425
bytes of JS glue. Conservative logical sizes for the SDK, rust-src and all
engine target files modified since preparation totaled about 3.46 GiB after the
native checks and lint, below
the 4 GiB installation/build cap; actual allocated space can be lower because
toolchain aliases share files. Rebuildable targets now live outside iCloud with
the original target path retained as a symlink.

Relevant native verification passed 13 library tests (including four new image
guards) and seven storage/integrity tests. Strict engine all-target/all-feature
Clippy and workspace formatting passed. No unchanged full workspace rerun is
claimed. Commands:

```sh
CARGO_BUILD_JOBS=2 nice -n 10 cargo test --locked -p weave-engine --features browser-image-experiment,recovery-testing --lib --test root_storage
CARGO_BUILD_JOBS=2 nice -n 10 cargo clippy --locked -p weave-engine --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

The prototype does not yet expose arbitrary compiled handlers, accepted-view
governance, peer exchange or effect dispatch to browser applications. It has not
demonstrated mobile browsers, physical power loss, eviction recovery, full
current-policy scenario parity or a production browser SDK. O(database size)
copy/write costs and the experimental 8 MiB cap remain explicit. Those limits
do not change the kernel's semantics or close the full portability gate.

## Hosted reproduction inputs

The build script defaults to the explicit `nightly-2026-09-19` toolchain name.
`WEAVE_RUST_TOOLCHAIN=nightly` is only a local alias reuse option; the script
checks its exact rustc version/hash before proceeding. For an isolated Linux
runner, the expected installation steps are:

```sh
rustup toolchain install nightly-2026-09-19 --profile minimal --component rust-src
git clone https://github.com/emscripten-core/emsdk.git "$RUNNER_TEMP/emsdk"
git -C "$RUNNER_TEMP/emsdk" checkout c59d6e841da55c2c21af32004c4c173cbd1c0f10
"$RUNNER_TEMP/emsdk/emsdk" install 6.0.9
"$RUNNER_TEMP/emsdk/emsdk" activate 6.0.9
export WEAVE_EMSDK="$RUNNER_TEMP/emsdk"
sh scripts/build_browser_image_probe.sh
```

Install Playwright **1.62.1** in a temporary runner directory and its Chromium
with required system libraries; that package provided the existing local browser
used above. Set NODE_PATH to that installation's node_modules and invoke both
browser scripts. Use Node **26.5.0** for the exact local host version. The SDK's
own configured Node is a build dependency and may be supplied by emsdk; no local
duplicate Node/browser installation was used for the owner proof. These commands
are the CI handoff, not a claim that Linux hosted browser acceptance already ran.
