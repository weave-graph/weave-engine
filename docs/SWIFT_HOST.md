# Swift binding for the trusted host facade

`hosts/swift/WeaveHost.swift` wraps the existing `weave_host_*` C ABI. It opens
native SQLite under fixed trusted app configuration, passes raw UTF-8 request and
SDK bytes with explicit lengths, and frees each owned response exactly once.
It decodes only the control envelope and opaque ASCII session token. Graph and
artifact payloads are returned as `Data`, never reconstructed through NSNumber.

The caller owns one executor and the host authority. `call` does not install
configuration, change the principal, or automatically retry an error. Explicit
`installHandler` and `setAdapterState` use the privileged C entry points and retain
the native durable-owner checks. `selectArtifact` is pure and preserves the full
SDK response and unused artifacts. The input/output bounds match the native host.

Every invoked Engine outcome carries `requiresFence`, including ordinary errors.
For this file-backed native host, SQLite provides the Engine transaction boundary;
this wrapper does not add a transaction around app files or an external service.
A poisoned/uncertain outcome withholds its payload, invalidates the Swift instance,
and requires close/reopen/inspection. It never replays a Program. Closing releases
the C session even after poisoning. Browser image durability is a different host
obligation and is not implemented by this wrapper.

## Executed application profile

`SourceProbe.swift` is a test application supplied with runtime fixture files. The
controller compiles both unchanged source variants with the actual native compiler
SDK, retains its original responses, and encodes Programs with Python's exact
integer representation. The app copies those requests and responses as bytes. It
stores SQLite and test control records in its own Application Support directory.
The reusable wrapper contains no fixture graph names, source recipes or peer keys.

The 2026-10-09 profile passed 24 fresh app launches and six compiler processes on
the existing iPhone 17 Pro/iOS26.4 simulator. Four app launches exit immediately
after preparation/completion returns, before writing or releasing the result.
The following launches recover duplicate immutable preparations/receipts. The
same trace checks atomic offline evidence/metadata rebind, old pinned history,
the integer `9007199254740993`, complete SDK inventory, private-reader and foreign
adapter denial, and rollback without a head or ghost event. Each variant has six
domain events and no pending delivery or effect intent at the end.

This supplies application-container evidence for R10–R12/R18/R19/R23/R35 and E10.
It does not close their full gates. The full retained-cluster/signed-peer/governance/
effect continuation runs separately in simulator-target Rust processes; this app
does not expose those operations through Swift. Physical-device behavior, app
upgrades, storage eviction/quota, OS power loss, energy and complete portable-host
acceptance remain open. No network is used; network connectivity is not disabled.

## Reproduce

Build once with the installed iOS simulator target and companion native compiler:

```sh
CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 IPHONEOS_DEPLOYMENT_TARGET=17.0 \
  cargo build --locked -p weave-native --target aarch64-apple-ios-sim
python3 scripts/check_ios_source_app.py \
  --simulator EXISTING_BOOTED_SIMULATOR_UUID \
  --library target/aarch64-apple-ios-sim/debug/libweave_native.a \
  --compiler-sdk /path/to/libweave_compiler_sdk.dylib \
  --fixtures /path/to/weave-language/examples/native_scenario \
  --report ios-source-app.json --evidence-dir ios-source-app-evidence
```

The controller compiles Swift against `weave_host.h`, installs uniquely named
ad-hoc test apps on the selected booted simulator, captures exact outcomes and
console diagnostics, and uninstalls only those apps in `finally`. It neither builds
Rust nor creates, boots, shuts down or deletes a simulator. The operator owns the
simulator power state. Hosted CI and production distribution are separate checks.
