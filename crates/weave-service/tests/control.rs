//! The control store schema: a part-1 (version 1) store upgrades in place,
//! keeping its tenants and tokens; an unknown future version is refused.
mod common;
use axum::http::StatusCode;
use common::*;
use std::sync::Arc;
use weave_engine::ManualClock;
use weave_service::log::Logger;
use weave_service::{Limits, Service, CONTROL_FILE};

#[tokio::test]
async fn a_version_1_control_store_upgrades_and_a_newer_one_is_refused() {
    let h = Harness::new();
    let root = h.tenant("acme", "root");
    let path = h.dir.path().join(CONTROL_FILE);
    {
        // Make it exactly a part-1 store: version 1, without the part-2 tables.
        let raw = rusqlite::Connection::open(&path).unwrap();
        raw.execute_batch(
            "DROP TABLE oidc_issuers; DROP TABLE oidc_bindings; DROP TABLE key_bindings;
             DROP TABLE signed_nonces; UPDATE control_meta SET value=1;",
        )
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
    assert_eq!(version, 2);
    let tables: i64 = raw
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN
             ('oidc_issuers','oidc_bindings','key_bindings','signed_nonces')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tables, 4);
    // The part-1 token still authenticates through the upgraded store.
    let reply = h.get("/v1/whoami", &root).await;
    assert_eq!(reply.status, StatusCode::OK);
    drop(service);
    raw.execute("UPDATE control_meta SET value=3", []).unwrap();
    let refused = reopen().err().unwrap();
    assert_eq!(refused.code, "E_STORAGE_VERSION");
}
