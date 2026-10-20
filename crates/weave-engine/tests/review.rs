//! B5 (#12): optional review of shared knowledge.
//!
//! A registry store's graphs publish directly by default; a graph Administer
//! holder can make one reviewed. Writes to a reviewed graph are proposals
//! that eligible reviewers accept or reject, under the existing governance
//! rules (immutable approval subjects, M of N, predecessor-authorized policy
//! transitions) on registry authority.
use serde_json::{json, Value};
use std::sync::Arc;
use weave_contract::{CommandResult, FactChange, Program, VERSION};
use weave_engine::{
    Engine, HostContext, ManualClock, RegistryChange, ReviewMode, ReviewPolicyChange,
    ReviewProposal, ReviewProposalFilter, ReviewProposalReceipt, ReviewStatus, ReviewVerdict,
    ReviewVote, TrustedClock,
};

const ADMIN: &str = "root";
const KB: &str = "team-kb";
const NOTES: &str = "alice-notes";
const BOT: &str = "alice-bot";

fn host(principal: &str) -> HostContext {
    HostContext::new(principal, Vec::<String>::new())
}
fn program(version: &str, commands: Value) -> Program {
    serde_json::from_value(json!({"version": version, "commands": commands})).unwrap()
}
fn changes(facts: Value) -> Vec<FactChange> {
    serde_json::from_value(Value::Array(
        facts
            .as_array()
            .unwrap()
            .iter()
            .map(|f| json!({"change": "assert", "fact": f}))
            .collect(),
    ))
    .unwrap()
}
fn node(id: &str) -> Value {
    json!({"kind": "node", "id": id, "entity_id": id, "space_id": "s",
        "provenance": {"method": {"kind": "model", "model": "extractor", "version": "1"}}})
}
fn code<T: std::fmt::Debug>(result: weave_engine::Result<T>) -> (String, String) {
    let error = result.unwrap_err();
    (error.code, error.message)
}
type R<T> = weave_engine::Result<T>;
type Check = Box<dyn Fn(&Store) -> String>;

struct Store {
    e: Engine,
    clock: Arc<ManualClock>,
}

