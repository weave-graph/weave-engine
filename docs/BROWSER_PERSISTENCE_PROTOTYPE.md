# Bounded browser persistence prototype

Status: proposed implementation profile, 2026-09-20. Native base is public
protocol 0.18/store 17. No browser persistence or full E10 acceptance is claimed.
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
WEAVE_EMSDK="$HOME/.cache/weave-toolchains/emsdk-c59d6e841da55c2c21af32004c4c173cbd1c0f10" sh scripts/build_browser_image_probe.sh
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
