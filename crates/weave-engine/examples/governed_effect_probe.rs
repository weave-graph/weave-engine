//! Deterministic trusted-host fault probe and fake sink. No network or real destination.
use ed25519_dalek::SigningKey;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc};
use weave_contract::{
    handler_registration::seal_handler_template, GraphData, GraphRef, GraphSchema, Program, VERSION,
};
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new("effect-owner", ["input".into(), "request".into()])
}
fn field<'a>(v: &'a Value, name: &str) -> &'a str {
    v[name].as_str().unwrap_or("")
}
fn fault(v: &Value, point: &str) {
    if field(v, "crash") == point {
        std::process::exit(if point == "before" { 86 } else { 87 });
    }
}
fn schema() -> GraphSchema {
    serde_json::from_value(json!({"id":"reference-request","revision":"1"})).unwrap()
}
fn seed(
    e: &mut Engine,
    supplied: Option<&Value>,
) -> std::result::Result<Value, Box<dyn std::error::Error>> {
    let data = GraphData {
        schema: Some(schema()),
        ..GraphData::default()
    };
    let p: Program = serde_json::from_value(
        json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"input","expected_head":null,"data":data}]}),
    )?;
    e.execute(&p, &host())?;
    let template = if let Some(value) = supplied {
        serde_json::from_value(value.clone())?
    } else {
        seal_handler_template(serde_json::from_value(
        json!({"format":"weave-handler-registration/1","protocol":VERSION,"name":"ReferenceRequest","revision":"1","input":{"graph_id":"input","branch_id":"main","metadata_depth":0},"event_types":["graph.committed","graph.accepted"],"recipe":{"bindings":[],"output":"$event"},"output_slot":"request","source_revisions":[],"definition_digest":""}),
    )?).map_err(|d|format!("{}: {}",d.code,d.message))?
    };
    let producer = AdapterManifest {
        id: "producer".into(),
        version: "1".into(),
        artifact_digest: template.definition_digest.clone(),
        config_revision: "1".into(),
        principal: host().principal,
        subscriptions: vec![SubscriptionScope {
            graph_id: "input".into(),
            branch_id: "main".into(),
        }],
        output_graphs: vec!["request".into()],
        effect_destinations: vec![],
        max_attempts: 5,
        lease_ms: 1000,
        max_pending_events: 100,
        projection_replay: true,
    };
    e.install_compiled_handler(
        &producer,
        &template,
        &HandlerOutputBinding {
            slot: "request".into(),
            graph_id: "request".into(),
            branch_id: "main".into(),
        },
        &host(),
    )?;
    e.set_adapter_state("producer", "running")?;
    let event = e.poll_adapter("producer")?.unwrap();
    let prepared = e.prepare_compiled_handler("producer", &event.id, &event.lease)?;
    e.complete_prepared_handler(
        "producer",
        &event.id,
        &event.lease,
        &prepared.preparation_id,
    )?;
    let source = GraphRef {
        graph_id: "request".into(),
        revision: e.head("request", "main")?.unwrap(),
    };
    let key = SigningKey::from_bytes(&[91; 32]);
    let policy = GovernancePolicy {
        view_id: "requests".into(),
        reference: GovernancePolicyRef {
            id: "policy".into(),
            revision: "1".into(),
        },
        members: vec![weave_policy::public_key(&key)],
        threshold: 1,
        proposers: vec![host().principal],
        readers: vec![],
        allowed_sources: vec![GovernanceSourceScope {
            graph_id: "request".into(),
            branch_id: "main".into(),
        }],
        not_before_ms: 0,
        expires_at_ms: 100000,
    };
    e.install_governance_root(&policy)?;
    let proposal = GovernanceProposal {
        id: "publish".into(),
        view_id: "requests".into(),
        policy: policy.reference.clone(),
        expected_head: None,
        expires_at_ms: 90000,
        action: GovernanceAction::Publish {
            source: source.clone(),
            branch_id: "main".into(),
        },
    };
    let pr = e.propose_governance(&proposal, &host())?;
    e.record_governance_approval(
        &sign_governance_approval(
            GovernanceApproval {
                proposal_id: "publish".into(),
                proposal_digest: pr.digest,
                view_id: "requests".into(),
                policy: policy.reference,
                expected_head: None,
                member: weave_policy::public_key(&key),
                issued_at_ms: 0,
                expires_at_ms: 80000,
                nonce: "approve".into(),
            },
            &key,
        )?,
        &host(),
    )?;
    let acceptance = e.accept_governance(
        &GovernanceDecisionRequest {
            proposal_id: "publish".into(),
            nonce: "accept".into(),
        },
        &host(),
    )?;
    let grant = GovernedEffectGrant {
        id: "execute-reference".into(),
        revision: "1".into(),
        principal: host().principal,
        view_id: "requests".into(),
        source: SubscriptionScope {
            graph_id: "request".into(),
            branch_id: "main".into(),
        },
        request_schema: schema(),
        destination_id: "reference-sink".into(),
        destination_principal: host().principal,
        encoder: EffectEncoder::CanonicalGraphV1,
        execution_id: "trace-execution".into(),
        start: EffectStart::ReplayHistory,
        not_before_ms: 0,
        expires_at_ms: 70000,
    };
    let m = AdapterManifest {
        id: "bridge".into(),
        version: "1".into(),
        artifact_digest: governed_effect_grant_digest(&grant)?,
        config_revision: "1".into(),
        principal: host().principal,
        subscriptions: vec![grant.source.clone()],
        output_graphs: vec![],
        effect_destinations: vec!["reference-sink".into()],
        max_attempts: 5,
        lease_ms: 1000,
        max_pending_events: 100,
        projection_replay: false,
    };
    e.install_governed_effect(&m, &grant, &host())?;
    e.set_adapter_state("bridge", "running")?;
    let delivery = e.poll_governance("bridge", "requests", &host())?.unwrap();
    Ok(json!({"source":source,"preparation":prepared,"acceptance":acceptance,"delivery":delivery}))
}
fn sink(path: &Path, v: &Value) -> std::result::Result<Value, Box<dyn std::error::Error>> {
    let mut c = rusqlite::Connection::open(path)?;
    c.execute_batch("CREATE TABLE IF NOT EXISTS receipts(key TEXT PRIMARY KEY,digest TEXT NOT NULL,response TEXT NOT NULL);CREATE TABLE IF NOT EXISTS actions(id INTEGER PRIMARY KEY AUTOINCREMENT,key TEXT NOT NULL,digest TEXT NOT NULL);")?;
    let payload: Vec<u8> = serde_json::from_value(v["payload"].clone())?;
    let key = field(v, "idempotency_key");
    let hash = format!("sha256:{:x}", Sha256::digest(&payload));
    let tx = c.transaction()?;
    let idempotent = v["idempotent"].as_bool().unwrap_or(true);
    if idempotent {
        let old: Option<(String, String)> = tx
            .query_row(
                "SELECT digest,response FROM receipts WHERE key=?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((digest, response)) = old {
            if digest != hash {
                return Err("sink idempotency conflict".into());
            }
            return Ok(serde_json::from_str(&response)?);
        }
    }
    tx.execute(
        "INSERT INTO actions(key,digest) VALUES (?1,?2)",
        params![key, hash],
    )?;
    let response =
        json!({"receipt_id":format!("sink-{}",tx.last_insert_rowid()),"response_digest":hash});
    if idempotent {
        tx.execute(
            "INSERT INTO receipts VALUES (?1,?2,?3)",
            params![key, hash, serde_json::to_string(&response)?],
        )?;
    }
    fault(v, "before");
    tx.commit()?;
    fault(v, "after");
    Ok(response)
}
fn run() -> std::result::Result<Value, Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let path = Path::new(args.get(1).ok_or("DB path required")?);
    let v: Value =
        serde_json::from_slice(&std::fs::read(args.get(2).ok_or("request path required")?)?)?;
    if field(&v, "mode") == "sink" {
        return sink(path, &v);
    }
    let mut e = Engine::open_with_clock(
        path,
        Arc::new(ManualClock::new(v["clock_ms"].as_i64().unwrap_or(10))),
    )?;
    let result = match field(&v, "mode") {
        "seed" => seed(&mut e, v.get("template"))?,
        "enqueue" => serde_json::to_value(e.enqueue_governed_effect_test_before_commit(
            "bridge",
            field(&v, "event"),
            field(&v, "lease"),
            &host(),
            || fault(&v, "before"),
        )?)?,
        "begin" => serde_json::to_value(e.begin_governed_effect_test_before_commit(
            field(&v, "intent"),
            &host(),
            || fault(&v, "before"),
        )?)?,
        "status" => serde_json::to_value(e.read_governed_effect(field(&v, "intent"), &host())?)?,
        "reconcile" => {
            e.reconcile_governed_effect_test_before_commit(
                field(&v, "intent"),
                field(&v, "attempt"),
                serde_json::from_value(v["outcome"].clone())?,
                &serde_json::from_value(v["evidence"].clone())?,
                &host(),
                || fault(&v, "before"),
            )?;
            json!({"ok":true})
        }
        "cancel" => {
            e.cancel_governed_effect(field(&v, "intent"), &host())?;
            json!({"ok":true})
        }
        "revoke" => {
            e.revoke_governed_effect_grant("bridge", &host())?;
            json!({"ok":true})
        }
        _ => return Err("unknown mode".into()),
    };
    fault(&v, "after");
    Ok(result)
}
fn main() {
    match run() {
        Ok(value) => println!("{value}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1)
        }
    }
}
