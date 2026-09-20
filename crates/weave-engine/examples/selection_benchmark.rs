//! Small fixed workload, reporting end-to-end refresh time separately from membership work.
use serde_json::json;
use std::time::Instant;
use weave_contract::*;
use weave_engine::*;
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let host = HostContext::new("bench", ["g".into()]);
    let mut engine = Engine::memory()?;
    let mut data: GraphData = serde_json::from_value(
        json!({"nodes":[{"id":"a","entity_id":"A","space_id":"s"},{"id":"b","entity_id":"B","space_id":"s"}],"edges":(0..256).map(|i|json!({"id":format!("e{i:03}"),"predicate":"p","from":"a","to":"b","valid_time":{"start":0}})).collect::<Vec<_>>()}),
    )?;
    fn commit(e: &mut Engine, d: &GraphData, h: &HostContext) -> weave_engine::Result<()> {
        let expected_head = e.head("g", "main")?;
        e.execute(
            &Program {
                version: VERSION.into(),
                source_revisions: vec![],
                commands: vec![Command::Commit {
                    graph_id: "g".into(),
                    branch_id: "main".into(),
                    expected_head,
                    data: d.clone(),
                }],
            },
            h,
        )?;
        Ok(())
    }
    commit(&mut engine, &data, &host)?;
    for id in ["full", "incremental"] {
        engine.register_view(
            &ViewDefinition {
                id: id.into(),
                clock: ViewClock::Fixed,
                expression: GraphExpression::Query {
                    query: serde_json::from_value(json!({"graph_id":"g"}))?,
                },
            },
            None,
            &host,
        )?;
    }
    engine.enroll_incremental_view("incremental", &host)?;
    engine.refresh_view("incremental", None, &host)?;
    let mut samples = Vec::new();
    for iteration in 1..=5 {
        data.edges[0]
            .properties
            .insert("correction".into(), iteration.into());
        commit(&mut engine, &data, &host)?;
        let mut values = Vec::new();
        let mut times = [0.0; 2];
        let mut work = ViewSelectionWork::default();
        for index in if iteration % 2 == 1 { [0, 1] } else { [1, 0] } {
            let id = if index == 0 { "full" } else { "incremental" };
            let started = Instant::now();
            let (value, counters) = engine.refresh_view_with_work(id, None, &host)?;
            times[index] = started.elapsed().as_secs_f64() * 1000.0;
            if index == 1 {
                work = counters;
            }
            values.push(value.result);
        }
        assert_eq!(values[0], values[1]);
        assert_eq!(work.memberships_evaluated, 1);
        assert_eq!(work.oracle_disagreements, 0);
        samples.push(json!({"iteration":iteration,"full_refresh_ms":times[0],"incremental_refresh_ms":times[1],"raw_records_loaded":work.source_records,"kernel_records_hashed":work.hashed_records,"memberships_evaluated":work.memberships_evaluated,"nodes_repinned":work.output_nodes_repinned,"claims_rendered":work.output_claims_rendered,"fallback_runs":work.fallback_runs,"result_bytes":serde_json::to_vec(&values[0])?.len()}));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"profile":"debug","storage":"in-memory SQLite","workload":"2 nodes, 256 claims, one claim property correction per iteration; alternating evaluation order","samples":samples,"limits":"small local fixture; full snapshot verification/comparison and output repinning remain; no speedup or capacity claim"})
        )?
    );
    Ok(())
}
