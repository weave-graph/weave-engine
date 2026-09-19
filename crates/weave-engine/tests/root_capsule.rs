use serde_json::json;
use weave_contract::{GraphRef, Program, QueryPlan};
use weave_engine::{Engine, HostContext};

fn host(who: &str) -> HostContext {
    HostContext::new(who, ["A", "B", "Extra"].map(String::from))
}
fn cycle(private_b: bool, changed: bool) -> (Engine, GraphRef) {
    let mut engine = Engine::memory().unwrap();
    let mut commits = Vec::new();
    for (graph, target) in [("A", "B"), ("B", "A")] {
        let readers = if private_b && graph == "B" {
            vec!["alice"]
        } else {
            vec![]
        };
        commits.push(json!({"graph_id":graph,"data":{
            "nodes":[{"id":graph,"entity_id":graph,"space_id":"s","readers":readers,
                "properties":{"changed":changed && graph == "B"}}],
            "attachments":[{"id":format!("proof-{graph}"),"host":{"kind":"graph"},
                "key":"evidence","value":{"kind":"graph","reference":{
                    "graph_id":target,"revision":format!("logical:shared:{target}")}},
                "valid_time":{"start":0,"end":null}}]
        }}));
    }
    let program: Program = serde_json::from_value(json!({"version":"0.4.0","commands":[{
        "op":"commit_batch","batch_id":"shared","commits":commits
    }]}))
    .unwrap();
    engine.execute(&program, &host("alice")).unwrap();
    (
        engine,
        GraphRef {
            graph_id: "A".into(),
            revision: "logical:shared:A".into(),
        },
    )
}
fn query(reference: &GraphRef) -> QueryPlan {
    serde_json::from_value(
        json!({"graph_id":reference.graph_id,"revision":reference.revision,
        "include_metadata":true,"max_depth":8}),
    )
    .unwrap()
}
#[test]
fn logical_cycle_transport_keeps_snapshots_quarantined_and_is_idempotent() {
    let (source, root) = cycle(false, false);
    let capsule = source.export_capsule(&root, &host("bob")).unwrap();
    assert_eq!(capsule.format, "weave-capsule-0.2");
    let encoded = serde_json::to_value(&capsule).unwrap();
    assert_eq!(encoded["manifests"].as_array().unwrap().len(), 1);
    assert_eq!(capsule.revisions.len(), 2);
    let mut dest = Engine::memory().unwrap();
    assert_eq!(dest.receive_capsule(&capsule, &host("bob")).unwrap(), 2);
    assert_eq!(dest.receive_capsule(&capsule, &host("bob")).unwrap(), 0);
    assert_eq!(dest.event_count().unwrap(), 0);
    assert_eq!(dest.head("A", "main").unwrap(), None);
    assert_eq!(dest.head("B", "main").unwrap(), None);
    assert_eq!(
        source.query(&query(&root), &host("bob")).unwrap(),
        dest.query(&query(&root), &host("bob")).unwrap()
    );
    dest.accept_revision(&root, "offline", None, &host("bob"))
        .unwrap();
    assert_eq!(dest.head("A", "offline").unwrap(), Some(root.revision));
    assert_eq!(dest.head("B", "main").unwrap(), None);
    assert_eq!(dest.event_count().unwrap(), 1);
}
#[test]
fn complete_manifest_export_cannot_disclose_hidden_members() {
    let (source, root) = cycle(true, false);
    let error = source.export_capsule(&root, &host("bob")).unwrap_err();
    assert_eq!(error.code, "E_UNAVAILABLE");
    assert!(!error.message.contains("B"));
    let permitted = source.export_capsule(&root, &host("alice")).unwrap();
    assert_eq!(permitted.revisions.len(), 2);
}
#[test]
fn tampered_and_incomplete_logical_batches_fail_before_storage() {
    let (source, root) = cycle(false, false);
    let capsule = source.export_capsule(&root, &host("bob")).unwrap();
    let mut dest = Engine::memory().unwrap();
    let mut tampered = capsule.clone();
    tampered.revisions[0].data.nodes[0].entity_id = "tampered".into();
    assert_eq!(
        dest.receive_capsule(&tampered, &host("bob"))
            .unwrap_err()
            .code,
        "E_INTEGRITY"
    );
    let mut incomplete = capsule.clone();
    incomplete.revisions.retain(|r| r.graph_id != "B");
    incomplete.external_dependencies.push(GraphRef {
        graph_id: "B".into(),
        revision: "logical:shared:B".into(),
    });
    assert_eq!(
        dest.receive_capsule(&incomplete, &host("bob"))
            .unwrap_err()
            .code,
        "E_INTEGRITY"
    );
    assert_eq!(dest.receive_capsule(&capsule, &host("bob")).unwrap(), 2);
    assert_eq!(dest.event_count().unwrap(), 0);
}
#[test]
fn a_logical_batch_identity_cannot_equivocate_after_receipt() {
    let (source, root) = cycle(false, false);
    let original = source.export_capsule(&root, &host("bob")).unwrap();
    let (other, _) = cycle(false, true);
    let changed = other.export_capsule(&root, &host("bob")).unwrap();
    let mut dest = Engine::memory().unwrap();
    dest.receive_capsule(&original, &host("bob")).unwrap();
    assert_eq!(
        dest.receive_capsule(&changed, &host("bob"))
            .unwrap_err()
            .code,
        "E_EQUIVOCATION"
    );
    assert_eq!(
        source.query(&query(&root), &host("bob")).unwrap(),
        dest.query(&query(&root), &host("bob")).unwrap()
    );
    assert_eq!(dest.event_count().unwrap(), 0);
}

