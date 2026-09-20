# Trusted host facade: step A

The safe Rust `weave_native::host::HostSession` accepts bounded runtime-supplied
operations under an immutable `HostContext`. This local embedding format is
independent of Program protocol 0.18 and store17; neither version changes. The
existing `weave_native_*` C ABI remains compatible.

This checkpoint implements Program execution and compiled handler operation. It
does not yet implement the combined cluster/transport/governance/effect facade,
generic persistent browser adapter or mobile acceptance in the
[combined proposal](proposals/COMBINED_PORTABLE_SCENARIO.md). The earlier browser
fixture's persistence evidence remains distinct.

## Safe Rust and authority

```rust
let mut session = weave_native::host::HostSession::new(engine, trusted_host)?;
let reply = session.call(request_bytes);
// Native file-backed SQLite follows Engine durability. An image host must fence
// this outcome before releasing any reply that has requires_fence == true.
```

Strict request example:

```json
{"format":"weave-host-request/1","operation":{"kind":"execute","program":{"version":"0.18.0","commands":[]}}}
```

The only operational kinds are `execute {program}`, `poll {adapter}`,
`prepare {adapter,event,lease}` and `complete {adapter,event,lease,preparation}`.
Unknown fields, unknown kinds and recursively duplicated decoded JSON keys reject
before invoking Engine. There is no operational actor, clock, write grant,
installation, raw completion, signing-key or registry opcode.

Trusted code separately calls `install_compiled_handler(bundle,name,manifest,output)`
and `set_adapter_state(adapter,state)`. These typed methods retain Engine's
installation checks; even trusted configuration cannot widen a session's output
scope. A handle is not authority for another principal's adapter. The new Engine
`*_for` methods validate the durable adapter ID, principal and output grants within
the same operation transaction, before processing or returning historical receipts.
Existing native ID-only APIs retain their trusted-host meaning and remain available.
Missing and foreign adapters share `E_HOST_AUTH`; budget/corruption failures retain
their separate bounded failure behavior. No process-local ownership cache is used.

## Complete SDK artifacts

`ArtifactBundle::parse(bytes)` accepts only successful
`weave-compiler-response/1` responses. It retains the **entire original byte array**,
returns a complete inventory of scalar/view/handler names, and exposes selected
Program/view/handler raw bytes. Scalar values remain opaque, retained data; this
helper neither evaluates them nor treats references as permission to traverse.

The helper validates the fixed SDK envelope, typed Program and canonical templates,
map-name/template-name agreement and complete artifact fingerprint. The latter is
a content identity, not a signature or proof of compiler origin. No returned
artifact is installed implicitly. A diagnostic response is not an artifact bundle.
`handler_templates` must be absent for the v1 artifact profile or a nonempty map
for v2; explicit null or empty map is rejected, as agreed with the SDK owner.

Fixed structs and bounded inventory visitors reject duplicates while decoding;
then a recursive scan rejects duplicates in all opaque payloads, including escaped
spellings of the same key. Only after this scan may fingerprinting use a JSON Value.
The aggregate 1,000 inventory members and 512-byte decoded names are checked during
map decoding, before retaining oversized maps or running the full duplicate scan.
Raw Program and template bytes are preserved rather than rewritten from a Value.

The operational request cap is **16 MiB**. The separate artifact input cap is the
SDK's **16 MiB + 4,096 bytes**. A full SDK response may pass the artifact helper even
though it cannot fit inside an operational request. Selection does not remove the
ordinary request-envelope overhead: a selected Program that makes the complete
request too large gets explicit `E_HOST_BUDGET`, never truncation or lost inventory.
The pure selector can return original bytes again with the complete inventory.

Both request/artifact profiles additionally cap JSON traversal at 1,000,000 values
and nesting depth 120; the JSON parser's own recursion checks also apply. These are
host profile bounds, not additional promises about every source the SDK can compile.
Template validators and the existing 16-command Program boundary apply as well.

## Outcomes, byte ownership and poisoning

Responses have format `weave-host-response/1`, `ok`, `value` or `error`,
`requires_fence`, and `poisoned`. The safe Rust reply exposes the exact same flags
alongside original response bytes. Requests that fail preflight have
`requires_fence=false`; every invoked Engine operation, including a returned error,
has `requires_fence=true`. This flag is a host obligation, not a claim that IDB or
any external store has committed. Do not infer safe replay from `ok=false`.

