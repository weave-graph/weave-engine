//! Application-facing recovery uses the same actual owner/state/receipt kernel.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use weave_contract::{
    handler_registration::seal_handler_template, CompiledHandlerTemplate, VERSION,
};
use weave_engine::*;
use weave_native::{artifacts::ArtifactBundle, host::HostSession};

fn authority() -> HostContext {
    HostContext::new("owner", ["input".into(), "output".into()])
}
fn call(s: &mut HostSession, format: &str, operation: Value) -> Value {
    let reply =
        s.call(&serde_json::to_vec(&json!({"format":format,"operation":operation})).unwrap());
    assert!(!reply.poisoned);
    serde_json::from_slice(&reply.bytes).unwrap()
}
fn operation(s: &mut HostSession, op: Value) -> Value {
    let reply = call(s, "weave-host-request/2", op.clone());
    assert!(reply["ok"].as_bool().unwrap(), "{op} => {reply}");
    assert_eq!(reply["requires_fence"], true);
    reply["value"].clone()
}
fn program(graph: &str, value: i64, expected: Option<&str>) -> Value {
    json!({"version":VERSION,"commands":[{"op":"commit","graph_id":graph,"expected_head":expected,"data":{"nodes":[{"id":"n","entity_id":"entity","space_id":"s","properties":{"value":value},"readers":["owner"]}]}}]})
}
fn write(s: &mut HostSession, graph: &str, value: i64, expected: Option<&str>) -> String {
    operation(
        s,
        json!({"kind":"execute","program":program(graph,value,expected)}),
    )[0]["revision"]
        .as_str()
        .unwrap()
        .into()
}
fn definition(id: &str, artifact: &[u8], effects: bool) -> RecordedActorDefinition {
    serde_json::from_value(json!({"manifest":{"id":id,"version":"1","artifact_digest":format!("sha256:{:x}",Sha256::digest(artifact)),"config_revision":"1","principal":"owner","subscriptions":[{"graph_id":"input","branch_id":"main"}],"output_graphs":["output"],"effect_destinations":if effects{vec!["sink"]}else{vec![]},"max_attempts":3,"lease_ms":1000,"max_pending_events":1000,"projection_replay":false},"event_schema":VERSION,"metadata_depth":0,"artifact":artifact})).unwrap()
}
fn state(s: &mut HostSession, adapter: &str, status: &str) {
    operation(
        s,
        json!({"kind":"lifecycle","adapter":adapter,"state":status}),
    );
}
fn bootstrap(s: &mut HostSession, adapter: &str) -> String {
    let inputs = operation(s, json!({"kind":"actor_inputs","adapter":adapter}));
    operation(s, json!({"kind":"actor_bootstrap","request":{"inputs":inputs,"state_revision":"seed","state":{"value":0}}})).as_str().unwrap().into()
}
fn complete(
    s: &mut HostSession,
    adapter: &str,
    event: &Value,
    value: i64,
    expected: Option<&str>,
) -> (Value, Value) {
    let inputs = operation(
        s,
        json!({"kind":"actor_run_inputs","adapter":adapter,"event":event["id"],"lease":event["lease"]}),
    );
    let request = json!({"adapter":adapter,"event":event["id"],"lease":event["lease"],"prior_state_digest":inputs["state_digest"],"state_revision":format!("state-{value}"),"state":{"value":value},"input_snapshots":inputs["input_snapshots"],"tool_results":[{"name":"measured","media_type":"application/json","value":value}],"program":program("output",value,expected)});
    let receipt = operation(s, json!({"kind":"actor_complete","request":request}));
    (request, receipt)
}
fn compiled(s: &mut HostSession) {
    let t:CompiledHandlerTemplate=seal_handler_template(serde_json::from_value(json!({"format":"weave-handler-registration/1","protocol":VERSION,"name":"Identity","revision":"1","input":{"graph_id":"input","branch_id":"main","metadata_depth":0},"event_types":["graph.accepted","graph.committed"],"recipe":{"bindings":[],"output":"$event"},"output_slot":"result","source_revisions":[],"definition_digest":""})).unwrap()).unwrap();
    let artifacts = json!({"program":{"version":VERSION,"commands":[]},"values":{},"view_templates":{},"handler_templates":{"Identity":t}});
    let fingerprint = weave_contract::identity::source_fingerprint(
        &json!({"profile":"weave-compiled-artifacts-v2","artifacts":artifacts}),
    )
    .unwrap();
    let bundle=ArtifactBundle::parse(&serde_json::to_vec(&json!({"format":"weave-compiler-response/1","ok":true,"artifact_fingerprint":fingerprint,"artifacts":artifacts})).unwrap()).unwrap();
    let mut m = definition("compiled", b"placeholder", false).manifest;
    m.artifact_digest = t.definition_digest.clone();
    m.projection_replay = true;
    let reply = s.install_compiled_handler(
        &bundle,
        "Identity",
        &m,
        &HandlerOutputBinding {
            slot: "result".into(),
            graph_id: "output".into(),
            branch_id: "main".into(),
        },
    );
    assert!(serde_json::from_slice::<Value>(&reply.bytes).unwrap()["ok"]
        .as_bool()
        .unwrap());
}