impl Store {
    /// Users alice, bob, carol, dave and erin. Group `reviewers` = {bob,
    /// carol}; group `team` = {alice, bob, carol}. `team` writes `team-kb`,
    /// `dave` and `erin` read it. `alice` administers her personal graph.
    /// `alice-bot` is alice's agent with Write on `team-kb`.
    fn new() -> Self {
        let clock = Arc::new(ManualClock::new(10));
        let s = Self {
            e: Engine::memory_with_clock(clock.clone()).unwrap(),
            clock,
        };
        s.e.enable_registry(&host(ADMIN)).unwrap();
        s.reg(
            ADMIN,
            json!([
                {"change": "add_principal", "id": "alice", "kind": "user"},
                {"change": "add_principal", "id": "bob", "kind": "user"},
                {"change": "add_principal", "id": "carol", "kind": "user"},
                {"change": "add_principal", "id": "dave", "kind": "user"},
                {"change": "add_principal", "id": "erin", "kind": "user"},
                {"change": "add_principal", "id": "reviewers", "kind": "group"},
                {"change": "add_principal", "id": "team", "kind": "group"},
                {"change": "add_principal", "id": "ops", "kind": "group"},
                {"change": "add_member", "group": "reviewers", "member": "bob"},
                {"change": "add_member", "group": "reviewers", "member": "carol"},
                {"change": "add_member", "group": "team", "member": "alice"},
                {"change": "add_member", "group": "team", "member": "bob"},
                {"change": "add_member", "group": "team", "member": "carol"},
                {"change": "add_member", "group": "ops", "member": "carol"},
                {"change": "add_member", "group": "ops", "member": "alice"},
                {"change": "grant", "graph_id": KB, "subject": "team", "permission": "write"},
                {"change": "grant", "graph_id": KB, "subject": "dave", "permission": "read"},
                {"change": "grant", "graph_id": KB, "subject": "erin", "permission": "read"},
                {"change": "grant", "graph_id": NOTES, "subject": "alice", "permission": "administer"},
                {"change": "add_agent", "id": BOT, "owner": "alice", "delegation": {
                    "scopes": [{"graph_id": KB, "permission": "write"}],
                    "exercise": {"kind": "owner_all"}}}
            ]),
        )
        .unwrap();
        s
    }
    fn now(&self) -> i64 {
        self.clock.unix_millis().unwrap()
    }
    fn tick(&self) {
        self.clock.set(self.now() + 10);
    }
    fn reg(&self, who: &str, changes: Value) -> R<String> {
        self.tick();
        let changes: Vec<RegistryChange> = serde_json::from_value(changes).unwrap();
        self.e.apply_registry(&changes, &host(who))
    }
    fn run_version(&mut self, who: &str, version: &str, commands: Value) -> R<Vec<CommandResult>> {
        self.tick();
        self.e.execute(&program(version, commands), &host(who))
    }
    fn run(&mut self, who: &str, commands: Value) -> R<Vec<CommandResult>> {
        self.run_version(who, VERSION, commands)
    }
    fn write(&mut self, who: &str, graph: &str, facts: Value) -> R<Vec<CommandResult>> {
        let changes: Vec<Value> = facts
            .as_array()
            .unwrap()
            .iter()
            .map(|f| json!({"change": "assert", "fact": f}))
            .collect();
        self.run(
            who,
            json!([{"op": "write", "graph_id": graph, "changes": changes}]),
        )
    }
    fn query_version(&mut self, who: &str, version: &str, graph: &str) -> R<Value> {
        let mut results = self.run_version(
            who,
            version,
            json!([{"op": "query", "query": {"graph_id": graph}}]),
        )?;
        match results.pop() {
            Some(CommandResult::Queried { result }) => Ok(serde_json::to_value(result).unwrap()),
            other => panic!("{other:?}"),
        }
    }
    fn nodes(&mut self, who: &str, graph: &str) -> Vec<String> {
        match self.query_version(who, VERSION, graph) {
            Ok(result) => {
                let mut ids: Vec<String> = result["graph"]["nodes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|n| n["id"].as_str().unwrap().to_string())
                    .collect();
                ids.sort();
                ids
            }
            Err(_) => vec![],
        }
    }
    fn reviewed(&self, graph: &str, reviewers: &[&str], threshold: u32) -> u64 {
        self.tick();
        let current = self.e.review_policy(graph, &host(ADMIN)).unwrap();
        let mode = ReviewMode::Reviewed {
            reviewers: reviewers.iter().map(|r| r.to_string()).collect(),
            threshold,
        };
        match self
            .e
            .set_review_policy(graph, &mode, current.revision, &host(ADMIN))
            .unwrap()
        {
            ReviewPolicyChange::Applied { policy } => policy.revision,
            other => panic!("{other:?}"),
        }
    }
    fn propose(&self, who: &str, graph: &str, facts: Value) -> R<ReviewProposalReceipt> {
        self.tick();
        self.e
            .propose_review(graph, "main", &changes(facts), &host(who))
    }
    fn vote(
        &self,
        who: &str,
        receipt: &ReviewProposalReceipt,
        verdict: ReviewVerdict,
    ) -> R<ReviewProposal> {
        self.tick();
        self.e.vote_review(
            &ReviewVote {
                proposal_id: receipt.proposal_id.clone(),
                digest: receipt.digest.clone(),
                verdict,
                note: None,
            },
            &host(who),
        )
    }
    fn list(&self, who: &str, graph: &str) -> Vec<String> {
        self.e
            .list_review_proposals(
                &ReviewProposalFilter {
                    graph_id: graph.into(),
                    ..Default::default()
                },
                &host(who),
            )
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect()
    }
}

#[test]
fn personal_graphs_publish_immediately() {
    let mut s = Store::new();
    let policy = s.e.review_policy(NOTES, &host("alice")).unwrap();
    assert_eq!(policy.revision, 0);
    assert_eq!(policy.mode, ReviewMode::Direct);
    s.write("alice", NOTES, json!([node("n1")])).unwrap();
    assert_eq!(s.nodes("alice", NOTES), vec!["n1"]);
    // An explicit direct policy is the same.
    s.tick();
    s.e.set_review_policy(NOTES, &ReviewMode::Direct, 0, &host("alice"))
        .unwrap();
    s.write("alice", NOTES, json!([node("n2")])).unwrap();
    assert_eq!(s.nodes("alice", NOTES), vec!["n1", "n2"]);
    assert_eq!(
        code(s.propose("alice", NOTES, json!([node("n3")]))).0,
        "E_REVIEW_NOT_REQUIRED"
    );
}

#[test]
fn a_team_graph_publishes_only_through_accepted_proposals() {
    let mut s = Store::new();
    s.write("alice", KB, json!([node("seed")])).unwrap();
    assert_eq!(s.reviewed(KB, &["reviewers"], 2), 1);
    // Every direct write path is refused, for agents and users alike.
    assert_eq!(
        code(s.write(BOT, KB, json!([node("n1")]))).0,
        "E_REVIEW_REQUIRED"
    );
    assert_eq!(
        code(s.write("alice", KB, json!([node("n1")]))).0,
        "E_REVIEW_REQUIRED"
    );
    let commit = s.run(
        ADMIN,
        json!([{"op": "commit", "graph_id": KB, "expected_head": null, "data": {"nodes": []}}]),
    );
    assert_eq!(code(commit).0, "E_REVIEW_REQUIRED");
    // A reader without Write still gets the authorization error.
    assert_eq!(
        code(s.write("dave", KB, json!([node("n1")]))).0,
        "E_FORBIDDEN"
    );
    let receipt = s
        .propose(
            BOT,
            KB,
            json!([node("n1"), node("n2"),
                {"kind": "edge", "id": "e1", "predicate": "near", "from": "n1", "to": "n2",
                 "valid_time": {"start": 0}},
                {"kind": "embedding_space", "id": "sp", "revision": "1", "encoder": "test",
                 "encoder_version": "v1", "preprocessing": "none", "dimensions": 2,
                 "metric": "euclidean"},
                {"kind": "embedding", "id": "m1", "space": {"id": "sp", "revision": "1"},
                 "subject": {"kind": "node", "node_id": "n1"}, "vector": [1.0, 0.0],
                 "valid_time": {"start": 0},
                 "provenance": {"method": {"kind": "model", "model": "e", "version": "1"}}}]),
        )
        .unwrap();
    assert_eq!(receipt.version, 1);
    let proposed_at = s.now();
    // Pending: readers see only the seed.
    let before = s.query_version("dave", VERSION, KB).unwrap();
    assert_eq!(s.nodes("dave", KB), vec!["seed"]);
    let pending = s.vote("bob", &receipt, ReviewVerdict::Approve).unwrap();
    assert_eq!(pending.status, ReviewStatus::Pending);
    assert_eq!(pending.approvals, vec!["bob"]);
    assert_eq!(s.query_version("dave", VERSION, KB).unwrap(), before);
    // A reader that is not a reviewer never sees the proposal.
    assert!(s.list("dave", KB).is_empty());
    let unknown = code(s.e.review_proposal("proposal:unknown", &host("dave")));
    assert_eq!(
        code(s.e.review_proposal(&receipt.proposal_id, &host("dave"))),
        unknown
    );
    assert_eq!(unknown.0, "E_UNAVAILABLE");
    // The second approval publishes the proposal as its proposer.
    let accepted = s.vote("carol", &receipt, ReviewVerdict::Approve).unwrap();
    let decided_at = s.now();
    assert_eq!(accepted.status, ReviewStatus::Accepted);
    let decision = accepted.decision.unwrap();
    assert_eq!(decision.reviewers, vec!["bob", "carol"]);
    let revision = decision.revision.unwrap();
    assert_eq!(
        s.e.head(KB, "main").unwrap().as_deref(),
        Some(revision.as_str())
    );
    assert_eq!(s.nodes("dave", KB), vec!["n1", "n2", "seed"]);
    let result = s.query_version("dave", VERSION, KB).unwrap();
    let entry = result["fact_provenance"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["fact"]["node_id"] == "n1")
        .unwrap()
        .clone();
    let provenance = &entry["provenance"];
    assert_eq!(provenance["writer"], BOT);
    assert_eq!(provenance["on_behalf_of"], json!(["alice"]));
    assert_eq!(provenance["recorded_at_ms"], decided_at);
    assert_eq!(provenance["method"]["kind"], "model");
    assert_eq!(
        provenance["review"],
        json!({"proposal_id": receipt.proposal_id, "decision_id": decision.decision_id,
            "reviewers": ["bob", "carol"], "proposed_at_ms": proposed_at,
            "decided_at_ms": decided_at})
    );
    let typed: weave_contract::provenance::FactProvenance =
        serde_json::from_value(provenance.clone()).unwrap();
    typed.validate().unwrap();
    // The seed was written directly and has no review.
    let seed = result["fact_provenance"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["fact"]["node_id"] == "seed")
        .unwrap();
    assert!(seed["provenance"].get("review").is_none());
    // Programs older than 1.3.0 never see the field.
    let old = s.query_version("dave", "1.2.0", KB).unwrap();
    assert!(old["fact_provenance"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["provenance"].get("review").is_none()));
    // As known before the decision, the facts were not there.
    let mut as_of = s
        .run(
            "dave",
            json!([{"op": "evaluate", "value": {"kind": "as_of", "unix_millis": decided_at - 1,
                "query": {"graph_id": KB}}}]),
        )
        .unwrap();
    let Some(CommandResult::Queried { result }) = as_of.pop() else {
        panic!()
    };
    let ids: Vec<&str> = result.graph.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, vec!["seed"]);
    // A decided proposal takes no more votes.
    assert_eq!(
        code(s.vote("bob", &receipt, ReviewVerdict::Reject)).0,
        "E_REVIEW_DECIDED"
    );
}

