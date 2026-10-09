//! Controlled process-kill host for schema migration acceptance, never activated by environment.
use serde_json::json;
use std::sync::Arc;
use weave_engine::{Engine, HostContext, ManualClock, SystemClock, TrustedClock};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if !(3..=4).contains(&args.len()) {
        return Err(
            "usage: storage_probe DATABASE seed|open|crash|after_commit [TRUSTED_TEST_CLOCK_MS]"
                .into(),
        );
    }
    let clock: Arc<dyn TrustedClock> = if let Some(time) = args.get(3) {
        Arc::new(ManualClock::new(time.parse()?))
    } else {
        Arc::new(SystemClock)
    };
    #[cfg(feature = "recovery-testing")]
    if args[2] == "crash" {
        Engine::open_test_before_schema_commit_with_clock(&args[1], clock, || {
            std::process::exit(82)
        })?;
        return Err("crash hook not reached".into());
    }
    let mut e = Engine::open_with_clock(&args[1], clock)?;
    match args[2].as_str() {
        "seed" => {
            let program = serde_json::from_value(
                json!({"version":weave_contract::VERSION,"commands":[{"op":"commit","graph_id":"migration","data":{
                    "nodes":[{"id":"a","entity_id":"A","space_id":"s"},{"id":"b","entity_id":"B","space_id":"s"}],
                    "edges":[{"id":"e","predicate":"p","from":"a","to":"b","valid_time":{"start":0}}]
                }}]}),
            )?;
            e.execute(
                &program,
                &HostContext::new("test-owner", ["migration".into()]),
            )?;
        }
        "open" => {}
        "after_commit" => std::process::exit(83),
        _ => return Err("unknown operation".into()),
    }
    println!(
        "{}",
        json!({"head":e.head("migration","main")?,"events":e.event_count()?})
    );
    Ok(())
}