#[test]
fn capabilities_negotiate_v2_and_v1_rejects_new_mutations_before_clock_or_store_work() {
    let clock = Arc::new(ManualClock::new(10));
    let mut s = HostSession::new(
        Engine::memory_with_clock(clock.clone()).unwrap(),
        authority(),
    )
    .unwrap();
    compiled(&mut s);
    let samples = clock.samples();
    let rejected = call(
        &mut s,
        "weave-host-request/1",
        json!({"kind":"lifecycle","adapter":"compiled","state":"running"}),
    );
    assert_eq!(rejected["error"]["code"], "E_HOST_VERSION");
    assert_eq!(rejected["requires_fence"], false);
    assert_eq!(clock.samples(), samples);
    let capability = operation(&mut s, json!({"kind":"capabilities"}));
    assert_eq!(capability["protocol"], VERSION);
    assert_eq!(capability["store_marker"], STORAGE_VERSION);
    let names = capability["operations"].as_array().unwrap();
    assert_eq!(names.len(), 31);
    assert_eq!(
        names.iter().collect::<std::collections::HashSet<_>>().len(),
        31
    );
    assert_eq!(
        call(
            &mut s,
            "weave-host-request/2",
            json!({"kind":"poll","adapter":"compiled"})
        )["error"]["code"],
        "E_PAUSED"
    );
    assert_eq!(
        call(
            &mut s,
            "weave-host-request/2",
            json!({"kind":"install_recorded_actor","definition":definition("invented",b"tool",false)})
        )["error"]["code"],
        "E_HOST_INPUT"
    );
}

#[test]
fn oversized_completion_programs_reject_before_clock_authority_or_storage_work() {
    let clock = Arc::new(ManualClock::new(10));
    let mut session = HostSession::new(
        Engine::memory_with_clock(clock.clone()).unwrap(),
        authority(),
    )
    .unwrap();
    let samples = clock.samples();
    for kind in ["actor_complete", "projection_complete"] {
        let mut oversized = program("output", 1, None);
        oversized["commands"] = json!(vec![oversized["commands"][0].clone(); 17]);
        let mut request = json!({"adapter":"not-installed","event":"event","lease":"lease","prior_state_digest":"digest","state_revision":"state","state":{},"input_snapshots":[],"program":oversized});
        if kind == "actor_complete" {
            request["tool_results"] = json!([]);
        }
        let reply = call(
            &mut session,
            "weave-host-request/2",
            json!({"kind":kind,"request":request}),
        );
        assert_eq!(reply["error"]["code"], "E_HOST_BUDGET");
        assert_eq!(reply["requires_fence"], false);
        assert_eq!(clock.samples(), samples);
    }
    assert!(!session.is_poisoned());
}

