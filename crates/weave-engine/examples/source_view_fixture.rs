//! Trusted local acceptance harness only. Fixed test keys/clock are not a public authority API.
use ed25519_dalek::SigningKey;
use serde_json::{json, Value};
use std::sync::Arc;
use weave_contract::*;
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new(
        "collector",
        [
            "Fleet".into(),
            "Empty".into(),
            "Saved".into(),
            "Marker".into(),
        ],
    )
}
fn graph(turn: i64) -> GraphData {
    serde_json::from_value(json!({
        "nodes":[{"id":"a","entity_id":"A","space_id":"s","properties":{"turn":turn}}, {"id":"b","entity_id":"B","space_id":"s"}],
        "edges":[{"id":"positive","predicate":"connected","from":"a","to":"b","valid_time":{"start":0,"end":10}}, {"id":"negative","predicate":"connected","from":"a","to":"b","valid_time":{"start":0,"end":10},"polarity":"negative"}]
    })).unwrap()
}
fn write(e: &mut Engine, graph_id: &str, data: GraphData) -> weave_engine::Result<GraphRef> {
    let expected_head = e.head(graph_id, "main")?;
    e.execute(
        &Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![Command::Commit {
                graph_id: graph_id.into(),
                branch_id: "main".into(),
                expected_head,
                data,
            }],
        },
        &host(),
    )?;
    Ok(GraphRef {
        graph_id: graph_id.into(),
        revision: e.head(graph_id, "main")?.unwrap(),
    })
}
fn accept(
    e: &Engine,
    view: &str,
    source: GraphRef,
    id: &str,
) -> weave_engine::Result<GovernanceReceipt> {
    let key = SigningKey::from_bytes(&[73; 32]);
    let reference = GovernancePolicyRef {
        id: format!("policy-{view}"),
        revision: "1".into(),
    };
    if id != "next" {
        e.install_governance_root(&GovernancePolicy {
            view_id: view.into(),
            reference: reference.clone(),
            members: vec![weave_policy::public_key(&key)],
            threshold: 1,
            proposers: vec!["collector".into()],
            readers: vec!["collector".into()],
            allowed_sources: vec![GovernanceSourceScope {
                graph_id: source.graph_id.clone(),
                branch_id: "main".into(),
            }],
            not_before_ms: 0,
            expires_at_ms: 10000,
        })?;
    }
    let expected_head = e.inspect_governance_head(view, &host())?.decision_id;
    let proposal = GovernanceProposal {
        id: id.into(),
        view_id: view.into(),
        policy: reference.clone(),
        expected_head: expected_head.clone(),
        expires_at_ms: 9000,
        action: GovernanceAction::Publish {
            source,
            branch_id: "main".into(),
        },
    };
    let proposed = e.propose_governance(&proposal, &host())?;
    let signed = sign_governance_approval(
        GovernanceApproval {
            proposal_id: id.into(),
            proposal_digest: proposed.digest,
            view_id: view.into(),
            policy: reference,
            expected_head,
            member: weave_policy::public_key(&key),
            issued_at_ms: 0,
            expires_at_ms: 9000,
            nonce: format!("approval-{id}"),
        },
        &key,
    )?;
    e.record_governance_approval(&signed, &host())?;
    e.accept_governance(
        &GovernanceDecisionRequest {
            proposal_id: id.into(),
            nonce: format!("decision-{id}"),
        },
        &host(),
    )
}
fn read_json<T: serde::de::DeserializeOwned>(path: &str) -> T {
    serde_json::from_reader(std::fs::File::open(path).expect("fixture file")).expect("fixture JSON")
}
fn tick(text: &str) -> Option<i64> {
    if text == "fixed" {
        None
    } else {
        Some(text.parse().expect("fixture tick"))
    }
}
fn run(a: &[String]) -> weave_engine::Result<Value> {
    let clock = Arc::new(ManualClock::new(if a[2] == "expired" { 10000 } else { 20 }));
    let mut e = Engine::open_with_clock(&a[1], clock)?;
    match a[2].as_str() {
        "seed" => {
            let source = write(&mut e, "Fleet", graph(0))?;
            let empty = write(&mut e, "Empty", GraphData::default())?;
            let accepted = accept(&e, "team", source.clone(), "initial")?;
            let empty_accepted = accept(&e, "empty", empty, "empty-initial")?;
            Ok(json!({"accepted":accepted,"empty":empty_accepted,"source":source}))
        }
        "register" => {
            let template: CompiledViewTemplate = read_json(&a[3]);
            Ok(serde_json::to_value(e.register_compiled_view(
                &a[4],
                &template,
                tick(&a[5]),
                &host(),
            )?)?)
        }
        "run" | "expired" | "outsider" => {
            let plan: Program = read_json(&a[3]);
            let h = if a[2] == "outsider" {
                HostContext::new("outsider", Vec::<String>::new())
            } else {
                host()
            };
            Ok(serde_json::to_value(e.execute(&plan, &h)?)?)
        }
        "accepted" => Ok(serde_json::to_value(e.query_accepted_view(
            &AcceptedViewSelection {
                view_id: a[3].clone(),
                decision_id: Some(a[4].clone()),
            },
            &host(),
        )?)?),
        "current" => Ok(serde_json::to_value(e.read_current_view(
            &CurrentViewSelection {
                view_id: a[3].clone(),
                definition_digest: a[4].clone(),
                time: tick(&a[5]).map_or(ViewReadTime::Fixed, |valid_at| ViewReadTime::Tick {
                    valid_at,
                }),
            },
            &host(),
        )?)?),
        "change" => Ok(serde_json::to_value(write(&mut e, "Fleet", graph(1))?)?),
        "publish" => {
            let source = GraphRef {
                graph_id: "Fleet".into(),
                revision: e.head("Fleet", "main")?.unwrap(),
            };
            Ok(serde_json::to_value(accept(&e, "team", source, "next")?)?)
        }
        "refresh" => Ok(serde_json::to_value(e.refresh_view(
            &a[3],
            tick(&a[4]),
            &host(),
        )?)?),
        "enroll" => {
            e.enroll_incremental_view(&a[3], &host())?;
            Ok(json!({"enrolled":true}))
        }
        "save" => Ok(serde_json::to_value(write(
            &mut e,
            "Saved",
            read_json(&a[3]),
        )?)?),
        "head" => Ok(json!({"revision":e.head(&a[3],"main")?})),
        "events" => Ok(json!({"events":e.event_count()?})),
        _ => panic!("unknown trusted fixture mode"),
    }
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    match run(&args) {
        Ok(value) => println!("{value}"),
        Err(error) => {
            eprintln!("{}", json!({"code":error.code,"message":error.message}));
            std::process::exit(1);
        }
    }
}
