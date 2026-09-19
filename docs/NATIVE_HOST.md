# Trusted native embedding boundary

`weave-native` exposes the same SQLite runtime to C-compatible native applications through a small versioned C ABI. It is a trusted embedding-host interface, not a remote service or a capability verifier. The application supplies a fixed principal and explicit writable graph IDs at open time; graph programs cannot change those grants. Signed remote admission remains a separate engine API. The ABI creates no personal credentials or cloud resources.

The header is `crates/weave-native/include/weave_native.h`. Open accepts a UTF-8 database path and host JSON with `principal` and `writable_graphs`. Execute accepts the existing versioned graph Program. Every operation returns an owned NUL-terminated JSON envelope: `ok=true,value=...` on success or `ok=false,error={code,message}` on rejection. Engine diagnostic codes are retained but storage error messages stay inside the host. Free every response exactly once with `weave_native_free`; never pass another allocator's pointer. Input pointers must remain readable for the specified length throughout the call. Rust cannot validate an arbitrary foreign pointer's allocation.

Handles are process-local monotonically allocated integers, never raw Engine pointers or reusable authority tokens. At most 128 handles can be open. Operations serialize through a process-wide mutex; a handle retains its initial authority until close. The Swift wrapper is deliberately non-Sendable and confined to its owning executor. A caught panic fails the handle registry closed and requires host restart; invalid foreign pointers, allocation exhaustion and process termination are not recoverable Rust errors.

Paths are capped at 4096 bytes, host JSON at 128 KiB, principal/graph identifiers at 512 bytes, writable IDs at 128, and input programs at 16 MiB / 16 commands. Responses use the engine's exported cumulative precommit materialization limit (currently 32 MiB) plus 4096 bytes for the ABI envelope and array separators. The ABI derives its limit from that engine constant. Review caught an initial smaller response cap that could hide a successfully committed operation behind a serialization error; regression tests now prove successful replies above 16 MiB are returned and engine-budget rejection above 32 MiB rolls back earlier writes. These are serialized-byte/object bounds, not sandboxed CPU or whole-process RSS quotas.

`hosts/swift/Weave.swift` is a minimal owner-executor wrapper with response ownership and structured errors. `Probe.swift` exercises separate process starts, SQLite persistence, pinned reads, exact Decimal values beyond binary64 integer precision, principal-specific visibility and atomic rollback. The simulator app exits after each probe stage; it is an acceptance harness, not a finished mobile product. No network calls are made, but device networking is not disabled.

Reproduce native macOS acceptance:

```sh
python3 scripts/root_native.py
```

For an already configured Apple simulator toolchain, install the Rust simulator target and supply installed runtime/device identifiers:

```sh
rustup target add aarch64-apple-ios-sim
python3 scripts/root_native.py --ios \
  --runtime com.apple.CoreSimulator.SimRuntime.iOS-26-5 \
  --device-type com.apple.CoreSimulator.SimDeviceType.iPhone-17e
```

`--simctl /path/to/simctl` selects an installed simulator control binary when Xcode and installed CoreSimulator versions differ. The harness creates a dedicated temporary simulator, builds an ad-hoc signed local probe, runs seed/read/rollback/read as separate app processes, then removes only its own simulator. It neither publishes an app nor modifies another project's device data. Generated app bundles live in a temporary directory to avoid file-provider signing attributes. Existing Xcode/simulator setup and any Apple license acceptance are outside this script.

E10 remains open: this profile covers native ABI and a bounded Swift persistence scenario. It does not provide browser durable execution, Android bindings, a complete mobile SDK/UI, peer synchronization or the full paper's phone-to-governance-to-effect scenario. Passing a simulator test is not physical-device or App Store acceptance.

Local macOS acceptance at protocol 0.13 is recorded in [the measurement report](measurements/2026-09-19-native-swift-macos.json). A local iOS 26.5 simulator seed succeeded during development, but a complete automated four-stage run is not yet verified: a fresh simulator boot exceeded the first timeout, and the next installation failed with CoreSimulator placeholder state. No full iOS acceptance claim is made from a successful build or single seed stage.
