//! B2 part 2 (#5, ADR-0008 sections 2 to 4): write-time propagation.
//!
//! - Section 4: a live version the writer cannot read is absent for every
//!   purpose except occupancy, in local and registry stores.
//! - Section 3 (1.2.0 programs): corrections never widen readers silently,
//!   and declared derivations are no broader than the evidence they cite.
//! - Section 2: the conjunction of group-based clauses keeps its first clause
//!   exactly and carries the rest.
use serde_json::{json, Value};
use std::sync::Arc;
use weave_contract::{CommandResult, Fact, FactChange, Program, VERSION};
use weave_engine::{Engine, HostContext, ManualClock, RegistryChange};

fn program(version: &str, commands: Value) -> Program {
    serde_json::from_value(json!({"version": version, "commands": commands})).unwrap()
}
fn writer(who: &str) -> HostContext {
    HostContext::new(who, ["g".to_string(), "h".to_string()])
}
fn reader(who: &str) -> HostContext {
    HostContext::new(who, Vec::<String>::new())
}
fn write(changes: Value) -> Value {
    json!([{"op": "write", "graph_id": "g", "changes": changes}])
}
fn node(id: &str, readers: &[&str]) -> Value {
    json!({"kind": "node", "id": id, "entity_id": format!("E-{id}"), "space_id": "s", "readers": readers})
}
fn assert_(fact: Value) -> Value {
    json!({"change": "assert", "fact": fact})
}
fn correct(fact: Value) -> Value {
    json!({"change": "correct", "fact": fact})
}

struct Store {
    e: Engine,
    clock: Arc<ManualClock>,
}
impl Store {
    fn new() -> Self {
        let clock = Arc::new(ManualClock::new(10));
        Self {
            e: Engine::memory_with_clock(clock.clone()).unwrap(),
            clock,
        }
    }
    /// A registry store: users alice, bob, carol; group team = {alice, bob};
    /// every user may write `g` and `h`.
    fn registry() -> Self {
        let s = Self::new();
        let admin = reader("admin");
        s.e.enable_registry(&admin).unwrap();
        s.reg(json!([
            {"change": "add_principal", "id": "alice", "kind": "user"},
            {"change": "add_principal", "id": "bob", "kind": "user"},
            {"change": "add_principal", "id": "carol", "kind": "user"},
            {"change": "add_principal", "id": "team", "kind": "group"},
            {"change": "add_member", "group": "team", "member": "alice"},
            {"change": "add_member", "group": "team", "member": "bob"},
            {"change": "add_principal", "id": "other", "kind": "group"},
            {"change": "add_member", "group": "other", "member": "alice"},
            {"change": "grant", "graph_id": "g", "subject": "alice", "permission": "write"},
            {"change": "grant", "graph_id": "g", "subject": "bob", "permission": "write"},
            {"change": "grant", "graph_id": "g", "subject": "carol", "permission": "write"},
            {"change": "grant", "graph_id": "snap", "subject": "alice", "permission": "write"},
            {"change": "grant", "graph_id": "snap", "subject": "bob", "permission": "read"},
            {"change": "grant", "graph_id": "snap", "subject": "carol", "permission": "read"}
        ]));
        s
    }
    fn reg(&self, changes: Value) {
        self.tick();
        let changes: Vec<RegistryChange> = serde_json::from_value(changes).unwrap();
        self.e.apply_registry(&changes, &reader("admin")).unwrap();
    }
    fn tick(&self) {
        use weave_engine::TrustedClock;
        self.clock.set(self.clock.unix_millis().unwrap() + 10);
    }
    fn run(
        &mut self,
        version: &str,
        host: &HostContext,
        commands: Value,
    ) -> Result<Vec<CommandResult>, (String, String)> {
        self.tick();
        self.e
            .execute(&program(version, commands), host)
            .map_err(|e| (e.code, e.message))
    }
    fn write(
        &mut self,
        host: &HostContext,
        changes: Value,
    ) -> Result<Vec<CommandResult>, (String, String)> {
        self.run(VERSION, host, write(changes))
    }
    fn revision(&mut self, host: &HostContext, changes: Value) -> String {
        match self.write(host, changes).unwrap().pop() {
            Some(
                CommandResult::Committed { revision, .. } | CommandResult::Unchanged { revision },
            ) => revision,
            other => panic!("{other:?}"),
        }
    }
}

#[track_caller]
fn code(r: Result<Vec<CommandResult>, (String, String)>) -> (String, String) {
    r.unwrap_err()
}

// ---------------------------------------------------------------- section 4

