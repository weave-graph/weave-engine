//! B1 (#38, ADR-0007 section 3): agents owned by a principal, with a narrowed
//! delegated scope. Authority is `can(owner) ∧ scope ∧ S_local(agent)`,
//! evaluated on every request; delegated writes record `on_behalf_of`. The
//! permission matrix covers user, group and agent × Discover, Read, Write and
//! Administer × graph, branch and record.
use serde_json::{json, Value};
use std::sync::Arc;
use weave_contract::{CommandResult, Program, VERSION};
use weave_engine::{Engine, HostContext, ManualClock, RegistryChange, TrustedClock};

const ADMIN: &str = "root";

fn host(principal: &str) -> HostContext {
    HostContext::new(principal, Vec::<String>::new())
}
fn program(version: &str, commands: Value) -> Program {
    serde_json::from_value(json!({"version": version, "commands": commands})).unwrap()
}
type R<T> = weave_engine::Result<T>;

struct Store {
    e: Engine,
    clock: Arc<ManualClock>,
}
impl Store {
    /// Users alice, bob and carol; group team = {alice}.
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
                {"change": "add_principal", "id": "team", "kind": "group"},
                {"change": "add_member", "group": "team", "member": "alice"}
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
    fn grant(&self, graph: &str, subject: &str, permission: &str) {
        self.reg(
            ADMIN,
            json!([{"change": "grant", "graph_id": graph, "subject": subject, "permission": permission}]),
        )
        .unwrap();
    }
    /// An agent of `owner` with the given scopes and exercise.
    fn agent(&self, who: &str, id: &str, owner: &str, delegation: Value) -> R<String> {
        self.reg(
            who,
            json!([{"change": "add_agent", "id": id, "owner": owner, "delegation": delegation}]),
        )
    }
    fn run_version(&mut self, who: &str, version: &str, commands: Value) -> R<Vec<CommandResult>> {
        self.tick();
        self.e.execute(&program(version, commands), &host(who))
    }
    fn run(&mut self, who: &str, commands: Value) -> R<Vec<CommandResult>> {
        self.run_version(who, VERSION, commands)
    }
    fn write_branch(&mut self, who: &str, graph: &str, branch: &str, facts: Value) -> R<String> {
        let changes: Vec<Value> = facts
            .as_array()
            .unwrap()
            .iter()
            .map(|f| json!({"change": "assert", "fact": f}))
            .collect();
        let out = self.run(
            who,
            json!([{"op": "write", "graph_id": graph, "branch_id": branch, "changes": changes}]),
        )?;
        match out.last().unwrap() {
            CommandResult::Committed { revision, .. } | CommandResult::Unchanged { revision } => {
                Ok(revision.clone())
            }
            other => panic!("{other:?}"),
        }
    }
    fn write(&mut self, who: &str, graph: &str, facts: Value) -> R<String> {
        self.write_branch(who, graph, "main", facts)
    }
    fn query(&mut self, who: &str, graph: &str) -> R<Value> {
        let out = self.run(who, json!([{"op": "query", "query": {"graph_id": graph}}]))?;
        Ok(serde_json::to_value(&out[0]).unwrap()["result"].clone())
    }
    fn nodes(&mut self, who: &str, graph: &str) -> Vec<String> {
        ids(&self.query(who, graph).unwrap()["graph"]["nodes"])
    }
}
fn ids(records: &Value) -> Vec<String> {
    let mut out: Vec<String> = records
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_string())
        .collect();
    out.sort();
    out
}
fn node(id: &str, readers: &[&str], groups: &[&str]) -> Value {
    json!({"kind": "node", "id": id, "entity_id": id.to_uppercase(), "space_id": "s",
        "readers": readers, "reader_groups": groups})
}
fn code<T: std::fmt::Debug>(r: R<T>) -> String {
    r.unwrap_err().code
}
fn scope(graph: &str, permission: &str) -> Value {
    json!({"graph_id": graph, "permission": permission})
}
fn owner_all(scopes: Value) -> Value {
    json!({"scopes": scopes, "exercise": {"kind": "owner_all"}})
}

