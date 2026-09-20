use serde_json::json;
use weave_contract::{
    handler_registration::seal_handler_template, CompiledHandlerTemplate, Program, VERSION,
};
use weave_engine::{AdapterManifest, Engine, HandlerOutputBinding, HostContext};
fn template() -> CompiledHandlerTemplate {
    seal_handler_template(serde_json::from_value(json!({"format":"weave-handler-registration/1","protocol":VERSION,"name":"identity","revision":"1","input":{"graph_id":"input","branch_id":"main","metadata_depth":0},"event_types":["graph.accepted","graph.committed"],"recipe":{"bindings":[],"output":"$event"},"output_slot":"out","source_revisions":[],"definition_digest":""})).unwrap()).unwrap()
}
fn manifest(t: &CompiledHandlerTemplate) -> AdapterManifest {
    serde_json::from_value(json!({"id":"compiled","version":"1","artifact_digest":t.definition_digest,"config_revision":"1","principal":"alice","subscriptions":[{"graph_id":"input","branch_id":"main"}],"output_graphs":["output"],"effect_destinations":[],"max_attempts":5,"lease_ms":1000,"max_pending_events":10,"projection_replay":true})).unwrap()
}
fn host() -> HostContext {
    HostContext::new("alice", ["input".into(), "output".into()])
}
fn output() -> HandlerOutputBinding {
    HandlerOutputBinding {
        slot: "out".into(),
        graph_id: "output".into(),
        branch_id: "main".into(),
    }
}
#[test]
fn registry_is_immutable_and_raw_completion_is_closed() {
    let mut e = Engine::memory().unwrap();
    let t = template();
    let m = manifest(&t);
    e.install_compiled_handler(&m, &t, &output(), &host())
        .unwrap();
    e.install_compiled_handler(&m, &t, &output(), &host())
        .unwrap();
    let p: Program = serde_json::from_value(json!({"version":VERSION,"commands":[]})).unwrap();
    assert_eq!(
        e.complete_handler(&m.id, "invented", "invented", &p)
            .unwrap_err()
            .code,
        "E_HANDLER_BOUND"
    );
    let mut changed = m.clone();
    changed.config_revision = "2".into();
    assert!(e
        .install_compiled_handler(&changed, &t, &output(), &host())
        .is_err());
    assert_eq!(e.event_count().unwrap(), 0);
}
#[test]
fn installation_does_not_adopt_legacy_or_grant_output_authority() {
    let e = Engine::memory().unwrap();
    let t = template();
    let m = manifest(&t);
    assert!(e
        .install_compiled_handler(&m, &t, &output(), &HostContext::new("alice", []))
        .is_err());
    e.install_adapter(&m, &host()).unwrap();
    assert_eq!(
        e.install_compiled_handler(&m, &t, &output(), &host())
            .unwrap_err()
            .code,
        "E_HANDLER_INSTALL"
    );
}