#[test]
fn an_approval_of_a_changed_proposal_is_rejected() {
    let mut s = Store::new();
    s.reviewed(KB, &["reviewers"], 2);
    let first = s.propose(BOT, KB, json!([node("draft")])).unwrap();
    s.vote("bob", &first, ReviewVerdict::Approve).unwrap();
    s.tick();
    let second =
        s.e.amend_review(
            &first.proposal_id,
            &changes(json!([node("final")])),
            &host(BOT),
        )
        .unwrap();
    assert_eq!(second.proposal_id, first.proposal_id);
    assert_eq!(second.version, 2);
    assert_ne!(second.digest, first.digest);
    // A vote on the version carol read, which is no longer the proposal.
    assert_eq!(
        code(s.vote("carol", &first, ReviewVerdict::Approve)),
        (
            "E_REVIEW_CHANGED".into(),
            "the proposal differs from the version reviewed".into()
        )
    );
    let forged = ReviewProposalReceipt {
        digest: "0".repeat(64),
        ..second.clone()
    };
    assert_eq!(
        code(s.vote("carol", &forged, ReviewVerdict::Approve)).0,
        "E_REVIEW_CHANGED"
    );
    // bob's approval of version 1 does not count for version 2.
    let pending = s.vote("carol", &second, ReviewVerdict::Approve).unwrap();
    assert_eq!(pending.status, ReviewStatus::Pending);
    assert_eq!(pending.approvals, vec!["carol"]);
    assert!(s.nodes("dave", KB).is_empty());
    let accepted = s.vote("bob", &second, ReviewVerdict::Approve).unwrap();
    assert_eq!(accepted.status, ReviewStatus::Accepted);
    assert_eq!(accepted.decision.unwrap().version, 2);
    assert_eq!(s.nodes("dave", KB), vec!["final"]);
    // Only the proposer amends, and only while pending.
    let other = s.propose(BOT, KB, json!([node("other")])).unwrap();
    s.tick();
    assert_eq!(
        code(s.e.amend_review(
            &other.proposal_id,
            &changes(json!([node("x")])),
            &host("bob")
        ))
        .0,
        "E_FORBIDDEN"
    );
    assert_eq!(
        code(s.e.amend_review(&first.proposal_id, &changes(json!([node("x")])), &host(BOT))).0,
        "E_REVIEW_DECIDED"
    );
}

