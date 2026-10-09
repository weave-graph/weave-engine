# Native accepted-view history verification

This additive native profile advances R04/E02, R08/E03 and R26/E11 without
closing the full paper gates. Protocol0.20, store20 and capsule0.4 are unchanged.
There is no migration or rewriting of original decisions.

- Six independent oracles cover late acceptance versus source recording, exact
  replay, half-open ranges, equal times and restart, persistent clock regression,
  foreign/missing/future cuts, corrupt or lost signed intermediates, historical
  approval expiry and current policy revocation.
- Full workspace: 505 test/doctest checks, strict all-target/all-feature lint and
  formatting. The serialized aggregate bound has a separate final focused run.
- Actual original compiler SDK/source variants: 214 runtime processes, 12
  compiler processes and 22 controlled deaths. Independent SQL observes graph
  receipt at20 and genuine team acceptance at30. Selection before acceptance,
  outsider and foreign-observer cuts fail closed. Exact and date selections agree
  with the ordinary accepted graph. The complete offline diagnostic, retained
  cluster, signed three-peer and unknown-effect reconciliation trace also passes.
- Real populated store20 compatibility: eight fresh processes verify all original
  table schemas/rows remain unchanged on historical reads, a source correction
  preserves the exact old decision, a new equal-time acceptance follows genuine
  ancestry, and the original runtime still reads the original accepted result.

Walks/ranges are bounded to 1,000 entries, share the trusted operation clock and
128 MiB /4,096-read budget, and bound serialized results to32 MiB. These limits
do not establish CPU/RSS isolation. Canonical/source accepted-time and range
selectors, retention/GC/expired replay, full reactor/effect/lifecycle semantics,
incremental operators/clustering, network transport, full portable applications
and formal/cryptographic/quality/platform assurance remain required.
