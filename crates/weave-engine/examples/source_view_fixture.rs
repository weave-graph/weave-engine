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
/// Governance lifetimes: (base, policy window, proposal and approval window).
/// The policy window is anchored at the seeding process; each proposal window is anchored
/// at the process that proposes it and must end within the seeded policy window.
type Lifetime = (i64, i64, i64);
/// ManualClock modes. The `expired` mode samples exactly the end of this policy window.
const MANUAL_LIFETIME: (i64, i64) = (10_000, 9_000);
/// SystemClock `live_*` modes. Every `live_*` process reads the current head policy, which
/// fails closed once wall-clock time passes its expiry, and `live_publish` must propose
/// within it. Wall-clock windows of 10 s and 1 s made those runs flaky on slow runners;
/// 60 and 30 minutes exceed any job timeout.
const LIVE_LIFETIME: (i64, i64) = (3_600_000, 1_800_000);
fn accept(
    e: &Engine,
    view: &str,
    source: GraphRef,
    id: &str,
    (lifetime_base, policy_lifetime, proposal_lifetime): Lifetime,
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
            not_before_ms: lifetime_base,
            expires_at_ms: lifetime_base + policy_lifetime,
        })?;
    }
    let expected_head = e.inspect_governance_head(view, &host())?.decision_id;
    let proposal = GovernanceProposal {
        id: id.into(),
        view_id: view.into(),
        policy: reference.clone(),
        expected_head: expected_head.clone(),
        expires_at_ms: lifetime_base + proposal_lifetime,
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
            issued_at_ms: lifetime_base,
            expires_at_ms: lifetime_base + proposal_lifetime,
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
    let clock: Arc<dyn TrustedClock> = if a[2].starts_with("live_")
        || ["recorded_register", "recorded_refresh"].contains(&a[2].as_str())
    {
        Arc::new(SystemClock)
    } else {
        Arc::new(ManualClock::new(if a[2] == "expired" { 10000 } else { 20 }))
    };
    let lifetime: Lifetime = if a[2].starts_with("live_") {
        (clock.unix_millis()?, LIVE_LIFETIME.0, LIVE_LIFETIME.1)
    } else {
        (0, MANUAL_LIFETIME.0, MANUAL_LIFETIME.1)
    };
    let mut e = Engine::open_with_clock(&a[1], clock)?;
    match a[2].as_str() {
        "seed" | "live_seed" => {
            let source = write(&mut e, "Fleet", graph(0))?;
            let empty = write(&mut e, "Empty", GraphData::default())?;
            let accepted = accept(&e, "team", source.clone(), "initial", lifetime)?;
            let empty_accepted = accept(&e, "empty", empty, "empty-initial", lifetime)?;
            Ok(json!({"accepted":accepted,"empty":empty_accepted,"source":source}))
        }
        "register" | "recorded_register" => {
            let template: CompiledViewTemplate = read_json(&a[3]);
            Ok(serde_json::to_value(e.register_compiled_view(
                &a[4],
                &template,
                tick(&a[5]),
                &host(),
            )?)?)
        }
        "run" | "expired" | "outsider" | "live_run" | "live_outsider" => {
            let plan: Program = read_json(&a[3]);
            let h = if a[2] == "outsider" || a[2] == "live_outsider" {
                HostContext::new("outsider", Vec::<String>::new())
            } else {
                host()
            };
            Ok(serde_json::to_value(e.execute(&plan, &h)?)?)
        }
        "accepted_history" => Ok(serde_json::to_value(e.query_accepted_history_for(
            &a[4],
            &read_json::<AcceptedViewHistoryCut>(&a[3]),
            &host(),
        )?)?),
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
        "change" | "live_change" => Ok(serde_json::to_value(write(&mut e, "Fleet", graph(1))?)?),
        "publish" | "live_publish" => {
            let source = GraphRef {
                graph_id: "Fleet".into(),
                revision: e.head("Fleet", "main")?.unwrap(),
            };
            Ok(serde_json::to_value(accept(
                &e, "team", source, "next", lifetime,
            )?)?)
        }
        "refresh" | "recorded_refresh" => Ok(serde_json::to_value(e.refresh_view(
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
