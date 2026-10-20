use serde_json::{json, Value};
use weave_engine::HostContext;
use weave_science::{Request, ScienceSession};

fn request(value: Value) -> Request {
    serde_json::from_value(value).unwrap()
}
fn session() -> ScienceSession {
    ScienceSession::memory(HostContext::new("alice", ["g".into()])).unwrap()
}
fn graph() -> Value {
    json!({"nodes":[
        {"id":"I","entity_id":"I","space_id":"vectors","properties":{"vector":[1,0]}},
        {"id":"C","entity_id":"C","space_id":"vectors","properties":{"vector":[-1,0]}},
        {"id":"B","entity_id":"B","space_id":"vectors","properties":{"vector":[0,1]}},
        {"id":"A","entity_id":"A","space_id":"vectors","properties":{"vector":[1,0]}},
        {"id":"H","entity_id":"H","space_id":"vectors","readers":["bob"],"properties":{"vector":[1,0]}}
    ],"edges":[
        {"id":"ab2","predicate":"link","from":"A","to":"B","valid_time":{"start":0}},
        {"id":"ab1","predicate":"link","from":"A","to":"B","valid_time":{"start":0}},
        {"id":"ba","predicate":"link","from":"B","to":"A","valid_time":{"start":0}},
        {"id":"bc","predicate":"link","from":"B","to":"C","valid_time":{"start":10}},
        {"id":"cc","predicate":"link","from":"C","to":"C","valid_time":{"start":0}},
        {"id":"negative","predicate":"link","from":"A","to":"C","polarity":"negative","valid_time":{"start":0}},
        {"id":"hidden","predicate":"link","from":"A","to":"H","valid_time":{"start":0}}
    ]})
}
fn import(session: &mut ScienceSession, data: Value, expected_head: Value) -> String {
    let result = session
        .handle(request(
            json!({"operation":"import","graph_id":"g","expected_head":expected_head,"data":data}),
        ))
        .unwrap();
    result["results"][0]["revision"].as_str().unwrap().into()
}
fn program(revision: &str) -> Value {
    json!({"version":"0.21.0","commands":[{"op":"query","query":{"graph_id":"g","revision":revision}}]})
}
fn analyze(session: &mut ScienceSession, revision: &str, analysis: Value) -> Value {
    session
        .handle(request(
            json!({"operation":"analyze","program":program(revision),"analysis":analysis}),
        ))
        .unwrap()
}

#[test]
fn degrees_components_and_paths_respect_positive_multigraph_time_and_policy() {
    let mut s = session();
    let r = import(&mut s, graph(), Value::Null);
    let degree = analyze(&mut s, &r, json!({"algorithm":"degree","valid_at":5}));
    assert_eq!(degree["analysis"]["node_count"], 4);
    assert_eq!(degree["analysis"]["edge_count"], 4);
    assert_eq!(
        degree["analysis"]["nodes"]["A"],
        json!({"in_degree":1,"out_degree":2,"total_degree":3})
    );
    assert_eq!(degree["analysis"]["nodes"]["C"]["total_degree"], 2);
    assert_eq!(degree["analysis"]["nodes"]["I"]["total_degree"], 0);
    assert_eq!(degree["semantics"]["excluded_negative_edges"], 1);
    assert!(!serde_json::to_string(&degree).unwrap().contains("hidden"));
    let weak = analyze(
        &mut s,
        &r,
        json!({"algorithm":"components","mode":"weak","valid_at":5}),
    );
    assert_eq!(
        weak["analysis"]["components"],
        json!([["A", "B"], ["C"], ["I"]])
    );
    let strong = analyze(
        &mut s,
        &r,
        json!({"algorithm":"components","mode":"strong","valid_at":10}),
    );
    assert_eq!(
        strong["analysis"]["components"],
        json!([["A", "B"], ["C"], ["I"]])
    );
    let before = analyze(
        &mut s,
        &r,
        json!({"algorithm":"shortest_paths","source":"A","target":"C","valid_at":9}),
    );
    assert_eq!(before["analysis"]["distances"]["C"], Value::Null);
    let at = analyze(
        &mut s,
        &r,
        json!({"algorithm":"shortest_paths","source":"A","target":"C","valid_at":10}),
    );
    assert_eq!(at["analysis"]["distances"]["C"], 2);
    assert_eq!(at["analysis"]["path"], json!(["A", "B", "C"]));
    let reverse = analyze(
        &mut s,
        &r,
        json!({"algorithm":"shortest_paths","source":"C","target":"A","directed":false,"valid_at":10}),
    );
    assert_eq!(reverse["analysis"]["path"], json!(["C", "B", "A"]));
}