#[test]
fn an_agent_reads_and_writes_only_where_its_owner_and_its_scope_both_allow() {
    let mut s = Store::new();
    s.grant("g", "alice", "write");
    s.grant("h", "alice", "write");
    s.grant("k", "alice", "read");
    for graph in ["g", "h", "k"] {
        s.write(ADMIN, graph, json!([node("x", &[], &[])])).unwrap();
    }
    s.agent(
        "alice",
        "a1",
        "alice",
        owner_all(json!([
            scope("g", "write"),
            scope("h", "read"),
            scope("k", "write")
        ])),
    )
    .unwrap();
    // Both allow Write on g.
    assert_eq!(s.nodes("a1", "g"), ["x"]);
    s.write("a1", "g", json!([node("y", &[], &[])])).unwrap();
    // The scope allows only Read on h.
    assert_eq!(s.nodes("a1", "h"), ["x"]);
    assert_eq!(
        code(s.write("a1", "h", json!([node("y", &[], &[])]))),
        "E_FORBIDDEN"
    );
    // The owner holds only Read on k.
    assert_eq!(
        code(s.write("a1", "k", json!([node("y", &[], &[])]))),
        "E_FORBIDDEN"
    );
    // A graph outside the scope is indistinguishable from a missing one,
    // even though the owner can read it.
    s.grant("z", "alice", "read");
    s.write(ADMIN, "z", json!([node("x", &[], &[])])).unwrap();
    let missing = s.query("a1", "nowhere").unwrap_err();
    let hidden = s.query("a1", "z").unwrap_err();
    assert_eq!(
        (hidden.code, hidden.message),
        (missing.code, missing.message)
    );
    let listed: Vec<String> =
        s.e.list_graphs(&host("a1"))
            .unwrap()
            .into_iter()
            .map(|l| l.graph_id)
            .collect();
    assert_eq!(listed, ["g", "h", "k"]);
}

#[test]
fn delegation_never_clears_record_restrictions_and_exercise_is_bounded_by_the_owner() {
    let mut s = Store::new();
    s.grant("g", "alice", "read");
    s.reg(
        ADMIN,
        json!([
            {"change": "add_principal", "id": "ops", "kind": "group"}
        ]),
    )
    .unwrap();
    s.agent(
        "alice",
        "only-self",
        "alice",
        json!({"scopes": [scope("g", "read")]}),
    )
    .unwrap();
    s.agent(
        "alice",
        "all",
        "alice",
        owner_all(json!([scope("g", "read")])),
    )
    .unwrap();
    s.agent(
        "alice",
        "team-only",
        "alice",
        json!({"scopes": [scope("g", "read")], "exercise": {"kind": "only", "identities": ["team"]}}),
    )
    .unwrap();
    // The agent "all" is put in ops, which its owner is not in.
    s.reg(
        ADMIN,
        json!([{"change": "add_member", "group": "ops", "member": "all"}]),
    )
    .unwrap();
    s.write(
        ADMIN,
        "g",
        json!([
            node("public", &[], &[]),
            node("for-alice", &["alice"], &[]),
            node("for-team", &[], &["team"]),
            node("for-ops", &[], &["ops"]),
            node("for-all", &["all"], &[])
        ]),
    )
    .unwrap();
    assert_eq!(s.nodes("only-self", "g"), ["public"]);
    assert_eq!(s.nodes("team-only", "g"), ["for-team", "public"]);
    // An agent in a group its owner is not in gains nothing from it, but
    // records naming the agent itself are visible to it and to its owner.
    assert_eq!(
        s.nodes("all", "g"),
        ["for-alice", "for-all", "for-team", "public"]
    );
    assert_eq!(
        s.nodes("alice", "g"),
        ["for-alice", "for-all", "for-team", "public"]
    );
    // Exercise follows the owner: when alice leaves team, so do her agents.
    s.reg(
        ADMIN,
        json!([{"change": "remove_member", "group": "team", "member": "alice"}]),
    )
    .unwrap();
    assert_eq!(s.nodes("team-only", "g"), ["public"]);
    assert_eq!(s.nodes("all", "g"), ["for-alice", "for-all", "public"]);
}

