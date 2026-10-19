//! `weave-hardware`: generate a fleet dataset, print the reference schema,
//! and import CSV or JSON exports incrementally into a Weave store.
use serde_json::json;
use std::path::PathBuf;
use std::process::ExitCode;
use weave_engine::{Engine, HostContext};
use weave_hardware::generate::{generate, Config};
use weave_hardware::import::{apply_updates, read_csv_dir, read_updates, Target};
use weave_hardware::schema::graph_schema;
use weave_hardware::table::parse_json_batch;
use weave_hardware::{error, import_batch, Result};

const USAGE: &str = "\
weave-hardware: hardware fleet reference model for Weave

USAGE:
  weave-hardware schema
  weave-hardware generate --out DIR [--units N] [--updates N] [--seed S]
  weave-hardware import --db PATH --actor PRINCIPAL --graph GRAPH
                        (--csv DIR | --json FILE) [--source LABEL]
  weave-hardware apply-updates --db PATH --actor PRINCIPAL --graph GRAPH
                        --updates FILE

`import` maps rows to facts with stable IDs and writes only new or changed
facts; re-running an identical import writes nothing. `apply-updates` writes
each line of an updates file as its own fact revision. The actor is trusted
local authority and is granted write access to GRAPH only. Every command
prints one JSON object.
";

struct Args(Vec<String>);

impl Args {
    fn value(&mut self, flag: &str) -> Result<Option<String>> {
        let Some(at) = self.0.iter().position(|a| a == flag) else {
            return Ok(None);
        };
        if at + 1 >= self.0.len() {
            return Err(error("E_INPUT", &format!("{flag} needs a value")));
        }
        let value = self.0.remove(at + 1);
        self.0.remove(at);
        Ok(Some(value))
    }
    fn need(&mut self, flag: &str) -> Result<String> {
        self.value(flag)?
            .ok_or_else(|| error("E_INPUT", &format!("{flag} is required")))
    }
    fn number<T: std::str::FromStr>(&mut self, flag: &str, default: T) -> Result<T> {
        match self.value(flag)? {
            None => Ok(default),
            Some(v) => v
                .parse()
                .map_err(|_| error("E_INPUT", &format!("{flag} needs a number"))),
        }
    }
    fn done(&self) -> Result<()> {
        match self.0.first() {
            None => Ok(()),
            Some(extra) => Err(error("E_INPUT", &format!("unexpected argument {extra:?}"))),
        }
    }
}

fn read(path: &str) -> Result<String> {
    std::fs::read_to_string(path).map_err(|e| error("E_HW_INPUT", &format!("{path}: {e}")))
}

fn store(args: &mut Args) -> Result<(Engine, HostContext, Target)> {
    let db = args.need("--db")?;
    let actor = args.need("--actor")?;
    let graph = args.need("--graph")?;
    let engine = Engine::open(&db)?;
    let host = HostContext::new(actor, [graph.clone()]);
    Ok((engine, host, Target::new(graph)))
}

fn run(mut args: Args) -> Result<serde_json::Value> {
    if args.0.is_empty() || args.0.iter().any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        std::process::exit(0);
    }
    let command = args.0.remove(0);
    match command.as_str() {
        "schema" => {
            args.done()?;
            Ok(serde_json::to_value(graph_schema())?)
        }
        "generate" => {
            let out = PathBuf::from(args.need("--out")?);
            let defaults = Config::default();
            let config = Config {
                seed: args.number("--seed", defaults.seed)?,
                units: args.number("--units", defaults.units)?,
                updates: args.number("--updates", defaults.updates)?,
            };
            args.done()?;
            if config.units == 0 || config.units > 1_000_000 {
                return Err(error("E_INPUT", "--units must be 1 to 1000000"));
            }
            if config.updates > 1_000_000 {
                return Err(error("E_INPUT", "--updates must be at most 1000000"));
            }
            let dataset = generate(&config);
            dataset.write_to(&out)?;
            Ok(json!({
                "out": out.display().to_string(),
                "config": config,
                "digest": dataset.digest(),
                "updates": dataset.updates.len(),
            }))
        }
        "import" => {
            let (mut engine, host, target) = store(&mut args)?;
            let source = args.value("--source")?;
            let batch = match (args.value("--csv")?, args.value("--json")?) {
                (Some(dir), None) => read_csv_dir(&PathBuf::from(dir), source)?,
                (None, Some(file)) => {
                    let mut batch = parse_json_batch(&read(&file)?)?;
                    if source.is_some() {
                        batch.source = source;
                    }
                    batch
                }
                _ => return Err(error("E_INPUT", "give exactly one of --csv or --json")),
            };
            args.done()?;
            Ok(serde_json::to_value(import_batch(
                &mut engine,
                &host,
                &target,
                &batch,
            )?)?)
        }
        "apply-updates" => {
            let (mut engine, host, target) = store(&mut args)?;
            let updates = read_updates(&read(&args.need("--updates")?)?)?;
            args.done()?;
            let report = apply_updates(&mut engine, &host, &target, &updates, |_| {})?;
            Ok(json!({"updates": updates.len(), "report": report}))
        }
        other => Err(error("E_INPUT", &format!("unknown command {other:?}"))),
    }
}

fn main() -> ExitCode {
    let args = Args(std::env::args().skip(1).collect());
    match run(args) {
        Ok(value) => {
            println!("{}", json!({"ok": true, "result": value}));
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!(
                "{}",
                json!({"ok": false, "error": {"code": e.code, "message": e.message}})
            );
            ExitCode::FAILURE
        }
    }
}
