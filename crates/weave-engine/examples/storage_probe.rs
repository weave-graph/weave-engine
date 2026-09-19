//! Controlled process-kill host for schema migration acceptance, never activated by environment.
use serde_json::json;
use weave_engine::{Engine, HostContext};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: storage_probe DATABASE seed|open|crash".into());
    }
    #[cfg(feature = "recovery-testing")]
    if args[2] == "crash" {
        Engine::open_test_before_schema_commit(&args[1], || std::process::exit(82))?;
        return Err("crash hook not reached".into());
    }
    let mut e = Engine::open(&args[1])?;
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
        _ => return Err("unknown operation".into()),
    }
    println!(
        "{}",
        json!({"head":e.head("migration","main")?,"events":e.event_count()?})
    );
    Ok(())
}