/// The write path compares a change only with versions the writer may read.
/// Before B2 part 2 an identical assert of a hidden fact was a no-op and a
/// different one a conflict, "another kind" named the hidden kind, and
/// `correct` or `retract` acted on the hidden fact.
#[test]
fn a_hidden_live_version_is_absent_except_for_occupancy() {
    for registry in [false, true] {
        let mut s = if registry {
            Store::registry()
        } else {
            Store::new()
        };
        let (author, other) = if registry {
            (reader("alice"), reader("carol"))
        } else {
            (writer("alice"), writer("carol"))
        };
        let hidden_edge = json!({"kind": "edge", "id": "link", "predicate": "p", "from": "open",
            "to": "open", "valid_time": {"start": 0}, "readers": ["alice"]});
        s.write(
            &author,
            json!([
                assert_(node("secret", &["alice"])),
                assert_(node("open", &[])),
                assert_(hidden_edge.clone())
            ]),
        )
        .unwrap();
        let unavailable = (
            "E_CONFLICT".to_string(),
            "fact ID is not available".to_string(),
        );
        // Identical content, other content and another kind: one answer.
        for fact in [
            node("secret", &["alice"]),
            node("secret", &[]),
            hidden_edge.clone(),
            json!({"kind": "attachment", "id": "link", "host": {"kind": "graph"}, "key": "k",
                "value": {"kind": "literal", "value": 1}, "valid_time": {"start": 0}}),
        ] {
            assert_eq!(
                code(s.write(&other, json!([assert_(fact)]))),
                unavailable,
                "registry {registry}"
            );
        }
        // Correct, retract and expectations: exactly as for a missing fact.
        let pairs = [
            (
                correct(node("secret", &["alice"])),
                correct(node("missing", &["alice"])),
            ),
            (
                json!({"change": "retract", "target": {"kind": "node", "id": "secret"}}),
                json!({"change": "retract", "target": {"kind": "node", "id": "missing"}}),
            ),
            (
                json!({"change": "correct", "fact": node("secret", &["alice"]),
                    "expected": {"kind": "unchanged_since", "revision": "fact:sha256:0000000000000000000000000000000000000000000000000000000000000000"}}),
                json!({"change": "correct", "fact": node("missing", &["alice"]),
                    "expected": {"kind": "unchanged_since", "revision": "fact:sha256:0000000000000000000000000000000000000000000000000000000000000000"}}),
            ),
            (
                json!({"change": "correct", "fact": node("secret", &["alice"]), "expected": {"kind": "absent"}}),
                json!({"change": "correct", "fact": node("missing", &["alice"]), "expected": {"kind": "absent"}}),
            ),
        ];
        for (hidden, missing) in pairs {
            assert_eq!(
                code(s.write(&other, json!([hidden.clone()]))),
                code(s.write(&other, json!([missing]))),
                "registry {registry}: {hidden}"
            );
        }
        // The fact is untouched and its author still owns it.
        s.write(&author, json!([{"change": "correct", "fact": node("secret", &["alice", "bob"]), "widen_readers": true}])).unwrap();
        // A visible fact behaves as before.
        s.write(&other, json!([assert_(node("open", &[]))]))
            .unwrap();
        assert_eq!(
            code(s.write(&other, json!([assert_(node("open", &["carol"]))]))).1,
            "fact is already live; use correct"
        );
    }
}

/// In a store without a registry, a writer may act on a version it recorded
/// itself, so trusted importers reconcile and correct the restricted records
/// they wrote. `reconcile_facts` plans a hidden key like an absent one.
#[test]
fn local_authors_keep_their_versions_and_reconciliation_hides_the_rest() {
    let mut s = Store::new();
    let importer = writer("importer");
    s.write(&importer, json!([assert_(node("unit", &["customer"]))]))
        .unwrap();
    // The importer is not a reader, but it recorded this version.
    let same: Fact = serde_json::from_value(node("unit", &["customer"])).unwrap();
    let plan =
        s.e.reconcile_facts("g", "main", std::slice::from_ref(&same), &importer)
            .unwrap();
    assert!(plan.changes.is_empty(), "identical to what it wrote");
    s.write(&importer, json!([{"change": "correct", "fact": node("unit", &["customer", "auditor"]), "widen_readers": true}])).unwrap();
    // Another writer sees the key as absent, and its write fails on occupancy.
    let other = writer("other");
    let plan = s.e.reconcile_facts("g", "main", &[same], &other).unwrap();
    assert!(matches!(
        plan.changes.as_slice(),
        [FactChange::Assert { .. }]
    ));
    let missing: Fact = serde_json::from_value(node("nowhere", &["customer"])).unwrap();
    let absent =
        s.e.reconcile_facts("g", "main", &[missing], &other)
            .unwrap();
    assert!(matches!(
        absent.changes.as_slice(),
        [FactChange::Assert { .. }]
    ));
    let changes = serde_json::to_value(&plan.changes).unwrap();
    assert_eq!(code(s.write(&other, changes)).1, "fact ID is not available");
    // A version someone else recorded is hidden from the importer too.
    let customer = writer("customer");
    s.write(&customer, json!([correct(node("unit", &["customer"]))]))
        .unwrap();
    assert_eq!(
        code(s.write(&importer, json!([correct(node("unit", &["customer"]))]))).1,
        "corrected fact is not live"
    );
}

