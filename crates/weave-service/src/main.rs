//! `weave-service` command line: serve, and trusted local administration of
//! tenants and tokens. See `docs/SERVICE.md`.
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use weave_engine::SystemClock;
use weave_service::control::DEFAULT_TOKEN_TTL_MS;
use weave_service::log::Logger;
use weave_service::tls::{self, ConnectionLimits};
use weave_service::{Limits, Service};

const USAGE: &str = "usage:
  weave-service tenant create --data DIR --tenant ID --admin PRINCIPAL
  weave-service token issue  --data DIR --tenant ID --principal PRINCIPAL [--ttl-days N] [--label TEXT]
  weave-service token revoke --data DIR --token-id ID
  weave-service oidc set    --data DIR --tenant ID --issuer URL --audience CLIENT_ID --jwks FILE
  weave-service oidc remove --data DIR --tenant ID --issuer URL
  weave-service quota set    --data DIR --tenant ID (--principal ID | --agents) --requests-per-minute N|unlimited --writes-per-hour N|unlimited
  weave-service quota remove --data DIR --tenant ID (--principal ID | --agents)
  weave-service quota list   --data DIR --tenant ID
  weave-service serve --data DIR --listen ADDR (--tls-cert PEM --tls-key PEM | --plaintext-loopback)";

fn flags(args: &[String]) -> Result<HashMap<String, String>, String> {
    let mut out = HashMap::new();
    let mut iter = args.iter();
    while let Some(flag) = iter.next() {
        let Some(name) = flag.strip_prefix("--") else {
            return Err(format!("unexpected argument {flag}"));
        };
        if name == "plaintext-loopback" || name == "agents" {
            out.insert(name.to_string(), String::new());
            continue;
        }
        let value = iter
            .next()
            .ok_or_else(|| format!("--{name} needs a value"))?;
        if out.insert(name.to_string(), value.clone()).is_some() {
            return Err(format!("--{name} given twice"));
        }
    }
    Ok(out)
}

fn required<'a>(flags: &'a HashMap<String, String>, name: &str) -> Result<&'a str, String> {
    flags
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| format!("--{name} is required"))
}

/// `--principal ID` or `--agents` (the tenant's default for agents).
fn quota_subject(flags: &HashMap<String, String>) -> Result<&str, String> {
    match (flags.get("principal"), flags.contains_key("agents")) {
        (Some(principal), false) => Ok(principal),
        (None, true) => Ok(weave_service::quota::AGENTS),
        _ => Err("name exactly one of --principal ID and --agents".into()),
    }
}

fn open(flags: &HashMap<String, String>, log: Logger) -> Result<Service, String> {
    let data = PathBuf::from(required(flags, "data")?);
    Service::open(&data, Arc::new(SystemClock), Limits::default(), log)
        .map_err(|e| format!("{}: {}", e.code, e.message))
}

