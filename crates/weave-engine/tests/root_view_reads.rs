use rusqlite::{params, Connection};
use serde_json::json;
use weave_contract::{GraphExpression, Program, VERSION};
use weave_engine::{Engine, HostContext, ViewClock, ViewDefinition, ViewFreshness};
#[test]
fn oversized_persisted_view_fields_fail_before_decode_and_recover_after_restore() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("view.sqlite");
    let mut e = Engine::open(&path).unwrap();
    let host = HostContext::new("alice", ["g".into()]);
    let p:Program=serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"g","data":{"nodes":[],"edges":[]}}]})).unwrap();
    e.execute(&p, &host).unwrap();
    let definition = ViewDefinition {
        id: "view".into(),
        expression: GraphExpression::Query {
            query: serde_json::from_value(json!({"graph_id":"g"})).unwrap(),
        },
        clock: ViewClock::Fixed,
    };
    e.register_view(&definition, None, &host).unwrap();
    let conn = Connection::open(&path).unwrap();
    for (column, limit) in [
        ("definition", 1024 * 1024),
        ("result", 32 * 1024 * 1024),
        ("dependencies", 4 * 1024 * 1024),
    ] {
        // SQL identifiers are fixed test constants, never externally supplied text.
        let original: String = conn
            .query_row(
                &format!("SELECT {column} FROM live_views WHERE id='view'"),
                [],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute(
            &format!("UPDATE live_views SET {column}=CAST(zeroblob(?1) AS TEXT) WHERE id='view'"),
            [limit + 1],
        )
        .unwrap();
        assert_eq!(
            e.read_view("view", None, ViewFreshness::AllowStale, &host)
                .unwrap_err()
                .code,
            "E_BUDGET"
        );
        assert_eq!(
            e.refresh_view("view", None, &host).unwrap_err().code,
            "E_BUDGET"
        );
        if column == "definition" {
            assert_eq!(
                e.register_view(&definition, None, &host).unwrap_err().code,
                "E_BUDGET"
            );
        }
        conn.execute(
            &format!("UPDATE live_views SET {column}=?1 WHERE id='view'"),
            params![original],
        )
        .unwrap();
        assert_eq!(
            e.read_view("view", None, ViewFreshness::AllowStale, &host)
                .unwrap()
                .generation,
            1
        );
    }
    conn.execute("UPDATE live_views SET generation=-1 WHERE id='view'", [])
        .unwrap();
    assert_eq!(
        e.read_view("view", None, ViewFreshness::AllowStale, &host)
            .unwrap_err()
            .code,
        "E_INTEGRITY"
    );
}
