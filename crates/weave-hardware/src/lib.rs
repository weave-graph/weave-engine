//! Hardware fleet reference model for Weave (roadmap A5).
//!
//! - [`schema`]: node types, relations and input tables, plus the engine
//!   graph schema that enforces them.
//! - [`series`]: [`SeriesRef`], a typed reference to a time range of an
//!   external series. No raw samples are stored in the graph.
//! - [`table`] and [`mapping`]: CSV and JSON rows to facts with stable IDs
//!   and claimed provenance.
//! - [`import`]: incremental, idempotent import through the A2 fact write.
//! - [`generate`]: a deterministic, seeded fleet dataset generator.
//! - [`query`]: helpers that answer fleet questions with existing operators.
//!
//! The reference is `docs/HARDWARE_MODEL.md`.
pub mod generate;
pub mod import;
pub mod mapping;
pub mod query;
pub mod schema;
pub mod series;
pub mod table;

pub use import::{import_batch, import_facts, ImportReport, Target};
pub use series::SeriesRef;
pub use table::{Batch, Row, Update};

/// Errors use the engine's `{code, message}` shape.
pub type Error = weave_engine::Error;
pub type Result<T> = std::result::Result<T, Error>;

/// Build an error.
pub fn error(code: &str, message: &str) -> Error {
    Error {
        code: code.into(),
        message: message.into(),
    }
}
