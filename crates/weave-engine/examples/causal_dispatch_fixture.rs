//! Fixed native causal-loop acceptance host; no ambient remote execution authority.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;
use weave_contract::{Program, VERSION};
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new(
        "loop-owner",
        ["loop-a".into(), "loop-b".into(), "loop-private".into()],
    )
}
fn exit_at(input: &Value, boundary: &str, code: i32) {
    if input["crash"].as_str() == Some(boundary) {
        std::process::exit(code);
    }
}
fn graph(input: &Value) -> std::result::Result<&str, Box<dyn std::error::Error>> {
    let graph = input["graph"].as_str().ok_or("graph required")?;
    if !matches!(graph, "loop-a" | "loop-b" | "loop-private") {
        return Err("fixed native scope".into());
    }
    Ok(graph)
}
fn program(e: &Engine, input: &Value) -> std::result::Result<Program, Box<dyn std::error::Error>> {
    let graph = graph(input)?;
    let reader = if input["private"] == true {
        "other"
    } else {
        "loop-owner"
    };
    Ok(serde_json::from_value(
        json!({"version":VERSION,"commands":[{"op":"commit","graph_id":graph,"expected_head":e.head(graph,"main")?,"data":{"nodes":[{"id":"n","entity_id":"loop-entity","space_id":"loop-space","properties":{"value":input["value"]},"readers":[reader]}]}}]}),
    )?)
}
fn run() -> std::result::Result<Value, Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("usage: causal_dispatch_fixture DATABASE OPERATION JSON_FILE".into());
    }
    if std::fs::metadata(&args[3])?.len() > 1024 * 1024 {
        return Err("fixture request exceeds limit".into());
    }
    let input: Value = serde_json::from_slice(&std::fs::read(&args[3])?)?;
    let path = Path::new(&args[1]);
    let clock = Arc::new(ManualClock::new(input["clock"].as_i64().unwrap_or(2000)));
    let mut e = Engine::open_test_before_schema_commit_with_clock(path, clock, || {
        exit_at(&input, "before-schema", 95)
    })?;
    exit_at(&input, "after-schema", 96);
    let id = input["id"].as_str().unwrap_or("loop-A");
    if !matches!(id, "loop-A" | "loop-B") {
        return Err("fixed adapter namespace".into());
    }
    let depth = input["depth"].as_u64().unwrap_or(4);
    let depth = u32::try_from(depth)?;
    match args[2].as_str() {
        "open" => Ok(json!({"store_marker":STORAGE_VERSION})),
        "input" => {
            e.execute(&program(&e, &input)?, &host())?;
            Ok(json!({"head":e.head(graph(&input)?,"main")?}))
        }
        "recipe" => Ok(serde_json::to_value(program(&e, &input)?)?),
        "install" => {
            let (source, output) = if id == "loop-A" {
                ("loop-a", "loop-b")
            } else {
                ("loop-b", "loop-a")
            };
            let digest = format!(
                "sha256:{:x}",
                Sha256::digest(include_bytes!("causal_dispatch_fixture.rs"))
            );
            let manifest: AdapterManifest = serde_json::from_value(
                json!({"id":id,"version":"1","artifact_digest":digest,"config_revision":"1","principal":"loop-owner","subscriptions":[{"graph_id":source,"branch_id":"main"}],"output_graphs":[output],"effect_destinations":[],"max_attempts":3,"lease_ms":10000,"max_pending_events":100,"projection_replay":true}),
            )?;
            e.install_adapter(&manifest, &host())?;
            e.set_causal_dispatch_policy_for(
                id,
                &CausalDispatchPolicy { max_depth: depth },
                &host(),
            )?;
            e.set_adapter_state_for(id, "running", &host())?;
            Ok(json!({"installed":id}))
        }
        "lifecycle" => {
            e.set_adapter_state_for(
                id,
                input["state"].as_str().ok_or("state required")?,
                &host(),
            )?;
            Ok(json!({"state":input["state"]}))
        }
        "policy" => {
            e.set_causal_dispatch_policy_for(
                id,
                &CausalDispatchPolicy { max_depth: depth },
                &host(),
            )?;
            Ok(json!({"max_depth":depth}))
        }
        "lag" => Ok(serde_json::to_value(
            e.adapter_lag_status_for(id, &host())?,
        )?),
        "poll" => {
            let result = e.poll_adapter_test_before_circuit_commit_for(id, &host(), || {
                exit_at(&input, "before-circuit", 112)
            });
            if result.as_ref().is_err_and(|e| e.code == "E_CIRCUIT_OPEN") {
                exit_at(&input, "after-circuit", 113);
            }
            Ok(serde_json::to_value(result?)?)
        }
        "complete" => {
            let event = input["event"]["id"].as_str().ok_or("event required")?;
            let lease = input["event"]["lease"].as_str().ok_or("lease required")?;
            let p: Program = serde_json::from_value(input["program"].clone())?;
            let receipt = e.complete_handler_test_before_commit(id, event, lease, &p, || {
                exit_at(&input, "before-completion", 110)
            })?;
            exit_at(&input, "after-completion", 111);
            Ok(serde_json::to_value(receipt)?)
        }
        "compact" => {
            let policy: RetentionPolicy =
                serde_json::from_value(input.get("policy").cloned().unwrap_or_else(
                    || json!({"history_before_ms":0,"replay_through_sequence":0}),
                ))?;
            let p = e.plan_retention(&policy)?;
            Ok(serde_json::to_value(e.compact_retention(&p)?)?)
        }
        _ => Err("unsupported operation".into()),
    }
}
fn main() {
    match run() {
        Ok(value) => println!("{value}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
