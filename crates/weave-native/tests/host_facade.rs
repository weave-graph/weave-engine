use serde_json::{json, Value};
use std::sync::Arc;
use weave_contract::{
    handler_registration::seal_handler_template, CompiledHandlerTemplate, Program,
};
use weave_engine::{AdapterManifest, Engine, HandlerOutputBinding, HostContext, ManualClock};
use weave_native::{
    artifacts::{ArtifactBundle, SDK_RESPONSE_LIMIT},
    host::HostSession,
};
fn host() -> HostContext {
    HostContext::new("owner", ["input".into(), "output".into()])
}
fn template() -> CompiledHandlerTemplate {
    seal_handler_template(serde_json::from_value(json!({
        "format":"weave-handler-registration/1","protocol":weave_contract::VERSION,"name":"Identity","revision":"1",
        "input":{"graph_id":"input","branch_id":"main","metadata_depth":0},
        "event_types":["graph.accepted","graph.committed"],"recipe":{"bindings":[],"output":"$event"},
        "output_slot":"result","source_revisions":[],"definition_digest":""
    })).unwrap()).unwrap()
}
fn envelope(artifacts: Value) -> Vec<u8> {
    let fingerprint = weave_contract::identity::source_fingerprint(&json!({
        "profile":if artifacts.get("handler_templates").is_some() {"weave-compiled-artifacts-v2"}else{"weave-compiled-artifacts-v1"},
        "artifacts":artifacts
    })).unwrap();
    serde_json::to_vec(&json!({"format":"weave-compiler-response/1","ok":true,"artifact_fingerprint":fingerprint,"artifacts":artifacts})).unwrap()
}
fn artifacts() -> Value {
    json!({"program":{"version":weave_contract::VERSION,"commands":[]},"values":{"Exact":9007199254740993i64},"view_templates":{}})
}
fn reply(s: &mut HostSession, operation: Value) -> Value {
    let r = s.call(
        &serde_json::to_vec(&json!({"format":"weave-host-request/1","operation":operation}))
            .unwrap(),
    );
    assert!(!r.poisoned);
    serde_json::from_slice(&r.bytes).unwrap()
}
#[test]
fn complete_sdk_inventory_and_large_integer_are_lossless() {
    let mut data = artifacts();
    data["handler_templates"] = json!({"Identity":template()});
    let bytes = envelope(data);
    let bundle = ArtifactBundle::parse(&bytes).unwrap();
    assert_eq!(bundle.original_bytes(), bytes);
    assert_eq!(bundle.inventory().values, ["Exact"]);
    assert_eq!(bundle.inventory().handler_templates, ["Identity"]);
    assert_eq!(
        serde_json::from_slice::<Value>(bundle.original_bytes()).unwrap()["artifacts"]["values"]
            ["Exact"]
            .as_i64(),
        Some(9007199254740993)
    );
    assert_eq!(
        serde_json::from_slice::<CompiledHandlerTemplate>(
            bundle.handler_bytes("Identity").unwrap()
        )
        .unwrap(),
        template()
    );
    assert!(bundle.view_bytes("Identity").is_none());
    assert_eq!(
        serde_json::from_slice::<Program>(bundle.program_bytes())
            .unwrap()
            .commands
            .len(),
        0
    );
}
#[test]
fn sdk_shape_duplicate_decoded_keys_and_fingerprint_tamper_fail() {
    let bytes = envelope(artifacts());
    let text = String::from_utf8(bytes).unwrap();
    let duplicate = text.replace(
        "\"Exact\":9007199254740993",
        "\"Exact\":9007199254740993,\"Ex\\u0061ct\":1",
    );
    assert!(ArtifactBundle::parse(duplicate.as_bytes()).is_err());
    let mut altered: Value = serde_json::from_str(&text).unwrap();
    altered["artifacts"]["values"]["Exact"] = json!(1);
    assert!(ArtifactBundle::parse(&serde_json::to_vec(&altered).unwrap()).is_err());
    for extra in [Value::Null, json!({})] {
        let mut data = artifacts();
        data["handler_templates"] = extra;
        assert!(ArtifactBundle::parse(&envelope(data)).is_err());
    }
    let mut data = artifacts();
    data["handler_templates"] = json!({"WrongName":template()});
    assert!(ArtifactBundle::parse(&envelope(data)).is_err());
    altered["format"] = json!("unknown");
    assert!(ArtifactBundle::parse(&serde_json::to_vec(&altered).unwrap()).is_err());
}
#[test]
fn aggregate_inventory_precharge_and_separate_sdk_cap() {
    let mut data = artifacts();
    data["values"] = Value::Object((0..1000).map(|n| (format!("v{n}"), json!(n))).collect());
    assert!(ArtifactBundle::parse(&envelope(data.clone())).is_ok());
    data["handler_templates"] = json!({"Identity":template()});
    assert_eq!(
        ArtifactBundle::parse(&envelope(data)).err().unwrap().code,
        "E_HOST_BUDGET"
    );
    let mut padded = envelope(artifacts());
    padded.resize(SDK_RESPONSE_LIMIT, b' ');
    let bundle = ArtifactBundle::parse(&padded).unwrap();
    assert_eq!(bundle.original_bytes(), padded);
    let mut s = HostSession::new(Engine::memory().unwrap(), host()).unwrap();
    let too_large = s.call(&padded);
    assert!(!too_large.requires_fence);
    assert_eq!(
        serde_json::from_slice::<Value>(&too_large.bytes).unwrap()["error"]["code"],
        "E_HOST_BUDGET"
    );
    padded.push(b' ');
    assert_eq!(
        ArtifactBundle::parse(&padded).err().unwrap().code,
        "E_HOST_BUDGET"
    );
}
#[test]
fn foreign_or_narrow_session_cannot_poll_prepare_complete_or_replay_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let clock = Arc::new(ManualClock::new(10));
    let engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    let mut owner = HostSession::new(engine, host()).unwrap();
    assert_eq!(
        reply(
            &mut owner,
            json!({"kind":"execute","program":{"version":weave_contract::VERSION,"commands":[{"op":"commit","graph_id":"input","data":{}}]}})
        )["ok"],
        true
    );
    let t = template();
    let mut data = artifacts();
    data["handler_templates"] = json!({"Identity":t});
    let bundle = ArtifactBundle::parse(&envelope(data)).unwrap();
    let m: AdapterManifest = serde_json::from_value(json!({"id":"adapter","version":"1","artifact_digest":t.definition_digest,"config_revision":"1","principal":"owner",
        "subscriptions":[{"graph_id":"input","branch_id":"main"}],"output_graphs":["output"],"effect_destinations":[],"max_attempts":5,"lease_ms":100,"max_pending_events":10,"projection_replay":true})).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(
            &owner
                .install_compiled_handler(
                    &bundle,
                    "Identity",
                    &m,
                    &HandlerOutputBinding {
                        slot: "result".into(),
                        graph_id: "output".into(),
                        branch_id: "main".into()
                    }
                )
                .bytes
        )
        .unwrap()["ok"],
        true
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&owner.set_adapter_state("adapter", "running").bytes)
            .unwrap()["ok"],
        true
    );
    let delivery = reply(&mut owner, json!({"kind":"poll","adapter":"adapter"}))["value"].clone();
    let prepare = json!({"kind":"prepare","adapter":"adapter","event":delivery["id"],"lease":delivery["lease"]});
    let prepared = reply(&mut owner, prepare.clone())["value"].clone();
    let complete = json!({"kind":"complete","adapter":"adapter","event":delivery["id"],"lease":delivery["lease"],"preparation":prepared["preparation_id"]});
    for authority in [
        HostContext::new("other", ["output".into()]),
        HostContext::new("owner", []),
    ] {
        let mut other = HostSession::new(
            Engine::open_with_clock(&path, clock.clone()).unwrap(),
            authority,
        )
        .unwrap();
        for request in [
            json!({"kind":"poll","adapter":"adapter"}),
            prepare.clone(),
            complete.clone(),
        ] {
            assert_eq!(reply(&mut other, request)["error"]["code"], "E_HOST_AUTH");
        }
        assert_eq!(
            reply(&mut other, json!({"kind":"poll","adapter":"missing"}))["error"]["code"],
            "E_HOST_AUTH"
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&other.set_adapter_state("adapter", "paused").bytes)
                .unwrap()["error"]["code"],
            "E_HOST_AUTH"
        );
    }
    assert_eq!(
        reply(&mut owner, complete.clone())["value"]["duplicate"],
        false
    );
    drop(owner);
    let mut other = HostSession::new(
        Engine::open_with_clock(&path, clock.clone()).unwrap(),
        HostContext::new("other", ["output".into()]),
    )
    .unwrap();
    assert_eq!(
        reply(&mut other, complete.clone())["error"]["code"],
        "E_HOST_AUTH"
    );
    let mut owner =
        HostSession::new(Engine::open_with_clock(&path, clock).unwrap(), host()).unwrap();
    assert_eq!(reply(&mut owner, complete)["value"]["duplicate"], true);
    assert_eq!(Engine::open(&path).unwrap().event_count().unwrap(), 2);
}
#[test]
fn operational_json_cannot_install_authority_or_bypass_compiled_completion() {
    let mut session = HostSession::new(Engine::memory().unwrap(), host()).unwrap();
    for op in [
        json!({"kind":"install_compiled_handler"}),
        json!({"kind":"complete_handler","program":{}}),
        json!({"kind":"execute","actor":"admin","program":{"version":weave_contract::VERSION,"commands":[]}}),
    ] {
        let r = reply(&mut session, op);
        assert_eq!(r["ok"], false);
        assert_eq!(r["requires_fence"], false);
    }
    let r = session.call(br#"{"format":"weave-host-request/1","operation":{"kind":"execute","program":{"version":"0.18.0","commands":[{"op":"commit","graph_id":"input","data":{"nodes":[{"id":"n","entity_id":"e","space_id":"s","properties":{"x":1,"\u0078":2}}]}}]}}}"#);
    assert!(!r.requires_fence);
    assert_eq!(
        serde_json::from_slice::<Value>(&r.bytes).unwrap()["ok"],
        false
    );
    let denial = reply(
        &mut session,
        json!({"kind":"execute","program":{"version":weave_contract::VERSION,"commands":[{"op":"commit","graph_id":"forbidden","data":{}}]}}),
    );
    assert_eq!(denial["ok"], false);
    assert_eq!(denial["requires_fence"], true); // Invoked Engine errors require host persistence handling too.
}