fn write(e: &mut Engine, id: &str, data: serde_json::Value) -> weave_contract::GraphRef {
    let p:Program=serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":id,"expected_head":e.head(id,"main").unwrap(),"data":data}]})).unwrap();
    e.execute(&p, &HostContext::new("alice", [id.into()]))
        .unwrap();
    weave_contract::GraphRef {
        graph_id: id.into(),
        revision: e.head(id, "main").unwrap().unwrap(),
    }
}
fn setup(e: &Engine, t: &CompiledHandlerTemplate) {
    e.install_compiled_handler(&manifest(t), t, &output(), &host())
        .unwrap();
    e.set_adapter_state("compiled", "running").unwrap();
}
fn query(e: &Engine, id: &str) -> weave_contract::QueryResult {
    e.query(
        &serde_json::from_value(json!({"graph_id":id,"include_metadata":true})).unwrap(),
        &host(),
    )
    .unwrap()
}
fn attachment(value: serde_json::Value) -> serde_json::Value {
    json!({"id":"detail","host":{"kind":"graph"},"key":"detail","value":value,"valid_time":{"start":0,"end":null}})
}
#[test]
fn preload_requires_whole_authority_and_rejects_live_or_truncated_input() {
    for mode in ["private", "live", "truncated"] {
        let mut e = Engine::memory().unwrap();
        let mut t = template();
        t.input.metadata_depth = 1;
        t.source_revisions.clear();
        let leaf = write(&mut e, "leaf", json!({}));
        let data = if mode == "private" {
            json!({"nodes":[{"id":"secret","entity_id":"s","space_id":"s","readers":["bob"]}]})
        } else {
            json!({"attachments":[attachment(json!({"kind":"graph","reference":leaf}))]})
        };
        let meta = write(&mut e, "meta", data);
        let value = if mode == "live" {
            json!({"kind":"live_graph","graph_id":"meta","branch_id":"main"})
        } else {
            json!({"kind":"graph","reference":meta})
        };
        write(&mut e, "input", json!({"attachments":[attachment(value)]}));
        t = seal_handler_template(t).unwrap();
        setup(&e, &t);
        let d = e.poll_adapter("compiled").unwrap().unwrap();
        let events = e.event_count().unwrap();
        assert!(
            e.prepare_compiled_handler("compiled", &d.id, &d.lease)
                .is_err(),
            "{mode}"
        );
        assert_eq!(e.event_count().unwrap(), events);
        assert!(e.head("output", "main").unwrap().is_none());
    }
}
#[test]
fn whole_preload_and_record_gates_survive_owned_output_and_reader_stripping() {
    let mut e = Engine::memory().unwrap();
    let meta = write(&mut e, "meta", json!({}));
    let input = write(
        &mut e,
        "input",
        json!({"nodes":[{"id":"n","entity_id":"E","space_id":"s"}],"attachments":[attachment(json!({"kind":"graph","reference":meta}))]}),
    );
    let mut t = template();
    t.input.metadata_depth = 1;
    t.source_revisions.clear();
    t = seal_handler_template(t).unwrap();
    setup(&e, &t);
    let d = e.poll_adapter("compiled").unwrap().unwrap();
    let p = e
        .prepare_compiled_handler("compiled", &d.id, &d.lease)
        .unwrap();
    e.complete_prepared_handler("compiled", &d.id, &d.lease, &p.preparation_id)
        .unwrap();
    let result = query(&e, "output");
    assert_eq!(result.graph.nodes.len(), 1);
    assert_ne!(result.graph.nodes[0].id, "n");
    assert_eq!(result.graph.nodes[0].entity_id, "E");
    for pin in [&input, &meta] {
        assert!(result
            .graph
            .influence
            .as_ref()
            .unwrap()
            .snapshots
            .contains(pin));
        assert!(result.graph.nodes[0].derived_snapshots.contains(pin));
        assert!(result
            .graph
            .attachments
            .iter()
            .all(|a| a.derived_snapshots.contains(pin)));
    }
    assert!(
        result.source_revisions.is_empty(),
        "ordinary descriptive attribution must not authenticate source manifest"
    );
    let mut copied = result.graph;
    copied.influence = None;
    copied.attachments.clear();
    for n in &mut copied.nodes {
        n.readers.clear();
    }
    write(&mut e, "copied", serde_json::to_value(copied).unwrap());
    assert_eq!(query(&e, "copied").graph.nodes.len(), 1);
}
#[test]
fn successive_occurrences_get_distinct_owned_local_ids_and_exact_entity_identity() {
    let mut e = Engine::memory().unwrap();
    write(
        &mut e,
        "input",
        json!({"nodes":[{"id":"n","entity_id":"E","space_id":"s"}]}),
    );
    setup(&e, &template());
    let mut ids = vec![];
    for version in [1, 2] {
        let d = e.poll_adapter("compiled").unwrap().unwrap();
        let p = e
            .prepare_compiled_handler("compiled", &d.id, &d.lease)
            .unwrap();
        e.complete_prepared_handler("compiled", &d.id, &d.lease, &p.preparation_id)
            .unwrap();
        let r = query(&e, "output");
        ids.push(r.graph.nodes[0].id.clone());
        assert_eq!(r.graph.nodes[0].entity_id, "E");
        if version == 1 {
            write(
                &mut e,
                "input",
                json!({"nodes":[{"id":"n","entity_id":"E","space_id":"s","properties":{"version":2}}]}),
            );
        }
    }
    assert_ne!(ids[0], ids[1]);
}
#[test]
fn lifecycle_and_forged_preparation_never_reuse_historical_receipt() {
    let mut e = Engine::memory().unwrap();
    write(&mut e, "input", json!({}));
    setup(&e, &template());
    let d = e.poll_adapter("compiled").unwrap().unwrap();
    let p = e
        .prepare_compiled_handler("compiled", &d.id, &d.lease)
        .unwrap();
    assert!(e
        .complete_prepared_handler("compiled", &d.id, &d.lease, "forged")
        .is_err());
    e.set_adapter_state("compiled", "paused").unwrap();
    assert!(e
        .prepare_compiled_handler("compiled", &d.id, &d.lease)
        .is_err());
    e.set_adapter_state("compiled", "running").unwrap();
    e.complete_prepared_handler("compiled", &d.id, &d.lease, &p.preparation_id)
        .unwrap();
    e.set_adapter_state("compiled", "paused").unwrap();
    assert!(e
        .complete_prepared_handler("compiled", &d.id, &d.lease, &p.preparation_id)
        .is_err());
    e.set_adapter_state("compiled", "running").unwrap();
    assert!(
        e.complete_prepared_handler("compiled", &d.id, &d.lease, &p.preparation_id)
            .unwrap()
            .duplicate
    );
}
#[cfg(feature = "recovery-testing")]
#[test]
fn preparation_and_completion_observer_unwinds_roll_back_same_engine() {
    let mut e = Engine::memory().unwrap();
    write(&mut e, "input", json!({}));
    setup(&e, &template());
    let d = e.poll_adapter("compiled").unwrap().unwrap();
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| e
        .prepare_compiled_handler_test_before_commit("compiled", &d.id, &d.lease, || panic!(
            "prepare observer"
        ))))
    .is_err());
    let p = e
        .prepare_compiled_handler("compiled", &d.id, &d.lease)
        .unwrap();
    assert!(!p.duplicate);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| e
        .complete_prepared_handler_test_before_commit(
            "compiled",
            &d.id,
            &d.lease,
            &p.preparation_id,
            || panic!("complete observer")
        )))
    .is_err());
    assert_eq!(e.event_count().unwrap(), 1);
    assert!(e.head("output", "main").unwrap().is_none());
    assert!(
        !e.complete_prepared_handler("compiled", &d.id, &d.lease, &p.preparation_id)
            .unwrap()
            .duplicate
    );
}
#[test]
fn canonical_body_corruption_and_retained_quota_fail_before_outputs() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("handler.sqlite");
    let mut e = Engine::open(&path).unwrap();
    write(&mut e, "input", json!({}));
    setup(&e, &template());
    let d = e.poll_adapter("compiled").unwrap().unwrap();
    let p = e
        .prepare_compiled_handler("compiled", &d.id, &d.lease)
        .unwrap();
    let sql = rusqlite::Connection::open(&path).unwrap();
    let original: String = sql
        .query_row("SELECT body FROM handler_preparations", [], |r| r.get(0))
        .unwrap();
    let mut v: serde_json::Value = serde_json::from_str(&original).unwrap();
    v["closure"] = json!([]);
    sql.execute("UPDATE handler_preparations SET body=?1", [v.to_string()])
        .unwrap();
    assert_eq!(
        e.complete_prepared_handler("compiled", &d.id, &d.lease, &p.preparation_id)
            .unwrap_err()
            .code,
        "E_HANDLER_INTEGRITY"
    );
    assert_eq!(e.event_count().unwrap(), 1);
    sql.execute("UPDATE handler_preparations SET body=?1", [original])
        .unwrap();
    e.complete_prepared_handler("compiled", &d.id, &d.lease, &p.preparation_id)
        .unwrap();
    write(
        &mut e,
        "input",
        json!({"nodes":[{"id":"n","entity_id":"n","space_id":"s"}]}),
    );
    for i in 0..255 {
        sql.execute(
            "INSERT INTO handler_preparations VALUES ('compiled',?1,'alice',?2,'{}','bad')",
            rusqlite::params![format!("retained-{i}"), format!("opaque-{i}")],
        )
        .unwrap();
    }
    let next = e.poll_adapter("compiled").unwrap().unwrap();
    let count = e.event_count().unwrap();
    assert_eq!(
        e.prepare_compiled_handler("compiled", &next.id, &next.lease)
            .unwrap_err()
            .code,
        "E_BACKPRESSURE"
    );
    assert_eq!(e.event_count().unwrap(), count);
}
#[test]
fn owned_identity_records_preserve_original_object_provenance() {
    let mut e = Engine::memory().unwrap();
    let source = write(
        &mut e,
        "input",
        json!({"nodes":[{"id":"a","entity_id":"A","space_id":"s"},{"id":"b","entity_id":"B","space_id":"s"}],"edges":[{"id":"claim","predicate":"link","from":"a","to":"b","valid_time":{"start":0,"end":null}}],"attachments":[attachment(json!({"kind":"literal","value":"authored"}))]}),
    );
    setup(&e, &template());
    let d = e.poll_adapter("compiled").unwrap().unwrap();
    let p = e
        .prepare_compiled_handler("compiled", &d.id, &d.lease)
        .unwrap();
    e.complete_prepared_handler("compiled", &d.id, &d.lease, &p.preparation_id)
        .unwrap();
    let r = query(&e, "output");
    let claim = weave_contract::AssertionRef {
        graph_id: source.graph_id.clone(),
        revision: source.revision.clone(),
        assertion_id: "claim".into(),
    };
    assert!(r.graph.edges[0]
        .derivations
        .iter()
        .all(|g| g.premises.contains(&claim)));
    assert!(r.graph.nodes.iter().all(|n| n
        .derived_nodes
        .iter()
        .any(|p| p.graph_id == source.graph_id
            && p.revision == source.revision
            && ["a", "b"].contains(&p.node_id.as_str()))));
    assert!(r
        .graph
        .attachments
        .iter()
        .find(|a| a.key == "detail")
        .unwrap()
        .derived_from
        .iter()
        .any(|p| p.graph_id == source.graph_id
            && p.revision == source.revision
            && p.assertion_id == "detail"));
    let explain:Program=serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"evaluate","value":{"kind":"explain","input":{"kind":"query","query":{"graph_id":"output"}}}}]})).unwrap();
    let result = serde_json::to_value(e.execute(&explain, &host()).unwrap()).unwrap();
    let encoded = result.to_string();
    assert!(encoded.contains(&source.revision));
    assert!(encoded.contains("claim"));
}