#[test]
fn an_agent_never_exceeds_its_owners_current_authority() {
    let mut s = Store::new();
    s.grant("g", "team", "write");
    s.write(ADMIN, "g", json!([node("x", &[], &[])])).unwrap();
    s.agent(
        "alice",
        "a1",
        "alice",
        owner_all(json!([scope("g", "write")])),
    )
    .unwrap();
    s.write("a1", "g", json!([node("y", &[], &[])])).unwrap();
    // The owner loses graph access through her group: the agent loses it on
    // its very next request, though its own delegation is unchanged.
    s.reg(
        ADMIN,
        json!([{"change": "remove_member", "group": "team", "member": "alice"}]),
    )
    .unwrap();
    assert_eq!(code(s.query("a1", "g")), "E_UNAVAILABLE");
    assert_eq!(
        code(s.write("a1", "g", json!([node("z", &[], &[])]))),
        "E_UNAVAILABLE"
    );
    // Restored with the owner's access.
    s.grant("g", "alice", "read");
    assert_eq!(s.nodes("a1", "g"), ["x", "y"]);
    assert_eq!(
        code(s.write("a1", "g", json!([node("z", &[], &[])]))),
        "E_FORBIDDEN"
    );
    // Suspending the owner suspends the agent's authority.
    s.reg(
        ADMIN,
        json!([{"change": "set_status", "id": "alice", "status": "suspended"}]),
    )
    .unwrap();
    assert_eq!(code(s.query("a1", "g")), "E_UNAVAILABLE");
}

#[test]
fn revoking_an_agent_takes_effect_on_its_next_request() {
    let mut s = Store::new();
    s.grant("g", "alice", "write");
    s.write(ADMIN, "g", json!([node("x", &[], &[])])).unwrap();
    s.agent(
        "alice",
        "a1",
        "alice",
        owner_all(json!([scope("g", "write")])),
    )
    .unwrap();
    assert_eq!(s.nodes("a1", "g"), ["x"]);
    // The owner suspends its agent.
    s.reg(
        "alice",
        json!([{"change": "set_status", "id": "a1", "status": "suspended"}]),
    )
    .unwrap();
    assert_eq!(code(s.query("a1", "g")), "E_UNAVAILABLE");
    s.reg(
        "alice",
        json!([{"change": "set_status", "id": "a1", "status": "active"}]),
    )
    .unwrap();
    assert_eq!(s.nodes("a1", "g"), ["x"]);
    // The owner narrows the delegation: effective on the next request.
    s.reg(
        "alice",
        json!([{"change": "set_delegation", "agent": "a1", "delegation": owner_all(json!([scope("g", "read")]))}]),
    )
    .unwrap();
    assert_eq!(
        code(s.write("a1", "g", json!([node("y", &[], &[])]))),
        "E_FORBIDDEN"
    );
    // Retirement is final.
    s.reg(
        "alice",
        json!([{"change": "set_status", "id": "a1", "status": "retired"}]),
    )
    .unwrap();
    assert_eq!(code(s.query("a1", "g")), "E_UNAVAILABLE");
    assert_eq!(
        code(s.reg(
            "alice",
            json!([{"change": "set_status", "id": "a1", "status": "active"}])
        )),
        "E_CONFLICT"
    );
    // A delegation expires on the trusted clock.
    let at = s.now() + 1_000;
    s.agent(
        "alice",
        "a2",
        "alice",
        json!({"scopes": [scope("g", "read")], "expires_at_ms": at}),
    )
    .unwrap();
    assert_eq!(s.nodes("a2", "g"), ["x"]);
    s.clock.set(at);
    assert_eq!(code(s.query("a2", "g")), "E_UNAVAILABLE");
    // Another user cannot revoke someone else's agent.
    assert_eq!(
        code(s.reg(
            "bob",
            json!([{"change": "set_status", "id": "a2", "status": "suspended"}])
        )),
        "E_FORBIDDEN"
    );
}