#[test]
fn a_policy_change_cannot_self_authorize() {
    let mut s = Store::new();
    // Tightening a direct graph is immediate for an Administer holder only.
    s.tick();
    let mode = ReviewMode::Reviewed {
        reviewers: vec!["bob".into(), ADMIN.into()],
        threshold: 1,
    };
    assert_eq!(
        code(s.e.set_review_policy(KB, &mode, 0, &host("alice"))).0,
        "E_FORBIDDEN"
    );
    assert_eq!(
        code(s.e.set_review_policy(KB, &mode, 7, &host(ADMIN))).0,
        "E_CONFLICT"
    );
    let ReviewPolicyChange::Applied { policy } =
        s.e.set_review_policy(KB, &mode, 0, &host(ADMIN)).unwrap()
    else {
        panic!()
    };
    assert_eq!(policy.revision, 1);
    // Replacing the reviewers is a proposal for the current reviewers.
    let replacement = ReviewMode::Reviewed {
        reviewers: vec!["erin".into()],
        threshold: 1,
    };
    s.tick();
    let ReviewPolicyChange::Proposed { proposal } =
        s.e.set_review_policy(KB, &replacement, 1, &host(ADMIN))
            .unwrap()
    else {
        panic!()
    };
    // The proposed reviewer has no say in its own installation.
    assert_eq!(
        code(s.vote("erin", &proposal, ReviewVerdict::Approve)).0,
        "E_UNAVAILABLE"
    );
    // The proposer is a current reviewer, but its own approval never counts.
    assert_eq!(
        code(s.vote(ADMIN, &proposal, ReviewVerdict::Approve)),
        (
            "E_FORBIDDEN".into(),
            "the principal may not review this proposal".into()
        )
    );
    assert_eq!(s.e.review_policy(KB, &host("dave")).unwrap().revision, 1);
    // A second, competing change of the same revision.
    s.tick();
    let ReviewPolicyChange::Proposed { proposal: rival } =
        s.e.set_review_policy(KB, &ReviewMode::Direct, 1, &host(ADMIN))
            .unwrap()
    else {
        panic!()
    };
    // The current quorum (bob) installs the replacement.
    let accepted = s.vote("bob", &proposal, ReviewVerdict::Approve).unwrap();
    assert_eq!(accepted.status, ReviewStatus::Accepted);
    let current = s.e.review_policy(KB, &host("dave")).unwrap();
    assert_eq!(current.revision, 2);
    assert_eq!(current.mode, replacement);
    // bob is no longer a reviewer of the graph; the rival is stale for erin.
    assert_eq!(
        code(s.vote("bob", &rival, ReviewVerdict::Approve)).0,
        "E_UNAVAILABLE"
    );
    assert_eq!(
        code(s.vote("erin", &rival, ReviewVerdict::Approve)).0,
        "E_REVIEW_STALE"
    );
    // Turning review off needs the current quorum too.
    s.tick();
    let ReviewPolicyChange::Proposed { proposal: off } =
        s.e.set_review_policy(KB, &ReviewMode::Direct, 2, &host(ADMIN))
            .unwrap()
    else {
        panic!()
    };
    assert_eq!(
        code(s.write("alice", KB, json!([node("n")]))).0,
        "E_REVIEW_REQUIRED"
    );
    s.vote("erin", &off, ReviewVerdict::Approve).unwrap();
    assert_eq!(
        s.e.review_policy(KB, &host("dave")).unwrap().mode,
        ReviewMode::Direct
    );
    s.write("alice", KB, json!([node("n")])).unwrap();
    assert_eq!(s.nodes("dave", KB), vec!["n"]);
    // Agents never review; unknown reviewers are unavailable.
    s.tick();
    let agent = ReviewMode::Reviewed {
        reviewers: vec![BOT.into()],
        threshold: 1,
    };
    assert_eq!(
        code(s.e.set_review_policy(KB, &agent, 3, &host(ADMIN))).0,
        "E_REVIEW_POLICY"
    );
    let ghost = ReviewMode::Reviewed {
        reviewers: vec!["ghost".into()],
        threshold: 1,
    };
    assert_eq!(
        code(s.e.set_review_policy(KB, &ghost, 3, &host(ADMIN))).0,
        "E_UNAVAILABLE"
    );
}

