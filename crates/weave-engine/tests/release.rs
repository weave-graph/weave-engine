//! B2 part 3 (#5, ADR-0008 sections 5 and 6): release policies and the
//! influence of restricted text on embeddings.
use serde_json::{json, Value};
use std::sync::Arc;
use weave_contract::{CommandResult, Program, VERSION};
use weave_engine::{
    Engine, HostContext, ManualClock, RegistryChange, ReleaseAudience, ReleaseKind, ReleasePolicy,
    ReleaseSource, TrustedClock,
};

fn program(commands: Value) -> Program {
    serde_json::from_value(json!({"version": VERSION, "commands": commands})).unwrap()
}
fn host(who: &str) -> HostContext {
    HostContext::new(who, Vec::<String>::new())
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
    fn tick(&self) {
        self.clock.set(self.clock.unix_millis().unwrap() + 10);
    }
    /// `count` customers of graph `graph`, each a distinct entity, readable
    /// only by `ops`.
    fn customers(&mut self, writer: &HostContext, graph: &str, count: usize) {
        self.tick();
        let changes: Vec<Value> = (0..count)
            .map(|i| {
                json!({"change": "assert", "fact": {"kind": "node", "id": format!("c{i}"),
                "entity_id": format!("customer-{i}"), "space_id": "crm", "readers": ["ops"]}})
            })
            .collect();
        self.e
            .execute(
                &program(json!([{"op": "write", "graph_id": graph, "changes": changes}])),
                writer,
            )
            .unwrap();
    }
    fn policy(
        &self,
        id: &str,
        graph: &str,
        min_k: u64,
        host: &HostContext,
    ) -> weave_engine::Result<String> {
        self.tick();
        self.e.install_release_policy(
            &ReleasePolicy {
                id: id.into(),
                revision: "1".into(),
                kind: ReleaseKind::AggregateCount { min_k },
                source: ReleaseSource {
                    graph_id: graph.into(),
                    space_id: Some("crm".into()),
                },
                audience: ReleaseAudience {
                    readers: vec!["analyst".into()],
                    reader_groups: vec![],
                },
                evaluator: "ops".into(),
            },
            host,
        )
    }
}

fn nodes(e: &mut Engine, who: &str, graph: &str) -> usize {
    match e
        .execute(
            &program(json!([{"op": "query", "query": {"graph_id": graph}}])),
            &host(who),
        )
        .unwrap()
        .pop()
    {
        Some(CommandResult::Queried { result }) => result.graph.nodes.len(),
        other => panic!("{other:?}"),
    }
}

/// The #5 acceptance test: a k-threshold aggregate is released and a
/// below-threshold aggregate is withheld, and both are audited.
#[test]
fn a_k_threshold_count_is_released_and_a_smaller_one_withheld() {
    let mut s = Store::new();
    let writer = HostContext::new("ops", ["large".to_string(), "small".to_string()]);
    s.customers(&writer, "large", 12);
    s.customers(&writer, "small", 4);
    let trusted = host("host");
    s.policy("large-count", "large", 10, &trusted).unwrap();
    s.policy("small-count", "small", 10, &trusted).unwrap();
    // The analyst cannot read a single customer...
    assert_eq!(nodes(&mut s.e, "analyst", "large"), 0);
    // ...but the policy releases their number, because it is at least k.
    s.tick();
    let released = s.e.release("large-count", &host("analyst")).unwrap();
    assert_eq!(released.released, Some(12));
    // Below k the count is withheld: the analyst learns only "below k".
    s.tick();
    let withheld = s.e.release("small-count", &host("analyst")).unwrap();
    assert_eq!(withheld.released, None);
    // Outside the audience, a policy is unavailable, exactly like a missing one.
    let outsider = s.e.release("large-count", &host("eve")).unwrap_err();
    let missing = s.e.release("no-such-policy", &host("eve")).unwrap_err();
    assert_eq!(
        (outsider.code.as_str(), outsider.message.as_str()),
        (missing.code.as_str(), missing.message.as_str())
    );
    assert_eq!(outsider.code, "E_UNAVAILABLE");
    // Both evaluations are audited, with requester and outcome.
    let audit = s.e.release_audit(&trusted).unwrap();
    let rows: Vec<(String, String, bool)> = audit
        .iter()
        .map(|a| (a.policy_id.clone(), a.requester.clone(), a.released))
        .collect();
    assert_eq!(
        rows,
        [
            ("large-count".to_string(), "analyst".to_string(), true),
            ("small-count".to_string(), "analyst".to_string(), false)
        ]
    );
    assert!(audit[0].recorded_at_ms < audit[1].recorded_at_ms);
    // The policy graph is engine-owned: a program cannot write it.
    let forged = s.e.execute(
        &program(json!([{"op": "write", "graph_id": "weave:release", "changes": [
            {"change": "assert", "fact": {"kind": "node", "id": "x", "entity_id": "X", "space_id": "s"}}]}])),
        &HostContext::new("analyst", ["weave:release".to_string()]),
    );
    assert_eq!(forged.unwrap_err().code, "E_RESERVED_NAMESPACE");
    // Policy and audit records are invisible to ordinary reads.
    assert_eq!(nodes(&mut s.e, "analyst", "weave:release"), 0);
}