#[test]
fn delegation_escalation_attempts_fail() {
    let mut s = Store::new();
    s.grant("g", "alice", "administer");
    s.grant("h", "alice", "write");
    s.write(ADMIN, "g", json!([node("x", &[], &[])])).unwrap();
    // Administer is not delegable.
    assert_eq!(
        code(s.agent(
            "alice",
            "a0",
            "alice",
            owner_all(json!([scope("g", "administer")]))
        )),
        "E_FORBIDDEN"
    );
    s.agent(
        "alice",
        "a1",
        "alice",
        json!({"scopes": [scope("g", "write")], "exercise": {"kind": "owner_all"}, "may_delegate": true,
            "expires_at_ms": 1_000_000}),
    )
    .unwrap();
    s.agent(
        "alice",
        "plain",
        "alice",
        owner_all(json!([scope("g", "write")])),
    )
    .unwrap();
    // An agent without may_delegate cannot create agents.
    assert_eq!(
        code(s.agent(
            "plain",
            "p1",
            "plain",
            json!({"scopes": [scope("g", "read")]})
        )),
        "E_FORBIDDEN"
    );
    // A sub-agent cannot exceed its owner's scope, exercise or expiry.
    for delegation in [
        json!({"scopes": [scope("h", "read")], "expires_at_ms": 1_000_000}),
        json!({"scopes": [{"graph_id": "g", "permission": "write", "branches": []}], "expires_at_ms": 2_000_000}),
        json!({"scopes": [scope("g", "read")]}),
        json!({"scopes": [scope("g", "read")], "exercise": {"kind": "only", "identities": ["bob"]}, "expires_at_ms": 1_000}),
    ] {
        assert_eq!(
            code(s.agent("a1", "sub", "a1", delegation.clone())),
            "E_FORBIDDEN",
            "{delegation}"
        );
    }
    s.agent(
        "a1",
        "sub",
        "a1",
        json!({"scopes": [scope("g", "read")], "exercise": {"kind": "only", "identities": ["team"]},
            "expires_at_ms": 1_000_000}),
    )
    .unwrap();
    // An agent cannot change its own delegation or status, its owner's
    // entries, memberships, group administrators or graph ACLs, and cannot
    // register principals.
    for change in [
        json!({"change": "set_delegation", "agent": "a1", "delegation": owner_all(json!([scope("h", "write")]))}),
        json!({"change": "set_status", "id": "a1", "status": "active"}),
        json!({"change": "set_status", "id": "alice", "status": "suspended"}),
        json!({"change": "add_member", "group": "team", "member": "a1"}),
        json!({"change": "remove_member", "group": "team", "member": "alice"}),
        json!({"change": "grant", "graph_id": "g", "subject": "a1", "permission": "administer"}),
        json!({"change": "add_principal", "id": "eve", "kind": "user"}),
        json!({"change": "add_agent", "id": "x", "owner": "alice", "delegation": {"scopes": []}}),
    ] {
        assert_eq!(
            code(s.reg("a1", json!([change.clone()]))),
            "E_FORBIDDEN",
            "{change}"
        );
    }
    // An agent of a graph administrator cannot read the graph's ACL.
    assert_eq!(
        code(s.e.registry_graph_acl("g", &host("a1"))),
        "E_FORBIDDEN"
    );
    // An agent cannot name a group it cannot discover.
    let unknown = code(s.write("a1", "g", json!([node("y", &[], &["nope"])])));
    assert_eq!(unknown, "E_UNAVAILABLE");
    s.reg(
        ADMIN,
        json!([{"change": "add_principal", "id": "ops", "kind": "group"},
        {"change": "add_member", "group": "ops", "member": "a1"}]),
    )
    .unwrap();
    assert_eq!(
        code(s.write("a1", "g", json!([node("y", &[], &["ops"])]))),
        unknown
    );
    // Its own group via its owner works.
    s.write("a1", "g", json!([node("y", &[], &["team"])]))
        .unwrap();
    // The ancestor narrows itself: the sub-agent is re-evaluated on its next
    // request even though the registry kept its delegation.
    assert_eq!(s.nodes("sub", "g"), ["x", "y"]);
    s.reg(
        "alice",
        json!([{"change": "set_delegation", "agent": "a1", "delegation": {"scopes": [scope("h", "write")], "may_delegate": true}}]),
    )
    .unwrap();
    assert_eq!(code(s.query("sub", "g")), "E_UNAVAILABLE");
    // A wrong owner or a group owner is rejected.
    assert_eq!(
        code(s.agent("bob", "b1", "alice", json!({"scopes": []}))),
        "E_FORBIDDEN"
    );
    assert_eq!(
        code(s.agent(ADMIN, "t1", "team", json!({"scopes": []}))),
        "E_REGISTRY"
    );
    // Agents are registered only with add_agent, and IDs are never reused.
    assert_eq!(
        code(s.reg(
            ADMIN,
            json!([{"change": "add_principal", "id": "q", "kind": "agent"}])
        )),
        "E_REGISTRY"
    );
    assert_eq!(
        code(s.agent("alice", "plain", "alice", json!({"scopes": []}))),
        "E_CONFLICT"
    );
}