#[test]
fn reviewers_see_proposals_only_within_their_read_scope() {
    let s = Store::new();
    s.reviewed(KB, &["reviewers"], 1);
    let mut open = node("open");
    open["readers"] = json!([]);
    let public = s.propose(BOT, KB, json!([open])).unwrap();
    let mut literal = node("carol-only");
    literal["readers"] = json!(["alice", "carol"]);
    let restricted = s.propose(BOT, KB, json!([literal])).unwrap();
    let mut grouped = node("ops-only");
    grouped["reader_groups"] = json!(["ops"]);
    let ops = s.propose(BOT, KB, json!([grouped])).unwrap();
    assert_eq!(s.list("bob", KB), vec![public.proposal_id.clone()]);
    assert_eq!(
        s.list("carol", KB),
        vec![
            public.proposal_id.clone(),
            restricted.proposal_id.clone(),
            ops.proposal_id.clone()
        ]
    );
    // The proposer and its owner see all of them; a plain reader none.
    assert_eq!(s.list(BOT, KB).len(), 3);
    assert_eq!(s.list("alice", KB).len(), 3);
    assert!(s.list("dave", KB).is_empty());
    // Outside the scope, a proposal is exactly a missing one.
    let missing = code(s.e.review_proposal("proposal:missing", &host("bob")));
    for hidden in [&restricted, &ops] {
        assert_eq!(
            code(s.e.review_proposal(&hidden.proposal_id, &host("bob"))),
            missing
        );
        assert_eq!(code(s.vote("bob", hidden, ReviewVerdict::Approve)), missing);
        let after = s.e.list_review_proposals(
            &ReviewProposalFilter {
                graph_id: KB.into(),
                after: Some(hidden.proposal_id.clone()),
                ..Default::default()
            },
            &host("bob"),
        );
        assert_eq!(code(after), missing);
    }
    // A reviewer within the scope decides it.
    let accepted = s
        .vote("carol", &restricted, ReviewVerdict::Approve)
        .unwrap();
    assert_eq!(accepted.status, ReviewStatus::Accepted);
    // A correction of a fact bob cannot read is outside bob's scope too.
    let mut corrected = node("carol-only");
    corrected["readers"] = json!(["alice", "carol"]);
    corrected["properties"] = json!({"x": 1});
    s.tick();
    let correction =
        s.e.propose_review(
            KB,
            "main",
            &serde_json::from_value::<Vec<FactChange>>(
                json!([{"change": "correct", "fact": corrected}]),
            )
            .unwrap(),
            &host(BOT),
        )
        .unwrap();
    assert!(!s.list("bob", KB).contains(&correction.proposal_id));
    assert!(s.list("carol", KB).contains(&correction.proposal_id));
    // An edge to a node bob cannot read is outside bob's scope.
    let edge = s
        .propose(
            BOT,
            KB,
            json!([node("bridge"), {"kind": "edge", "id": "eb", "predicate": "p",
                "from": "bridge", "to": "carol-only", "valid_time": {"start": 0}}]),
        )
        .unwrap();
    assert!(!s.list("bob", KB).contains(&edge.proposal_id));
}