#[test]
fn actual_compiled_cancellation_rebuild_and_old_retry_survive_application_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("compiled.sqlite");
    let mut s = HostSession::new(Engine::open(&path).unwrap(), authority()).unwrap();
    let input = write(&mut s, "input", 0, None);
    compiled(&mut s);
    state(&mut s, "compiled", "paused");
    let inputs = operation(
        &mut s,
        json!({"kind":"compiled_rebuild_inputs","adapter":"compiled"}),
    );
    operation(
        &mut s,
        json!({"kind":"compiled_rebuild","request":{"inputs":inputs,"nonce":"initial"}}),
    );
    write(&mut s, "input", 1, Some(&input));
    state(&mut s, "compiled", "running");
    let event = operation(&mut s, json!({"kind":"poll","adapter":"compiled"}));
    let request = json!({"adapter":"compiled","event":event["id"],"expected_lease":event["lease"],"nonce":"dispose","reason":"owner_stop"});
    let receipt = operation(&mut s, json!({"kind":"cancel_handler","request":request}));
    assert_eq!(receipt["rebuild_required"], true);
    state(&mut s, "compiled", "paused");
    let inputs = operation(
        &mut s,
        json!({"kind":"compiled_rebuild_inputs","adapter":"compiled"}),
    );
    let rebuilt = operation(
        &mut s,
        json!({"kind":"compiled_rebuild","request":{"inputs":inputs,"nonce":"reconstruct"}}),
    );
    let c = rusqlite::Connection::open(&path).unwrap();
    let revision: String = c
        .query_row(
            "SELECT revision FROM heads WHERE graph_id='output' AND branch_id='main'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rebuilt["output"]["revision"], revision);
    drop(s);
    let mut s = HostSession::new(Engine::open(&path).unwrap(), authority()).unwrap();
    let old = operation(&mut s, json!({"kind":"cancel_handler","request":request}));
    assert_eq!(old["duplicate"], true);
    state(&mut s, "compiled", "running");
    assert!(operation(&mut s, json!({"kind":"poll","adapter":"compiled"})).is_null());
    assert_eq!(
        c.query_row(
            "SELECT revision FROM heads WHERE graph_id='output' AND branch_id='main'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        revision
    );
    assert_eq!(
        operation(&mut s, json!({"kind":"lag","adapter":"compiled"}))
            ["visible_backlog_lower_bound"],
        0
    );
}

#[test]
fn actual_actor_completion_upgrade_rollback_and_default_observation_use_the_host_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("actor.sqlite");
    let mut s = HostSession::new(Engine::open(&path).unwrap(), authority()).unwrap();
    let seed = write(&mut s, "input", 0, None);
    let first = definition("actor", b"actual native artifact v1", false);
    let r = s.install_recorded_actor(&first);
    assert!(serde_json::from_slice::<Value>(&r.bytes).unwrap()["ok"]
        .as_bool()
        .unwrap());
    bootstrap(&mut s, "actor");
    let input = write(&mut s, "input", 1, Some(&seed));
    state(&mut s, "actor", "running");
    let event = operation(&mut s, json!({"kind":"poll","adapter":"actor"}));
    assert_eq!(
        operation(
            &mut s,
            json!({"kind":"actor_delivery_mode","adapter":"actor","event":event["id"],"lease":event["lease"]})
        ),
        "compute"
    );
    let (request, receipt) = complete(&mut s, "actor", &event, 1, None);
    let output = receipt["handler"]["results"][0]["revision"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        operation(
            &mut s,
            json!({"kind":"actor_receipt","adapter":"actor","event":event["id"]})
        ),
        receipt
    );
    assert_eq!(
        operation(&mut s, json!({"kind":"actor_complete","request":request}))["handler"]
            ["duplicate"],
        true
    );
    state(&mut s, "actor", "paused");
    let inputs = operation(
        &mut s,
        json!({"kind":"actor_migration_inputs","adapter":"actor"}),
    );
    let second = definition("actor-v2", b"actual native artifact v2", false);
    operation(
        &mut s,
        json!({"kind":"actor_migrate","request":{"inputs":inputs,"destination":second,"nonce":"upgrade","disposition":{"kind":"upgrade"}}}),
    );
    state(&mut s, "actor-v2", "running");
    write(&mut s, "input", 2, Some(&input));
    let event = operation(&mut s, json!({"kind":"poll","adapter":"actor-v2"}));
    let (_, second_receipt) = complete(&mut s, "actor-v2", &event, 2, Some(&output));
    let output = second_receipt["handler"]["results"][0]["revision"]
        .as_str()
        .unwrap()
        .to_string();
    state(&mut s, "actor-v2", "paused");
    let inputs = operation(
        &mut s,
        json!({"kind":"actor_migration_inputs","adapter":"actor-v2"}),
    );
    let restored = definition("restored", b"actual native artifact v1", false);
    let transfer = operation(
        &mut s,
        json!({"kind":"actor_migrate","request":{"inputs":inputs,"destination":restored,"nonce":"rollback","disposition":{"kind":"rollback","restore_from":"actor"}}}),
    );
    drop(s);
    let mut s = HostSession::new(Engine::open(&path).unwrap(), authority()).unwrap();
    state(&mut s, "restored", "running");
    let historical = operation(&mut s, json!({"kind":"poll","adapter":"restored"}));
    assert_eq!(
        operation(
            &mut s,
            json!({"kind":"actor_delivery_mode","adapter":"restored","event":historical["id"],"lease":historical["lease"]})
        ),
        "observe"
    );
    assert_eq!(
        call(
            &mut s,
            "weave-host-request/2",
            json!({"kind":"actor_run_inputs","adapter":"restored","event":historical["id"],"lease":historical["lease"]})
        )["ok"],
        false
    );
    let observed = operation(
        &mut s,
        json!({"kind":"actor_observe","request":{"adapter":"restored","event":historical["id"],"lease":historical["lease"],"prior_state_digest":transfer["state_digest"],"nonce":"observe"}}),
    );
    assert_eq!(observed["source_receipt"], second_receipt);
    assert_eq!(
        operation(
            &mut s,
            json!({"kind":"actor_observation","adapter":"restored","event":historical["id"]})
        ),
        observed
    );
    assert_eq!(
        operation(&mut s, json!({"kind":"actor_state","adapter":"restored"}))["state"]["value"],
        2
    );
    let c = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        c.query_row(
            "SELECT revision FROM heads WHERE graph_id='output' AND branch_id='main'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        output
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM events", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        5
    );
}

