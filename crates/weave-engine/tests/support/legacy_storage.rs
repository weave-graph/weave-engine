//! Synthetic pre-retention fixtures cannot retain tables introduced by store22.
//! Real historical binaries are covered separately by the migration controllers.
pub fn strip_retention_schema(connection: &rusqlite::Connection) {
    connection
        .execute_batch(
            "DROP TABLE compiled_migrations;
DROP TABLE delivery_cancellations;
DROP TABLE projection_rebuild_requests;
DROP TABLE projection_migrations;
DROP TABLE retention_adapter_states;
DROP TABLE retention_projection_receipts;
DROP TABLE retention_stateful_adapters;
DROP TABLE retention_view_epochs;
DROP TABLE retention_retired_branches;
DROP TABLE retention_roots;
DROP TABLE retention_tombstones;
DROP TABLE retention_policy;",
        )
        .unwrap();
}
