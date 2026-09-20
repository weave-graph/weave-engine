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
