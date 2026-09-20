# Independent operation-clock review

Reviewed native clock foundation `25d6c2e` and corrective freeze
`175065b2841195c6c996b11299bbe42635b376cc`. This is a bounded source review and
executable regression check, not an exhaustive security certification.

## Findings and closure

1. `submit_identity_candidate` originally performed source authorization and
   candidate persistence without an operation clock or transaction. Two independent
   tests reproduced zero additional clock samples and a successful proposal write
   with a failing clock. The correction establishes a real SQL transaction and
   writer scope before authorization; unavailable time cannot reserve a candidate ID.
2. A feature-only recovery observer panic during handler completion skipped the
   manual transaction's rollback. Catching that panic outside the API left output
   visible on the same connection. The correction rolls back and resumes the
   original panic. Equivalent guards cover identity acceptance, signed proposal
   integration, governance acceptance, and governance acknowledgment. The observer
   remains a trusted recovery-testing hook, not an untrusted production callback;
   its panic is not misreported as a clock failure.

The unchanged three failing regressions pass on the corrective freeze. The positive
independent effect test also passes: unavailable time leaves a pending intent
unchanged, dispatch and nested authorization share one sample, and a retry cannot
cross an already-unknown outcome fence.

## Evidence

On the corrective freeze, this focused command passed **69 tests**:

```sh
cargo test --locked -p weave-engine --all-features \
  --test operation_clock --test root_operation_clock \
  --test identity_acceptance --test admission --test dispatch \
  --test governance --test governance_delivery --test integration
```

The independent `contended_writer_does_not_sample_clock_and_retry_captures_fresh_time`
test also passed on the original foundation: another SQLite connection holds the
writer, the failed operation takes no clock sample and writes nothing, and a later
retry records fresh time. This adds evidence for the documented writer-before-clock
ordering; the existing concurrent-read test establishes a real snapshot before its
clock callback changes the head through another connection.

Relevant reviewed boundaries:

- Snapshot acquisition precedes outer clock capture; nested scopes reuse captured
  time. Scope guards clear on success, ordinary error, and unwind.
- Signed cached reads revalidate current expiry independently of query `valid_at`.
  Governance cached acknowledgment similarly revalidates current policy expiry.
- Unrenewed expired leases cannot complete handlers or create effect intents.
  Dispatch rechecks current event authority before recording an unknown outcome.
- Constructor-installed clocks are trusted host state, not graph or request fields.
  View ticks, assertion intervals, and signed claimed timestamps remain data.
- Source queries, metadata, native resolvers, views, capsules, mounts, signed
  admission, and governance/dispatcher operations use their enclosing storage and
  clock scopes. Candidate submission was the concrete uncovered admission path.

## Limits

No remaining blocker was identified in this reviewed slice. These tests do not
simulate failed SQLite rollback I/O, arbitrary process corruption, or production
clock manipulation. The callback-panic rollback test and process-death recovery
are different evidence; only the former was independently rerun in this review.
Time is protected against regression within one engine instance, not across
restarts or database restores. Clock-free trusted administrative head/count and
lifecycle diagnostics are not reader-authorized payload APIs. The stage does not
provide distributed time attestation, automatic subscriptions, or remote authority
through source programs.

## Root integration verification

Root merged the corrected foundation with the public protocol0.15 pairing and this writer-contention regression. All301 workspace test/doctest checks passed, including the API migration compile-fail doctest; strict all-target/all-feature lint and formatting passed. Root also executed dispatch7, governance6 and signed-admission6 actual process-death checks. The protocol crate remains byte-identical to engine5067292, and the source compiler pairing stays5c3e909. Hosted results are tracked separately; no decision-graph exposure is implied.