#[test]
fn ownership_chains_are_bounded_and_sub_agents_record_the_whole_chain() {
    let mut s = Store::new();
    s.grant("g", "alice", "write");
    let delegation = json!({"scopes": [scope("g", "write")], "exercise": {"kind": "owner_all"}, "may_delegate": true});
    let mut owner = "alice".to_string();
    for i in 1..=8 {
        let id = format!("a{i}");
        s.agent(&owner, &id, &owner, delegation.clone()).unwrap();
        owner = id;
    }
    // A ninth link exceeds the 8-link bound of weave-policy chains.
    assert_eq!(
        code(s.agent("a8", "a9", "a8", delegation.clone())),
        "E_BUDGET"
    );
    // A delegated write records the writer and the chain to the root user.
    s.write("a2", "g", json!([node("x", &[], &[])])).unwrap();
    let out = s
        .run(
            "alice",
            json!([{"op": "query", "query": {"graph_id": "g"}}]),
        )
        .unwrap();
    let provenance =
        &serde_json::to_value(&out[0]).unwrap()["result"]["fact_provenance"][0]["provenance"];
    assert_eq!(provenance["writer"], "a2");
    assert_eq!(provenance["on_behalf_of"], json!(["a1", "alice"]));
    // Programs older than 1.1.0 never see the field.
    let old = s
        .run_version(
            "alice",
            "1.0.0",
            json!([{"op": "query", "query": {"graph_id": "g"}}]),
        )
        .unwrap();
    let old = serde_json::to_value(&old[0]).unwrap();
    assert_eq!(
        old["result"]["fact_provenance"][0]["provenance"]["writer"],
        "a2"
    );
    assert!(old["result"]["fact_provenance"][0]["provenance"]
        .get("on_behalf_of")
        .is_none());
    // A user's own write records no delegation.
    s.write("alice", "g", json!([node("y", &[], &[])])).unwrap();
    let out = s
        .run(
            "alice",
            json!([{"op": "query", "query": {"graph_id": "g"}}]),
        )
        .unwrap();
    let entries = serde_json::to_value(&out[0]).unwrap()["result"]["fact_provenance"].clone();
    let alice: Vec<&Value> = entries
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["provenance"]["writer"] == "alice")
        .collect();
    assert_eq!(alice.len(), 1);
    assert!(alice[0]["provenance"].get("on_behalf_of").is_none());
    // Agents write through fact writes only, so delegation is always recorded.
    assert_eq!(
        code(s.run(
            "a1",
            json!([{"op": "commit", "graph_id": "g", "branch_id": "snap", "expected_head": null,
            "data": {"nodes": []}}])
        )),
        "E_FORBIDDEN"
    );
}

#[test]
fn owners_audit_their_agents_and_others_cannot() {
    let mut s = Store::new();
    s.grant("g", "alice", "read");
    s.agent(
        "alice",
        "a1",
        "alice",
        owner_all(json!([scope("g", "read")])),
    )
    .unwrap();
    let record = s.e.registry_principal("a1", &host("alice")).unwrap();
    assert_eq!(record.owner.as_deref(), Some("alice"));
    assert_eq!(record.delegation.unwrap().scopes.len(), 1);
    assert_eq!(
        s.e.registry_agents("alice", &host("alice")).unwrap(),
        ["a1"]
    );
    assert_eq!(s.e.registry_principal("a1", &host("a1")).unwrap().id, "a1");
    assert_eq!(
        code(s.e.registry_principal("a1", &host("bob"))),
        "E_UNAVAILABLE"
    );
    assert_eq!(
        code(s.e.registry_agents("alice", &host("bob"))),
        "E_UNAVAILABLE"
    );
    // The registry graph shows the owner its agents' records.
    let mine = s.nodes("alice", "weave:registry");
    assert!(mine.contains(&"a1".to_string()), "{mine:?}");
    assert!(!s.nodes("bob", "weave:registry").contains(&"a1".to_string()));
}