#[test]
fn foreign_and_narrow_application_sessions_cannot_read_or_mutate_actual_actor_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("authority.sqlite");
    let clock = Arc::new(ManualClock::new(10));
    let mut owner = HostSession::new(
        Engine::open_with_clock(&path, clock.clone()).unwrap(),
        authority(),
    )
    .unwrap();
    let seed = write(&mut owner, "input", 0, None);
    let r = owner.install_recorded_actor(&definition("actor", b"native tool", false));
    assert!(serde_json::from_slice::<Value>(&r.bytes).unwrap()["ok"]
        .as_bool()
        .unwrap());
    bootstrap(&mut owner, "actor");
    write(&mut owner, "input", 1, Some(&seed));
    state(&mut owner, "actor", "running");
    let event = operation(&mut owner, json!({"kind":"poll","adapter":"actor"}));
    let before = operation(&mut owner, json!({"kind":"actor_state","adapter":"actor"}));
    let c = rusqlite::Connection::open(&path).unwrap();
    let prior: String = c
        .query_row(
            "SELECT body FROM recorded_actor_states WHERE adapter='actor'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    for host in [
        HostContext::new("foreign", ["output".into()]),
        HostContext::new("owner", []),
    ] {
        let mut session =
            HostSession::new(Engine::open_with_clock(&path, clock.clone()).unwrap(), host).unwrap();
        for request in [
            json!({"kind":"lifecycle","adapter":"actor","state":"paused"}),
            json!({"kind":"lag","adapter":"actor"}),
            json!({"kind":"actor_inputs","adapter":"actor"}),
            json!({"kind":"actor_state","adapter":"actor"}),
            json!({"kind":"actor_migration_inputs","adapter":"actor"}),
            json!({"kind":"actor_run_inputs","adapter":"actor","event":event["id"],"lease":event["lease"]}),
            json!({"kind":"actor_delivery_mode","adapter":"actor","event":event["id"],"lease":event["lease"]}),
            json!({"kind":"actor_cancel","request":{"adapter":"actor","event":event["id"],"expected_lease":event["lease"],"nonce":"foreign","reason":"owner_stop"}}),
        ] {
            let reply = call(&mut session, "weave-host-request/2", request);
            assert_eq!(reply["error"]["code"], "E_HOST_AUTH");
            assert!(reply.get("value").is_none());
        }
    }
    assert_eq!(
        operation(&mut owner, json!({"kind":"actor_state","adapter":"actor"})),
        before
    );
    assert_eq!(
        c.query_row(
            "SELECT body FROM recorded_actor_states WHERE adapter='actor'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        prior
    );
    assert_eq!(
        c.query_row(
            "SELECT state FROM dispatch_adapters WHERE id='actor'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "running"
    );
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM recorded_actor_cancellations",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn host_actor_cancellation_preserves_unknown_effects_until_trusted_broker_reconciliation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("unknown.sqlite");
    let mut e = Engine::open(&path).unwrap();
    e.execute(
        &serde_json::from_value(program("input", 0, None)).unwrap(),
        &authority(),
    )
    .unwrap();
    let seed = e.head("input", "main").unwrap().unwrap();
    e.install_recorded_actor_for(&definition("actor", b"effect tool", true), &authority())
        .unwrap();
    e.bootstrap_recorded_actor_for(
        &RecordedActorBootstrap {
            inputs: e.recorded_actor_inputs_for("actor", &authority()).unwrap(),
            state_revision: "seed".into(),
            state: json!({"value":0}),
        },
        &authority(),
    )
    .unwrap();
    e.execute(
        &serde_json::from_value(program("input", 1, Some(&seed))).unwrap(),
        &authority(),
    )
    .unwrap();
    e.set_adapter_state_for("actor", "running", &authority())
        .unwrap();
    let event = e.poll_adapter_for("actor", &authority()).unwrap().unwrap();
    let intent = e
        .request_effect(
            "actor",
            &event.id,
            &event.lease,
            "sink",
            "action",
            json!({"request":1}),
        )
        .unwrap();
    e.begin_effect_dispatch(&intent.id).unwrap();
    drop(e);
    let request = json!({"adapter":"actor","event":event.id,"expected_lease":event.lease,"nonce":"dispose","reason":"owner_stop"});
    let mut s = HostSession::new(Engine::open(&path).unwrap(), authority()).unwrap();
    assert_eq!(
        operation(&mut s, json!({"kind":"lag","adapter":"actor"}))["visible_unknown_effects"],
        1
    );
    assert_eq!(
        call(
            &mut s,
            "weave-host-request/2",
            json!({"kind":"actor_cancel","request":request})
        )["error"]["code"],
        "E_EFFECT_UNKNOWN"
    );
    assert_eq!(
        call(
            &mut s,
            "weave-host-request/2",
            json!({"kind":"reconcile_effect","id":intent.id,"outcome":"confirmed","response":{}})
        )["error"]["code"],
        "E_HOST_INPUT"
    );
    drop(s);
    let broker = Engine::open(&path).unwrap();
    assert_eq!(
        broker.effect_intent(&intent.id).unwrap().unwrap().state,
        "unknown"
    );
    // This unit fixture records a trusted ledger outcome; it does not perform physical I/O.
    broker
        .reconcile_effect(&intent.id, "confirmed", json!({"fixture_outcome":true}))
        .unwrap();
    let terminal = broker.effect_intent(&intent.id).unwrap().unwrap();
    drop(broker);
    let mut s = HostSession::new(Engine::open(&path).unwrap(), authority()).unwrap();
    assert_eq!(
        operation(&mut s, json!({"kind":"actor_cancel","request":request}))["rebuild_required"],
        true
    );
    assert_eq!(
        operation(&mut s, json!({"kind":"actor_cancel","request":request}))["duplicate"],
        true
    );
    drop(s);
    assert_eq!(
        Engine::open(&path)
            .unwrap()
            .effect_intent(&intent.id)
            .unwrap()
            .unwrap(),
        terminal
    );
}
