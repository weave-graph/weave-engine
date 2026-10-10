# Generic browser host image profile

Protocol 0.21 / store 29; experimental trusted application embedding. This
profile extends the [first persistence experiment](BROWSER_PERSISTENCE_PROTOTYPE.md)
to the ordinary [request2 lifecycle facade](HOST_LIFECYCLE.md). Full browser and
mobile application/device acceptance, portable recorded tools/effects, source
actor compilation and every wider original requirement remain mandatory.

## Shared runtime and authority

`weave-host` now owns the original Rust facade, complete artifact helper and
cluster journal. `weave-native` re-exports its original public Rust module paths;
the C/Swift entry points and signatures remain unchanged. The thin
`weave-browser-host` example links the shared facade without linking a native
dynamic library as an Emscripten side module. Registry dependency versions,
contract bytes, protocol and store marker are unchanged.

One worker owns one ordinary Engine, bundled SQLite in DELETE mode, an immutable
trusted principal/output configuration and a live operation clock. Raw requests
cannot grant authority or reconcile a broker. Request1 retains its four existing
operations; request2 exposes the same 31 bounded operations as native. Exposure
does not constitute full execution evidence for every operation. The verified
browser lifecycle below uses actual pure compiled sources, rather than a fixed
domain recipe inside the transport.

The app supplies `open` with `store`, explicit `create` and `authority_raw`.
`call` carries `request_raw`. Trusted `install_handler` carries the entire
`sdk_raw` and `config_raw`; `install_actor` carries its definition in
`config_raw`. `retain_sdk` validates and retains a complete SDK inventory before
an app selects a genuinely changed template for ordinary compiled migration.
Retention installs no adapter and confers no authority. Initial configuration
and installation belong to the trusted embedding, never to source evaluation.
Portable actor/tool/effect execution is not established by the installation API.

Graph, template and SDK JSON travels as untouched UTF-8. The worker parses only
the outer `weave-browser-outcome/1` frame, bounded image-export metadata and its
own journal structure. Its `response_json` remains a string. Callers must retain
raw numeric payloads or use an exact integer decoder; parsing and rewriting a
graph with JavaScript Number can round integers beyond 2^53. The independent
Python image inspector verifies actual reconstructed content and exact values.

## Complete generation and uncertainty

A lifetime exclusive Web Lock covers the storage namespace, one live engine
and every in-flight operation/fence. A second tab fails closed. After operation
execution, every `requires_fence` outcome, including ordinary engine errors,
exports a quiescent whole SQLite image. A strict IndexedDB readwrite transaction
writes its typed namespace header and one format2 image record. That record
contains a decimal u64 generation, image Blob/hash and complete artifact journal
UTF-8/hash. Both original and upgraded SDK inventories retain unselected scalar
values and view templates. No result is posted before transaction completion.

SDK admission failures do not enter the journal. A whole-generation fence failure
discards the pending operational response and poisons the worker. No subsequent
read can expose unflushed state. Reopen verifies the current complete generation;
it never falls back to an older record or initializes a missing/corrupt store.
Explicit creation rejects an existing header/image, including a broken store.
Current owner/output checks also apply to historical lifecycle receipts after
reopen with foreign or narrowed trusted authority.

An interrupted write has an unknown outcome. Terminate the worker, reopen durable
state and inspect actual heads or exact receipts before deciding what to do.
Arbitrary Programs are not automatically replayed. Exact completion/rebuild/
migration retries can identify an already committed receipt without rewinding
later state. Transaction request success alone is not an acknowledgment.

Strict durability is a browser hint whose observed value is recorded, not a
physical power-loss guarantee. Web Locks coordinate cooperating contexts;
hashes detect corruption, not malicious same-origin replacement or authenticity.
Storage eviction/deletion and a hostile same-origin application remain outside
this trusted local persistence profile. Generation format1 belongs to the older
fixed experiment and is rejected by this distinct format2 profile.

## Bounds and reproduction

The combined SQLite image plus serialized whole journal has an 8 MiB cap and at
most 16 retained inventory/installation entries. Native/request/response and
command bounds remain those of the shared facade. Authority configuration is
128 KiB; handler configuration is 256 KiB; actor configuration is 2 MiB; SDK
response admission is 16 MiB plus 4 KiB. All byte limits apply before native
transport copying; the tighter combined persistence cap also applies afterward.
Serialized budgets are not CPU/RSS isolation. WASM linear memory is bounded to
256 MiB; other browser allocations remain separate.

The pinned Rust 1.100.0 nightly and Emscripten 6.0.9 source revisions match the
first image experiment. The generic host explicitly reserves a 4 MiB stack with
stack-overflow checks. The toolchain's default 64 KiB stack reproduced a memory
trap during actual compiled preparation; the checked stack passes both complete
source cases. A trap poisons the worker and withholds operational results.
Initial failed build/trap/controller evidence is retained separately.

```sh
WEAVE_EMSDK=/path/to/pinned/emsdk sh scripts/build_browser_host.sh
python3 scripts/prepare_browser_host_sources.py \
  --compiler-sdk /path/to/libweave_compiler_sdk.so --output /tmp/host-sources
NODE_PATH=/path/to/existing/playwright/node_modules node scripts/check_browser_host.cjs \
  target/wasm32-unknown-emscripten/debug/examples/browser_host.js \
  /tmp/host-sources /tmp/host-evidence
```

The source/evidence directories must be new. The harness uses an owned temporary
browser profile and local server, and cleans them after the run. Worker fault
hooks (`after-sql`, `during-idb`, `after-idb`, `abort`, `synthetic-quota`) are
explicit experimental verification controls. Real quota enforcement is tested
separately. Hosted browser CI preserves raw source/SDK/request/response/image
artifacts and also runs the original store17-to-current persistence profile.
See [verification](VERIFICATION_BROWSER_HOST.md).