#[test]
fn exact_vectors_are_stable_scope_selected_and_numerically_bounded() {
    let mut s = session();
    let r = import(&mut s, graph(), Value::Null);
    for metric in ["cosine", "euclidean"] {
        let nearest = analyze(
            &mut s,
            &r,
            json!({"algorithm":"nearest_vectors","space_id":"vectors","query":[1,0],"metric":metric,"k":2}),
        );
        assert_eq!(nearest["analysis"]["candidate_count"], 4);
        assert_eq!(
            nearest["analysis"]["neighbors"],
            json!([{"id":"A","entity_id":"A","space_id":"vectors","distance":0.0},{"id":"I","entity_id":"I","space_id":"vectors","distance":0.0}])
        );
    }
    let huge = json!({"nodes":[{"id":"n","entity_id":"n","space_id":"vectors","properties":{"vector":[1e300,1e300]}}]});
    let r2 = import(&mut s, huge, json!(r));
    let nearest = analyze(
        &mut s,
        &r2,
        json!({"algorithm":"nearest_vectors","space_id":"vectors","query":[1e300,1e300],"metric":"cosine"}),
    );
    assert!(
        nearest["analysis"]["neighbors"][0]["distance"]
            .as_f64()
            .unwrap()
            < 1e-14
    );
    let wrong = s.handle(request(json!({"operation":"analyze","program":program(&r2),"analysis":{"algorithm":"nearest_vectors","space_id":"vectors","query":[1],"metric":"euclidean"}}))).unwrap_err();
    assert_eq!(wrong.code, "E_SCIENCE_VECTOR");
    let overflow = json!({"nodes":[{"id":"n","entity_id":"n","space_id":"vectors","properties":{"vector":[1e308]}}]});
    let r3 = import(&mut s, overflow, json!(r2));
    let err = s.handle(request(json!({"operation":"analyze","program":program(&r3),"analysis":{"algorithm":"nearest_vectors","space_id":"vectors","query":[-1e308],"metric":"euclidean"}}))).unwrap_err();
    assert_eq!(err.code, "E_SCIENCE_NUMERIC");
}

#[test]
fn pagerank_matches_closed_form_and_reports_nonconvergence() {
    let mut s = session();
    let data = json!({"nodes":[{"id":"A","entity_id":"A","space_id":"s"},{"id":"B","entity_id":"B","space_id":"s"}],"edges":[
        {"id":"ab","predicate":"p","from":"A","to":"B","valid_time":{"start":0}},
        {"id":"bb","predicate":"p","from":"B","to":"B","valid_time":{"start":0}}
    ]});
    let r = import(&mut s, data, Value::Null);
    let output = analyze(&mut s, &r, json!({"algorithm":"pagerank"}));
    assert_eq!(output["analysis"]["converged"], true);
    assert!((output["analysis"]["scores"]["A"].as_f64().unwrap() - 0.075).abs() < 1e-12);
    assert!((output["analysis"]["scores"]["B"].as_f64().unwrap() - 0.925).abs() < 1e-12);
    let one = analyze(
        &mut s,
        &r,
        json!({"algorithm":"pagerank","max_iterations":1}),
    );
    assert_eq!(one["analysis"]["converged"], false);
    assert_eq!(one["analysis"]["iterations"], 1);
}