#[test]
fn hidden_owner_access_changes_agent_output_only_by_removing_data() {
    // ADR-0007 section 10, item 5: pairs that differ only in the owner's
    // access give the agent the same output, or strictly less.
    let outputs = |owner_in_team: bool| {
        let mut s = Store::new();
        if !owner_in_team {
            s.reg(
                ADMIN,
                json!([{"change": "remove_member", "group": "team", "member": "alice"}]),
            )
            .unwrap();
        }
        s.grant("g", "alice", "read");
        s.agent(
            "alice",
            "a1",
            "alice",
            owner_all(json!([scope("g", "read")])),
        )
        .unwrap();
        s.write(
            ADMIN,
            "g",
            json!([node("public", &[], &[]), node("for-team", &[], &["team"])]),
        )
        .unwrap();
        s.nodes("a1", "g")
    };
    let wide = outputs(true);
    let narrow = outputs(false);
    assert_eq!(wide, ["for-team", "public"]);
    assert_eq!(narrow, ["public"]);
    assert!(narrow.iter().all(|n| wide.contains(n)));
}

/// The permission matrix: user (direct entry), group member and agent, at
/// each permission level, over graph, branch and record operations.
#[test]
fn permission_matrix_user_group_agent_by_permission_by_graph_branch_record() {
    let levels = [
        None,
        Some("discover"),
        Some("read"),
        Some("write"),
        Some("administer"),
    ];
    let rank = |l: Option<&str>| match l {
        None => 0,
        Some("discover") => 1,
        Some("read") => 2,
        Some("write") => 3,
        _ => 4,
    };
    for level in levels {
        let mut s = Store::new();
        s.reg(
            ADMIN,
            json!([{"change": "add_member", "group": "team", "member": "bob"},
            {"change": "remove_member", "group": "team", "member": "alice"}]),
        )
        .unwrap();
        // Subjects: alice by a direct entry, bob through team, agent ag of carol.
        if let Some(level) = level {
            s.grant("m", "alice", level);
            s.grant("m", "team", level);
            s.grant("m", "carol", level);
        }
        s.agent(
            "carol",
            "ag",
            "carol",
            owner_all(json!([scope("m", "write")])),
        )
        .unwrap();
        // Graph m: main is protected with branch entries for every subject;
        // locked is protected with none.
        s.reg(ADMIN, json!([
            {"change": "protect_branch", "graph_id": "m", "branch_id": "main"},
            {"change": "protect_branch", "graph_id": "m", "branch_id": "locked"},
            {"change": "grant_branch_write", "graph_id": "m", "branch_id": "main", "subject": "alice"},
            {"change": "grant_branch_write", "graph_id": "m", "branch_id": "main", "subject": "team"},
            {"change": "grant_branch_write", "graph_id": "m", "branch_id": "main", "subject": "carol"}
        ]))
        .unwrap();
        s.write(
            ADMIN,
            "m",
            json!([
                node("public", &[], &[]),
                node("for-alice", &["alice"], &[]),
                node("for-team", &[], &["team"]),
                node("for-carol", &["carol"], &[]),
                node("for-nobody", &["dave"], &[])
            ]),
        )
        .unwrap();
        for (who, own_record, kind) in [
            ("alice", "for-alice", "user"),
            ("bob", "for-team", "group"),
            ("ag", "for-carol", "agent"),
        ] {
            let held = rank(level);
            // Administer is not delegable: an agent holds at most Write.
            let e = if kind == "agent" { held.min(3) } else { held };
            let ctx = format!("{kind} at {level:?}");
            // Graph: Discover.
            let listed =
                s.e.list_graphs(&host(who))
                    .unwrap()
                    .iter()
                    .any(|l| l.graph_id == "m");
            assert_eq!(listed, e >= 1, "{ctx}: listing");
            // Graph: Read, and record restrictions inside it.
            match s.query(who, "m") {
                Ok(result) => {
                    assert!(e >= 2, "{ctx}: read");
                    assert_eq!(
                        ids(&result["graph"]["nodes"]),
                        {
                            let mut v = vec![own_record.to_string(), "public".to_string()];
                            v.sort();
                            v
                        },
                        "{ctx}: records"
                    );
                }
                Err(error) => {
                    let expected = if e == 0 {
                        "E_UNAVAILABLE"
                    } else {
                        "E_FORBIDDEN"
                    };
                    assert!(e < 2, "{ctx}: read refused");
                    assert_eq!(error.code, expected, "{ctx}: read code");
                }
            }
            // Graph: Write on an unprotected branch.
            let draft = s.write_branch(
                who,
                "m",
                &format!("draft-{who}"),
                json!([node("w", &[], &[])]),
            );
            match draft {
                Ok(_) => assert!(e >= 3, "{ctx}: write"),
                Err(error) => {
                    assert!(e < 3, "{ctx}: write refused");
                    let expected = if e == 0 {
                        "E_UNAVAILABLE"
                    } else {
                        "E_FORBIDDEN"
                    };
                    assert_eq!(error.code, expected, "{ctx}: write code");
                }
            }
            // Branch: a protected branch with an entry needs Read; one without
            // an entry needs the holder's own Administer.
            let main = s.write_branch(
                who,
                "m",
                "main",
                json!([node(&format!("m-{who}"), &["dave"], &[])]),
            );
            assert_eq!(
                main.is_ok(),
                e >= 2,
                "{ctx}: protected branch with entry: {main:?}"
            );
            let locked = s.write_branch(who, "m", "locked", json!([node("l", &[], &[])]));
            assert_eq!(
                locked.is_ok(),
                kind != "agent" && e == 4,
                "{ctx}: protected branch without entry"
            );
            // Administer: the ACL.
            let acl = s.e.registry_graph_acl("m", &host(who));
            match acl {
                Ok(_) => assert_eq!(e, 4, "{ctx}: administer"),
                Err(error) => {
                    assert!(e < 4);
                    let expected = if e == 0 {
                        "E_UNAVAILABLE"
                    } else {
                        "E_FORBIDDEN"
                    };
                    assert_eq!(error.code, expected, "{ctx}: administer code");
                }
            }
            let edit = s.reg(
                who,
                json!([{"change": "grant", "graph_id": "m", "subject": "dave-x", "permission": "read"}]),
            );
            assert!(edit.is_err(), "{ctx}: unknown subject or no authority");
            let edit = s.reg(
                who,
                json!([{"change": "grant", "graph_id": "m", "subject": "carol", "permission": level.unwrap_or("discover")}]),
            );
            assert_eq!(
                edit.is_ok(),
                e == 4 && kind != "agent",
                "{ctx}: ACL edit {edit:?}"
            );
        }
    }
}

#[test]
fn local_stores_and_users_are_unaffected_by_agent_support() {
    // A user's identity is unchanged when it owns no agent.
    let mut s = Store::new();
    s.grant("g", "alice", "read");
    s.write(ADMIN, "g", json!([node("a", &["alice"], &[])]))
        .unwrap();
    assert_eq!(s.nodes("alice", "g"), ["a"]);
    // In a store without a registry, an "agent" is just a literal principal.
    let clock = Arc::new(ManualClock::new(5));
    let mut local = Engine::memory_with_clock(clock).unwrap();
    let h = HostContext::new("a1", ["g".to_string()]);
    local
        .execute(
            &program(VERSION, json!([{"op": "write", "graph_id": "g", "changes": [{"change": "assert", "fact": node("a", &[], &[])}]}])),
            &h,
        )
        .unwrap();
    let out = local
        .execute(
            &program(
                VERSION,
                json!([{"op": "query", "query": {"graph_id": "g"}}]),
            ),
            &h,
        )
        .unwrap();
    let provenance =
        &serde_json::to_value(&out[0]).unwrap()["result"]["fact_provenance"][0]["provenance"];
    assert!(provenance.get("on_behalf_of").is_none());
}
