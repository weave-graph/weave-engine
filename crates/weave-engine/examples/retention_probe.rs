//! Fixed trusted native crash controller; administrative diagnostics are not principal APIs.
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc};
use weave_contract::*;
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new("owner", ["input".into(), "orphan".into(), "output".into()])
}
fn write(engine: &Engine, graph: &str, value: i64) -> Program {
    serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":graph,"expected_head":engine.head(graph,"main").unwrap(),"data":{"nodes":[{"id":"n","entity_id":"e","space_id":"s","readers":["owner"],"properties":{"value":value}}]}}]})).unwrap()
}
fn manifest() -> AdapterManifest {
    AdapterManifest {
        id: "projection".into(),
        version: "1".into(),
        artifact_digest: format!("sha256:{}", "a".repeat(64)),
        config_revision: "1".into(),
        principal: "owner".into(),
        subscriptions: vec![SubscriptionScope {
            graph_id: "input".into(),
            branch_id: "main".into(),
        }],
        output_graphs: vec!["output".into()],
        effect_destinations: vec![],
        max_attempts: 3,
        lease_ms: 100,
        max_pending_events: 100,
        projection_replay: true,
    }
}
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let path = Path::new(&args[1]);
    let mode = &args[2];
    let request_path = Path::new(&args[3]);
    let time = if mode == "seed" {
        10
    } else if mode == "next" {
        20
    } else {
        30
    };
    let clock: Arc<dyn TrustedClock> = if mode == "plan_existing" {
        Arc::new(SystemClock)
    } else {
        Arc::new(ManualClock::new(time))
    };
    let mut engine = Engine::open_with_clock(path, clock)?;
    let output = match mode.as_str() {
        "plan_existing" => json!(engine.plan_retention(&RetentionPolicy::default())?),
        "seed" => {
            engine.execute(&write(&engine, "input", 1), &host())?;
            engine.execute(&write(&engine, "orphan", 99), &host())?;
            let orphan = engine.head("orphan", "main")?.unwrap();
            engine.release_branch_for("orphan", "main", &orphan, &host())?;
            engine.install_adapter(&manifest(), &host())?;
            engine.set_adapter_state("projection", "running")?;
            let inputs = engine.projection_rebase_inputs_for("projection", &host())?;
            json!(engine.rebase_projection_for(
                &ProjectionRebaseRequest {
                    inputs,
                    state_revision: "seed".into(),
                    state: json!({"total":1})
                },
                &host()
            )?)
        }
        "next" => {
            engine.execute(&write(&engine, "input", 2), &host())?;
            let state = engine.projection_state_for("projection", &host())?;
            let delivery = engine.poll_adapter_for("projection", &host())?.unwrap();
            let request = ProjectionCompletionRequest {
                adapter: "projection".into(),
                event: delivery.id,
                lease: delivery.lease,
                prior_state_digest: format!(
                    "sha256:{:x}",
                    Sha256::digest(serde_json::to_vec(&state)?)
                ),
                state_revision: "completed".into(),
                state: json!({"total":3}),
                input_snapshots: vec![delivery.graph],
                program: write(&engine, "output", 3),
            };
            std::fs::write(request_path, serde_json::to_vec(&request)?)?;
            json!(request)
        }
        "complete" | "complete_crash" | "complete_after" | "raw" => {
            let request: ProjectionCompletionRequest =
                serde_json::from_slice(&std::fs::read(request_path)?)?;
            if mode == "raw" {
                json!({"error":engine.complete_handler(&request.adapter,&request.event,&request.lease,&request.program).unwrap_err().code})
            } else {
                let receipt =
                    engine.complete_projection_test_before_commit(&request, &host(), || {
                        if mode == "complete_crash" {
                            std::process::exit(82);
                        }
                    })?;
                if mode == "complete_after" {
                    std::process::exit(83);
                }
                json!(receipt)
            }
        }
        "compact" | "compact_crash" | "compact_after" => {
            let policy = RetentionPolicy {
                history_before_ms: 25,
                replay_through_sequence: engine.events()?.last().unwrap().sequence as i64,
            };
            let plan = engine.plan_retention(&policy)?;
            let receipt = engine.compact_retention_test_before_commit(&plan, || {
                if mode == "compact_crash" {
                    std::process::exit(82);
                }
            })?;
            if mode == "compact_after" {
                std::process::exit(83);
            }
            json!(receipt)
        }
        "rebase" | "rebase_crash" | "rebase_after" => {
            let inputs = engine.projection_rebase_inputs_for("projection", &host())?;
            let receipt = engine.rebase_projection_test_before_commit(
                &ProjectionRebaseRequest {
                    inputs,
                    state_revision: "rebuilt".into(),
                    state: json!({"total":2}),
                },
                &host(),
                || {
                    if mode == "rebase_crash" {
                        std::process::exit(82);
                    }
                },
            )?;
            if mode == "rebase_after" {
                std::process::exit(83);
            }
            json!(receipt)
        }
        "inspect" => {
            let state = engine.projection_state_for("projection", &host());
            json!({"marker":STORAGE_VERSION,"events":engine.event_count()?,"output":engine.head("output","main")?,"state":state.as_ref().ok(),"state_error":state.err().map(|e|e.code)})
        }
        _ => panic!("unknown fixed fixture mode"),
    };
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}
