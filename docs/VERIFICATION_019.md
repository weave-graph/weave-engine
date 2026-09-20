# Protocol 0.19 joint verification

Compiler freeze: `9dec64937fbd191f1698fa2f1bb568acc6ed7776`.
Native semantic freeze: `a6adb949de08284dc9f7e8d013e38c1380416e12`.
Protocol0.19, SQLite marker18, capsule0.4. Later publication commits add documentation and integration gates without changing these semantic artifacts.

- Portable contract: 119 tests and strict lint; native/WASM conformance has 43,746 identical bytes, including clipped and repeated identity Window, Sequence, and empty alternative influence through Support.
- Independent native oracles: 7 final tests cover 24 principal/carrier combinations, 28 temporal relation cases, original record authority, distinct denied snapshot IDs, cycle handling and old-wire rollback. Earlier full native workspace/all-feature baseline passed 462 tests; this count belongs to the earlier native freeze, not a claimed final cumulative total.
- Compiler: 191 workspace tests, strict lint/fmt, 17 source/runtime suites, 25-request native/WASM SDK parity (558,761 identical bytes), fixed conformance parity (20,050 bytes), and unchanged historical view/handler compatibility. Root independently checked SDK/CLI artifact and fingerprint equivalence for seven examples, invalid-input recovery and handle lifetimes across 18 requests.
- Actual source compiler→native runtime: 22 processes/12 checks, including pinned replay, correction as a new revision, returned-wrapper metadata, detached private proof persistence and empty-metadata scalar protection.
- Six populated migration suites: handler stores16/17, view/signed-receipt stores14/15/17, and an unknown governed effect at store17 all migrate to18. Before/after commit deaths preserve historical rows and exact replay; old runtimes refuse; an independently committed sink action does not dispatch twice.
- Real browser matrix: 16 cases, including four old17→18 migration interruption boundaries, IndexedDB transaction abort and real quota failure, exclusive ownership, atomic authorization rollback, near8MiB image restore and actual browser-process crash/reopen. Migration compares all returned logical bytes while explicitly allowing the fresh query's protocol field to change; exact i64 values never pass through floating-point normalization.

The final browser WASM SHA256 is `dd833b712cbd85993bc64cb246e52d90bb5216ccc7301dcd64e8faba97bf3a1d`, executed with Chromium151.0.7922.34. The browser remains an experimental bounded image host; no physical power-loss claim or complete portable scenario follows.

Reproducible gates live in `.github/workflows/ci.yml`, `integration.yml`, and `browser.yml`. The exact compiler Git archive passed locked offline builds and repeated temporal/SDK acceptance. The freshly linked native library passed actual current0.19 and historical0.18 SDK installation, prepare/complete/restart and foreign-principal negatives. Root independently passed strict raw artifact/duplicate key checks, atomic rollback and handle lifetime tests. Hosted results are recorded separately with the paired publication. The next integrated native scenario is specified in [scenario B](proposals/NATIVE_SCENARIO_B.md).

Full recorded-time queries, general incremental/clustering behavior, source effects, adapter lifecycle/retention and complete multi-peer browser/mobile acceptance remain open. Passing these checks does not close the full white-paper scope.
