//! Test-only host process boundary probes and a small honest end-to-end measurement.
use serde_json::json;
use weave_contract::*;
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new("alice", ["g".into()])
}
fn write(e: &mut Engine, n: i64) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let expected_head = e.head("g", "main")?;
    e.execute(&serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"g","expected_head":expected_head,"data":{"nodes":[{"id":"a","entity_id":"A","space_id":"s"},{"id":"b","entity_id":"B","space_id":"s"}],"edges":[{"id":"e","predicate":"p","from":"a","to":"b","valid_time":{"start":0,"end":10},"properties":{"n":n}}]}}]}))?,&host())?;
    Ok(())
}
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).ok_or("db required")?;
    let mode = args.get(2).ok_or("mode required")?;
    let mut e = Engine::open(path)?;
    match mode.as_str() {
        "prepare" => {
            write(&mut e, 0)?;
            e.register_view(
                &ViewDefinition {
                    id: "v".into(),
                    clock: ViewClock::Tick,
                    expression: GraphExpression::Query {
                        query: serde_json::from_value(json!({"graph_id":"g"}))?,
                    },
                },
                Some(0),
                &host(),
            )?;
            e.enroll_incremental_view("v", &host())?;
            e.enable_view_schedule("v", &host())?;
            e.drain_view_work(&host())?;
        }
        "change" => write(&mut e, 1)?,
        "tick" => {
            e.request_view_tick("v", 5, &host())?;
            e.request_view_tick("v", 11, &host())?;
        }
        #[cfg(feature = "recovery-testing")]
        "scan_before" => {
            e.scan_view_work_test_before_commit(
                &host(),
                ViewScanBudget {
                    max_events: 16,
                    max_views: 16,
                },
                || std::process::exit(90),
            )?;
        }
        "scan" | "scan_after" => {
            e.scan_view_work(
                &host(),
                ViewScanBudget {
                    max_events: 16,
                    max_views: 16,
                },
            )?;
            if mode == "scan_after" {
                std::process::exit(91);
            }
        }
        #[cfg(feature = "recovery-testing")]
        "drain_before" => {
            e.drain_view_work_test_before_commit(&host(), || std::process::exit(92))?;
        }
        "drain" | "drain_after" => {
            let out = e.drain_view_work(&host())?;
            if mode == "drain_after" {
                std::process::exit(93);
            }
            println!(
                "{}",
                json!({"worked":out.is_some(),"generation":out.map(|o|o.generation)})
            );
            return Ok(());
        }
        "inspect" => {}
        _ => return Err("unsupported mode".into()),
    }
    let c = rusqlite::Connection::open(path)?;
    let (generation, tick): (i64, Option<i64>) = c.query_row(
        "SELECT generation,tick FROM live_views WHERE id='v'",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let (pending, requested, manifest): (i64, Option<i64>, Option<String>) = c.query_row(
        "SELECT pending,requested_tick,processed_manifest FROM view_schedules WHERE id='v'",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let cursor: i64 = c.query_row(
        "SELECT sequence FROM view_schedule_cursors WHERE principal='alice'",
        [],
        |r| r.get(0),
    )?;
    println!(
        "{}",
        json!({"generation":generation,"tick":tick,"pending":pending,"requested":requested,"cursor":cursor,"manifest":manifest.map(|m|serde_json::from_str::<serde_json::Value>(&m).unwrap()),"events":e.event_count()?,"head":e.head("g","main")?})
    );
    Ok(())
}
