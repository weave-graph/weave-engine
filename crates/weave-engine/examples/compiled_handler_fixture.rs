//! Trusted local acceptance harness only. JSON requests here are fixture controls,
//! not a public authority API; the principal and graph grants are fixed below.
use serde_json::{json, Value};
use std::sync::Arc;
use weave_contract::*;
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new(
        "collector",
        [
            "Installation",
            "Quality",
            "Warnings",
            "Empty",
            "EmptyWarnings",
            "Other",
            "Marker",
        ]
        .map(String::from),
    )
}
fn text<'a>(request: &'a Value, key: &str) -> &'a str {
    request[key].as_str().expect("fixture string field")
}
fn run(db: &str, request: Value) -> weave_engine::Result<Value> {
    let clock = Arc::new(ManualClock::new(request["now"].as_i64().unwrap_or(20)));
    let mut engine = Engine::open_with_clock(db, clock)?;
    match text(&request, "mode") {
        "run" => {
            let h = if request["outsider"] == true {
                HostContext::new("outsider", Vec::<String>::new())
            } else {
                host()
            };
            let program: Program = serde_json::from_value(request["program"].clone())?;
            Ok(serde_json::to_value(engine.execute(&program, &h)?)?)
        }
        "install" => {
            let template: CompiledHandlerTemplate =
                serde_json::from_value(request["template"].clone())?;
            let output = text(&request, "output");
            let manifest = AdapterManifest {
                id: text(&request, "id").into(),
                version: "1".into(),
                artifact_digest: template.definition_digest.clone(),
                config_revision: "1".into(),
                principal: "collector".into(),
                subscriptions: vec![SubscriptionScope {
                    graph_id: template.input.graph_id.clone(),
                    branch_id: template.input.branch_id.clone(),
                }],
                output_graphs: vec![output.into()],
                effect_destinations: vec![],
                max_attempts: 10,
                lease_ms: 1000,
                max_pending_events: 1000,
                projection_replay: true,
            };
            let binding = HandlerOutputBinding {
                slot: template.output_slot.clone(),
                graph_id: output.into(),
                branch_id: "main".into(),
            };
            engine.install_compiled_handler(&manifest, &template, &binding, &host())?;
            engine.set_adapter_state(&manifest.id, "running")?;
            Ok(json!({"installed":true}))
        }
        "poll" => Ok(serde_json::to_value(
            engine.poll_adapter(text(&request, "id"))?,
        )?),
        "prepare" => {
            #[cfg(feature = "recovery-testing")]
            if request["kill_before_commit"] == true {
                engine.prepare_compiled_handler_test_before_commit(
                    text(&request, "id"),
                    text(&request, "event"),
                    text(&request, "lease"),
                    || std::process::exit(94),
                )?;
                panic!("prepare hook not reached");
            }
            let receipt = engine.prepare_compiled_handler(
                text(&request, "id"),
                text(&request, "event"),
                text(&request, "lease"),
            )?;
            if request["kill_after_commit"] == true {
                std::process::exit(92);
            }
            Ok(serde_json::to_value(receipt)?)
        }
        "complete" => {
            #[cfg(feature = "recovery-testing")]
            if request["kill_before_commit"] == true {
                engine.complete_prepared_handler_test_before_commit(
                    text(&request, "id"),
                    text(&request, "event"),
                    text(&request, "lease"),
                    text(&request, "preparation"),
                    || std::process::exit(94),
                )?;
                panic!("complete hook not reached");
            }
            let receipt = engine.complete_prepared_handler(
                text(&request, "id"),
                text(&request, "event"),
                text(&request, "lease"),
                text(&request, "preparation"),
            )?;
            if request["kill_after_commit"] == true {
                std::process::exit(92);
            }
            Ok(serde_json::to_value(receipt)?)
        }
        "raw" => {
            let program: Program = serde_json::from_value(request["program"].clone())?;
            Ok(serde_json::to_value(engine.complete_handler(
                text(&request, "id"),
                text(&request, "event"),
                text(&request, "lease"),
                &program,
            )?)?)
        }
        "accept" => {
            let reference: GraphRef = serde_json::from_value(request["reference"].clone())?;
            engine.accept_revision(&reference, "main", request["expected"].as_str(), &host())?;
            Ok(json!({"accepted":true}))
        }
        "state" => {
            engine.set_adapter_state(text(&request, "id"), text(&request, "state"))?;
            Ok(json!({"updated":true}))
        }
        "head" => Ok(json!({"head":engine.head(text(&request,"graph"),"main")?})),
        _ => panic!("unknown fixture mode"),
    }
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let request: Value =
        serde_json::from_reader(std::fs::File::open(&args[2]).expect("request file"))
            .expect("request JSON");
    match run(&args[1], request) {
        Ok(value) => println!("{}", value),
        Err(error) => {
            eprintln!("{}", serde_json::to_string(&error).unwrap());
            std::process::exit(1);
        }
    }
}
