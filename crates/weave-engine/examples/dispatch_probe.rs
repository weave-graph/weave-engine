//! Test host driver. Crash boundaries are explicit invocation options, never runtime environment variables.
use serde_json::{json, Value};
use std::{env, fs, io::Read};
use weave_engine::{AdapterManifest, Engine, HostContext};
fn field<'a>(v: &'a Value, k: &str) -> Result<&'a str, Box<dyn std::error::Error>> {
    v[k].as_str().ok_or_else(|| format!("missing {k}").into())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        return Err("usage: dispatch_probe DATABASE REQUEST.json".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(&args[2])?
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("input budget".into());
    }
    let v: Value = serde_json::from_slice(&bytes)?;
    let mut e = Engine::open(&args[1])?;
    let result = (|| -> Result<Value, Box<dyn std::error::Error>> {
        Ok(match field(&v, "op")? {
            "install" => {
                let m: AdapterManifest = serde_json::from_value(v["manifest"].clone())?;
                let authority = HostContext::new(&m.principal, m.output_graphs.clone());
                e.install_adapter(&m, &authority)?;
                e.set_adapter_state(&m.id, "running")?;
                json!({"installed":true})
            }
            "poll" => serde_json::to_value(
                e.poll_adapter(field(&v, "adapter")?, v["now_ms"].as_i64().ok_or("now_ms")?)?,
            )?,
            "complete" => {
                let program = serde_json::from_value(v["program"].clone())?;
                #[cfg(feature = "recovery-testing")]
                if v["crash_before_commit"].as_bool() == Some(true) {
                    e.complete_handler_test_before_commit(
                        field(&v, "adapter")?,
                        field(&v, "event")?,
                        field(&v, "lease")?,
                        &program,
                        || std::process::exit(78),
                    )?;
                    return Err("crash hook not reached".into());
                }
                serde_json::to_value(e.complete_handler(
                    field(&v, "adapter")?,
                    field(&v, "event")?,
                    field(&v, "lease")?,
                    &program,
                )?)?
            }
            "request_effect" => serde_json::to_value(e.request_effect(
                field(&v, "adapter")?,
                field(&v, "event")?,
                field(&v, "lease")?,
                field(&v, "destination")?,
                field(&v, "key")?,
                v["payload"].clone(),
            )?)?,
            "begin_effect" => serde_json::to_value(e.begin_effect_dispatch(field(&v, "id")?)?)?,
            "effect" => serde_json::to_value(e.effect_intent(field(&v, "id")?)?)?,
            "reconcile" => {
                e.reconcile_effect(
                    field(&v, "id")?,
                    field(&v, "outcome")?,
                    v["response"].clone(),
                )?;
                json!({"reconciled":true})
            }
            _ => return Err("unknown operation".into()),
        })
    })();
    match result {
        Ok(value) => {
            if v["crash_after_commit"].as_bool() == Some(true) {
                std::process::exit(77)
            }
            println!("{}", value)
        }
        Err(e) => {
            eprintln!("{}", json!({"error":e.to_string()}));
            std::process::exit(1)
        }
    }
    Ok(())
}
