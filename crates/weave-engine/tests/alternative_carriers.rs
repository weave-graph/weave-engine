//! Native composition checks beyond the independent Boolean/temporal oracles.
use serde_json::{json, Value};
use weave_contract::{Command, GraphData, GraphRef, Program, QueryPlan, QueryResult};
use weave_engine::{Engine, HostContext};
fn host(principal: &str) -> HostContext {
    HostContext::new(principal, ["Proof".into(), "Input".into(), "Saved".into()])
}
fn commit(engine: &mut Engine, name: &str, data: Value) -> GraphRef {
    let program: Program = serde_json::from_value(json!({"version":weave_contract::VERSION,"commands":[{
        "op":"commit","graph_id":name,"expected_head":engine.head(name,"main").unwrap(),"data":data
    }]})).unwrap();
    engine.execute(&program, &host("alice")).unwrap();
    GraphRef {
        graph_id: name.into(),
        revision: engine.head(name, "main").unwrap().unwrap(),
    }
}
fn query(engine: &Engine, graph: &str, principal: &str) -> QueryResult {
    let q: QueryPlan = serde_json::from_value(json!({"graph_id":graph})).unwrap();
    engine.query(&q, &host(principal)).unwrap()
}
#[test]
fn generated_copies_retain_grouped_original_edge_and_attachment_readers() {
    for private_edge in [false, true] {
        let mut engine = Engine::memory().unwrap();
        let proof = commit(
            &mut engine,
            "Proof",
            json!({"nodes":[{"id":"n","entity_id":"n","space_id":"s"}],"edges":[{"id":"proof","predicate":"p","from":"n","to":"n","valid_time":{"start":0,"end":10}}]}),
        );
        let node_proof = json!({"graph_id":proof.graph_id,"revision":proof.revision,"node_id":"n"});
        let assertion_proof =
            json!({"graph_id":proof.graph_id,"revision":proof.revision,"assertion_id":"proof"});
        let group =
            json!({"operator":"test","premises":[assertion_proof],"node_premises":[node_proof]});
        let input = commit(
            &mut engine,
            "Input",
            json!({
                "nodes":[{"id":"a","entity_id":"a","space_id":"s"},{"id":"b","entity_id":"b","space_id":"s"}],
                "edges":[{"id":"e","predicate":"p","from":"a","to":"b","valid_time":{"start":0,"end":10},"derived_from":[assertion_proof],"derivations":[group],"readers":if private_edge {vec!["alice"]}else{vec![]}}],
                "attachments":[{"id":"a","host":{"kind":"graph"},"key":"note","value":{"kind":"literal","value":"private attachment"},"valid_time":{"start":0,"end":10},"derivations":[group],"readers":if private_edge {vec![]}else{vec!["alice"]}}]
            }),
        );
        let source = query(&engine, "Input", "alice");
        assert!(source.graph.edges[0].derivations[0]
            .premises
            .iter()
            .any(|r| r.graph_id == input.graph_id
                && r.revision == input.revision
                && r.assertion_id == "e"));
        assert!(source.graph.attachments[0].derivations[0]
            .premises
            .iter()
            .any(|r| r.graph_id == input.graph_id
                && r.revision == input.revision
                && r.assertion_id == "a"));
        let program:Program=serde_json::from_value(json!({"version":weave_contract::VERSION,"commands":[{"op":"evaluate","value":{"kind":"window","window":{"start":1,"end":9},"input":{"kind":"query","query":{"graph_id":"Input"}}}}]})).unwrap();
        let result = engine
            .execute(&program, &host("alice"))
            .unwrap()
            .pop()
            .unwrap();
        let weave_contract::CommandResult::Queried { result } = result else {
            panic!("query")
        };
        let mut copied = result.graph;
        copied.influence = None;
        copied.nodes.iter_mut().for_each(|n| n.readers.clear());
        copied.edges.iter_mut().for_each(|e| e.readers.clear());
        copied
            .attachments
            .iter_mut()
            .for_each(|a| a.readers.clear());
        // Persist only generated records, without relying on the result envelope.
        let saved: GraphData = copied;
        engine
            .execute(
                &Program {
                    version: weave_contract::VERSION.into(),
                    source_revisions: vec![],
                    commands: vec![Command::Commit {
                        graph_id: "Saved".into(),
                        branch_id: "main".into(),
                        expected_head: None,
                        data: saved,
                    }],
                },
                &host("alice"),
            )
            .unwrap();
        let owner = query(&engine, "Saved", "alice");
        assert_eq!(owner.graph.edges.len(), 1);
        assert_eq!(owner.graph.attachments.len(), 1);
        let other = query(&engine, "Saved", "bob");
        if private_edge {
            assert!(other.graph.edges.is_empty());
        } else {
            assert!(other.graph.attachments.is_empty());
            assert_eq!(other.graph.edges.len(), 1);
        }
    }
}
