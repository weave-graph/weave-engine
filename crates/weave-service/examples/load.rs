//! B3 (#39) load measurement: concurrent read and write throughput of the
//! service for N clients, over real TLS on loopback, through the final
//! authentication path (API tokens or OIDC ID tokens, plus the registry and
//! audit log on every request).
//!
//! ```sh
//! cargo run --release -p weave-service --example load -- \
//!   --clients 50 --seconds 20 --write-percent 20 --tenants 1 --auth api
//! ```
//!
//! Each client is its own registry user with its own credential and one
//! keep-alive TLS connection. A read is a `query` of one seed node's edges;
//! a write asserts one node and one edge with IDs minted for that client.
//! The program prints one JSON report.
#![allow(dead_code)]
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::rustls::{self, ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;
use weave_engine::{HostContext, SystemClock};
use weave_service::log::Logger;
use weave_service::tls::{self, ConnectionLimits};
use weave_service::{Limits, Service};

#[derive(Debug, Clone)]
pub struct Args {
    pub clients: usize,
    pub seconds: u64,
    pub warmup_seconds: u64,
    pub write_percent: u32,
    pub tenants: usize,
    /// `api` or `oidc`.
    pub auth: String,
    pub seed_nodes: usize,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            clients: 50,
            seconds: 20,
            warmup_seconds: 2,
            write_percent: 20,
            tenants: 1,
            auth: "api".into(),
            seed_nodes: 1000,
        }
    }
}

fn parse() -> Args {
    let mut args = Args::default();
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i + 1 < raw.len() {
        let value = &raw[i + 1];
        match raw[i].as_str() {
            "--clients" => args.clients = value.parse().expect("--clients"),
            "--seconds" => args.seconds = value.parse().expect("--seconds"),
            "--warmup" => args.warmup_seconds = value.parse().expect("--warmup"),
            "--write-percent" => args.write_percent = value.parse().expect("--write-percent"),
            "--tenants" => args.tenants = value.parse().expect("--tenants"),
            "--auth" => args.auth = value.clone(),
            "--seed-nodes" => args.seed_nodes = value.parse().expect("--seed-nodes"),
            other => panic!("unknown flag {other}"),
        }
        i += 2;
    }
    args
}

/// A deterministic per-client generator (xorshift64*).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

struct Connection {
    stream: BufReader<tokio_rustls::client::TlsStream<TcpStream>>,
    token: String,
}

impl Connection {
    async fn open(port: u16, roots: RootCertStore, token: String) -> Self {
        let config =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_root_certificates(roots)
                .with_no_client_auth();
        let tcp = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        tcp.set_nodelay(true).unwrap();
        let tls = TlsConnector::from(Arc::new(config))
            .connect(ServerName::try_from("localhost").unwrap(), tcp)
            .await
            .unwrap();
        Self {
            stream: BufReader::new(tls),
            token,
        }
    }
    /// One keep-alive request; returns the status and the body.
    async fn request(&mut self, method: &str, path: &str, body: Option<&Value>) -> (u16, Value) {
        let body = body.map(|b| b.to_string()).unwrap_or_default();
        let head = format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            self.token,
            body.len()
        );
        let stream = self.stream.get_mut();
        stream.write_all(head.as_bytes()).await.unwrap();
        stream.write_all(body.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut line = String::new();
        self.stream.read_line(&mut line).await.unwrap();
        let status: u16 = line.split(' ').nth(1).unwrap_or("0").parse().unwrap_or(0);
        let mut length = 0usize;
        loop {
            line.clear();
            self.stream.read_line(&mut line).await.unwrap();
            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                break;
            }
            if let Some((name, value)) = trimmed.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    length = value.trim().parse().unwrap();
                }
            }
        }
        let mut bytes = vec![0u8; length];
        self.stream.read_exact(&mut bytes).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}

/// Latencies in microseconds, by operation.
#[derive(Default)]
struct Samples {
    reads: Vec<u64>,
    writes: Vec<u64>,
    errors: u64,
    error_codes: Vec<String>,
}