The response writer is bounded to **32 MiB + 4,096 bytes**, reserving envelope space;
it does not rely on Program's existing budget to bound other service results.
Post-operation encoding failure, storage error or unwind returns
`E_HOST_UNCERTAIN` and poisons the session. The test performs a real SQLite commit,
then an actual over-limit serialization, and verifies both blocked further calls
and the committed event/head after reopen. Existing receipts/inspection decide
recovery; no arbitrary Program or effect operation is automatically repeated.
Allocator aborts/process death remain abrupt failure boundaries.

A browser host must withhold success **and error** outcomes until a quiescent image
has committed through its durable generation transaction. For a poisoned reply it
must discard any operational payload, publish at most a host uncertainty status,
and discard/reopen the worker. It cannot export that poisoned session or turn the
uncertainty response into a durable acknowledgment. `export_image(&mut self)` also
poisons on export failure; `poison()` lets the trusted host invalidate after an IDB
failure. This feature reuses Engine's experimental 8 MiB image guard, not new browser
persistence wiring. The next adapter must count its journal in the whole generation
cap and test all persistence fences again with these arbitrary operations.

Pass UTF-8 bytes/strings unchanged through JS; do not round-trip graph/artifact
numbers through JSON Number. The new [C header](../crates/weave-native/include/weave_host.h)
uses opaque ASCII `host:N` tokens and length-bounded buffers. Responses are owned
NUL-terminated JSON released exactly once with `weave_native_free`. Tokens are
never reused within the process, and closed handles fail. The existing numeric
legacy handles are separate and unchanged.

New thin exports are `weave_host_open`, `weave_host_call`, `weave_host_close`,
`weave_host_artifact_select`, `weave_host_install_handler` and
`weave_host_set_adapter_state`. Open takes a trusted path and config separately;
install/state are privileged entry points, not remote methods. The selector takes
an SDK buffer plus strict `{kind:original|program|view|handler,name?}` selection,
and returns complete inventory plus exact selected JSON. Caller retains its original
SDK response. The current C opener uses ordinary native SQLite; arbitrary browser
image open/export and iOS bindings are not claimed by this checkpoint.

Config is bounded to 128 KiB/128 writable graphs, installation config to 256 KiB,
lifecycle config to 2 KiB, selector config to 1 KiB and registry to 128 live sessions.
One call owns a session; C calls serialize internally. Byte/work limits do not
constitute a total transient-heap, allocator or process-RSS ceiling. Source compiler
limits are separate. A trusted native embedding host must not concurrently open the same database through
another SQLite library copy in the same process. An independent probe observed stale
counts in that configuration; its cause is unresolved. Connections through the one
linked Engine copy and separate reader processes are the verified paths. Browser
images continue to require exclusive ownership.

No new toolchain, cache, contract dependency or store table is
introduced; `serde_json/raw_value` preserves already-present JSON byte slices.

## Verification

`cargo test --locked -p weave-native --all-features` covers the old ABI, safe facade,
strict artifact shape/bounds, duplicate decoded keys, foreign/narrow authority,
reopen and duplicate completion, real post-commit serialization failure and image
export invalidation. Focused engine handler/dispatcher/operation-clock suites cover
the transaction factoring and unchanged raw-completion protection.

```sh
CARGO_BUILD_JOBS=2 nice -n 10 cargo build --locked -p weave-native --all-features
python3 scripts/check_host_facade.py \
  --library target/debug/libweave_native.dylib \
  --compiler-sdk /path/to/preserved/libweave_compiler_sdk.dylib
```

The no-build script supplies two different source programs and an exact pinned
module to the actual compiler SDK at runtime. Both include a scalar above 2^53,
a view and a different handler. It checks complete inventory/exact selection,
normal installation, prepare/complete, foreign denial, closed token rejection and
historical receipt replay after reopen through the C ABI. Optional
`--sdk-fixture-dir` consumes independently generated complete SDK responses without
reconstructing their artifact bodies. The local report is
[2026-09-20-host-facade.json](measurements/2026-09-20-host-facade.json).
These native results do not claim new browser or mobile execution evidence.