/// In a registry store a reference to a record the writer cannot read fails
/// exactly like a reference to a missing record. A local store's trusted
/// pipelines may reference records by ID (declared).
#[test]
fn references_to_hidden_records_fail_like_missing_ones_in_registry_stores() {
    let mut s = Store::registry();
    let alice = reader("alice");
    s.write(
        &alice,
        json!([
            assert_(node("secret", &["alice"])),
            assert_(node("open", &[]))
        ]),
    )
    .unwrap();
    let carol = reader("carol");
    let edge = |id: &str, to: &str| {
        assert_(
            json!({"kind": "edge", "id": id, "predicate": "p", "from": "open", "to": to,
            "valid_time": {"start": 0}}),
        )
    };
    let attachment = |id: &str, host: &str| {
        assert_(
            json!({"kind": "attachment", "id": id, "host": {"kind": "node", "id": host}, "key": "k",
            "value": {"kind": "literal", "value": 1}, "valid_time": {"start": 0}}),
        )
    };
    assert_eq!(
        code(s.write(&carol, json!([edge("e1", "secret")]))),
        code(s.write(&carol, json!([edge("e1", "missing")])))
    );
    assert_eq!(
        code(s.write(&carol, json!([attachment("m1", "secret")]))),
        code(s.write(&carol, json!([attachment("m1", "missing")])))
    );
    s.write(&carol, json!([edge("e3", "open")])).unwrap();
    // A local store keeps resolving references against every live record.
    let mut local = Store::new();
    local
        .write(
            &writer("alice"),
            json!([
                assert_(node("secret", &["alice"])),
                assert_(node("open", &[]))
            ]),
        )
        .unwrap();
    local
        .write(&writer("embedder"), json!([edge("e1", "secret")]))
        .unwrap();
}

/// A hidden dependent still refuses a retraction, with the visible error.
#[test]
fn a_hidden_dependent_still_refuses_a_retraction() {
    let mut s = Store::registry();
    let alice = reader("alice");
    let bob = reader("bob");
    s.write(&bob, json!([assert_(node("shared", &[]))]))
        .unwrap();
    s.write(
        &alice,
        json!([assert_(
            json!({"kind": "edge", "id": "mine", "predicate": "p",
        "from": "shared", "to": "shared", "valid_time": {"start": 0}, "readers": ["alice"]})
        )]),
    )
    .unwrap();
    assert_eq!(
        code(s.write(
            &bob,
            json!([{"change": "retract", "target": {"kind": "node", "id": "shared"}}])
        ))
        .0,
        "E_DEPENDENT_FACT"
    );
}

// ---------------------------------------------------------------- section 3

/// From 1.2.0 a correction that admits more readers needs `widen_readers`.
#[test]
fn corrections_never_widen_readers_silently() {
    let mut s = Store::registry();
    let alice = reader("alice");
    s.write(
        &alice,
        json!([assert_(
            json!({"kind": "node", "id": "a", "entity_id": "A", "space_id": "s",
        "readers": ["alice"], "reader_groups": ["team"]})
        )]),
    )
    .unwrap();
    let widened = (
        "E_READERS_WIDENED".to_string(),
        "a correction may not widen readers without widen_readers".to_string(),
    );
    let with = |readers: &[&str], groups: &[&str]| {
        json!({"kind": "node", "id": "a", "entity_id": "A", "space_id": "s",
            "readers": readers, "reader_groups": groups})
    };
    // Public, an extra principal, an extra group: all widen.
    for fact in [
        with(&[], &[]),
        with(&["alice", "carol"], &["team"]),
        with(&["alice"], &["team", "other"]),
    ] {
        assert_eq!(
            code(s.write(&alice, json!([correct(fact.clone())]))),
            widened,
            "{fact}"
        );
    }
    // Narrowing and equal clauses need no flag.
    s.write(&alice, json!([correct(with(&["alice"], &[]))]))
        .unwrap();
    s.write(&alice, json!([correct(with(&["alice"], &[]))]))
        .unwrap();
    // The flag publishes it.
    s.write(
        &alice,
        json!([{"change": "correct", "fact": with(&[], &[]), "widen_readers": true}]),
    )
    .unwrap();
    // A 1.1.0 program keeps its published meaning: an omitted list replaces readers.
    s.write(&alice, json!([correct(with(&["alice"], &[]))]))
        .unwrap();
    s.run("1.1.0", &alice, write(json!([correct(with(&[], &[]))])))
        .unwrap();
    // ...and cannot name the flag.
    assert_eq!(
        code(s.run(
            "1.1.0",
            &alice,
            write(
                json!([{"change": "correct", "fact": with(&["alice"], &[]), "widen_readers": true}])
            )
        ))
        .0,
        "E_VERSION"
    );
}

