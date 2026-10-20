//! B3 (#39): the `weave-service` binary: trusted tenant and token
//! administration, and a long-running server process.
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

fn cli(args: &[&str]) -> (bool, Value, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_weave-service"))
        .args(args)
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    let value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
    (
        out.status.success(),
        value,
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn the_binary_administers_tenants_and_tokens_and_serves_requests() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().to_str().unwrap();
    let (ok, created, _) = cli(&[
        "tenant", "create", "--data", data, "--tenant", "acme", "--admin", "root",
    ]);
    assert!(ok);
    let root = created["token"]["token"].as_str().unwrap().to_string();
    assert!(root.starts_with("wvs_"));
    let (ok, _, err) = cli(&[
        "tenant", "create", "--data", data, "--tenant", "acme", "--admin", "x",
    ]);
    assert!(!ok && err.contains("E_CONFLICT"), "{err}");
    let (ok, _, err) = cli(&[
        "tenant", "create", "--data", data, "--tenant", "../up", "--admin", "x",
    ]);
    assert!(!ok && err.contains("E_INPUT"), "{err}");
    let (ok, issued, _) = cli(&[
        "token",
        "issue",
        "--data",
        data,
        "--tenant",
        "acme",
        "--principal",
        "root",
        "--ttl-days",
        "1",
    ]);
    assert!(ok);
    assert!(issued["token"]["token_id"]
        .as_str()
        .unwrap()
        .starts_with("tok-"));
    let (ok, _, err) = cli(&[
        "token",
        "issue",
        "--data",
        data,
        "--tenant",
        "acme",
        "--principal",
        "nobody",
    ]);
    assert!(!ok && err.contains("E_UNAVAILABLE"), "{err}");

    // A long-running server process on loopback.
    let mut server = Command::new(env!("CARGO_BIN_EXE_weave-service"))
        .args([
            "serve",
            "--data",
            data,
            "--listen",
            "127.0.0.1:0",
            "--plaintext-loopback",
        ])
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut log = BufReader::new(server.stderr.take().unwrap());
    let mut line = String::new();
    log.read_line(&mut line).unwrap();
    let listening: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(listening["event"], "listening");
    let address = listening["address"].as_str().unwrap().to_string();
    let get = |token: &str| {
        let mut stream = std::net::TcpStream::connect(&address).unwrap();
        write!(
            stream,
            "GET /v1/whoami HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    };
    let response = get(&root);
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains(r#""principal":"root""#));
    // Revocation from the operator's shell applies to the running server.
    let token_id = created["token"]["token_id"].as_str().unwrap();
    let (ok, _, _) = cli(&["token", "revoke", "--data", data, "--token-id", token_id]);
    assert!(ok);
    assert!(get(&root).starts_with("HTTP/1.1 401"));
    let (ok, _, _) = cli(&["token", "revoke", "--data", data, "--token-id", "tok-none"]);
    assert!(!ok);
    // The request log is JSON lines without the token.
    let mut request_line = String::new();
    log.read_line(&mut request_line).unwrap();
    let entry: Value = serde_json::from_str(&request_line).unwrap();
    assert_eq!(entry["event"], "request");
    assert_eq!(entry["route"], "/v1/whoami");
    assert!(!request_line.contains(&root[4..]));
    server.kill().unwrap();
    let _ = server.wait();
    // Serving without TLS material and without the loopback flag fails.
    let (ok, _, err) = cli(&["serve", "--data", data, "--listen", "127.0.0.1:0"]);
    assert!(!ok && err.contains("tls-cert"), "{err}");
}

#[test]
fn the_binary_sets_lists_and_removes_quotas() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().to_str().unwrap();
    let run = |args: &[&str]| {
        let mut all: Vec<&str> = args.to_vec();
        all.extend(["--data", data, "--tenant", "acme"]);
        cli(&all)
    };
    let (ok, _, _) = cli(&[
        "tenant", "create", "--data", data, "--tenant", "acme", "--admin", "root",
    ]);
    assert!(ok);
    let (ok, set, err) = run(&[
        "quota",
        "set",
        "--agents",
        "--requests-per-minute",
        "120",
        "--writes-per-hour",
        "unlimited",
    ]);
    assert!(ok, "{err}");
    assert_eq!(set["subject"], "weave:agents");
    assert_eq!(set["quota"]["requests_per_minute"], 120);
    assert!(set["quota"]["writes_per_hour"].is_null());
    let (ok, _, _) = run(&[
        "quota",
        "set",
        "--principal",
        "root",
        "--requests-per-minute",
        "1",
        "--writes-per-hour",
        "0",
    ]);
    assert!(ok);
    let (ok, listed, _) = run(&["quota", "list"]);
    assert!(ok);
    assert_eq!(listed["quotas"].as_array().unwrap().len(), 2);
    // Exactly one subject, and whole numbers or `unlimited`.
    let (ok, _, err) = run(&[
        "quota",
        "set",
        "--agents",
        "--principal",
        "x",
        "--requests-per-minute",
        "1",
        "--writes-per-hour",
        "1",
    ]);
    assert!(!ok && err.contains("exactly one"), "{err}");
    let (ok, _, err) = run(&[
        "quota",
        "set",
        "--agents",
        "--requests-per-minute",
        "lots",
        "--writes-per-hour",
        "1",
    ]);
    assert!(!ok && err.contains("whole number"), "{err}");
    let (ok, _, _) = run(&["quota", "remove", "--principal", "root"]);
    assert!(ok);
    let (ok, _, _) = run(&["quota", "remove", "--principal", "root"]);
    assert!(!ok);
}