#[test]
fn pinned_results_reproduce_after_correction_and_restart() {
    let path = std::env::temp_dir().join(format!(
        "weave-science-test-{}-{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let host = HostContext::new("alice", ["g".into()]);
    let mut s = ScienceSession::open(&path, host.clone()).unwrap();
    let r = import(&mut s, graph(), Value::Null);
    let old = analyze(&mut s, &r, json!({"algorithm":"degree","valid_at":10}));
    let mut corrected = graph();
    corrected["edges"] = json!([]);
    let new_r = import(&mut s, corrected, json!(r));
    drop(s);
    let mut reopened = ScienceSession::open(&path, host).unwrap();
    let repeated = analyze(
        &mut reopened,
        &r,
        json!({"algorithm":"degree","valid_at":10}),
    );
    assert_eq!(old, repeated);
    assert_eq!(
        analyze(&mut reopened, &new_r, json!({"algorithm":"degree"}))["analysis"]["edge_count"],
        0
    );
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn analysis_mutation_budget_output_and_partial_fail_explicitly() {
    let mut s = session();
    let r = import(&mut s, graph(), Value::Null);
    let mut p = program(&r);
    p["commands"]
        .as_array_mut()
        .unwrap()
        .push(json!({"op":"commit","graph_id":"g","expected_head":r,"data":{}}));
    let err = s
        .handle(request(
            json!({"operation":"analyze","program":p,"analysis":{"algorithm":"degree"}}),
        ))
        .unwrap_err();
    assert_eq!(err.code, "E_SCIENCE_READ_ONLY");
    let err = s.handle(request(json!({"operation":"analyze","program":program(&r),"analysis":{"algorithm":"degree"},"limits":{"max_work":1}}))).unwrap_err();
    assert_eq!(err.code, "E_SCIENCE_BUDGET");
    let err = s.handle(request(json!({"operation":"analyze","program":program(&r),"analysis":{"algorithm":"degree"},"limits":{"max_output_bytes":1}}))).unwrap_err();
    assert_eq!(err.code, "E_SCIENCE_OUTPUT");
    assert_eq!(
        analyze(&mut s, &r, json!({"algorithm":"degree"}))["analysis"]["edge_count"],
        5
    );
    let mut missing = graph();
    missing["nodes"][0]["metadata"] = json!([{"graph_id":"missing","revision":"missing-revision"}]);
    let missing_r = import(&mut s, missing, json!(r));
    let mut query = program(&missing_r);
    query["commands"][0]["query"]["include_metadata"] = json!(true);
    let err = s
        .handle(request(
            json!({"operation":"analyze","program":query,"analysis":{"algorithm":"degree"}}),
        ))
        .unwrap_err();
    assert_eq!(err.code, "E_SCIENCE_PARTIAL");
    let partial = s.handle(request(json!({"operation":"analyze","program":query,"analysis":{"algorithm":"degree"},"allow_partial":true}))).unwrap();
    assert_eq!(partial["input"]["coverage"], "partial");
    assert!(!partial["input"]["diagnostics"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn import_does_not_expand_host_write_authority() {
    let mut s = ScienceSession::memory(HostContext::new("alice", Vec::<String>::new())).unwrap();
    let err = s
        .handle(request(
            json!({"operation":"import","graph_id":"g","data":graph()}),
        ))
        .unwrap_err();
    assert_eq!(err.code, "E_FORBIDDEN");
}

#[test]
fn strict_json_rejects_duplicate_keys_and_trailing_input() {
    assert!(weave_science::parse_request(
        br#"{"operation":"capabilities","operation":"capabilities"}"#
    )
    .is_err());
    assert!(weave_science::parse_request(br#"{"operation":"import","graph_id":"g","data":{"nodes":[{"id":"n","entity_id":"n","space_id":"s","properties":{"secret":1,"secret":2}}]}}"#).is_err());
    assert!(weave_science::parse_request(br#"{"operation":"capabilities"} {}"#).is_err());
    assert!(weave_science::parse_request(br#"{"operation":"capabilities"}"#).is_ok());
}

#[test]
fn incremental_import_appends_fact_batches_after_a_snapshot() {
    let mut s = session();
    let base = import(&mut s, graph(), Value::Null);
    let batch = |changes: Value, expected: Value| {
        request(
            json!({"operation":"import","graph_id":"g","expected_head":expected,"changes":changes}),
        )
    };
    let first = s
        .handle(batch(
            json!([
                {"change":"assert","fact":{"kind":"node","id":"D","entity_id":"D","space_id":"vectors","properties":{"vector":[0,-1]}}},
                {"change":"assert","fact":{"kind":"edge","id":"cd","predicate":"link","from":"C","to":"D","valid_time":{"start":0}}}
            ]),
            json!(base),
        ))
        .unwrap();
    assert_eq!(first["results"][0]["kind"], "committed");
    let r1 = first["results"][0]["revision"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(r1.starts_with("fact:sha256:"));
    // A batch without a head check, then a stale head check that writes nothing.
    let second = s
        .handle(batch(
            json!([{"change":"retract","target":{"kind":"edge","id":"cc"}}]),
            Value::Null,
        ))
        .unwrap();
    let r2 = second["results"][0]["revision"]
        .as_str()
        .unwrap()
        .to_string();
    let stale = s
        .handle(batch(
            json!([{"change":"retract","target":{"kind":"edge","id":"bc"}}]),
            json!(r1),
        ))
        .unwrap_err();
    assert_eq!(stale.code, "E_CONFLICT");
    let degree = analyze(&mut s, &r2, json!({"algorithm":"degree"}));
    let earlier = analyze(&mut s, &r1, json!({"algorithm":"degree"}));
    assert_ne!(
        degree["analysis"], earlier["analysis"],
        "the retraction is visible at r2 only"
    );
    // Full-snapshot import remains available on top of fact revisions.
    let snapshot = import(&mut s, graph(), json!(r2));
    assert!(snapshot.starts_with("sha256:"));
    // Exactly one of data and changes.
    for body in [
        json!({"operation":"import","graph_id":"g"}),
        json!({"operation":"import","graph_id":"g","data":graph(),"changes":[]}),
    ] {
        assert_eq!(s.handle(request(body)).unwrap_err().code, "E_INPUT");
    }
}

#[test]
fn json_api_exposes_expand_as_of_changes_and_explain() {
    let mut s = session();
    let first = import(&mut s, graph(), Value::Null);
    let execute = |s: &mut ScienceSession, value: Value| {
        s.handle(request(json!({"operation":"execute","program":
            {"version":"0.23.0","commands":[{"op":"evaluate","value":value}]}})))
            .unwrap()["results"][0]["result"]
            .clone()
    };
    let ids = |result: &Value, list: &str| -> Vec<String> {
        let mut ids: Vec<String> = result["graph"][list]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_string())
            .collect();
        ids.sort();
        ids
    };
    // k-hop expansion: the hidden edge and node stay out for alice.
    let expand = json!({"kind":"expand","graph_id":"g","seeds":["A"],"hops":2,"valid_at":20});
    let expanded = execute(&mut s, expand.clone());
    assert_eq!(ids(&expanded, "nodes"), ["A", "B", "C"]);
    assert_eq!(
        ids(&expanded, "edges"),
        ["ab1", "ab2", "ba", "bc", "cc", "negative"]
    );
    assert_eq!(expanded["coverage"], "complete");
    assert!(!expanded["fact_provenance"].as_array().unwrap().is_empty());
    // An expansion is analysis input like any graph value.
    let degree = s
        .handle(request(json!({"operation":"analyze","program":
            {"version":"0.23.0","commands":[{"op":"evaluate","value":expand}]},
            "analysis":{"algorithm":"degree"}})))
        .unwrap();
    assert_eq!(
        degree["input"]["graph"]["nodes"].as_array().unwrap().len(),
        3
    );
    // explain turns it into a derivation graph.
    let explained = execute(
        &mut s,
        json!({"kind":"explain","input":{"kind":"expand","graph_id":"g","seeds":["A"],"hops":1}}),
    );
    assert!(!explained["graph"]["nodes"].as_array().unwrap().is_empty());
    // A correction, then as_of and changes across it.
    let now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
    };
    std::thread::sleep(std::time::Duration::from_millis(5));
    let before = now();
    std::thread::sleep(std::time::Duration::from_millis(5));
    let second = s
        .handle(request(
            json!({"operation":"import","graph_id":"g","changes":[
            {"change":"retract","target":{"kind":"edge","id":"bc"}}]}),
        ))
        .unwrap()["results"][0]["revision"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(first, second);
    let head = execute(
        &mut s,
        json!({"kind":"query","query":{"graph_id":"g","predicate":"link","from":"B"}}),
    );
    let change_set = execute(
        &mut s,
        json!({"kind":"changes","graph_id":"g","from":{"kind":"cursor","revision":first}}),
    );
    assert_eq!(
        change_set["changes"]["entries"].as_array().unwrap().len(),
        1
    );
    assert_eq!(change_set["changes"]["entries"][0]["change"], "retracted");
    assert_eq!(change_set["changes"]["to_revision"], json!(second));
    assert!(ids(&head, "edges").iter().all(|e| e != "bc"));
    let known = execute(
        &mut s,
        json!({"kind":"as_of","unix_millis":before,
        "query":{"graph_id":"g","predicate":"link","from":"B"}}),
    );
    assert_eq!(ids(&known, "edges"), ["ba", "bc"]);
    assert_eq!(known["input_snapshots"][0]["revision"], json!(first));
    let capabilities = s
        .handle(request(json!({"operation":"capabilities"})))
        .unwrap();
    for operator in [
        "expand",
        "as_of",
        "changes",
        "explain",
        "nearest",
        "similar_candidates",
    ] {
        assert!(capabilities["query_operators"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o == operator));
    }
    // A change set is not analysis input.
    let error = s
        .handle(request(json!({"operation":"analyze","program":
            {"version":"0.23.0","commands":[{"op":"evaluate","value":
                {"kind":"changes","graph_id":"g","from":{"kind":"cursor","revision":first}}}]},
            "analysis":{"algorithm":"degree"}})))
        .unwrap_err();
    assert_eq!(error.code, "E_SCIENCE_RESULT");
}
