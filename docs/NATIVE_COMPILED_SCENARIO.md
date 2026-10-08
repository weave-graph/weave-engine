# Source-backed offline scenario and retained cluster journal

The native reference now runs actual compiler SDK artifacts through an atomic offline
evidence rebind, a sealed diagnostic handler, a durable retained cluster, signed
whole-capsule peer exchange, explicit integration, team governance and an unknown
external-effect fence. Both fixture variants execute; runtime libraries contain no
fixture graph names, source recipes or peer keys.

This is a bounded native acceptance profile for R02/R08/R10–R12/R14/R18/R20–R26/R28/R38
and E03–E09/E11/E13/E15. It does not close any full paper gate. The source fixtures
are the unchanged language commit `f18d35cf93ce3f45ce4e7b4b3c6971110e9d0ad4`, a direct
child of the protocol-0.19 compiler. The separate unfinished host-operation parser
and audit are excluded from this pair.

## Retained computation

`Engine::capture_retained_cluster_for` accepts one pure pinned Cluster Bind/Evaluate
whose exact source is the subscribed event. It captures the complete result, actual
runtime source identity, installed adapter manifest digest, output CAS, source
manifest and semantic dependency closure. It rejects compiled-handler or governed
effect adapter IDs. There is no ordinary JSON raw-completion operation.

The prior handoff required Complete cluster coverage. That was incompatible with
the existing navigation contract, which always returns scoped Partial to avoid
advertising a global directory. Capture and completion preserve that Partial marker
and its `I_CLUSTER_SCOPED` diagnostic. Independently, every exact semantic input is
loaded, integrity checked and required to be wholly currently visible. Missing or
restricted input cannot be admitted merely because navigation is scoped Partial.

The source snapshot gate is retained even for empty output, then propagated to
generated records. Current input/result authority, exact runtime/event/manifest
binding, full reconstructed closure and completion/result equality are checked
inside the same Engine transaction, before the existing historical receipt path.
A retry never reevaluates, changes output CAS, regenerates IDs or acknowledges
blocked work. Descriptive derivation input snapshots remain distinct from semantic
authority gates; whole transport still includes explanatory ancestry.

`weave_native::cluster_journal::ClusterJournal` durably retains the original complete
SDK response and typed computation in its own FULL-synchronous SQLite transaction
before Engine completion. A single process writer owns an OS file lock. Journal
identity binds the configured host store and actual runtime source. A separate
observed receipt is appended after completion; no atomic transaction across the two
databases is claimed. Kernel receipt without journal identity fails closed.

The journal allows 16 records, 2 MiB per serialized record plus original bundle, and
8 MiB aggregate serialized retained/observed bytes. Oversized stored rows report
`E_HOST_JOURNAL_BUDGET` before decoding; missing rows report
`E_HOST_JOURNAL_MISSING`. No truncation or automatic eviction occurs. These are byte
budgets, not RSS isolation. Storage/uncertain-outcome failures poison HostSession.

This journal belongs to the trusted native embedding application. Its digest detects
corruption, not compiler authenticity or protection from that trusted application.
Existing raw Engine APIs remain trusted operations; this profile neither sandboxes
native code nor makes an ordinary adapter kernel-managed. No Program, capsule,
canonical contract, SQLite engine marker or existing C ABI changes are introduced.

## Reproduce

Build sequentially with two jobs and incremental compilation disabled:

```sh
CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 cargo build --locked \
  -p weave-native --example native_scenario \
  -p weave-engine --example three_peer_trace --features recovery-testing
```

Build the companion compiler SDK from the exact fixture commit, then supply its
native library and source directory at runtime:

```sh
python3 scripts/check_native_scenario.py \
  --compiler-sdk /path/to/libweave_compiler_sdk.dylib \
  --host target/debug/examples/native_scenario \
  --peer-host target/debug/examples/three_peer_trace \
  --fixtures /path/to/weave-language/examples/native_scenario \
  --report native-scenario-bc.json
```

Use `.so` or `.dll` on the corresponding host. Omit `--peer-host` for the isolated
B profile. `--evidence-dir` optionally retains exact SDK request/response bytes.
Every compiler and runtime action launches a fresh bounded child process. SQLite
inspection closes connections explicitly. Only owned temporary stores are cleaned.

The supplied manifests are trusted test configuration, not capabilities. The
combined profile binds the installed principal to a deterministic test signing key.
Peer keys stay in the trusted test executable. Signed response verification is
separate from isolated proposal receipt and explicit local retention of dependency
heads; received bytes alone never grant re-export or governed acceptance.

## Executed boundaries

The trace tests before/after diagnostic preparation/completion, journal preparation,
Engine cluster completion and signed export commits; exact reply reuse; missing
journal after a kernel receipt; corrupted and rehashed trimmed closure; an unavailable
exact premise; narrowed and foreign authority; current governance expiry; immutable
old source history; private annotation omission; concurrent workstation organization;
and one fake destination action retained as Unknown until explicit reconciliation.

The native destination is a test file, with no external communication. The effect
request and handler completion retain their existing separate transaction boundaries.
This does not establish a generic exactly-once destination protocol or a new remote
Execute grant. Serialized fixture transfer is authenticated exchange without a
network service, selective proofs or a production peer-key lifecycle.

The three-platform CI job pairs the exact fixture revision, builds the same process
hosts and uploads its report. Hosted success and publication must be verified after
both branches are published; a local run does not attest Linux/Windows CI.

Browser worker bindings, generic mobile application bindings, retention/rebase,
causal-loop limits, stateful upgrades, broader recorded-history querying, incremental
cluster maintenance and full requirement/resource acceptance remain separate gates.

The [Swift facade and source-backed app profile](SWIFT_HOST.md) now cover native
Program/diagnostic operations inside a simulator app container. Cluster/peer/
governance/effect application bindings and the complete browser/mobile profile
remain separate requirements.

For the combined Rust-process trace on an existing booted simulator, build both
examples for `aarch64-apple-ios-sim`, then use:

```sh
python3 scripts/check_native_scenario_ios.py \
  --simulator EXISTING_BOOTED_SIMULATOR_UUID \
  --compiler-sdk /path/to/libweave_compiler_sdk.dylib \
  --host target/aarch64-apple-ios-sim/debug/examples/native_scenario \
  --peer-host target/aarch64-apple-ios-sim/debug/examples/three_peer_trace \
  --fixtures /path/to/weave-language/examples/native_scenario \
  --report ios-native-scenario-bc.json
```

This controller runs the native compiler on macOS and the Rust binaries via
`simctl spawn`, using host-owned temporary SQLite/artifact files. Its 202-process
success is not application-container evidence. It makes no simulator-engine RSS,
physical-device, power-loss or mobile app lifecycle claim. Controller child RSS
includes the compiler and excludes simulator grandchildren.

## Current verification checkpoint

The 2026-10-09 exact engine archive `dcc063f` passed 480 workspace tests/doctests;
the exact companion compiler archive `f18d35c` passed 191. The archived compiler and
runtime passed 202 macOS runtime processes, 12 compiler processes and 22 controlled
exits in 19.792 seconds. The same combined trace passed on the iOS26.4 simulator in
77.290 seconds. These small fixture measurements include fresh process startup and
make no throughput or production SLO claim. Strict workspace/all-target/all-feature
Clippy passed on the unchanged Rust implementation. See the resumed measurement
record for artifact identities and the separate Swift application profile. These
numbers do not attest full paper conformance or hosted CI success.
