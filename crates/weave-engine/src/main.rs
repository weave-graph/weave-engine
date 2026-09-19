use std::{collections::HashSet, env, fs, io::Read, process};
use weave_contract::Program;
use weave_engine::{Engine, HostContext};
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("run") {
        return Err(
            "usage: weave-engine run --db PATH --actor PRINCIPAL [--write GRAPH] PLAN.json".into(),
        );
    }
    let mut db = None;
    let mut actor = None;
    let mut graphs = HashSet::new();
    let mut path = None;
    let mut i = 1;
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
    let mut bytes = Vec::new();
    fs::File::open(path.ok_or("plan file required")?)?
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("plan exceeds 16 MiB".into());
    }
    let plan: Program = serde_json::from_slice(&bytes)?;
    let host = HostContext::new(actor.ok_or("--actor required")?, graphs);
    let mut engine = Engine::open(db.ok_or("--db required")?)?;
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
