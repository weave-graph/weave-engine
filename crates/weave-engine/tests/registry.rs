//! B1 (#38, ADR-0007): the principal registry, group readers, graph ACLs,
//! protected branches, Discover and revocation, through the public engine
//! interface.
use rusqlite::Connection;
use serde_json::{json, Value};
use std::sync::Arc;
use weave_contract::{CommandResult, Program, VERSION};
use weave_engine::{Engine, HostContext, ManualClock, RegistryChange, ADMINISTRATORS};

const ADMIN: &str = "root";

fn host(principal: &str) -> HostContext {
    HostContext::new(principal, Vec::<String>::new())
}
fn program(commands: Value) -> Program {
    serde_json::from_value(json!({"version": VERSION, "commands": commands})).unwrap()
}
struct Store {
    e: Engine,
    clock: Arc<ManualClock>,
    path: Option<std::path::PathBuf>,
    _dir: Option<tempfile::TempDir>,
}
impl Store {
    fn new() -> Self {
        let clock = Arc::new(ManualClock::new(10));
        Self {
            e: Engine::memory_with_clock(clock.clone()).unwrap(),
            clock,
            path: None,
            _dir: None,
        }
    }
    fn on_disk() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.db");
        let clock = Arc::new(ManualClock::new(10));
        Self {
            e: Engine::open_with_clock(&path, clock.clone()).unwrap(),
            clock,
            path: Some(path),
            _dir: Some(dir),
        }
    }
    /// A store with the registry on, users alice, bob and carol, and group team.
    fn registry() -> Self {
        let mut s = Self::new();
        s.enable();
        s
    }
    fn enable(&mut self) {
        self.e.enable_registry(&host(ADMIN)).unwrap();
        self.reg(
            ADMIN,
            json!([
                {"change": "add_principal", "id": "alice", "kind": "user"},
                {"change": "add_principal", "id": "bob", "kind": "user"},
                {"change": "add_principal", "id": "carol", "kind": "user"},
                {"change": "add_principal", "id": "team", "kind": "group", "display_name": "Team"},
                {"change": "add_member", "group": "team", "member": "alice"}
            ]),
        )
        .unwrap();
    }
    fn tick(&self) {
        self.clock.set(self.clock_now() + 10);
    }
    fn clock_now(&self) -> i64 {
        use weave_engine::TrustedClock;
        self.clock.unix_millis().unwrap()
    }
    fn reg(&self, who: &str, changes: Value) -> weave_engine::Result<String> {
        self.tick();
        let changes: Vec<RegistryChange> = serde_json::from_value(changes).unwrap();
        self.e.apply_registry(&changes, &host(who))
    }
    fn run(&mut self, who: &str, commands: Value) -> weave_engine::Result<Vec<CommandResult>> {
        self.tick();
        self.e.execute(&program(commands), &host(who))
    }
    fn run_as(
        &mut self,
        h: &HostContext,
        commands: Value,
    ) -> weave_engine::Result<Vec<CommandResult>> {
        self.tick();
        self.e.execute(&program(commands), h)
    }
    fn write(&mut self, who: &str, graph: &str, facts: Value) -> weave_engine::Result<String> {
        let changes: Vec<Value> = facts
            .as_array()
            .unwrap()
            .iter()
            .map(|f| json!({"change": "assert", "fact": f}))
            .collect();
        let out = self.run(
            who,
            json!([{"op": "write", "graph_id": graph, "changes": changes}]),
        )?;
        Ok(revision(&out))
    }
    fn query(&mut self, who: &str, graph: &str) -> weave_engine::Result<Value> {
        let out = self.run(who, json!([{"op": "query", "query": {"graph_id": graph}}]))?;
        Ok(result(out))
    }
    fn nodes(&mut self, who: &str, graph: &str) -> Vec<String> {
        ids(&self.query(who, graph).unwrap()["graph"]["nodes"])
    }
    fn grant(&self, graph: &str, subject: &str, permission: &str) {
        self.reg(
            ADMIN,
            json!([{"change": "grant", "graph_id": graph, "subject": subject, "permission": permission}]),
        )
        .unwrap();
    }
}
fn revision(out: &[CommandResult]) -> String {
    match out.last().unwrap() {
        CommandResult::Committed { revision, .. } | CommandResult::Unchanged { revision } => {
            revision.clone()
        }
        other => panic!("not a write: {other:?}"),
    }
}
fn result(out: Vec<CommandResult>) -> Value {
    serde_json::to_value(&out[0]).unwrap()["result"].clone()
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
fn error(r: weave_engine::Result<impl std::fmt::Debug>) -> (String, String) {
    let e = r.unwrap_err();
    (e.code, e.message)
}
/// A graph `g` everyone in the test can read, written by root.
fn shared_graph(s: &mut Store) {
    s.grant("g", "alice", "read");
    s.grant("g", "bob", "read");
    s.grant("g", "carol", "read");
    s.write(
        ADMIN,
        "g",
        json!([
            node("public", &[], &[]),
            node("for-team", &[], &["team"]),
            node("for-bob", &["bob"], &[]),
            node("bob-or-team", &["bob"], &["team"]),
            node("team-literal", &["team"], &[])
        ]),
    )
    .unwrap();
}

#[test]
fn group_readers_resolve_through_membership_and_groups_never_match_as_literals() {
    let mut s = Store::registry();
    shared_graph(&mut s);
    assert_eq!(s.nodes("alice", "g"), ["bob-or-team", "for-team", "public"]);
    assert_eq!(s.nodes("bob", "g"), ["bob-or-team", "for-bob", "public"]);
    assert_eq!(s.nodes("carol", "g"), ["public"]);
    // A group ID in `readers` is a literal that matches no request principal,
    // and a group can never be a request principal.
    assert!(!s.nodes(ADMIN, "g").contains(&"team-literal".to_string()));
    assert_eq!(error(s.query("team", "g")).0, "E_UNAVAILABLE");
}

#[test]
fn nested_groups_resolve_transitively_through_active_groups_only() {
    let mut s = Store::registry();
    s.reg(
        ADMIN,
        json!([
            {"change": "add_principal", "id": "org", "kind": "group"},
            {"change": "add_member", "group": "org", "member": "team"}
        ]),
    )
    .unwrap();
    s.grant("g", "alice", "read");
    s.write(ADMIN, "g", json!([node("org-only", &[], &["org"])]))
        .unwrap();
    assert_eq!(s.nodes("alice", "g"), ["org-only"]);
    // Suspending the intermediate group removes it, and what it reaches, from closures.
    s.reg(
        ADMIN,
        json!([{"change": "set_status", "id": "team", "status": "suspended"}]),
    )
    .unwrap();
    assert!(s.nodes("alice", "g").is_empty());
    s.reg(
        ADMIN,
        json!([{"change": "set_status", "id": "team", "status": "active"}]),
    )
    .unwrap();
    assert_eq!(s.nodes("alice", "g"), ["org-only"]);
}

#[test]
fn removing_a_member_hides_group_facts_on_the_next_query_without_rewriting_revisions() {
    let mut s = Store::on_disk();
    s.enable();
    shared_graph(&mut s);
    let before_rows = data_rows(s.path.as_ref().unwrap());
    let first = s.query("alice", "g").unwrap();
    assert!(ids(&first["graph"]["nodes"]).contains(&"for-team".to_string()));
    let pinned = first["snapshots"]["g"].as_str().unwrap().to_string();
    s.reg(
        ADMIN,
        json!([{"change": "remove_member", "group": "team", "member": "alice"}]),
    )
    .unwrap();
    // The very next request no longer sees group-restricted facts.
    assert_eq!(s.nodes("alice", "g"), ["public"]);
    // History and pinned revisions are read under current membership.
    let at_pin = s
        .run(
            "alice",
            json!([{"op": "query", "query": {"graph_id": "g", "revision": pinned}}]),
        )
        .unwrap();
    assert_eq!(ids(&result(at_pin)["graph"]["nodes"]), ["public"]);
    let as_of = s
        .run(
            "alice",
            json!([{"op": "evaluate", "value": {"kind": "as_of", "query": {"graph_id": "g"}, "unix_millis": s.clock_now()}}]),
        )
        .unwrap();
    assert_eq!(ids(&result(as_of)["graph"]["nodes"]), ["public"]);
    // No stored data revision, log or index row changed: only registry facts were added.
    assert_eq!(data_rows(s.path.as_ref().unwrap()), before_rows);
    // Re-adding restores access, including history (joining grants the group's history).
    s.reg(
        ADMIN,
        json!([{"change": "add_member", "group": "team", "member": "alice"}]),
    )
    .unwrap();
    let at_pin = s
        .run(
            "alice",
            json!([{"op": "query", "query": {"graph_id": "g", "revision": pinned}}]),
        )
        .unwrap();
    assert_eq!(
        ids(&result(at_pin)["graph"]["nodes"]),
        ["bob-or-team", "for-team", "public"]
    );
}

/// Every revision, fact-log and fact-index row outside the registry graph.
fn data_rows(path: &std::path::Path) -> Vec<String> {
    let db = Connection::open(path).unwrap();
    let mut out = vec![];
    for sql in [
        "SELECT revision,graph_id,branch_id,parent,recorded_at,data FROM revisions WHERE graph_id<>'weave:registry' ORDER BY revision",
        "SELECT r.revision,r.writer,r.on_behalf_of,r.recorded_at_ms FROM fact_revisions r WHERE r.graph_id<>'weave:registry' ORDER BY r.revision",
        "SELECT l.position,l.kind,l.fact_id,l.body,l.writer FROM fact_log l JOIN fact_revisions r ON r.line=l.line WHERE r.graph_id<>'weave:registry' ORDER BY l.position",
    ] {
        let mut statement = db.prepare(sql).unwrap();
        let columns = statement.column_count();
        let rows = statement
            .query_map([], |r| {
                Ok((0..columns)
                    .map(|i| format!("{:?}", r.get::<_, rusqlite::types::Value>(i).unwrap()))
                    .collect::<Vec<_>>()
                    .join("|"))
            })
            .unwrap();
        out.extend(rows.map(Result::unwrap));
    }
    out
}

#[test]
fn group_nesting_depth_cycles_and_closure_size_are_rejected_at_write_time() {
    let s = Store::registry();
    let groups: Vec<Value> = (1..=5)
        .map(|i| json!({"change": "add_principal", "id": format!("g{i}"), "kind": "group"}))
        .collect();
    s.reg(ADMIN, Value::Array(groups)).unwrap();
    // g1 ⊂ g2 ⊂ g3 ⊂ g4 is four deep, the bound.
    for i in 1..4 {
        s.reg(
            ADMIN,
            json!([{"change": "add_member", "group": format!("g{}", i + 1), "member": format!("g{i}")}]),
        )
        .unwrap();
    }
    assert_eq!(
        error(s.reg(
            ADMIN,
            json!([{"change": "add_member", "group": "g5", "member": "g4"}])
        ))
        .0,
        "E_BUDGET"
    );
    // A user below the chain does not make it deeper, but a fifth level above does.
    s.reg(
        ADMIN,
        json!([{"change": "add_member", "group": "g1", "member": "alice"}]),
    )
    .unwrap();
    // Cycles are rejected.
    for (group, member) in [("g1", "g4"), ("g1", "g1")] {
        assert_eq!(
            error(s.reg(
                ADMIN,
                json!([{"change": "add_member", "group": group, "member": member}])
            ))
            .0,
            "E_BUDGET",
            "{group} <- {member}"
        );
    }
    // Closure size: a principal may be in at most 1024 groups.
    for batch in 0..2 {
        let adds: Vec<Value> = (0..600)
            .map(|i| json!({"change": "add_principal", "id": format!("w{}", batch * 600 + i), "kind": "group"}))
            .collect();
        s.reg(ADMIN, Value::Array(adds)).unwrap();
    }
    let joins: Vec<Value> = (0..1000)
        .map(|i| json!({"change": "add_member", "group": format!("w{i}"), "member": "bob"}))
        .collect();
    s.reg(ADMIN, Value::Array(joins)).unwrap();
    let more: Vec<Value> = (1000..1030)
        .map(|i| json!({"change": "add_member", "group": format!("w{i}"), "member": "bob"}))
        .collect();
    assert_eq!(error(s.reg(ADMIN, Value::Array(more))).0, "E_BUDGET");
    // The rejected write changed nothing: bob is still in exactly 1000 groups.
    assert_eq!(
        s.e.registry_memberships("bob", &host(ADMIN)).unwrap().len(),
        1000
    );
}

#[test]
fn graphs_without_discover_are_indistinguishable_from_nonexistent_graphs() {
    let mut s = Store::registry();
    s.write(ADMIN, "hidden", json!([node("x", &[], &[])]))
        .unwrap();
    s.grant("seen", "alice", "discover");
    s.write(ADMIN, "seen", json!([node("y", &[], &[])]))
        .unwrap();
    let missing = error(s.query("alice", "nowhere"));
    assert_eq!(missing.0, "E_UNAVAILABLE");
    assert_eq!(error(s.query("alice", "hidden")), missing);
    // Every operation that names the graph gives the nonexistent-graph error.
    for graph in ["hidden", "nowhere"] {
        let ops = [
            json!({"op": "evaluate", "value": {"kind": "expand", "graph_id": graph, "seeds": ["x"], "hops": 1}}),
            json!({"op": "evaluate", "value": {"kind": "changes", "graph_id": graph, "from": {"kind": "as_of", "unix_millis": 0}, "to": {"kind": "head"}}}),
            json!({"op": "evaluate", "value": {"kind": "as_of", "query": {"graph_id": graph}, "unix_millis": 1000}}),
            json!({"op": "evaluate", "value": {"kind": "nearest", "graph_id": graph, "space": {"id": "s", "revision": "1"}, "query": {"kind": "vector", "vector": [1.0]}, "k": 1}}),
            json!({"op": "write", "graph_id": graph, "changes": [{"change": "assert", "fact": node("z", &[], &[])}]}),
        ];
        let codes: Vec<(String, String)> = ops
            .iter()
            .map(|op| error(s.run("alice", json!([op]))))
            .collect();
        if graph == "hidden" {
            let reference: Vec<(String, String)> = ops
                .iter()
                .map(|op| {
                    let text = serde_json::to_string(op)
                        .unwrap()
                        .replace("\"hidden\"", "\"nowhere\"");
                    error(s.run(
                        "alice",
                        json!([serde_json::from_str::<Value>(&text).unwrap()]),
                    ))
                })
                .collect();
            assert_eq!(codes, reference);
        }
        assert!(
            codes.iter().all(|(code, _)| code == "E_UNAVAILABLE"),
            "{codes:?}"
        );
    }
    // Listings omit it and report no total.
    let listing = s.e.list_graphs(&host("alice")).unwrap();
    let names: Vec<&str> = listing.iter().map(|l| l.graph_id.as_str()).collect();
    assert_eq!(names, ["seen"]);
    // Discover without Read: the holder knows the graph exists, so E_FORBIDDEN.
    assert_eq!(error(s.query("alice", "seen")).0, "E_FORBIDDEN");
    // Administrators discover every graph.
    let all: Vec<String> =
        s.e.list_graphs(&host(ADMIN))
            .unwrap()
            .into_iter()
            .map(|l| l.graph_id)
            .collect();
    assert_eq!(all, ["hidden", "seen"]);
}

#[test]
fn graph_acls_order_discover_read_write_administer_and_groups_grant_through_closure() {
    let mut s = Store::registry();
    s.write(ADMIN, "g", json!([node("a", &[], &[])])).unwrap();
    s.grant("g", "team", "write");
    s.grant("g", "bob", "read");
    s.grant("g", "carol", "administer");
    // alice writes through the team group.
    s.write("alice", "g", json!([node("b", &[], &[])])).unwrap();
    // bob reads but cannot write.
    assert_eq!(s.nodes("bob", "g"), ["a", "b"]);
    assert_eq!(
        error(s.write("bob", "g", json!([node("c", &[], &[])]))).0,
        "E_FORBIDDEN"
    );
    // Only Administer edits the ACL.
    assert_eq!(
        error(s.reg(
            "alice",
            json!([{"change": "grant", "graph_id": "g", "subject": "bob", "permission": "write"}])
        ))
        .0,
        "E_FORBIDDEN"
    );
    s.reg(
        "carol",
        json!([{"change": "grant", "graph_id": "g", "subject": "bob", "permission": "write"}]),
    )
    .unwrap();
    s.write("bob", "g", json!([node("c", &[], &[])])).unwrap();
    // A non-administrator cannot see or edit another graph's ACL.
    assert_eq!(
        error(s.e.registry_graph_acl("g", &host("bob"))).0,
        "E_FORBIDDEN"
    );
    assert_eq!(
        error(s.e.registry_graph_acl("other", &host("bob"))).0,
        "E_UNAVAILABLE"
    );
    let acl = s.e.registry_graph_acl("g", &host("carol")).unwrap();
    assert_eq!(acl.grants.len(), 3);
    // Revoking is effective on the next request.
    s.reg(
        "carol",
        json!([{"change": "revoke", "graph_id": "g", "subject": "team"}]),
    )
    .unwrap();
    assert_eq!(error(s.query("alice", "g")).0, "E_UNAVAILABLE");
}

#[test]
fn protected_branches_need_a_branch_entry_or_administer() {
    let mut s = Store::registry();
    s.grant("g", "alice", "write");
    s.grant("g", "bob", "write");
    s.write("alice", "g", json!([node("a", &[], &[])])).unwrap();
    s.reg(
        ADMIN,
        json!([
            {"change": "protect_branch", "graph_id": "g", "branch_id": "main"},
            {"change": "grant_branch_write", "graph_id": "g", "branch_id": "main", "subject": "team"}
        ]),
    )
    .unwrap();
    // alice is in team; bob only holds graph Write, which does not apply.
    s.write("alice", "g", json!([node("b", &[], &[])])).unwrap();
    assert_eq!(
        error(s.write("bob", "g", json!([node("c", &[], &[])]))).0,
        "E_FORBIDDEN"
    );
    // Graph Write still applies to unprotected branches.
    s.run(
        "bob",
        json!([{"op": "write", "graph_id": "g", "branch_id": "draft",
            "changes": [{"change": "assert", "fact": node("c", &[], &[])}]}]),
    )
    .unwrap();
    // Administer writes any branch.
    s.write(ADMIN, "g", json!([node("d", &[], &[])])).unwrap();
    s.reg(
        ADMIN,
        json!([{"change": "unprotect_branch", "graph_id": "g", "branch_id": "main"}]),
    )
    .unwrap();
    s.write("bob", "g", json!([node("c", &[], &[])])).unwrap();
}

#[test]
fn reader_group_writes_fail_closed_with_one_code_for_every_unusable_group() {
    let mut s = Store::registry();
    s.grant("g", "alice", "write");
    s.grant("g", "bob", "write");
    s.reg(
        ADMIN,
        json!([
            {"change": "add_principal", "id": "secret", "kind": "group"},
            {"change": "add_member", "group": "secret", "member": "carol"},
            {"change": "add_principal", "id": "old", "kind": "group"},
            {"change": "add_member", "group": "old", "member": "alice"},
            {"change": "set_status", "id": "old", "status": "retired"}
        ]),
    )
    .unwrap();
    s.write("alice", "g", json!([node("ok", &[], &["team"])]))
        .unwrap();
    // A group the writer is not in, a hidden group, a retired group, a user
    // and an unknown ID all give the same error.
    let reference = error(s.write("alice", "g", json!([node("x", &[], &["nope"])])));
    assert_eq!(reference.0, "E_UNAVAILABLE");
    for group in ["secret", "old", "bob", "nope"] {
        assert_eq!(
            error(s.write("alice", "g", json!([node("x", &[], &[group])]))),
            reference,
            "{group}"
        );
    }
    assert_eq!(
        error(s.write("bob", "g", json!([node("x", &[], &["team"])]))),
        reference
    );
    // Snapshot commits validate the same way.
    assert_eq!(
        error(s.run(
            "alice",
            json!([{"op": "commit", "graph_id": "g", "branch_id": "snap", "expected_head": null,
                "data": {"nodes": [{"id": "x", "entity_id": "X", "space_id": "s", "reader_groups": ["secret"]}]}}])
        )),
        reference
    );
}

#[test]
fn hidden_registry_state_does_not_change_visible_output() {
    // Two stores that differ only in a group carol cannot see: who is in it,
    // and the facts restricted to it. Every output carol gets is identical.
    let outputs = |hidden_members: &[&str], hidden_facts: bool| {
        let mut s = Store::registry();
        let mut changes = vec![json!({"change": "add_principal", "id": "secret", "kind": "group"})];
        for m in hidden_members {
            changes.push(json!({"change": "add_member", "group": "secret", "member": m}));
        }
        s.reg(ADMIN, Value::Array(changes)).unwrap();
        s.grant("g", "carol", "write");
        let mut facts = vec![
            node("a", &[], &[]),
            node("b", &[], &[]),
            json!({"kind": "edge", "id": "ab", "predicate": "p", "from": "a", "to": "b", "valid_time": {"start": 0}}),
            json!({"kind": "embedding_space", "id": "sp", "revision": "1", "encoder": "e",
                "encoder_version": "v", "preprocessing": "p", "dimensions": 2, "metric": "euclidean"}),
            json!({"kind": "embedding", "id": "va", "space": {"id": "sp", "revision": "1"},
                "subject": {"kind": "node", "node_id": "a"}, "vector": [1.0, 0.0], "valid_time": {"start": 0}}),
            json!({"kind": "embedding", "id": "vb", "space": {"id": "sp", "revision": "1"},
                "subject": {"kind": "node", "node_id": "b"}, "vector": [0.0, 1.0], "valid_time": {"start": 0}}),
        ];
        if hidden_facts {
            facts.push(json!({"kind": "embedding", "id": "vh", "space": {"id": "sp", "revision": "1"},
                "subject": {"kind": "node", "node_id": "a"}, "vector": [1.0, 0.01], "valid_time": {"start": 0},
                "reader_groups": ["secret"]}));
            facts.push(node("h", &[], &["secret"]));
            facts.push(
                json!({"kind": "edge", "id": "ah", "predicate": "p", "from": "a", "to": "h",
                "valid_time": {"start": 0}, "reader_groups": ["secret"]}),
            );
            facts.push(json!({"kind": "attachment", "id": "m", "host": {"kind": "node", "id": "a"}, "key": "k",
                "value": {"kind": "literal", "value": "x"}, "valid_time": {"start": 0}, "reader_groups": ["secret"]}));
        }
        s.write(ADMIN, "g", Value::Array(facts)).unwrap();
        let mut seen = vec![];
        let now = s.clock_now();
        for op in [
            json!({"op": "query", "query": {"graph_id": "g"}}),
            json!({"op": "query", "query": {"graph_id": "g", "predicate": "p"}}),
            json!({"op": "evaluate", "value": {"kind": "expand", "graph_id": "g", "seeds": ["a"], "hops": 2}}),
            json!({"op": "evaluate", "value": {"kind": "changes", "graph_id": "g", "from": {"kind": "as_of", "unix_millis": 0}, "to": {"kind": "head"}}}),
            json!({"op": "evaluate", "value": {"kind": "as_of", "query": {"graph_id": "g"}, "unix_millis": now}}),
            json!({"op": "evaluate", "value": {"kind": "nearest", "graph_id": "g", "space": {"id": "sp", "revision": "1"},
                "query": {"kind": "vector", "vector": [1.0, 0.0]}, "k": 3}}),
            json!({"op": "evaluate", "value": {"kind": "similar_candidates", "graph_id": "g", "space": {"id": "sp", "revision": "1"},
                "k": 2, "limit": 10}}),
            json!({"op": "write", "graph_id": "g", "branch_id": "x", "changes": [{"change": "assert", "fact": node("c", &[], &["secret"])}]}),
        ] {
            let out = s.run("carol", json!([op]));
            seen.push(match out {
                Ok(results) => normalize(serde_json::to_value(results).unwrap()),
                Err(e) => json!({"code": e.code, "message": e.message}),
            });
        }
        seen.push(json!(s
            .e
            .list_graphs(&host("carol"))
            .unwrap()
            .iter()
            .map(|l| l.graph_id.clone())
            .collect::<Vec<_>>()));
        seen
    };
    let baseline = outputs(&[], false);
    // The reads succeed and the write naming the hidden group is refused.
    for (i, out) in baseline[..7].iter().enumerate() {
        assert!(out.get("code").is_none(), "operation {i}: {out}");
    }
    assert_eq!(baseline[7]["code"], "E_UNAVAILABLE");
    assert_eq!(outputs(&["alice", "bob"], false), baseline);
    assert_eq!(outputs(&["alice"], true), baseline);
}

/// Revision IDs are content-derived, so stores with different hidden facts
/// differ in them; everything else must match (A3 decision on paired tests).
fn normalize(mut value: Value) -> Value {
    fn walk(v: &mut Value) {
        match v {
            Value::String(s) if s.starts_with("fact:sha256:") || s.starts_with("sha256:") => {
                *s = "REV".into()
            }
            Value::Array(items) => items.iter_mut().for_each(walk),
            Value::Object(map) => {
                // Keys may embed revision IDs (snapshot maps).
                let entries: Vec<(String, Value)> = std::mem::take(map).into_iter().collect();
                for (k, mut v) in entries {
                    walk(&mut v);
                    let k = if k.starts_with("fact:sha256:") {
                        "REV".into()
                    } else {
                        k
                    };
                    map.insert(k, v);
                }
            }
            _ => {}
        }
    }
    walk(&mut value);
    value
}

#[test]
fn the_registry_graph_is_written_only_through_the_registry_api_and_read_under_its_rules() {
    let mut s = Store::registry();
    for commands in [
        json!([{"op": "write", "graph_id": "weave:registry", "changes": [{"change": "assert", "fact": node("x", &[], &[])}]}]),
        json!([{"op": "commit", "graph_id": "weave:registry", "data": {"nodes": []}}]),
    ] {
        let h = HostContext::new(ADMIN, ["weave:registry".to_string()]);
        assert_eq!(error(s.run_as(&h, commands)).0, "E_RESERVED_NAMESPACE");
    }
    // A principal reads its own record; administrators read everything,
    // including the registry's history.
    let mine = s.nodes("alice", "weave:registry");
    assert_eq!(mine, ["alice", "team"]);
    let all = s.nodes(ADMIN, "weave:registry");
    assert!(all.contains(&"bob".to_string()) && all.contains(&ADMINISTRATORS.to_string()));
    let before = s.clock_now();
    s.reg(
        ADMIN,
        json!([{"change": "remove_member", "group": "team", "member": "alice"}]),
    )
    .unwrap();
    // "Who was in team at time T": an as-of query of the registry.
    let then = s
        .run(
            ADMIN,
            json!([{"op": "evaluate", "value": {"kind": "as_of", "query": {"graph_id": "weave:registry", "predicate": "weave:member", "to": "team"}, "unix_millis": before}}]),
        )
        .unwrap();
    let then = result(then);
    assert_eq!(then["graph"]["edges"].as_array().unwrap().len(), 1);
    let now = s
        .run(
            ADMIN,
            json!([{"op": "query", "query": {"graph_id": "weave:registry", "predicate": "weave:member", "to": "team"}}]),
        )
        .unwrap();
    assert!(result(now)["graph"]["edges"].as_array().unwrap().is_empty());
    // The registry API applies the same rules.
    assert_eq!(
        s.e.registry_principal("alice", &host("alice")).unwrap().id,
        "alice"
    );
    assert_eq!(
        error(s.e.registry_principal("bob", &host("alice"))).0,
        "E_UNAVAILABLE"
    );
    assert_eq!(
        error(s.e.registry_members("team", &host("bob"))).0,
        "E_UNAVAILABLE"
    );
}

#[test]
fn registry_changes_are_authorized_and_ids_are_never_reused() {
    let s = Store::registry();
    // Only tenant administrators register principals and change statuses.
    assert_eq!(
        error(s.reg(
            "alice",
            json!([{"change": "add_principal", "id": "eve", "kind": "user"}])
        ))
        .0,
        "E_FORBIDDEN"
    );
    assert_eq!(
        error(s.reg(
            "alice",
            json!([{"change": "add_member", "group": ADMINISTRATORS, "member": "alice"}])
        ))
        .0,
        "E_UNAVAILABLE"
    );
    // Group administrators manage members of their group only.
    s.reg(
        ADMIN,
        json!([{"change": "set_group_administrators", "group": "team", "administrators": ["bob"]}]),
    )
    .unwrap();
    s.reg(
        "bob",
        json!([{"change": "add_member", "group": "team", "member": "carol"}]),
    )
    .unwrap();
    assert_eq!(
        s.e.registry_members("team", &host("bob")).unwrap(),
        ["alice", "carol"]
    );
    // Retired IDs are never reissued.
    s.reg(
        ADMIN,
        json!([{"change": "set_status", "id": "carol", "status": "retired"}]),
    )
    .unwrap();
    for (changes, code) in [
        (
            json!([{"change": "add_principal", "id": "carol", "kind": "user"}]),
            "E_CONFLICT",
        ),
        (
            json!([{"change": "set_status", "id": "carol", "status": "active"}]),
            "E_CONFLICT",
        ),
        (
            json!([{"change": "add_principal", "id": "weave:x", "kind": "user"}]),
            "E_RESERVED_NAMESPACE",
        ),
        (
            json!([{"change": "remove_member", "group": ADMINISTRATORS, "member": ADMIN}]),
            "E_CONFLICT",
        ),
    ] {
        assert_eq!(error(s.reg(ADMIN, changes.clone())).0, code, "{changes}");
    }
    // A suspended or retired principal can do nothing.
    assert_eq!(
        error(s.reg(
            "carol",
            json!([{"change": "add_member", "group": "team", "member": "bob"}])
        ))
        .0,
        "E_FORBIDDEN"
    );
    assert_eq!(
        error(s.e.enable_registry(&host(ADMIN))).0,
        "E_CONFLICT",
        "enabling is one-way"
    );
}

#[test]
fn stores_without_a_registry_keep_the_literal_profile_and_reject_group_readers() {
    let mut s = Store::new();
    let h = HostContext::new("alice", ["g".to_string()]);
    s.run_as(
        &h,
        json!([{"op": "write", "graph_id": "g", "changes": [{"change": "assert", "fact": node("a", &["alice"], &[])}]}]),
    )
    .unwrap();
    let rejected = s.run_as(
        &h,
        json!([{"op": "write", "graph_id": "g", "changes": [{"change": "assert", "fact": node("b", &[], &["team"])}]}]),
    );
    assert_eq!(error(rejected).0, "E_UNAVAILABLE");
    assert!(!s.e.registry_enabled().unwrap());
    let listing = s.e.list_graphs(&h).unwrap();
    assert_eq!(listing.len(), 1);
    assert_eq!(
        error(s.e.apply_registry(
            &[RegistryChange::AddMember {
                group: "x".into(),
                member: "y".into()
            }],
            &h
        ))
        .0,
        "E_REGISTRY_DISABLED"
    );
}

#[test]
fn authority_reducing_changes_advance_the_admission_epoch() {
    let s = Store::registry();
    let ctx = weave_policy::AdmissionContext {
        audience: "tenant".into(),
        now_ms: 0,
        policy_epoch: "e1".into(),
        roots: vec![],
        revoked_capabilities: Default::default(),
        revoked_keys: Default::default(),
        consumed_nonces: Default::default(),
    };
    s.e.install_admission_policy(&ctx).unwrap();
    let epoch = |s: &Store| -> String { s.e.admission_policy_epoch().unwrap().unwrap() };
    s.reg(
        ADMIN,
        json!([{"change": "add_member", "group": "team", "member": "bob"}]),
    )
    .unwrap();
    assert_eq!(epoch(&s), "e1", "additive changes keep the epoch");
    let revision = s
        .reg(
            ADMIN,
            json!([{"change": "remove_member", "group": "team", "member": "bob"}]),
        )
        .unwrap();
    assert_eq!(epoch(&s), format!("registry:{revision}"));
}

#[test]
fn assertion_edges_keep_group_restrictions_without_flattening_them() {
    let mut s = Store::registry();
    s.grant("g", "alice", "read");
    s.run(
        ADMIN,
        json!([{"op": "commit", "graph_id": "g", "expected_head": null, "data": {
            "profile": "explicit",
            "nodes": [{"id": "a", "entity_id": "A", "space_id": "s"}, {"id": "b", "entity_id": "B", "space_id": "s"}],
            "structural_edges": [{"id": "s1", "predicate": "p", "from": "a", "to": "b", "reader_groups": ["team"]}],
            "assertions": [{"id": "x1", "edge_id": "s1", "source": "src", "valid_time": {"start": 0}}]}}]),
    )
    .unwrap();
    let out = s.query("alice", "g").unwrap();
    let edges = out["graph"]["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 1);
    // The derived edge carries the group clause, never resolved members.
    assert_eq!(edges[0]["reader_groups"], json!(["team"]));
    assert!(edges[0]["readers"].as_array().unwrap().is_empty());
    assert!(s.query("bob", "g").is_err());
}
