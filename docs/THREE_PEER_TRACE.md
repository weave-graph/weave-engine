# Native three-peer existing-API trace

`examples/three_peer_trace.rs` is an explicitly trusted test host, driven by `scripts/check_three_peer_trace.py`. The controller starts a fresh native process for every operation against one of three independent SQLite files: phone P, workstation W and team T. It passes serialized capsules between processes, controls delivery/drop/retry, and checks persisted state after process exit. There is no network service, production credential, fabricated delivery acknowledgment or implicit acceptance.

Run from the repository root:

```sh
CARGO_BUILD_JOBS=2 cargo build --locked -p weave-engine --example three_peer_trace --features recovery-testing
python3 scripts/check_three_peer_trace.py --report /tmp/weave-three-peer-report.json
```

The example accepts only local fixture requests. Its fixed keys, manual trusted clock, exported capsules, adapter programs and policy installation are test-host authority. Signed proposal requests exercise the existing recipient admission verifier; they do not make export a remotely authenticated endpoint. All peers share the same owner. A separate reviewer is denied warning and accepted-view content; this is not a declassification or different-owner publication policy.

## Executed transitions

1. W creates installation and required evidence snapshots in one logical batch. A separate private annotation graph stays outside the transferred closure. P receives a signed proposal without changing heads/events, repeats it after restart, then explicitly integrates the root and accepts its evidence dependency. P retains both original pins and forks its local branches.
2. With no W/T process running, P commits new negative measurement evidence and a named edge metadata rebind atomically. A trusted recipe evaluates exact metadata, finite rules and a warning filter. Its result retains both evidence and attachment premises. The graph event triggers an installed diagnostic adapter; clearing output readers is rejected. Killing before handler commit leaves no warning, killing after commit loses only the response, and exact retry produces no duplicate event.
3. A second adapter consumes the persisted warning and publishes a separate phone cluster organization. Original evidence remains readable. This path exposed and now covers the flattened proof-index ordering correction described in `WHOLE_SNAPSHOT_AUTHORIZATION.md`.
4. W concurrently changes its main installation. Reconnection imports P's changed history through signed quarantine. A stale main-head CAS fails without writes; explicit integration into `phone-import` succeeds and exact replay is duplicate. A dropped cluster transfer causes no receiver operation; subsequent receipt and duplicate delivery are safe. W keeps its main head, P's history/organization, and a separately generated workstation organization.
5. T receives the phone organization and uses actual owner-governance proposal, signed approval and acceptance APIs. The exact accepted occurrence carries a genuine decision assertion; explanation works. Another reader is denied and the original owner loses access when the current policy expires.
6. A maintenance adapter observes that genuine decision graph. The host creates an effect intent, fences dispatch durably, writes one action to a fake local destination and exits before reporting success. Restart finds `unknown`; retry refuses a second dispatch. Explicit destination evidence reconciles it to `confirmed`.

Every script assertion is part of acceptance. The JSON report records operation counts, elapsed wall time, response byte counts/hashes, capsule byte totals and trusted final event counts. Timing includes process startup and SQLite open; peak RSS is not measured. This is a small correctness fixture, not a throughput/SLO or speedup benchmark. Opaque governance occurrence IDs vary between runs, so response hashes identify that run's evidence rather than fixed golden outputs.

The recorded local run in `measurements/2026-09-20-three-peer-trace.json` passed 88 child invocations, transferred 52,585 serialized capsule bytes, and ended with P/W/T event counts 8/9/2. These are fixture measurements, not scaling claims.

## Remaining boundaries

Export and recipe execution remain trusted native-host calls. Adapter manifests do not yet load or verify executable recipes. The events are `graph.committed` and `graph.accepted`, not a typed `MetaGraphRebound` stream. The trace uses whole capsules, explicit branch/dependency acceptance and retained conflicts, not selective proofs, semantic merge, implicit multi-root acceptance or a peer transport protocol. Handler completion and effect intent creation remain separate operations. The fake destination establishes the unknown-effect fence and explicit reconciliation, not general external exactly-once behavior.

The next design review can use this executable trace to evaluate authenticated revision export or sealed handler execution. Browser persistence, different-owner release, full incremental organizations and the remaining white-paper gates stay open. The broader design is in `proposals/THREE_PEER_SCENARIO.md`.

## Independent verification

The orchestrator independently inspected the common whole-snapshot comparator and all nine visibility gates, then reran all five independent authorization regressions on `4e93ae0`. All passed. A separate fresh three-store run also passed all 88 child invocations with the same 52,585 transferred capsule bytes and 8/9/2 final event counts. Temporary databases and the fake destination were removed automatically. The Rust workflow now executes this trace on Linux, macOS and Windows; hosted success is recorded separately after execution. No protocol or SQLite marker change is required.