/// Paired hidden-input test for the review read path: two stores that differ
/// only in proposals a reviewer may not see give the reviewer identical
/// listings, proposals, errors and votes.
#[test]
fn hidden_proposals_never_change_a_reviewers_outcomes() {
    fn world(hidden: bool) -> (Store, Vec<String>) {
        let s = Store::new();
        s.reg(
            ADMIN,
            json!([{"change": "grant", "graph_id": "vault", "subject": "alice", "permission": "write"}]),
        )
        .unwrap();
        s.reviewed(KB, &["reviewers"], 2);
        s.reviewed("vault", &["reviewers"], 1);
        // Hidden proposals are made at the same trusted times as the visible
        // ones, so only the hidden input differs between the stores.
        let propose = |who: &str, graph: &str, fact: Value| {
            s.e.propose_review(graph, "main", &changes(json!([fact])), &host(who))
                .unwrap()
        };
        let mut visible = vec![];
        for step in 0..4 {
            s.clock.set(1_000 + 100 * step);
            visible.push(propose(BOT, KB, node(&format!("v{step}"))).proposal_id);
            if hidden {
                let mut secret = node(&format!("h{step}"));
                secret["readers"] = json!(["alice", "carol"]);
                propose(BOT, KB, secret);
                let mut grouped = node(&format!("g{step}"));
                grouped["reader_groups"] = json!(["ops"]);
                let r = propose(BOT, KB, grouped);
                if step % 2 == 0 {
                    s.e.vote_review(
                        &ReviewVote {
                            proposal_id: r.proposal_id,
                            digest: r.digest,
                            verdict: ReviewVerdict::Reject,
                            note: None,
                        },
                        &host("carol"),
                    )
                    .unwrap();
                }
                propose("alice", "vault", node(&format!("x{step}")));
            }
        }
        s.clock.set(2_000);
        (s, visible)
    }
    let (a, ids_a) = world(false);
    let (b, ids_b) = world(true);
    // Proposal IDs and digests are random per store: compare up to them.
    let normalize = |text: String, ids: &[String], store: &Store| -> String {
        let mut text = text;
        for (i, id) in ids.iter().enumerate() {
            let digest = store.e.review_proposal(id, &host(BOT)).unwrap().digest;
            text = text
                .replace(id, &format!("P{i}"))
                .replace(&digest, &format!("D{i}"));
        }
        text
    };
    let outcome = |s: &Store, ids: &[String], f: &dyn Fn(&Store) -> String| normalize(f(s), ids, s);
    let checks: Vec<Check> = vec![
        Box::new(|s| {
            format!(
                "{:?}",
                s.e.list_review_proposals(
                    &ReviewProposalFilter {
                        graph_id: KB.into(),
                        ..Default::default()
                    },
                    &host("bob")
                )
            )
        }),
        Box::new(|s| {
            format!(
                "{:?}",
                s.e.list_review_proposals(
                    &ReviewProposalFilter {
                        graph_id: KB.into(),
                        limit: Some(2),
                        ..Default::default()
                    },
                    &host("bob")
                )
            )
        }),
        Box::new(|s| {
            format!(
                "{:?}",
                s.e.list_review_proposals(
                    &ReviewProposalFilter {
                        graph_id: KB.into(),
                        status: Some(ReviewStatus::Rejected),
                        ..Default::default()
                    },
                    &host("bob")
                )
            )
        }),
        Box::new(|s| {
            format!(
                "{:?}",
                s.e.list_review_proposals(
                    &ReviewProposalFilter {
                        graph_id: "vault".into(),
                        ..Default::default()
                    },
                    &host("bob")
                )
            )
        }),
        Box::new(|s| {
            format!(
                "{:?}",
                s.e.list_review_proposals(
                    &ReviewProposalFilter {
                        graph_id: KB.into(),
                        ..Default::default()
                    },
                    &host("dave")
                )
            )
        }),
        Box::new(|s| {
            format!(
                "{:?}",
                s.e.review_proposal("proposal:00000000000000000000000000000000", &host("bob"))
            )
        }),
        Box::new(|s| format!("{:?}", s.e.review_policy("vault", &host("bob")))),
    ];
    for check in &checks {
        assert_eq!(
            outcome(&a, &ids_a, check.as_ref()),
            outcome(&b, &ids_b, check.as_ref())
        );
    }
    // Votes and the published result do not depend on hidden proposals.
    for (s, ids) in [(&a, &ids_a), (&b, &ids_b)] {
        for id in ids {
            let digest = s.e.review_proposal(id, &host("bob")).unwrap().digest;
            for who in ["bob", "carol"] {
                s.tick();
                s.e.vote_review(
                    &ReviewVote {
                        proposal_id: id.clone(),
                        digest: digest.clone(),
                        verdict: ReviewVerdict::Approve,
                        note: None,
                    },
                    &host(who),
                )
                .unwrap();
            }
        }
    }
    let (mut a, mut b) = (a, b);
    assert_eq!(a.nodes("dave", KB), vec!["v0", "v1", "v2", "v3"]);
    assert_eq!(a.nodes("dave", KB), b.nodes("dave", KB));
    // The whole published result, review provenance included, up to the
    // random proposal and decision identities.
    let published = |s: &mut Store, ids: &[String]| -> String {
        let mut text = s.query_version("dave", VERSION, KB).unwrap().to_string();
        for (i, id) in ids.iter().enumerate() {
            let shown = s.e.review_proposal(id, &host("bob")).unwrap();
            let decision = shown.decision.unwrap().decision_id;
            text = text
                .replace(id, &format!("P{i}"))
                .replace(&decision, &format!("X{i}"));
        }
        text
    };
    assert_eq!(published(&mut a, &ids_a), published(&mut b, &ids_b));
    assert_eq!(a.nodes("bob", KB), b.nodes("bob", KB));
}