fn percentile(sorted: &[u64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[index] as f64 / 1000.0
}

fn summary(mut samples: Vec<u64>, seconds: f64) -> Value {
    samples.sort_unstable();
    json!({
        "requests": samples.len(),
        "per_second": (samples.len() as f64 / seconds * 10.0).round() / 10.0,
        "latency_ms": {
            "p50": percentile(&samples, 0.50),
            "p95": percentile(&samples, 0.95),
            "p99": percentile(&samples, 0.99),
            "max": percentile(&samples, 1.0),
        },
    })
}

fn program(commands: Value) -> Value {
    json!({"version": weave_contract::VERSION, "commands": commands})
}

fn oidc_token(
    key: &ring::signature::Ed25519KeyPair,
    issuer: &str,
    audience: &str,
    subject: &str,
) -> String {
    use weave_service::oidc::base64url_encode as b64;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let header = json!({"alg": "EdDSA", "kid": "load"});
    let claims =
        json!({"iss": issuer, "aud": audience, "sub": subject, "exp": now + 3600, "nbf": now - 5});
    let signing = format!(
        "{}.{}",
        b64(header.to_string().as_bytes()),
        b64(claims.to_string().as_bytes())
    );
    format!("{signing}.{}", b64(key.sign(signing.as_bytes()).as_ref()))
}

/// Set up the tenants and run the load. Returns the JSON report.
pub async fn run(args: Args) -> Value {
    let dir = tempfile::tempdir().unwrap();
    let service = Service::open(
        dir.path(),
        Arc::new(SystemClock),
        Limits::default(),
        Logger::discard(),
    )
    .unwrap();
    let issuer = "https://idp.load.test";
    let rng = ring::rand::SystemRandom::new();
    let key = ring::signature::Ed25519KeyPair::from_pkcs8(
        ring::signature::Ed25519KeyPair::generate_pkcs8(&rng)
            .unwrap()
            .as_ref(),
    )
    .unwrap();
    let jwks = {
        use ring::signature::KeyPair;
        json!({"keys": [{"kty": "OKP", "crv": "Ed25519", "kid": "load",
            "x": weave_service::oidc::base64url_encode(key.public_key().as_ref())}]})
        .to_string()
    };
    let setup = Instant::now();
    let mut credentials = Vec::new();
    for t in 0..args.tenants {
        let tenant = format!("t{t}");
        service.create_tenant(&tenant, "root").unwrap();
        let state = service.tenant(&tenant).unwrap();
        let users: Vec<usize> = (0..args.clients)
            .filter(|c| c % args.tenants == t)
            .collect();
        state
            .with_state(|s| {
                let root = HostContext::new("root", Vec::<String>::new());
                let mut changes = vec![json!({"change": "add_principal", "id": "load-team", "kind": "group"})];
                for u in &users {
                    changes.push(json!({"change": "add_principal", "id": format!("u{u}"), "kind": "user"}));
                    changes.push(json!({"change": "add_member", "group": "load-team", "member": format!("u{u}")}));
                }
                changes.push(json!({"change": "grant", "graph_id": "load", "subject": "load-team", "permission": "write"}));
                s.engine
                    .apply_registry(&serde_json::from_value::<Vec<_>>(json!(changes)).unwrap(), &root)
                    .unwrap();
                let mut facts = Vec::new();
                for n in 0..args.seed_nodes {
                    facts.push(json!({"change": "assert", "fact": {"kind": "node", "id": format!("n{n}"),
                        "entity_id": format!("E{n}"), "space_id": "s", "properties": {"i": n}}}));
                }
                for n in 0..args.seed_nodes {
                    facts.push(json!({"change": "assert", "fact": {"kind": "edge", "id": format!("e{n}"),
                        "predicate": "weave:next", "from": format!("n{n}"),
                        "to": format!("n{}", (n + 1) % args.seed_nodes), "valid_time": {"start": 0}}}));
                }
                let seed: weave_contract::Program = serde_json::from_value(program(
                    json!([{"op": "write", "graph_id": "load", "changes": facts}]),
                ))
                .unwrap();
                s.engine.execute(&seed, &root).unwrap();
                // One small follow-up write, so reads start at a steady-state
                // head rather than at the bulk seed revision (see the report).
                let settle: weave_contract::Program = serde_json::from_value(program(json!([
                    {"op": "write", "graph_id": "load", "changes": [{"change": "assert", "fact":
                        {"kind": "node", "id": "settle", "entity_id": "E-settle", "space_id": "s"}}]}
                ])))
                .unwrap();
                s.engine.execute(&settle, &root).unwrap();
            })
            .unwrap();
        if args.auth == "oidc" {
            let audience = format!("weave-{tenant}");
            service
                .set_oidc_issuer(&tenant, issuer, &audience, &jwks)
                .unwrap();
        }
        for u in users {
            let principal = format!("u{u}");
            let token = if args.auth == "oidc" {
                let audience = format!("weave-{tenant}");
                // Bind the subject as the administrator would.
                let subject = format!("sub-{u}");
                service
                    .bind_oidc_subject(&tenant, issuer, &subject, &principal)
                    .unwrap();
                oidc_token(&key, issuer, &audience, &subject)
            } else {
                service
                    .issue_token(&tenant, &principal, "load", 24 * 3600 * 1000, Some("load"))
                    .unwrap()
                    .token
            };
            credentials.push((u, token));
        }
    }
    let setup_seconds = setup.elapsed().as_secs_f64();

    let generated = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let cert = CertificateDer::from(generated.cert.der().to_vec());
    let private =
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(generated.key_pair.serialize_der()));
    let config = tls::tls_config_from(vec![cert.clone()], private).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(tls::serve_tls(
        listener,
        config,
        service.router(),
        ConnectionLimits::default(),
        Logger::discard(),
        async {
            let _ = stopped.await;
        },
    ));
    let mut roots = RootCertStore::empty();
    roots.add(cert).unwrap();

    let start = Instant::now() + Duration::from_millis(500);
    let measure_from = start + Duration::from_secs(args.warmup_seconds);
    let deadline = measure_from + Duration::from_secs(args.seconds);
    let mut tasks = Vec::new();
    for (u, token) in credentials {
        let roots = roots.clone();
        let args = args.clone();
        tasks.push(tokio::spawn(async move {
            let mut connection = Connection::open(port, roots, token).await;
            let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ (u as u64 + 1));
            let mut ids: Vec<String> = Vec::new();
            let mut samples = Samples::default();
            tokio::time::sleep_until(start.into()).await;
            while Instant::now() < deadline {
                if ids.len() < 2 {
                    let (status, body) = connection
                        .request("POST", "/v1/ids", Some(&json!({"count": 1000})))
                        .await;
                    assert_eq!(status, 200, "{body}");
                    ids = body["ids"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_str().unwrap().to_string())
                        .collect();
                }
                let target = format!("n{}", rng.next() % args.seed_nodes as u64);
                let write = (rng.next() % 100) < args.write_percent as u64;
                let body = if write {
                    let node = ids.pop().unwrap();
                    let edge = ids.pop().unwrap();
                    program(json!([{"op": "write", "graph_id": "load", "changes": [
                        {"change": "assert", "fact": {"kind": "node", "id": node, "entity_id": format!("E-{node}"),
                            "space_id": "s", "properties": {"client": u}}},
                        {"change": "assert", "fact": {"kind": "edge", "id": edge, "predicate": "weave:mentions",
                            "from": node, "to": target, "valid_time": {"start": 0}}}]}]))
                } else {
                    program(json!([{"op": "query", "query": {"graph_id": "load",
                        "predicate": "weave:next", "from": target}}]))
                };
                let sent = Instant::now();
                let (status, reply) = connection.request("POST", "/v1/execute", Some(&body)).await;
                let micros = sent.elapsed().as_micros() as u64;
                if sent < measure_from {
                    continue;
                }
                if status != 200 {
                    samples.errors += 1;
                    if samples.error_codes.len() < 5 {
                        samples.error_codes.push(reply["error"]["code"].as_str().unwrap_or("?").into());
                    }
                } else if write {
                    samples.writes.push(micros);
                } else {
                    samples.reads.push(micros);
                }
            }
            samples
        }));
    }
    let mut all = Samples::default();
    for task in tasks {
        let samples = task.await.unwrap();
        all.reads.extend(samples.reads);
        all.writes.extend(samples.writes);
        all.errors += samples.errors;
        all.error_codes.extend(samples.error_codes);
    }
    let _ = stop.send(());
    let _ = server.await;
    let seconds = args.seconds as f64;
    let total = all.reads.len() + all.writes.len();
    let mut audited = 0i64;
    for t in 0..args.tenants {
        let tenant = service.tenant(&format!("t{t}")).unwrap();
        audited += tenant
            .with_state(|s| s.audit.verify_chain().unwrap().unwrap())
            .unwrap();
    }
    json!({
        "profile": "weave-service-load/1",
        "clients": args.clients,
        "tenants": args.tenants,
        "auth": args.auth,
        "transport": "HTTP/1.1 over TLS 1.3 (rustls, ring), loopback, one keep-alive connection per client",
        "write_percent": args.write_percent,
        "seed_nodes": args.seed_nodes,
        "warmup_seconds": args.warmup_seconds,
        "measured_seconds": seconds,
        "setup_seconds": (setup_seconds * 10.0).round() / 10.0,
        "total_per_second": (total as f64 / seconds * 10.0).round() / 10.0,
        "reads": summary(all.reads, seconds),
        "writes": summary(all.writes, seconds),
        "errors": all.errors,
        "error_codes": all.error_codes,
        "audit_entries_verified": audited,
    })
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let args = parse();
    let report = run(args).await;
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}
