# Accepted-view governance — next bounded native stage

Status: native pointer/approval/atomic decision stages implemented; see [current profile and evidence](GOVERNANCE.md). Graph-valued decisions, authorized streams and the broader stages below remain design, not completed. Targets E11 requirements R25/R26 and the engine paper §§9–10.2. Optional means a deployment may choose owner publication or reviewed publication; the requested project still needs both profiles. This stage does not close federation consensus, policy-graph interpretation or all governance requirements.

## Boundary and proposed API

An accepted view is a named, policy-controlled pointer over existing immutable evidence. Receipt/import/mount membership does not create acceptance. No changes are made to authors' source branches or historical evidence. Separate accepted views can select different revisions without claiming universal agreement.

A trusted host installs only the initial governance policy root, out of band. Proposed immutable `GovernancePolicy` contains view ID, policy ID/revision, eligible approval verification keys (maximum 32), threshold (1..membership count), proposer subjects, current readers, permitted source graph/branch scopes, validity limits and maximum proposal bytes. Owner mode is an explicit one-member threshold profile. Source authority labels are not verification keys. No caller JSON can install a root or redefine the effective members.

Native APIs are proposed as `install_governance_root`, `propose_view`, `record_approval`, `accept_view`, `propose_policy_transition`, `accept_policy_transition`, `query_accepted_view`, and a principal-scoped decision stream. Initial administration remains a trusted embedding API. Approval signatures themselves use strict Ed25519 verification with a separate domain/version; reuse reviewed cryptographic primitives without overloading an unrelated capability action. A remote admission facade must be explicit and independently tested before being claimed.

Each proposal binds view ID, complete current policy pin, exact source GraphRef, applicable branch scope, full expected accepted-head decision revision, proposal ID, decision deadline, and a unique body digest. It is immutable and isolated from accepted heads, structural/schema registries and events until admission. Reusing an ID with a changed body rejects. Source pins do not make private evidence readable; proposal inspection and acceptance recheck all current source and proof authority.

An approval binds the exact proposal digest, policy revision, expected decision head, voter key, vote kind, approval expiry and unique nonce. Only distinct current eligible keys count. Duplicate signatures, delegated keys not present in policy, changed proposal bytes, expired votes and old-policy votes cannot increase a quorum. Signature attribution is not evidence truth. Review annotations remain separate scoped evidence, not an implied affirmative vote.

## Atomic decision and ordering

Inside one SQLite transaction the kernel rechecks current policy head/revocation, source dependencies, proposer eligibility, each approval signature and exact binding, unique quorum, source branch scope, expected accepted head, resource budgets, and nonce/body replay. It writes the decision revision, accepted pointer, approval-use/audit records, receipt and durable domain event atomically. A pre-COMMIT process death leaves none; a post-COMMIT lost response returns the original durable receipt without a second event.

The native SQLite authority provides one explicit local serialization point per view. Two conflicting quorums with the same expected head may both be valid votes, but at most one decision wins CAS. The losing proposal remains an auditable unaccepted conflict. This is not a distributed consensus algorithm. A disconnected replica may capture proposals/votes; it cannot claim this authority's accepted head until coordinated admission succeeds. A federation requiring failover or Byzantine agreement needs a separately specified ordering protocol.

Policy transition is a special immutable proposal authorized by the preceding policy's threshold and expected policy head, never by the proposed new membership. The decision atomically installs the next version and invalidates pending approvals bound to the old policy. Bootstrap authority does not remain an undocumented bypass. Emergency recovery/revocation authority must be an explicit initial policy capability or remains unsupported; there is no unreviewed administrative shortcut in an ordinary plan.

## Access paths, persistence and audit

Decision material belongs to an internal reserved namespace; raw commit, batch, capsule import, fork and revision acceptance cannot fabricate governance events or accepted decision records. Decisions contain pinned policy/proposal/source proof, not a public list of hidden reviewers or unrelated proposals. Public-facing counts/streams are scoped to current authorization and use recipient-local cursors. Internal audit tables may retain full signatures under trusted host administration; logs are not a permission-free copy.

`query_accepted_view` rechecks the current policy and current source dependencies even for a pinned historical decision or receipt retry. Source snapshots remain immutable and directly readable through independent permitted paths. Revocation stops new governed reads and decisions; it cannot recall copies already disclosed. A current decision cannot be substituted for an old pinned one, and an old policy cannot bypass current revocation.

Reusable graph results must preserve the acceptance-decision proof and original source node/assertion influences, including after saving with readers cleared. The typed-context persistable whole-value witness work establishes the empty-value influence requirement: an empty accepted result used by Support must not lose the decision authorization that influenced it. The implementation must resolve this carrier before exposing an acceptance-derived graph value; native pointer administration alone can be implemented independently. Cache/view/dispatch/change-stream paths must revalidate the same current authority without requiring source-head movement.

## Acceptance and staged implementation

1. Portable signed approval encoding and strict bounded verification: exact body/domain binding; repeated signer; invalid key; wrong view/policy/head; changed bytes; expiry; threshold bounds. No authority from raw signer strings.
2. Native isolated proposals and root policy storage; owner and reviewed profiles; immutable IDs; branch/pinned-source reachability and private evidence checks. No accepted changes from receive/propose.
3. Atomic acceptance/CAS and policy transition under prior policy. Independent concurrent conflicting quorum test, full rollback on final rejection, restart and exact nonce replay.
4. Decision graph/proof carrier and reusable query integration, current revocation across direct/historical query, copied saved value, view, adapter stream and cached signed response; namespace poisoning tests.
5. Actual two-engine offline proposal/vote capture, authority restart/reconnect, stale-policy conflict preservation; process-death hooks only in test features. Root independent review before publication.

Still mandatory beyond this slice: signed graph-valued policy interpretation, broader capability operation coverage, explicit multi-authority delegation and recovery, retained audit/stream expiry and GC, quorum membership changes under network partitions, supported federation consistency profiles, selective disclosure of decision proofs, and policy-driven compatibility/broadcast/world crossing. No claim of Sybil resistance or consensus follows merely from collecting signatures.