#[test]
fn rejected_proposals_stay_in_history_and_never_publish() {
    let mut s = Store::new();
    s.reviewed(KB, &["reviewers"], 2);
    let receipt = s.propose(BOT, KB, json!([node("wrong")])).unwrap();
    s.vote("carol", &receipt, ReviewVerdict::Approve).unwrap();
    s.tick();
    let rejected =
        s.e.vote_review(
            &ReviewVote {
                proposal_id: receipt.proposal_id.clone(),
                digest: receipt.digest.clone(),
                verdict: ReviewVerdict::Reject,
                note: Some("the source log says otherwise".into()),
            },
            &host("bob"),
        )
        .unwrap();
    assert_eq!(rejected.status, ReviewStatus::Rejected);
    let decision = rejected.decision.clone().unwrap();
    assert_eq!(decision.reviewers, vec!["bob"]);
    assert_eq!(decision.revision, None);
    assert_eq!(
        decision.note.as_deref(),
        Some("the source log says otherwise")
    );
    // Never visible as an accepted fact, in any read.
    assert!(s.e.head(KB, "main").unwrap().is_none());
    assert!(s.nodes("dave", KB).is_empty());
    assert!(s.nodes("carol", KB).is_empty());
    assert_eq!(
        code(s.vote("carol", &receipt, ReviewVerdict::Approve)).0,
        "E_REVIEW_DECIDED"
    );
    // It stays in history for the proposer, its owner and the reviewers.
    for who in [BOT, "alice", "bob", "carol"] {
        let shown =
            s.e.review_proposal(&receipt.proposal_id, &host(who))
                .unwrap();
        assert_eq!(shown, rejected, "{who}");
        let listed =
            s.e.list_review_proposals(
                &ReviewProposalFilter {
                    graph_id: KB.into(),
                    status: Some(ReviewStatus::Rejected),
                    ..Default::default()
                },
                &host(who),
            )
            .unwrap();
        assert_eq!(listed, vec![rejected.clone()]);
    }
    // A later, accepted proposal publishes only its own facts.
    let good = s.propose(BOT, KB, json!([node("right")])).unwrap();
    s.vote("bob", &good, ReviewVerdict::Approve).unwrap();
    s.vote("carol", &good, ReviewVerdict::Approve).unwrap();
    assert_eq!(s.nodes("dave", KB), vec!["right"]);
    let mut feed = s
        .run(
            "dave",
            json!([{"op": "evaluate", "value": {"kind": "changes", "graph_id": KB,
                "from": {"kind": "as_of", "unix_millis": 0}, "to": {"kind": "head"}}}]),
        )
        .unwrap();
    let text = serde_json::to_string(&feed.pop()).unwrap();
    assert!(text.contains("right") && !text.contains("wrong"), "{text}");
    // A withdrawn proposal is history too.
    let withdrawn = s.propose(BOT, KB, json!([node("draft")])).unwrap();
    s.tick();
    assert_eq!(
        code(s.e.withdraw_review(&withdrawn.proposal_id, &host("bob"))).0,
        "E_FORBIDDEN"
    );
    let shown =
        s.e.withdraw_review(&withdrawn.proposal_id, &host("alice"))
            .unwrap();
    assert_eq!(shown.status, ReviewStatus::Withdrawn);
    assert_eq!(
        code(s.vote("bob", &withdrawn, ReviewVerdict::Approve)).0,
        "E_REVIEW_DECIDED"
    );
}