#[test]
fn separately_received_revisions_cannot_form_an_ancestry_cycle() {
    use sha2::{Digest, Sha256};
    use weave_contract::{GraphData, ManifestMember, SnapshotManifest};
    use weave_engine::{Capsule, CapsuleRevision};
    fn fragment(batch: &str, parent_batch: &str) -> Capsule {
        let graph = "A".to_string();
        let branch = "main".to_string();
        let parent = Some(format!("logical:{parent_batch}:A"));
        let data: GraphData = serde_json::from_value(json!({"nodes":[]})).unwrap();
        let digest = format!(
            "sha256:{:x}",
            Sha256::digest(
                serde_json::to_vec(&("weave-revision-v0.1", &graph, &branch, &parent, &data))
                    .unwrap()
            )
        );
        let revision = format!("logical:{batch}:A");
        let root = GraphRef {
            graph_id: graph.clone(),
            revision: revision.clone(),
        };
        Capsule {
            format: "weave-capsule-0.2".into(),
            root,
            external_dependencies: vec![GraphRef {
                graph_id: graph.clone(),
                revision: parent.clone().unwrap(),
            }],
            manifests: vec![SnapshotManifest {
                batch_id: batch.into(),
                members: vec![ManifestMember {
                    graph_id: graph.clone(),
                    branch_id: branch.clone(),
                    revision: revision.clone(),
                    parent: parent.clone(),
                    content_digest: digest,
                }],
            }],
            revisions: vec![CapsuleRevision {
                graph_id: graph,
                branch_id: branch,
                revision,
                parent,
                data,
            }],
        }
    }
    let first = fragment("first", "second");
    let second = fragment("second", "first");
    let mut dest = Engine::memory().unwrap();
    assert_eq!(dest.receive_capsule(&first, &host("alice")).unwrap(), 1);
    assert_eq!(
        dest.receive_capsule(&second, &host("alice"))
            .unwrap_err()
            .code,
        "E_INTEGRITY"
    );
    assert_eq!(dest.receive_capsule(&first, &host("alice")).unwrap(), 0);
    assert_eq!(dest.event_count().unwrap(), 0);
}