/// From 1.2.0 a fact citing restricted evidence is no broader than it.
#[test]
fn declared_derivations_are_no_broader_than_their_evidence() {
    let mut s = Store::registry();
    let alice = reader("alice");
    let revision = s.revision(
        &alice,
        json!([assert_(
            json!({"kind": "node", "id": "secret", "entity_id": "S",
        "space_id": "s", "reader_groups": ["team"]})
        )]),
    );
    let cite = |id: &str, readers: &[&str], groups: &[&str], carried: bool| {
        let mut fact = json!({"kind": "node", "id": id, "entity_id": id.to_uppercase(), "space_id": "s",
            "readers": readers, "reader_groups": groups,
            "provenance": {"method": {"kind": "human"},
                "evidence": [{"kind": "node", "graph_id": "g", "revision": revision, "node_id": "secret"}]}});
        if carried {
            fact["derived_nodes"] =
                json!([{"graph_id": "g", "revision": revision, "node_id": "secret"}]);
        }
        assert_(fact)
    };
    let broader = (
        "E_DERIVED_READERS".to_string(),
        "a derived fact's readers are broader than an input it cites".to_string(),
    );
    for change in [
        cite("d1", &[], &[], false),
        cite("d2", &["carol"], &[], false),
        cite("d3", &[], &["team", "other"], false),
    ] {
        assert_eq!(
            code(s.write(&alice, json!([change.clone()]))),
            broader,
            "{change}"
        );
    }
    // Within the evidence's clause, or carrying it, is accepted.
    s.write(&alice, json!([cite("ok1", &[], &["team"], false)]))
        .unwrap();
    s.write(&alice, json!([cite("ok2", &[], &[], true)]))
        .unwrap();
    // A correction is checked too.
    assert_eq!(
        code(s.write(
            &alice,
            json!([{"change": "correct", "fact": cite("ok1", &[], &[], false)["fact"].clone(),
            "widen_readers": true}])
        )),
        broader
    );
    // Evidence the writer cannot see is not consulted: carol's outcome is the
    // one for evidence that does not exist.
    let carol = reader("carol");
    let missing = json!({"kind": "node", "id": "c2", "entity_id": "C2", "space_id": "s",
        "provenance": {"method": {"kind": "human"},
            "evidence": [{"kind": "node", "graph_id": "g", "revision": revision, "node_id": "nowhere"}]}});
    s.write(&carol, json!([cite("c1", &[], &[], false)]))
        .unwrap();
    s.write(&carol, json!([assert_(missing)])).unwrap();
    // 1.1.0 programs keep their published meaning.
    s.run(
        "1.1.0",
        &alice,
        write(json!([cite("legacy", &[], &[], false)])),
    )
    .unwrap();
}

// ---------------------------------------------------------------- section 2