#[test]
fn proposers_and_agents_never_count_as_their_own_reviewers() {
    let s = Store::new();
    s.reg(
        ADMIN,
        json!([{"change": "add_member", "group": "reviewers", "member": "alice"}]),
    )
    .unwrap();
    s.reviewed(KB, &["reviewers"], 1);
    // A reviewer's own proposal needs someone else.
    let own = s.propose("alice", KB, json!([node("mine")])).unwrap();
    assert_eq!(
        code(s.vote("alice", &own, ReviewVerdict::Approve)).0,
        "E_FORBIDDEN"
    );
    // The agent's owner reviews the agent's work.
    let bot = s.propose(BOT, KB, json!([node("bots")])).unwrap();
    assert_eq!(
        code(s.vote(BOT, &bot, ReviewVerdict::Approve)).0,
        "E_FORBIDDEN"
    );
    let accepted = s.vote("alice", &bot, ReviewVerdict::Approve).unwrap();
    assert_eq!(accepted.status, ReviewStatus::Accepted);
    assert_eq!(accepted.decision.unwrap().reviewers, vec!["alice"]);
}

#[test]
fn stale_proposals_fail_without_recording_the_vote() {
    let s = Store::new();
    s.reviewed(KB, &["reviewers"], 1);
    let first = s.propose(BOT, KB, json!([node("n9")])).unwrap();
    let mut other = node("n9");
    other["properties"] = json!({"different": true});
    let second = s.propose("alice", KB, json!([other])).unwrap();
    s.vote("bob", &first, ReviewVerdict::Approve).unwrap();
    let head = s.e.head(KB, "main").unwrap();
    assert_eq!(
        code(s.vote("bob", &second, ReviewVerdict::Approve)),
        (
            "E_REVIEW_STALE".into(),
            "the proposal no longer applies to the graph".into()
        )
    );
    assert_eq!(s.e.head(KB, "main").unwrap(), head);
    let shown =
        s.e.review_proposal(&second.proposal_id, &host("bob"))
            .unwrap();
    assert_eq!(shown.status, ReviewStatus::Pending);
    assert!(shown.approvals.is_empty());
    // A proposer who lost Write cannot publish through a reviewer either.
    let third = s.propose(BOT, KB, json!([node("late")])).unwrap();
    s.reg(
        ADMIN,
        json!([{"change": "set_delegation", "agent": BOT, "delegation": {
            "scopes": [{"graph_id": KB, "permission": "read"}],
            "exercise": {"kind": "owner_all"}}}]),
    )
    .unwrap();
    assert_eq!(
        code(s.vote("bob", &third, ReviewVerdict::Approve)).0,
        "E_REVIEW_STALE"
    );
    // A proposal must be a valid write by the proposer now, and change
    // something.
    let mut changed = node("n9");
    changed["properties"] = json!({"again": true});
    assert_eq!(
        code(s.propose("alice", KB, json!([changed]))).0,
        "E_CONFLICT"
    );
    assert_eq!(
        code(s.propose("alice", KB, json!([node("n9")]))),
        ("E_FACT_WRITE".into(), "the proposal changes nothing".into())
    );
    assert_eq!(
        code(s.propose("dave", KB, json!([node("d")]))).0,
        "E_FORBIDDEN"
    );
    assert_eq!(
        code(s.propose("dave", "nowhere", json!([node("d")]))).0,
        "E_UNAVAILABLE"
    );
}

#[test]
fn review_needs_the_registry_and_never_uses_writable_graphs() {
    let clock = Arc::new(ManualClock::new(10));
    let e = Engine::memory_with_clock(clock).unwrap();
    let trusted = HostContext::new("host", ["g".to_string()]);
    assert_eq!(
        code(e.review_policy("g", &trusted)).0,
        "E_REGISTRY_DISABLED"
    );
    assert_eq!(
        code(e.propose_review("g", "main", &changes(json!([node("n")])), &trusted)).0,
        "E_REGISTRY_DISABLED"
    );
    // In a registry store, a host grant of the graph is not authority.
    let s = Store::new();
    s.reviewed(KB, &["reviewers"], 1);
    s.tick();
    let granted = HostContext::new("dave", [KB.to_string()]);
    assert_eq!(
        code(s.e.propose_review(KB, "main", &changes(json!([node("n")])), &granted)).0,
        "E_FORBIDDEN"
    );
}
