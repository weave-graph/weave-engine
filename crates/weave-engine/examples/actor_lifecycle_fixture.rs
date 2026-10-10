//! Trusted native actor, independent durable tool journal and idempotent sink.
//! This is a bounded acceptance fixture, not a remote authority API or sandbox.
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;
use weave_contract::{Program, QueryPlan, VERSION};
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new(
        "owner",
        [
            "actor-input".into(),
            "actor-output".into(),
            "actor-other".into(),
        ],
    )
}
#[path = "actor_tools/v1.rs"]
mod native_v1;
#[path = "actor_tools/v2.rs"]
mod native_v2;
fn definition(id: &str, version: u32) -> RecordedActorDefinition {
    let artifact = if version == 1 {
        include_bytes!("actor_tools/v1.rs").to_vec()
    } else {
        include_bytes!("actor_tools/v2.rs").to_vec()
    };
    RecordedActorDefinition {
        manifest: serde_json::from_value(json!({"id":id,"version":version.to_string(),"artifact_digest":format!("sha256:{:x}",Sha256::digest(&artifact)),"config_revision":"1","principal":"owner","subscriptions":[{"graph_id":"actor-input","branch_id":"main"}],"output_graphs":["actor-output"],"effect_destinations":["reference-sink"],"max_attempts":5,"lease_ms":10000,"max_pending_events":100,"projection_replay":false})).expect("fixed manifest"),
        event_schema:VERSION.into(),state_protocol:"weave-recorded-opaque-state/1".into(),metadata_depth:2,artifact,
    }
}
fn write(e: &Engine, graph: &str, value: Value) -> Program {
    serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":graph,"expected_head":e.head(graph,"main").expect("head"),"data":{"nodes":[{"id":"n","entity_id":"actor-entity","space_id":"actor-space","readers":["owner"],"properties":{"value":value}}]}}]})).expect("fixed program")
}
fn sidecar(path: &Path, suffix: &str) -> Connection {
    Connection::open(path.with_extension(suffix)).expect("independent destination/journal")
}
fn exit_at(input: &Value, boundary: &str, code: i32) {
    if input["crash"].as_str() == Some(boundary) {
        std::process::exit(code);
    }
}
fn run() -> std::result::Result<Value, Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("usage: actor_lifecycle_fixture DATABASE OPERATION JSON_FILE".into());
    }
    if std::fs::metadata(&args[3])?.len() > 4 * 1024 * 1024 {
        return Err("fixture input exceeds limit".into());
    }
    let input: Value = serde_json::from_slice(&std::fs::read(&args[3])?)?;
    let path = Path::new(&args[1]);
    let id = input["id"].as_str().unwrap_or("native-v1");
    let version = input["version"].as_u64().unwrap_or(1) as u32;
    if !matches!(version, 1 | 2) {
        return Err("unsupported native artifact version".into());
    }
    let clock = Arc::new(ManualClock::new(input["clock"].as_i64().unwrap_or(100)));
    let mut e = Engine::open_test_before_schema_commit_with_clock(path, clock, || {
        exit_at(&input, "before-schema", 95)
    })?;
    exit_at(&input, "after-schema", 96);
    match args[2].as_str() {
        "mode" => {
            let event: DispatchEnvelope = serde_json::from_value(input["event"].clone())?;
            Ok(serde_json::to_value(e.recorded_actor_delivery_mode_for(
                id,
                &event.id,
                &event.lease,
                &host(),
            )?)?)
        }
        "definition" => Ok(serde_json::to_value(definition(id, version))?),
        "migration_inputs" => Ok(serde_json::to_value(
            e.recorded_actor_migration_inputs_for(id, &host())?,
        )?),
        "migrate" => {
            let request: RecordedActorMigration = serde_json::from_value(input["request"].clone())?;
            let receipt = e.migrate_recorded_actor_test_before_commit(&request, &host(), || {
                exit_at(&input, "before-migration", 104)
            })?;
            exit_at(&input, "after-migration", 105);
            Ok(serde_json::to_value(receipt)?)
        }
        "observe" => {
            let request: RecordedActorObservation =
                serde_json::from_value(input["request"].clone())?;
            let receipt = e.observe_recorded_actor_test_before_commit(&request, &host(), || {
                exit_at(&input, "before-observation", 106)
            })?;
            exit_at(&input, "after-observation", 107);
            Ok(serde_json::to_value(receipt)?)
        }
        "observation" => Ok(serde_json::to_value(e.recorded_actor_observation_for(
            id,
            input["event"].as_str().ok_or("missing event")?,
            &host(),
        )?)?),
        "open" => Ok(json!({"events":e.event_count()?,"marker":STORAGE_VERSION})),
        "seed" => {
            e.execute(&write(&e, "actor-input", json!(1)), &host())?;
            e.install_recorded_actor_for(&definition(id, version), &host())?;
            Ok(json!({"installed":true}))
        }
        "inputs" => Ok(serde_json::to_value(
            e.recorded_actor_inputs_for(id, &host())?,
        )?),
        "bootstrap" => {
            let request: RecordedActorBootstrap = serde_json::from_value(input["request"].clone())?;
            let digest =
                e.bootstrap_recorded_actor_test_before_commit(&request, &host(), || {
                    exit_at(&input, "before-bootstrap", 97)
                })?;
            exit_at(&input, "after-bootstrap", 98);
            Ok(json!({"state_digest":digest}))
        }
        "state" => Ok(serde_json::to_value(
            e.recorded_actor_state_for(id, &host())?,
        )?),
        "lifecycle" => {
            e.set_adapter_state_for(id, input["state"].as_str().ok_or("missing state")?, &host())?;
            Ok(json!({"changed":true}))
        }
        "input" | "other" => {
            let graph = if args[2] == "input" {
                "actor-input"
            } else {
                "actor-other"
            };
            Ok(serde_json::to_value(e.execute(
                &write(&e, graph, input["value"].clone()),
                &host(),
            )?)?)
        }
        "poll" => Ok(serde_json::to_value(e.poll_adapter_for(id, &host())?)?),
        "prepare" => {
            let event: DispatchEnvelope = serde_json::from_value(input["event"].clone())?;
            // The independent host journal is committed before the kernel completion.
            let mut journal = sidecar(path, "lifecycle.tools.sqlite");
            journal.execute_batch("CREATE TABLE IF NOT EXISTS tool_runs(adapter TEXT NOT NULL,event TEXT NOT NULL,input_digest TEXT NOT NULL,request TEXT NOT NULL,sample TEXT NOT NULL,artifact_digest TEXT NOT NULL,PRIMARY KEY(adapter,event));")?;
            let tx = journal.transaction()?;
            let prior: Option<String> = tx
                .query_row(
                    "SELECT request FROM tool_runs WHERE adapter=?1 AND event=?2",
                    params![id, event.id],
                    |r| r.get(0),
                )
                .optional()?;
            let request = if let Some(body) = prior {
                // Current state authorization precedes reading an existing tool result.
                e.recorded_actor_state_for(id, &host())?;
                let mut request: RecordedActorCompletion = serde_json::from_str(&body)?;
                request.lease = event.lease.clone();
                request
            } else {
                let actual =
                    e.recorded_actor_run_inputs_for(id, &event.id, &event.lease, &host())?;
                let query: QueryPlan = serde_json::from_value(
                    json!({"graph_id":actual.primary_input.graph_id,"revision":actual.primary_input.revision}),
                )?;
                let value = e.query(&query, &host())?;
                let input_digest =
                    format!("sha256:{:x}", Sha256::digest(serde_json::to_vec(&actual)?));
                let registered = definition(id, version);
                let definition_digest = format!(
                    "sha256:{:x}",
                    Sha256::digest(serde_json::to_vec(&registered)?)
                );
                if actual.registration_digest != definition_digest {
                    return Err("installed artifact differs from executable native tool".into());
                }
                let (sample, result) = if version == 1 {
                    native_v1::compute(&event.id, &value.graph.nodes[0].properties["value"])
                } else {
                    native_v2::compute(&event.id, &value.graph.nodes[0].properties["value"])
                };
                let request = RecordedActorCompletion {
                    adapter: id.into(),
                    event: event.id.clone(),
                    lease: event.lease.clone(),
                    prior_state_digest: actual.state_digest,
                    state_revision: format!("state-{}", event.id),
                    state: result.clone(),
                    input_snapshots: actual.input_snapshots,
                    tool_results: vec![RecordedToolResult {
                        name: "native-sample".into(),
                        media_type: "application/json".into(),
                        value: result.clone(),
                    }],
                    program: write(&e, "actor-output", result),
                };
                tx.execute(
                    "INSERT INTO tool_runs VALUES (?1,?2,?3,?4,?5,?6)",
                    params![
                        id,
                        event.id,
                        input_digest,
                        serde_json::to_string(&request)?,
                        sample,
                        registered.manifest.artifact_digest
                    ],
                )?;
                request
            };
            tx.commit()?;
            exit_at(&input, "after-tool-journal", 99);
            Ok(serde_json::to_value(request)?)
        }
        "effect" => {
            let event: DispatchEnvelope = serde_json::from_value(input["event"].clone())?;
            let effect = e.request_effect(
                id,
                &event.id,
                &event.lease,
                "reference-sink",
                &format!("action-{}", event.id),
                input["payload"].clone(),
            )?;
            Ok(serde_json::to_value(effect)?)
        }
        "dispatch" => {
            let id = input["intent"].as_str().ok_or("missing intent")?;
            let effect = e.begin_effect_dispatch(id)?;
            exit_at(&input, "after-unknown", 100);
            let mut sink = sidecar(path, "lifecycle.sink.sqlite");
            sink.execute_batch("CREATE TABLE IF NOT EXISTS physical_receipts(destination_key TEXT PRIMARY KEY,payload TEXT NOT NULL,receipt TEXT NOT NULL);")?;
            let tx = sink.transaction()?;
            let payload = serde_json::to_string(&effect.payload)?;
            let old: Option<(String, String)> = tx
                .query_row(
                    "SELECT payload,receipt FROM physical_receipts WHERE destination_key=?1",
                    [&effect.idempotency_key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let receipt = if let Some((previous, receipt)) = old {
                if previous != payload {
                    return Err("destination idempotency conflict".into());
                }
                receipt
            } else {
                let receipt = format!(
                    "physical:{:x}",
                    Sha256::digest(effect.idempotency_key.as_bytes())
                );
                tx.execute(
                    "INSERT INTO physical_receipts VALUES (?1,?2,?3)",
                    params![effect.idempotency_key, payload, receipt],
                )?;
                receipt
            };
            tx.commit()?;
            exit_at(&input, "after-physical", 101);
            e.reconcile_effect(id, "confirmed", json!({"receipt":receipt}))?;
            Ok(json!({"physical_receipt":receipt}))
        }
        "fail_absent" => {
            let id = input["intent"].as_str().ok_or("missing intent")?;
            let effect = e.effect_intent(id)?.ok_or("missing intent")?;
            let sink = sidecar(path, "lifecycle.sink.sqlite");
            sink.execute_batch("CREATE TABLE IF NOT EXISTS physical_receipts(destination_key TEXT PRIMARY KEY,payload TEXT NOT NULL,receipt TEXT NOT NULL);")?;
            let present: bool = sink.query_row(
                "SELECT EXISTS(SELECT 1 FROM physical_receipts WHERE destination_key=?1)",
                [&effect.idempotency_key],
                |r| r.get(0),
            )?;
            if present {
                return Err("destination effect exists".into());
            }
            // This fixture has no surviving dispatch worker. This is destination-specific
            // absence evidence, not an inference from a network timeout.
            e.reconcile_effect(id, "failed", json!({"reference_sink_absent":true}))?;
            Ok(json!({"recorded_absence":true}))
        }
        "compact" => {
            let policy: RetentionPolicy = serde_json::from_value(input["policy"].clone())?;
            let plan = e.plan_retention(&policy)?;
            Ok(serde_json::to_value(e.compact_retention(&plan)?)?)
        }
        "reconcile" => {
            let id = input["intent"].as_str().ok_or("missing intent")?;
            let effect = e.effect_intent(id)?.ok_or("missing intent")?;
            let sink = sidecar(path, "lifecycle.sink.sqlite");
            let row: Option<(String, String)> = sink
                .query_row(
                    "SELECT payload,receipt FROM physical_receipts WHERE destination_key=?1",
                    [&effect.idempotency_key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let (payload, receipt) = row.ok_or("destination outcome still unknown")?;
            if serde_json::from_str::<Value>(&payload)? != effect.payload {
                return Err("destination payload mismatch".into());
            }
            e.reconcile_effect(id, "confirmed", json!({"receipt":receipt}))?;
            Ok(json!({"physical_receipt":receipt}))
        }
        "complete" => {
            let request: RecordedActorCompletion =
                serde_json::from_value(input["request"].clone())?;
            let receipt =
                e.complete_recorded_actor_test_before_commit(&request, &host(), || {
                    exit_at(&input, "before-complete", 102)
                })?;
            exit_at(&input, "after-complete", 103);
            Ok(serde_json::to_value(receipt)?)
        }
        "query" => Ok(serde_json::to_value(e.query(
            &serde_json::from_value(json!({"graph_id":"actor-output"}))?,
            &host(),
        )?)?),
        _ => Err("unknown actor fixture operation".into()),
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
