//! The control store schema: a part-1 (version 1) or part-2 (version 2)
//! store upgrades in place to version 3, keeping its tenants and tokens; an
//! unknown future version is refused.
mod common;
use axum::http::StatusCode;
use common::*;
use std::sync::Arc;
use weave_engine::ManualClock;
use weave_service::log::Logger;
use weave_service::{Limits, Service, CONTROL_FILE};

fn table_count(raw: &rusqlite::Connection, names: &str) -> i64 {
    raw.query_row(
        &format!("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ({names})"),
        [],
        |r| r.get(0),
    )
    .unwrap()
}

#[tokio::test]
async fn older_control_stores_upgrade_and_a_newer_one_is_refused() {
    for (from, tables) in [
        (
            1,
            "DROP TABLE oidc_issuers; DROP TABLE oidc_bindings; DROP TABLE key_bindings;
             DROP TABLE signed_nonces; DROP TABLE quotas;",
        ),
        (2, "DROP TABLE quotas;"),
    ] {
        let h = Harness::new();
        let root = h.tenant("acme", "root");
        let path = h.dir.path().join(CONTROL_FILE);
        {
            // Make it exactly a store of version `from`.
            let raw = rusqlite::Connection::open(&path).unwrap();
            raw.execute_batch(&format!("{tables} UPDATE control_meta SET value={from};"))
                .unwrap();
        }
        let reopen = || {
            Service::open(
                h.dir.path(),
                Arc::new(ManualClock::new(START_MS + 1_000_000)),
                Limits::default(),
                Logger::discard(),
            )
        };
        let service = reopen().unwrap();
        let raw = rusqlite::Connection::open(&path).unwrap();
        let version: i64 = raw
            .query_row("SELECT value FROM control_meta", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 3, "from {from}");
        assert_eq!(
            table_count(
                &raw,
                "'oidc_issuers','oidc_bindings','key_bindings','signed_nonces','quotas'"
            ),
            5
        );
        // The old token still authenticates through the upgraded store.
        let reply = h.get("/v1/whoami", &root).await;
        assert_eq!(reply.status, StatusCode::OK);
        drop(service);
        raw.execute("UPDATE control_meta SET value=4", []).unwrap();
        let refused = reopen().err().unwrap();
        assert_eq!(refused.code, "E_STORAGE_VERSION");
    }
}
