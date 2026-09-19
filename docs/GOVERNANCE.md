# Native accepted-view governance

This stage implements owner and threshold acceptance over immutable source snapshots. It is a trusted native host API, independent of the language wire protocol. The SQLite schema advances from 8 to 9 transactionally. Raw graph properties, imported capsules, and signed generic graph plans cannot install governance roots, cast approvals, or change accepted heads.

## Admission and current authority

The host installs one initial policy per view. A policy binds exact member public keys, a distinct-member threshold, collectors/proposers, readers, allowed graph/branch scopes, and a validity interval. A single-member threshold of one is the owner profile. Subsequent policies require a proposal approved by the preceding policy's quorum. The replacement cannot authorize itself; there is no bootstrap override after an accepted decision.

A collector proposes either publication of a pinned source graph or a complete policy replacement, with an exact expected accepted head and expiry. Proposals remain isolated until acceptance. Ed25519 approvals bind the proposal digest, view, policy revision, expected head, member, nonce, and validity interval under a dedicated domain. The stored proposal digest includes its collector identity. The collector is a trusted `HostContext` principal; member approvals are independently signed. This API is not a network endpoint or a replacement for remote request admission.

Approval covers the exact stored source snapshot body. A LiveGraph metadata handle inside that body is not approval of future target revisions. Required live metadata is checked for current availability during admission, but no accepted expansion graph is returned by this API. Any future reusable accepted expansion must pin and separately preserve its actual influence and authority.

At proposal, approval collection, acceptance, and receipt retry, the collector must still be permitted by the current policy. Publication also rechecks current source authorization, transitive assertion/node restrictions, required metadata availability, and pinned revision reachability in an allowed accepted branch. Policy replacement rechecks the predecessor's existing source under the predecessor policy. A source must be wholly visible in this first profile; there is no selective publication or declassification. Native head inspection independently applies current policy readers and current source authorization.

One immutable approval is retained per member and proposal. An expired extra vote does not block an otherwise valid quorum. An expired member cannot replace its old vote on the same proposal; renewal requires a fresh proposal and fresh approvals. A policy that expires without an approved successor has no recovery override in this profile.

## Atomic decision and retry

Acceptance revalidates a current quorum and compares the expected head inside the same SQLite savepoint as the immutable decision, policy/head update, durable receipt, and typed governance event. Two connections competing on the same prior head yield one winner. Hosts may retry bounded SQLite busy errors; the retried loser receives a compare-and-swap conflict.

Exact lost-response retry returns the original decision only while current policy, source authority, proposal lifetime, and enough approvals remain valid. A changed body under the same collector nonce fails. A policy-transition request is deliberately rejected on retry after it installs the new policy epoch; its old approval epoch no longer authorizes the operation. Hosts can inspect the current head separately under current authority. Historical admission evidence does not confer current read authority.

The durable outbox has typed `view.accepted` and `policy.changed` events. These are separate from graph-commit events: accepting a pointer does not fabricate a graph mutation. The unscoped event count is a trusted administrative diagnostic only. Authorized delivery, acknowledgments, replay, and dissemination through the engine bus remain required next work.

## Bounds and exposure

Policies have at most 32 members, collectors, readers, and source scopes. Bodies are limited to 64 KiB before storage and before loading into Rust. A view retains at most 1,000 proposals and 64 MiB of conservatively charged governance records; collector receipt count is bounded. Cumulative operation reads use the existing engine read budget. These are logical record limits, not physical SQLite file or process-memory guarantees. Retention/garbage collection is not implemented.

The API exposes a currently authorized native head descriptor, not a reusable accepted graph result. Generic graph influence and a real immutable decision assertion with current-policy gates must be implemented and reviewed before accepted empty/scalar graph values can be exposed. No voter list, hidden partition count, or graph-shaped decision proof is synthesized by this stage.

## Evidence and remaining scope

- [Native acceptance tests](../crates/weave-engine/tests/governance.rs): owner/quorum profiles, immutable signer identity, invalid signatures, expiry, predecessor-policy transition, restart receipts, branch/reader checks, and a real two-connection conflicting-quorum race.
- [Process-death acceptance](../scripts/check_governance_recovery.py): feature-only local test host exits after acceptance SQL before commit (all acceptance rows roll back), and after commit before response (restart returns one durable receipt/event), with changed-body and expired-quorum rejection.
- [Test host](../crates/weave-engine/examples/governance_probe.rs) uses fixed local keys and is built only with `recovery-testing`.

This does not complete E11 governance or E05 event delivery. Required follow-ups include authorized governance outbox dispatch, signed remote collector admission, reusable accepted graph influence, broader merge/review policies, and operational retention. The full white-paper scope remains mapped in the project workflow.