/// An explicit assertion restricted to a group, on a structural edge
/// restricted to a principal: the derived edge keeps the first clause exactly
/// and carries its source assertion, so a reader satisfying both sees it with
/// a usable clause (the B1 meet of the two clauses matched no one).
#[test]
fn a_derived_conjunction_keeps_its_first_clause_and_carries_the_rest() {
    let mut s = Store::registry();
    let alice = reader("alice");
    s.run(VERSION, &alice, json!([{"op": "commit", "graph_id": "snap", "data": {"profile": "explicit",
        "nodes": [{"id": "a", "entity_id": "A", "space_id": "s"}, {"id": "b", "entity_id": "B", "space_id": "s"}],
        "structural_edges": [{"id": "s1", "predicate": "p", "from": "a", "to": "b", "readers": ["alice"]}],
        "assertions": [{"id": "claim", "edge_id": "s1", "source": "x", "valid_time": {"start": 0},
            "reader_groups": ["team"]}]}}]))
        .unwrap();
    let query = |s: &mut Store, who: &str| {
        let out = s
            .run(
                VERSION,
                &reader(who),
                json!([{"op": "query", "query": {"graph_id": "snap"}}]),
            )
            .unwrap();
        match out.into_iter().next() {
            Some(CommandResult::Queried { result }) => *result,
            other => panic!("{other:?}"),
        }
    };
    let seen = query(&mut s, "alice");
    let edge = seen.graph.edges.iter().find(|e| e.id == "claim").unwrap();
    assert_eq!(
        (edge.readers.clone(), edge.reader_groups.clone()),
        (vec![], vec!["team".to_string()])
    );
    assert!(edge.derived_from.iter().any(|r| r.assertion_id == "claim"));
    // bob is in team but cannot read the structural edge; carol neither.
    for who in ["bob", "carol"] {
        assert!(query(&mut s, who).graph.edges.is_empty(), "{who}");
    }
}

// ---------------------------------------------------------------- revocation

/// ADR-0007 section 10, item 6: after an authority-reducing change, nothing
/// computed for the old authority is returned: not a compiled view, not a
/// change set, not a ranking.
#[test]
fn revocation_reaches_views_change_sets_and_rankings_on_the_next_request() {
    use weave_contract::view_registration::{
        seal_template, CompiledViewTemplate, VIEW_TEMPLATE_FORMAT,
    };
    use weave_contract::{CurrentViewSelection, ViewClock, ViewReadTime};
    let mut s = Store::registry();
    let alice = reader("alice");
    s.write(&alice, json!([
        assert_(json!({"kind": "embedding_space", "id": "sp", "revision": "1", "encoder": "e",
            "encoder_version": "v", "preprocessing": "p", "dimensions": 2, "metric": "euclidean"})),
        assert_(json!({"kind": "node", "id": "t", "entity_id": "T", "space_id": "s", "reader_groups": ["team"]})),
        assert_(json!({"kind": "embedding", "id": "vt", "space": {"id": "sp", "revision": "1"},
            "subject": {"kind": "node", "node_id": "t"}, "vector": [1.0, 0.0], "valid_time": {"start": 0},
            "reader_groups": ["team"]}))
    ]))
    .unwrap();
    let template = seal_template(CompiledViewTemplate {
        format: VIEW_TEMPLATE_FORMAT.into(),
        protocol: VERSION.into(),
        name: "team".into(),
        revision: "1".into(),
        expression: serde_json::from_value(json!({"kind": "query", "query": {"graph_id": "g"}}))
            .unwrap(),
        clock: ViewClock::Fixed,
        source_revisions: vec![],
        definition_digest: String::new(),
    })
    .unwrap();
    s.tick();
    s.e.register_compiled_view("v", &template, None, &alice)
        .unwrap();
    let selection = CurrentViewSelection {
        view_id: "v".into(),
        definition_digest: template.definition_digest.clone(),
        time: ViewReadTime::Fixed,
    };
    let read = |s: &mut Store| -> Vec<String> {
        s.tick();
        match s.e.read_current_view(&selection, &alice) {
            Ok(snapshot) => snapshot
                .result
                .graph
                .nodes
                .iter()
                .map(|n| n.id.clone())
                .collect(),
            Err(_) => vec![],
        }
    };
    let evaluate = |s: &mut Store, value: Value| -> String {
        s.run(VERSION, &alice, json!([{"op": "evaluate", "value": value}]))
            .map(|r| serde_json::to_string(&r).unwrap())
            .unwrap_or_else(|e| format!("{e:?}"))
    };
    let changes =
        json!({"kind": "changes", "graph_id": "g", "from": {"kind": "as_of", "unix_millis": 0}});
    let nearest = json!({"kind": "nearest", "graph_id": "g", "space": {"id": "sp", "revision": "1"},
        "query": {"kind": "vector", "vector": [1.0, 0.0]}, "k": 3});
    assert_eq!(read(&mut s), ["t"]);
    assert!(evaluate(&mut s, changes.clone()).contains("\"t\""));
    assert!(evaluate(&mut s, nearest.clone()).contains("\"vt\""));
    s.reg(json!([{"change": "remove_member", "group": "team", "member": "alice"}]));
    assert!(
        read(&mut s).is_empty(),
        "the view never serves the old authority"
    );
    assert!(!evaluate(&mut s, changes).contains("\"t\""));
    assert!(!evaluate(&mut s, nearest).contains("\"vt\""));
}
