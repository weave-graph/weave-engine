//! Trusted process-recovery fixture. All domain names and authority come from
//! separately supplied test configuration, never a public operational opcode.
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::Path, sync::Arc};
use weave_contract::CompiledHandlerTemplate;
use weave_engine::{
    AdapterManifest, Engine, HandlerOutputBinding, HostContext, ManualClock, SubscriptionScope,
};
use weave_native::{
    artifacts::{ArtifactBundle, SDK_RESPONSE_LIMIT},
    cluster_journal::ClusterJournal,
    host::{HostReply, HostSession},
};

#[derive(Deserialize)]
struct Config {
    variant: String,
    principal: String,
    reviewer: String,
    graphs: BTreeMap<String, String>,
    handlers: BTreeMap<String, String>,
    adapters: BTreeMap<String, String>,
    output_slot: String,
}
fn text<'a>(request: &'a Value, key: &str) -> &'a str {
    request[key].as_str().expect("trusted fixture string")
}
fn read(path: &Path, limit: usize) -> Vec<u8> {
    assert!(
        fs::metadata(path).unwrap().len() <= limit as u64,
        "fixture file limit"
    );
    let bytes = fs::read(path).unwrap();
    assert!(bytes.len() <= limit);
    bytes
}
fn bundle(request: &Value) -> Result<ArtifactBundle, Box<dyn std::error::Error>> {
    Ok(ArtifactBundle::parse(&read(
        Path::new(text(request, "artifact")),
        SDK_RESPONSE_LIMIT,
    ))?)
}
fn call(session: &mut HostSession, operation: Value) -> HostReply {
    session.call(
        &serde_json::to_vec(&json!({"format":"weave-host-request/1", "operation":operation}))
            .unwrap(),
    )
}
fn manifest(
    config: &Config,
    key: &str,
    input: &str,
    output: &str,
    digest: String,
) -> AdapterManifest {
    AdapterManifest {
        id: config.adapters[key].clone(),
        version: "1".into(),
        artifact_digest: digest,
        config_revision: "1".into(),
        principal: config.principal.clone(),
        subscriptions: vec![SubscriptionScope {
            graph_id: input.into(),
            branch_id: "main".into(),
        }],
        output_graphs: vec![output.into()],
        effect_destinations: vec![],
        max_attempts: 20,
        lease_ms: 1000,
        max_pending_events: 100,
        projection_replay: true,
    }
}
fn emit(reply: HostReply) {
    println!("{}", String::from_utf8(reply.bytes).unwrap());
}
fn run(db: &Path, config: Config, request: Value) -> Result<(), Box<dyn std::error::Error>> {
    let engine = Engine::open_with_clock(
        db,
        Arc::new(ManualClock::new(request["now"].as_i64().unwrap_or(100))),
    )?;
    let principal = if request["reviewer"] == true {
        &config.reviewer
    } else {
        &config.principal
    };
    let scopes: Vec<String> = if request["narrowed"] == true {
        vec![]
    } else {
        config.graphs.values().cloned().collect()
    };
    let authority = HostContext::new(principal, scopes);
    if text(&request, "mode") == "inspect" {
        // Trusted test observability only; no principal-facing inventory API.
        let heads = config
            .graphs
            .iter()
            .map(|(key, graph)| Ok((key.clone(), engine.head(graph, "main")?)))
            .collect::<weave_engine::Result<BTreeMap<_, _>>>()?;
        println!(
            "{}",
            json!({"heads":heads,"events":engine.events()?.len(),"runtime_source":engine.runtime_source_identity()?})
        );
        return Ok(());
    }
    if text(&request, "mode") == "raw" {
        let mut engine = engine;
        let program = serde_json::from_value(request["program"].clone())?;
        match engine.complete_handler(
            text(&request, "adapter"),
            text(&request, "event"),
            text(&request, "lease"),
            &program,
        ) {
            Ok(v) => println!("{}", json!({"ok":true,"value":v})),
            Err(e) => println!("{}", json!({"ok":false,"error":e})),
        }
        return Ok(());
    }
    let mut session = HostSession::new(engine, authority)?;
    let before = request["kill_before_commit"] == true;
    let after = request["kill_after_commit"] == true;
    match text(&request, "mode") {
        "execute_artifact" => {
            let artifact = bundle(&request)?;
            let mut bytes = b"{\"format\":\"weave-host-request/1\",\"operation\":{\"kind\":\"execute\",\"program\":".to_vec();
            bytes.extend_from_slice(artifact.program_bytes());
            bytes.extend_from_slice(b"}}");
            emit(session.call(&bytes));
        }
        "call" => emit(call(&mut session, request["operation"].clone())),
        "install" => {
            let artifact = bundle(&request)?;
            let name = &config.handlers["selected"];
            let template: CompiledHandlerTemplate =
                serde_json::from_slice(artifact.handler_bytes(name).ok_or("missing handler")?)?;
            let output = HandlerOutputBinding {
                slot: config.output_slot.clone(),
                graph_id: config.graphs["warnings"].clone(),
                branch_id: "main".into(),
            };
            let adapter = manifest(
                &config,
                "diagnostic",
                &template.input.graph_id,
                &output.graph_id,
                template.definition_digest,
            );
            let installed = session.install_compiled_handler(&artifact, name, &adapter, &output);
            if serde_json::from_slice::<Value>(&installed.bytes)?["ok"] != true {
                emit(installed);
                return Ok(());
            }
            emit(session.set_adapter_state(&adapter.id, "running"));
        }
        "install_cluster" => {
            let adapter = manifest(
                &config,
                "cluster",
                &config.graphs["warnings"],
                &config.graphs["clusters"],
                format!("sha256:{}", "c".repeat(64)),
            );
            let installed = session.install_cluster_adapter(&adapter);
            if serde_json::from_slice::<Value>(&installed.bytes)?["ok"] != true {
                emit(installed);
                return Ok(());
            }
            emit(session.set_adapter_state(&adapter.id, "running"));
        }
        "prepare" | "complete" => {
            let adapter = text(&request, "adapter");
            let event = text(&request, "event");
            let lease = text(&request, "lease");
            let preparing = text(&request, "mode") == "prepare";
            #[cfg(feature = "recovery-testing")]
            if before {
                let reply = if preparing {
                    session.prepare_test_before_commit(adapter, event, lease, || {
                        std::process::exit(94)
                    })
                } else {
                    session.complete_test_before_commit(
                        adapter,
                        event,
                        lease,
                        text(&request, "preparation"),
                        || std::process::exit(94),
                    )
                };
                emit(reply);
                return Ok(());
            }
            #[cfg(not(feature = "recovery-testing"))]
            assert!(!before, "recovery feature required");
            let op = if preparing {
                json!({"kind":"prepare","adapter":adapter,"event":event,"lease":lease})
            } else {
                json!({"kind":"complete","adapter":adapter,"event":event,"lease":lease,"preparation":text(&request,"preparation")})
            };
            let reply = call(&mut session, op);
            if after {
                std::process::exit(92);
            }
            emit(reply);
        }
        "cluster_prepare" | "cluster_complete" => {
            let journal_path = db.with_extension("host.db");
            let create = !journal_path.exists();
            let mut journal =
                ClusterJournal::open(&journal_path, &config.variant, &session, create)?;
            let reply = if text(&request, "mode") == "cluster_prepare" {
                let artifact = bundle(&request)?;
                #[cfg(feature = "recovery-testing")]
                {
                    journal.prepare_test_observers(
                        &mut session,
                        text(&request, "adapter"),
                        text(&request, "event"),
                        text(&request, "lease"),
                        &artifact,
                        "main",
                        || {
                            if before {
                                std::process::exit(94);
                            }
                        },
                        || {
                            if after {
                                std::process::exit(92);
                            }
                        },
                    )
                }
                #[cfg(not(feature = "recovery-testing"))]
                {
                    assert!(!before && !after);
                    journal.prepare(
                        &mut session,
                        text(&request, "adapter"),
                        text(&request, "event"),
                        text(&request, "lease"),
                        &artifact,
                        "main",
                    )
                }
            } else {
                #[cfg(feature = "recovery-testing")]
                {
                    journal.complete_test_observers(
                        &mut session,
                        text(&request, "record"),
                        text(&request, "lease"),
                        || {
                            if before {
                                std::process::exit(94);
                            }
                        },
                        || {
                            if after {
                                std::process::exit(92);
                            }
                        },
                    )
                }
                #[cfg(not(feature = "recovery-testing"))]
                {
                    assert!(!before && !after);
                    journal.complete(
                        &mut session,
                        text(&request, "record"),
                        text(&request, "lease"),
                    )
                }
            };
            emit(reply);
        }
        _ => return Err("unknown fixture action".into()),
    }
    Ok(())
}
fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    let config = serde_json::from_slice(&read(Path::new(&args[2]), 64 * 1024)).unwrap();
    let request = serde_json::from_slice(&read(
        Path::new(&args[3]),
        weave_native::host::REQUEST_LIMIT,
    ))
    .unwrap();
    if let Err(e) = run(Path::new(&args[1]), config, request) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
