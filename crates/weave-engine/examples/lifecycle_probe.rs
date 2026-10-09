//! Fixed trusted native lifecycle recovery fixture. No application execution interface.
use serde_json::json;
use std::{path::Path, sync::Arc};
use weave_contract::*;
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new("owner", ["input".into(), "output".into(), "orphan".into()])
}
fn write(
    engine: &mut Engine,
    graph: &str,
    value: i64,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let program: Program = serde_json::from_value(
        json!({"version":VERSION,"commands":[{"op":"commit","graph_id":graph,"expected_head":engine.head(graph,"main")?,"data":{"nodes":[{"id":"n","entity_id":"e","space_id":"s","readers":["owner"],"properties":{"value":value}}]}}]}),
    )?;
    engine.execute(&program, &host())?;
    Ok(())
}
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let path = Path::new(&args[1]);
    let mode = &args[2];
    let request = Path::new(&args[3]);
    let clock = Arc::new(ManualClock::new(60));
    if mode == "open_crash" {
        Engine::open_test_before_schema_commit_with_clock(path, clock, || std::process::exit(82))?;
        return Ok(());
    }
    let mut engine = Engine::open_with_clock(path, clock)?;
    if mode == "open_after" {
        std::process::exit(83);
    }
    let output = match mode.as_str() {
        "inspect" => {
            let state = engine.projection_state_for("projection", &host());
            json!({"marker":STORAGE_VERSION,"events":engine.event_count()?,"output":engine.head("output","main")?,"state":state.as_ref().ok(),"state_error":state.err().map(|e|e.code)})
        }
        "duplicate_completion" => {
            let actual: ProjectionCompletionRequest =
                serde_json::from_slice(&std::fs::read(request)?)?;
            json!(engine.complete_projection_for(&actual, &host())?)
        }
        "prepare_upgrade" => {
            engine.set_adapter_state_for("projection", "paused", &host())?;
            let mut destination: AdapterManifest = serde_json::from_value(
                json!({"id":"projection2","version":"2","artifact_digest":format!("sha256:{}","b".repeat(64)),"config_revision":"2","principal":"owner","subscriptions":[{"graph_id":"input","branch_id":"main"}],"output_graphs":["output"],"effect_destinations":[],"max_attempts":3,"lease_ms":100,"max_pending_events":100,"projection_replay":true}),
            )?;
            destination.version = "2".into();
            let actual = ProjectionMigrationRequest {
                inputs: engine.projection_migration_inputs_for("projection", &host())?,
                destination,
                event_schema: VERSION.into(),
                nonce: "upgrade".into(),
                disposition: ProjectionMigrationKind::Upgrade,
                state_revision: "upgrade".into(),
                state: json!({"sum":2,"algorithm":2}),
            };
            std::fs::write(request, serde_json::to_vec(&actual)?)?;
            json!(actual)
        }
        "prepare_rollback" => {
            let mut original: ProjectionMigrationRequest =
                serde_json::from_slice(&std::fs::read(request)?)?;
            original.inputs = engine.projection_migration_inputs_for("projection2", &host())?;
            original.destination.id = "projection3".into();
            original.destination.version = "1".into();
            original.destination.config_revision = "1".into();
            original.destination.artifact_digest = format!("sha256:{}", "a".repeat(64));
            original.disposition = ProjectionMigrationKind::Rollback {
                restore_from: "projection".into(),
            };
            original.nonce = "rollback".into();
            original.state_revision = "rebuilt".into();
            original.state = json!({"total":2});
            std::fs::write(request, serde_json::to_vec(&original)?)?;
            json!(original)
        }
        "migrate" | "migrate_crash" | "migrate_after" => {
            let actual: ProjectionMigrationRequest =
                serde_json::from_slice(&std::fs::read(request)?)?;
            let receipt = engine.migrate_projection_test_before_commit(&actual, &host(), || {
                if mode == "migrate_crash" {
                    std::process::exit(82);
                }
            })?;
            if mode == "migrate_after" {
                std::process::exit(83);
            }
            json!(receipt)
        }
        "seed_cancel" => {
            write(&mut engine, "input", 1)?;
            let template = handler_registration::seal_handler_template(serde_json::from_value(
                json!({"format":"weave-handler-registration/1","protocol":VERSION,"name":"identity","revision":"1","input":{"graph_id":"input","branch_id":"main","metadata_depth":0},"event_types":["graph.accepted","graph.committed"],"recipe":{"bindings":[],"output":"$event"},"output_slot":"out","source_revisions":[],"definition_digest":""}),
            )?).map_err(|e| std::io::Error::other(e.message))?;
            let manifest: AdapterManifest = serde_json::from_value(
                json!({"id":"compiled","version":"1","artifact_digest":template.definition_digest,"config_revision":"1","principal":"owner","subscriptions":[{"graph_id":"input","branch_id":"main"}],"output_graphs":["output"],"effect_destinations":[],"max_attempts":3,"lease_ms":10000,"max_pending_events":100,"projection_replay":true}),
            )?;
            engine.install_compiled_handler(
                &manifest,
                &template,
                &HandlerOutputBinding {
                    slot: "out".into(),
                    graph_id: "output".into(),
                    branch_id: "main".into(),
                },
                &host(),
            )?;
            engine.set_adapter_state_for("compiled", "running", &host())?;
            let delivery = engine.poll_adapter_for("compiled", &host())?.unwrap();
            let preparation = engine.prepare_compiled_handler_for(
                "compiled",
                &delivery.id,
                &delivery.lease,
                &host(),
            )?;
            write(&mut engine, "output", 99)?;
            assert_eq!(
                engine
                    .complete_prepared_handler_for(
                        "compiled",
                        &delivery.id,
                        &delivery.lease,
                        &preparation.preparation_id,
                        &host()
                    )
                    .unwrap_err()
                    .code,
                "E_CONFLICT"
            );
            let actual = DeliveryCancellationRequest {
                adapter: "compiled".into(),
                event: delivery.id,
                expected_lease: delivery.lease,
                nonce: "stale".into(),
                reason: DeliveryCancellationReason::StaleOutput,
            };
            std::fs::write(request, serde_json::to_vec(&actual)?)?;
            json!({"request":actual,"preparation":preparation})
        }
        "cancel" | "cancel_crash" | "cancel_after" => {
            let actual: DeliveryCancellationRequest =
                serde_json::from_slice(&std::fs::read(request)?)?;
            let receipt =
                engine.cancel_handler_delivery_test_before_commit(&actual, &host(), || {
                    if mode == "cancel_crash" {
                        std::process::exit(82);
                    }
                })?;
            if mode == "cancel_after" {
                std::process::exit(83);
            }
            json!(receipt)
        }
        "canceled_preparation" => {
            let actual: DeliveryCancellationRequest =
                serde_json::from_slice(&std::fs::read(request)?)?;
            json!({"error":engine.prepare_compiled_handler_for(&actual.adapter,&actual.event,&actual.expected_lease,&host()).unwrap_err().code})
        }
        _ => panic!("unknown fixed fixture mode"),
    };
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}
