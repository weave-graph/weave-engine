//! B3 (#39): the load harness (`examples/load.rs`) runs in CI as a short
//! smoke test: 50 concurrent TLS clients, reads and writes, both
//! authentication paths, no errors, and a verified audit chain. The
//! published numbers come from release runs (docs/benchmarks/).
#[path = "../examples/load.rs"]
mod load;

#[tokio::test(flavor = "multi_thread")]
async fn fifty_clients_read_and_write_without_errors() {
    for auth in ["api", "oidc"] {
        let report = load::run(load::Args {
            clients: 50,
            seconds: 2,
            warmup_seconds: 0,
            write_percent: 20,
            tenants: 2,
            auth: auth.into(),
            seed_nodes: 100,
        })
        .await;
        assert_eq!(report["errors"], 0, "{report}");
        assert!(
            report["reads"]["requests"].as_u64().unwrap() > 0,
            "{report}"
        );
        assert!(
            report["writes"]["requests"].as_u64().unwrap() > 0,
            "{report}"
        );
        // Every request was audited, and the chains verify.
        let requests = report["reads"]["requests"].as_i64().unwrap()
            + report["writes"]["requests"].as_i64().unwrap();
        assert!(report["audit_entries_verified"].as_i64().unwrap() >= requests);
    }
}
