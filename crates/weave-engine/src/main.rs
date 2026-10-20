use std::{collections::HashSet, env, fs, io::Read, process};
use weave_engine::{Engine, HostContext};
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    let usage = "usage: weave-engine run --db PATH --actor PRINCIPAL [--write GRAPH] PLAN.json\n       weave-engine registry enable --db PATH --actor PRINCIPAL\n       weave-engine registry apply --db PATH --actor PRINCIPAL CHANGES.json\n       weave-engine graphs --db PATH --actor PRINCIPAL\n       weave-engine review --db PATH --actor PRINCIPAL REQUEST.json";
    let (command, first) = match (
        args.first().map(String::as_str),
        args.get(1).map(String::as_str),
    ) {
        (Some("run"), _) => ("run", 1),
        (Some("graphs"), _) => ("graphs", 1),
        (Some("review"), _) => ("review", 1),
        (Some("registry"), Some("enable")) => ("enable", 2),
        (Some("registry"), Some("apply")) => ("apply", 2),
        _ => return Err(usage.into()),
    };
    let mut db = None;
    let mut actor = None;
    let mut graphs = HashSet::new();
    let mut path = None;
    let mut i = first;
    while i < args.len() {
        match args[i].as_str() {
            "--db" | "--actor" | "--write" => {
                let key = &args[i];
                i += 1;
                let value = args.get(i).ok_or("missing flag value")?.clone();
                match key.as_str() {
                    "--db" => db = Some(value),
                    "--actor" => actor = Some(value),
                    _ => {
                        graphs.insert(value);
                    }
                }
            }
            arg if arg.starts_with('-') => return Err(format!("unknown flag: {arg}").into()),
            _ => {
                if path.replace(args[i].clone()).is_some() {
                    return Err("only one plan path permitted".into());
                }
            }
        }
        i += 1;
    }
    let host = HostContext::new(actor.ok_or("--actor required")?, graphs);
    let engine_path = db.ok_or("--db required")?;
    let report = |outcome: weave_engine::Result<serde_json::Value>| -> Result<(), Box<dyn std::error::Error>> {
        match outcome {
            Ok(value) => println!("{}", serde_json::to_string_pretty(&value)?),
            Err(error) => {
                eprintln!("{}", serde_json::to_string(&error)?);
                process::exit(1);
            }
        }
        Ok(())
    };
    match command {
        "enable" => {
            let engine = Engine::open(&engine_path)?;
            return report(
                engine
                    .enable_registry(&host)
                    .map(|r| serde_json::json!({"revision": r})),
            );
        }
        "graphs" => {
            let engine = Engine::open(&engine_path)?;
            return report(engine.list_graphs(&host).map(|g| serde_json::json!(g)));
        }
        _ => {}
    }
    let mut bytes = Vec::new();
    fs::File::open(path.ok_or("plan file required")?)?
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("plan exceeds 16 MiB".into());
    }
    if command == "apply" {
        let changes: Vec<weave_engine::RegistryChange> = serde_json::from_slice(&bytes)?;
        let engine = Engine::open(&engine_path)?;
        return report(
            engine
                .apply_registry(&changes, &host)
                .map(|r| serde_json::json!({"revision": r})),
        );
    }
    if command == "review" {
        let request: weave_engine::ReviewRequest = serde_json::from_slice(&bytes)?;
        let engine = Engine::open(&engine_path)?;
        return report(engine.review_request(&request, &host));
    }
    // Stable core-v1 decode codes (E_VERSION, E_UNKNOWN_FIELD, ...) reach the caller.
    let plan = match weave_contract::decode_program(&bytes) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("{}", serde_json::to_string(&error)?);
            process::exit(1);
        }
    };
    let mut engine = Engine::open(&engine_path)?;
    match engine.execute(&plan, &host) {
        Ok(result) => println!("{}", serde_json::to_string_pretty(&result)?),
        Err(error) => {
            eprintln!("{}", serde_json::to_string(&error)?);
            process::exit(1);
        }
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!(
            "{}",
            serde_json::json!({"code":"E_INPUT","message":error.to_string()})
        );
        process::exit(1);
    }
}