fn run(args: &[String]) -> Result<(), String> {
    let (command, rest) = match args {
        [a, b, rest @ ..] if a == "tenant" || a == "token" || a == "oidc" || a == "quota" => {
            (format!("{a} {b}"), rest)
        }
        [a, rest @ ..] => (a.clone(), rest),
        [] => return Err(USAGE.into()),
    };
    let flags = flags(rest)?;
    let api = |e: weave_service::error::ApiError| format!("{}: {}", e.code, e.message);
    match command.as_str() {
        "tenant create" => {
            let service = open(&flags, Logger::discard())?;
            let token = service
                .create_tenant(required(&flags, "tenant")?, required(&flags, "admin")?)
                .map_err(api)?;
            // The bootstrap token is shown once, here.
            println!(
                "{}",
                json!({"tenant": required(&flags, "tenant")?, "token": token})
            );
            Ok(())
        }
        "token issue" => {
            let service = open(&flags, Logger::discard())?;
            let ttl_ms = match flags.get("ttl-days") {
                None => DEFAULT_TOKEN_TTL_MS,
                Some(days) => days
                    .parse::<i64>()
                    .map_err(|_| "--ttl-days must be a whole number".to_string())?
                    .saturating_mul(24 * 3600 * 1000),
            };
            let principal = required(&flags, "principal")?;
            let token = service
                .issue_token(
                    required(&flags, "tenant")?,
                    principal,
                    "operator",
                    ttl_ms,
                    flags.get("label").map(String::as_str),
                )
                .map_err(api)?;
            println!("{}", json!({"token": token}));
            Ok(())
        }
        "token revoke" => {
            let service = open(&flags, Logger::discard())?;
            let token_id = required(&flags, "token-id")?;
            let revoked = service.revoke_token(token_id).map_err(api)?;
            println!("{}", json!({"token_id": token_id, "revoked": revoked}));
            if revoked {
                Ok(())
            } else {
                Err("no such token".into())
            }
        }
        "oidc set" => {
            let service = open(&flags, Logger::discard())?;
            let jwks = std::fs::read_to_string(required(&flags, "jwks")?)
                .map_err(|_| "cannot read the --jwks file".to_string())?;
            service
                .set_oidc_issuer(
                    required(&flags, "tenant")?,
                    required(&flags, "issuer")?,
                    required(&flags, "audience")?,
                    &jwks,
                )
                .map_err(api)?;
            println!(
                "{}",
                json!({"tenant": required(&flags, "tenant")?, "issuer": required(&flags, "issuer")?, "configured": true})
            );
            Ok(())
        }
        "oidc remove" => {
            let service = open(&flags, Logger::discard())?;
            let removed = service
                .remove_oidc_issuer(required(&flags, "tenant")?, required(&flags, "issuer")?)
                .map_err(api)?;
            println!("{}", json!({"removed": removed}));
            if removed {
                Ok(())
            } else {
                Err("no such issuer".into())
            }
        }
        "quota set" => {
            let service = open(&flags, Logger::discard())?;
            let limit = |name: &str| -> Result<Option<u64>, String> {
                match required(&flags, name)? {
                    "unlimited" => Ok(None),
                    n => n
                        .parse::<u64>()
                        .map(Some)
                        .map_err(|_| format!("--{name} must be a whole number or unlimited")),
                }
            };
            let quota = weave_service::quota::Quota {
                requests_per_minute: limit("requests-per-minute")?,
                writes_per_hour: limit("writes-per-hour")?,
            };
            let tenant = required(&flags, "tenant")?;
            let subject = quota_subject(&flags)?;
            service.set_quota(tenant, subject, quota).map_err(api)?;
            println!(
                "{}",
                json!({"tenant": tenant, "subject": subject, "quota": quota})
            );
            Ok(())
        }
        "quota remove" => {
            let service = open(&flags, Logger::discard())?;
            let removed = service
                .remove_quota(required(&flags, "tenant")?, quota_subject(&flags)?)
                .map_err(api)?;
            println!("{}", json!({"removed": removed}));
            if removed {
                Ok(())
            } else {
                Err("no such quota".into())
            }
        }
        "quota list" => {
            let service = open(&flags, Logger::discard())?;
            let rows = service.quotas(required(&flags, "tenant")?).map_err(api)?;
            let rows: Vec<_> = rows
                .into_iter()
                .map(|(subject, quota)| json!({"subject": subject, "quota": quota}))
                .collect();
            println!("{}", json!({"quotas": rows}));
            Ok(())
        }
        "serve" => {
            let log = Logger::stderr();
            let service = open(&flags, log.clone())?;
            let listen = required(&flags, "listen")?.to_string();
            let plaintext = flags.contains_key("plaintext-loopback");
            let tls_config = if plaintext {
                None
            } else {
                Some(tls::tls_config(
                    &PathBuf::from(required(&flags, "tls-cert")?),
                    &PathBuf::from(required(&flags, "tls-key")?),
                )?)
            };
            let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::bind(&listen)
                    .await
                    .map_err(|e| format!("cannot listen on {listen}: {e}"))?;
                let address = listener.local_addr().map_err(|e| e.to_string())?;
                log.event(
                    "info",
                    "listening",
                    json!({"address": address.to_string(), "tls": !plaintext}),
                );
                let shutdown = async {
                    let _ = tokio::signal::ctrl_c().await;
                };
                let router = service.router();
                match tls_config {
                    Some(config) => {
                        tls::serve_tls(
                            listener,
                            config,
                            router,
                            ConnectionLimits::default(),
                            log.clone(),
                            shutdown,
                        )
                        .await;
                        Ok(())
                    }
                    None => {
                        tls::serve_plaintext_loopback(
                            listener,
                            router,
                            ConnectionLimits::default(),
                            shutdown,
                        )
                        .await
                    }
                }
            })
        }
        _ => Err(USAGE.into()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{}", json!({"error": message}));
            ExitCode::FAILURE
        }
    }
}