#[test]
fn release_policies_are_managed_by_tenant_administrators_in_registry_stores() {
    let mut s = Store::new();
    let admin = host("admin");
    s.e.enable_registry(&admin).unwrap();
    s.tick();
    let changes: Vec<RegistryChange> = serde_json::from_value(json!([
        {"change": "add_principal", "id": "ops", "kind": "user"},
        {"change": "add_principal", "id": "analyst", "kind": "user"},
        {"change": "grant", "graph_id": "large", "subject": "ops", "permission": "write"}
    ]))
    .unwrap();
    s.e.apply_registry(&changes, &admin).unwrap();
    s.customers(&host("ops"), "large", 11);
    assert_eq!(
        s.policy("p", "large", 10, &host("ops")).unwrap_err().code,
        "E_FORBIDDEN"
    );
    s.policy("p", "large", 10, &admin).unwrap();
    s.tick();
    assert_eq!(
        s.e.release("p", &host("analyst")).unwrap().released,
        Some(11)
    );
    assert_eq!(
        s.e.release_audit(&host("analyst")).unwrap_err().code,
        "E_FORBIDDEN"
    );
    assert_eq!(s.e.release_audit(&admin).unwrap().len(), 1);
    // Raising k withholds on the next request; the change is a new revision.
    s.policy("p", "large", 20, &admin).unwrap();
    s.tick();
    assert_eq!(s.e.release("p", &host("analyst")).unwrap().released, None);
    // Revoking the evaluator's access withholds too.
    s.tick();
    let revoke: Vec<RegistryChange> = serde_json::from_value(
        json!([{"change": "revoke", "graph_id": "large", "subject": "ops"}]),
    )
    .unwrap();
    s.e.apply_registry(&revoke, &admin).unwrap();
    s.policy("p", "large", 1, &admin).unwrap();
    s.tick();
    assert_eq!(s.e.release("p", &host("analyst")).unwrap().released, None);
}

/// ADR-0008 section 5: an embedding computed from restricted text that cites
/// it is no broader than the text.
#[test]
fn an_embedding_of_restricted_text_is_no_broader_than_the_text() {
    let mut s = Store::new();
    let writer = HostContext::new("ops", ["g".to_string()]);
    s.tick();
    let out = s
        .e
        .execute(
            &program(json!([{"op": "write", "graph_id": "g", "changes": [
                {"change": "assert", "fact": {"kind": "embedding_space", "id": "sp", "revision": "1",
                    "encoder": "e", "encoder_version": "v", "preprocessing": "p", "dimensions": 2,
                    "metric": "euclidean"}},
                {"change": "assert", "fact": {"kind": "node", "id": "note", "entity_id": "NOTE",
                    "space_id": "s", "readers": ["ops", "customer"]}}
            ]}])),
            &writer,
        )
        .unwrap();
    let Some(CommandResult::Committed { revision, .. }) = out.last() else {
        panic!("{out:?}")
    };
    let embedding = |id: &str, readers: Value| {
        json!([{"op": "write", "graph_id": "g", "changes": [{"change": "assert", "fact": {
            "kind": "embedding", "id": id, "space": {"id": "sp", "revision": "1"},
            "subject": {"kind": "node", "node_id": "note"}, "vector": [1.0, 0.0],
            "valid_time": {"start": 0}, "readers": readers,
            "provenance": {"method": {"kind": "model", "model": "e", "version": "v"},
                "evidence": [{"kind": "node", "graph_id": "g", "revision": revision, "node_id": "note"}]}}}]}])
    };
    s.tick();
    let public =
        s.e.execute(&program(embedding("v1", json!([]))), &writer)
            .unwrap_err();
    assert_eq!(public.code, "E_DERIVED_READERS");
    let wider =
        s.e.execute(
            &program(embedding("v2", json!(["ops", "customer", "eve"]))),
            &writer,
        )
        .unwrap_err();
    assert_eq!(wider.code, "E_DERIVED_READERS");
    s.e.execute(
        &program(embedding("v3", json!(["ops", "customer"]))),
        &writer,
    )
    .unwrap();
    s.e.execute(&program(embedding("v4", json!(["customer"]))), &writer)
        .unwrap();
}
