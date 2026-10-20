//! Engine hooks for the B3 service (#39, ADR-0007 section 4): graph creation
//! with an engine-assigned ID, and correct/retract of a fact only with Read
//! on it.
use serde_json::{json, Value};
use std::sync::Arc;
use weave_contract::{CommandResult, Program, VERSION};
use weave_engine::{
    Engine, HostContext, ManualClock, Permission, RegistryChange, TrustedClock,
    CREATED_GRAPH_PREFIX,
};

fn host(principal: &str) -> HostContext {
    HostContext::new(principal, Vec::<String>::new())
}

struct Store {
    e: Engine,
    clock: Arc<ManualClock>,
}

impl Store {
    fn new() -> Self {
        let clock = Arc::new(ManualClock::new(10));
        let e = Engine::memory_with_clock(clock.clone()).unwrap();
        Self { e, clock }
    }
    fn registry() -> Self {
        let s = Self::new();
        s.e.enable_registry(&host("root")).unwrap();
        s.reg(json!([
            {"change": "add_principal", "id": "alice", "kind": "user"},
            {"change": "add_principal", "id": "bob", "kind": "user"},
            {"change": "add_principal", "id": "team", "kind": "group"},
            {"change": "add_member", "group": "team", "member": "alice"},
            {"change": "grant", "graph_id": "g", "subject": "bob", "permission": "write"},
            {"change": "grant", "graph_id": "g", "subject": "alice", "permission": "write"}
        ]));
        s
    }
    fn tick(&self) {
        self.clock.set(self.clock.unix_millis().unwrap() + 10);
    }
    fn reg(&self, changes: Value) {
        self.tick();
        let changes: Vec<RegistryChange> = serde_json::from_value(changes).unwrap();
        self.e.apply_registry(&changes, &host("root")).unwrap();
    }
    fn run(
        &mut self,
        h: &HostContext,
        commands: Value,
    ) -> weave_engine::Result<Vec<CommandResult>> {
        self.tick();
        let program: Program =
            serde_json::from_value(json!({"version": VERSION, "commands": commands})).unwrap();
        self.e.execute(&program, h)
    }
    fn write(
        &mut self,
        h: &HostContext,
        changes: Value,
    ) -> weave_engine::Result<Vec<CommandResult>> {
        self.run(
            h,
            json!([{"op": "write", "graph_id": "g", "changes": changes}]),
        )
    }
}

fn node(id: &str, readers: &[&str], groups: &[&str]) -> Value {
    json!({"kind": "node", "id": id, "entity_id": format!("E-{id}"), "space_id": "s",
        "readers": readers, "reader_groups": groups})
}
fn assert_(fact: Value) -> Value {
    json!({"change": "assert", "fact": fact})
}
fn error(r: weave_engine::Result<Vec<CommandResult>>) -> (String, String) {
    let e = r.unwrap_err();
    (e.code, e.message)
}

#[test]
fn correct_and_retract_treat_an_unreadable_fact_as_absent() {
    let mut s = Store::registry();
    s.write(
        &host("root"),
        json!([
            assert_(node("public", &[], &[])),
            assert_(node("for-root", &["root"], &[])),
            assert_(node("for-team", &[], &["team"])),
            assert_(json!({"kind": "edge", "id": "e-public", "predicate": "p",
                "from": "public", "to": "for-root", "valid_time": {"start": 0}}))
        ]),
    )
    .unwrap();
    let retract =
        |kind: &str, id: &str| json!([{"change": "retract", "target": {"kind": kind, "id": id}}]);
    let correct = |id: &str| json!([{"change": "correct", "fact": node(id, &[], &[])}]);
    // For bob each hidden fact fails exactly like one that does not exist.
    for (hidden, kind) in [
        ("for-root", "node"),
        ("for-team", "node"),
        ("e-public", "edge"),
    ] {
        assert_eq!(
            error(s.write(&host("bob"), retract(kind, hidden))),
            error(s.write(&host("bob"), retract(kind, "missing"))),
            "{hidden}"
        );
    }
    for hidden in ["for-root", "for-team"] {
        assert_eq!(
            error(s.write(&host("bob"), correct(hidden))),
            error(s.write(&host("bob"), correct("missing"))),
        );
        // An expectation does not tell them apart either.
        let expected = json!([{"change": "correct", "fact": node(hidden, &[], &[]),
            "expected": {"kind": "absent"}}]);
        let missing = json!([{"change": "correct", "fact": node("missing", &[], &[]),
            "expected": {"kind": "absent"}}]);
        assert_eq!(
            error(s.write(&host("bob"), expected)),
            error(s.write(&host("bob"), missing))
        );
    }
    // Readers who can read them may correct and retract them.
    s.write(&host("alice"), correct("for-team")).unwrap();
    s.write(&host("root"), retract("edge", "e-public")).unwrap();
    s.write(&host("root"), retract("node", "for-root")).unwrap();
    s.write(&host("bob"), correct("public")).unwrap();
}

#[test]
fn a_store_without_a_registry_keeps_its_write_rule() {
    let mut s = Store::new();
    let writer = HostContext::new("etl", ["g".to_string()]);
    s.write(
        &writer,
        json!([assert_(node("restricted", &["someone"], &[]))]),
    )
    .unwrap();
    // The local trusted-host profile is unchanged: `writable_graphs` decides.
    s.write(
        &writer,
        json!([{"change": "retract", "target": {"kind": "node", "id": "restricted"}}]),
    )
    .unwrap();
}

#[test]
fn create_graph_assigns_an_opaque_id_and_grants_the_creator_administer() {
    let mut s = Store::registry();
    let a = s.e.create_graph(&[7; 32], &host("bob")).unwrap();
    assert!(a.starts_with(CREATED_GRAPH_PREFIX) && a.len() == 34);
    let acl = s.e.registry_graph_acl(&a, &host("bob")).unwrap();
    assert_eq!(acl.grants.len(), 1);
    assert_eq!(acl.grants[0].subject, "bob");
    assert_eq!(acl.grants[0].permission, Permission::Administer);
    // Nobody else can discover it.
    assert!(s
        .e
        .list_graphs(&host("alice"))
        .unwrap()
        .iter()
        .all(|g| g.graph_id != a));
    // The creator writes it.
    s.run(
        &host("bob"),
        json!([{"op": "write", "graph_id": a, "changes": [assert_(node("n", &[], &[]))]}]),
    )
    .unwrap();
    // Different entropy, different graph; the same entropy collides.
    let b = s.e.create_graph(&[8; 32], &host("alice")).unwrap();
    assert_ne!(a, b);
    assert_eq!(
        s.e.create_graph(&[7; 32], &host("alice")).unwrap_err().code,
        "E_CONFLICT"
    );
    // Groups and unregistered or suspended principals cannot create graphs.
    for who in ["team", "mallory"] {
        assert_eq!(
            s.e.create_graph(&[9; 32], &host(who)).unwrap_err().code,
            "E_FORBIDDEN"
        );
    }
    s.reg(json!([{"change": "set_status", "id": "bob", "status": "suspended"}]));
    assert_eq!(
        s.e.create_graph(&[10; 32], &host("bob")).unwrap_err().code,
        "E_FORBIDDEN"
    );
    // A store without a registry has no graph creation.
    let plain = Store::new();
    assert_eq!(
        plain.e.create_graph(&[1; 32], &host("x")).unwrap_err().code,
        "E_REGISTRY_DISABLED"
    );
}
