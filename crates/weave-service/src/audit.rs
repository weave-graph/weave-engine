//! The per-tenant read and write audit log (ADR-0007 section 9).
//!
//! Each tenant has its own append-only audit database next to its store.
//! An entry records the principal, `on_behalf_of`, the credential ID, the
//! operation, the outcome and error code, the graphs and revisions involved,
//! a digest of the exact response body and the recorded time. Its fact rows
//! name every fact the response returned (`read`) or the request wrote
//! (`write`), so the log answers "who read fact F" and "who wrote fact F".
//!
//! Entries are hash-chained: each stores the digest of the previous entry,
//! and triggers reject every UPDATE and DELETE. A denied request is recorded
//! with its error code and the graph IDs the request itself named, never
//! with the hidden object that caused the denial.
use crate::error::ApiError;
use crate::ids::sha256_hex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

/// Most entries or fact rows one audit read returns.
pub const MAX_AUDIT_PAGE: usize = 1000;
const GENESIS: &str = "genesis";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok,
    /// `E_FORBIDDEN`, `E_UNAVAILABLE` or an authentication failure.
    Denied,
    Error,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::Denied => "denied",
            Outcome::Error => "error",
        }
    }
    fn parse(value: &str) -> Outcome {
        match value {
            "ok" => Outcome::Ok,
            "denied" => Outcome::Denied,
            _ => Outcome::Error,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    Read,
    Write,
}

impl Access {
    fn as_str(self) -> &'static str {
        match self {
            Access::Read => "read",
            Access::Write => "write",
        }
    }
}

/// One fact a response returned or a request wrote.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FactAccess {
    pub access: Access,
    pub graph_id: String,
    pub revision: String,
    /// `node`, `assertion` or `embedding` for reads (the result's `FactRef`
    /// kind); the written `FactKind` for writes.
    pub kind: String,
    pub fact_id: String,
    /// `assert`, `correct` or `retract` for writes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<String>,
}

/// An entry before it is appended.
#[derive(Debug, Clone, Default)]
pub struct NewEntry {
    pub request_id: String,
    pub principal: String,
    pub on_behalf_of: Vec<String>,
    pub credential: String,
    pub operation: String,
    pub outcome: Option<Outcome>,
    pub code: Option<String>,
    /// Graphs read or written (success), or graph IDs the request named
    /// (failure).
    pub graphs: Vec<String>,
    pub revisions: Vec<String>,
    pub facts: Vec<FactAccess>,
}

/// An entry as auditors read it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    pub seq: i64,
    pub recorded_at_ms: i64,
    pub request_id: String,
    pub principal: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_behalf_of: Vec<String>,
    pub credential: String,
    pub operation: String,
    pub outcome: Outcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub graphs: Vec<String>,
    pub revisions: Vec<String>,
    pub fact_count: i64,
    pub result_digest: String,
    pub digest: String,
}

/// One access to a fact, for "who read / wrote fact F".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FactAccessEntry {
    pub seq: i64,
    pub recorded_at_ms: i64,
    pub principal: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_behalf_of: Vec<String>,
    pub operation: String,
    pub request_id: String,
    #[serde(flatten)]
    pub fact: FactAccess,
}

pub struct AuditLog {
    conn: Connection,
}

fn storage(error: rusqlite::Error) -> ApiError {
    let _ = error;
    ApiError::new(crate::error::codes::AUDIT, "audit log unavailable")
}

impl AuditLog {
    pub fn open(path: &Path) -> Result<Self, ApiError> {
        let conn = Connection::open(path).map_err(storage)?;
        conn.busy_timeout(Duration::from_secs(5)).map_err(storage)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS audit_entries(
               seq INTEGER PRIMARY KEY AUTOINCREMENT,
               recorded_at_ms INTEGER NOT NULL,
               request_id TEXT NOT NULL,
               principal TEXT NOT NULL,
               on_behalf_of TEXT NOT NULL,
               credential TEXT NOT NULL,
               operation TEXT NOT NULL,
               outcome TEXT NOT NULL,
               code TEXT,
               graphs TEXT NOT NULL,
               revisions TEXT NOT NULL,
               fact_count INTEGER NOT NULL,
               result_digest TEXT NOT NULL,
               previous TEXT NOT NULL,
               digest TEXT NOT NULL UNIQUE);
             CREATE TABLE IF NOT EXISTS audit_facts(
               seq INTEGER NOT NULL REFERENCES audit_entries(seq),
               access TEXT NOT NULL,
               graph_id TEXT NOT NULL,
               revision TEXT NOT NULL,
               kind TEXT NOT NULL,
               fact_id TEXT NOT NULL,
               change TEXT);
             CREATE INDEX IF NOT EXISTS audit_facts_by_fact ON audit_facts(graph_id, fact_id, seq);
             CREATE TRIGGER IF NOT EXISTS audit_entries_no_update BEFORE UPDATE ON audit_entries
               BEGIN SELECT RAISE(ABORT, 'the audit log is append-only'); END;
             CREATE TRIGGER IF NOT EXISTS audit_entries_no_delete BEFORE DELETE ON audit_entries
               BEGIN SELECT RAISE(ABORT, 'the audit log is append-only'); END;
             CREATE TRIGGER IF NOT EXISTS audit_facts_no_update BEFORE UPDATE ON audit_facts
               BEGIN SELECT RAISE(ABORT, 'the audit log is append-only'); END;
             CREATE TRIGGER IF NOT EXISTS audit_facts_no_delete BEFORE DELETE ON audit_facts
               BEGIN SELECT RAISE(ABORT, 'the audit log is append-only'); END;",
        )
        .map_err(storage)?;
        Ok(Self { conn })
    }

    /// Append one entry and its fact rows atomically. `response` is the exact
    /// response body; its digest is stored, never the body.
    pub fn append(
        &mut self,
        entry: &NewEntry,
        response: &[u8],
        now_ms: i64,
    ) -> Result<i64, ApiError> {
        let tx = self.conn.transaction().map_err(storage)?;
        let previous: String = tx
            .query_row(
                "SELECT digest FROM audit_entries ORDER BY seq DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)?
            .unwrap_or_else(|| GENESIS.into());
        let outcome = entry.outcome.unwrap_or(Outcome::Ok);
        let result_digest = format!("sha256:{}", sha256_hex(response));
        let mut graphs = entry.graphs.clone();
        graphs.sort();
        graphs.dedup();
        let mut revisions = entry.revisions.clone();
        revisions.sort();
        revisions.dedup();
        let mut facts = entry.facts.clone();
        facts.sort();
        facts.dedup();
        let encode = |v: &Vec<String>| serde_json::to_string(v).expect("strings serialize");
        let body = serde_json::json!({
            "previous": previous,
            "recorded_at_ms": now_ms,
            "request_id": entry.request_id,
            "principal": entry.principal,
            "on_behalf_of": entry.on_behalf_of,
            "credential": entry.credential,
            "operation": entry.operation,
            "outcome": outcome.as_str(),
            "code": entry.code,
            "graphs": graphs,
            "revisions": revisions,
            "facts": facts,
            "result_digest": result_digest,
        });
        let digest = format!(
            "sha256:{}",
            sha256_hex(&serde_json::to_vec(&body).expect("entries serialize"))
        );
        tx.execute(
            "INSERT INTO audit_entries(recorded_at_ms,request_id,principal,on_behalf_of,credential,
             operation,outcome,code,graphs,revisions,fact_count,result_digest,previous,digest)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![
                now_ms,
                entry.request_id,
                entry.principal,
                encode(&entry.on_behalf_of),
                entry.credential,
                entry.operation,
                outcome.as_str(),
                entry.code,
                encode(&graphs),
                encode(&revisions),
                facts.len() as i64,
                result_digest,
                previous,
                digest
            ],
        )
        .map_err(storage)?;
        let seq = tx.last_insert_rowid();
        {
            let mut insert = tx
                .prepare("INSERT INTO audit_facts VALUES (?1,?2,?3,?4,?5,?6,?7)")
                .map_err(storage)?;
            for fact in &facts {
                insert
                    .execute(params![
                        seq,
                        fact.access.as_str(),
                        fact.graph_id,
                        fact.revision,
                        fact.kind,
                        fact.fact_id,
                        fact.change
                    ])
                    .map_err(storage)?;
            }
        }
        tx.commit().map_err(storage)?;
        Ok(seq)
    }

    /// Entries after sequence number `after`, oldest first.
    pub fn entries(&self, after: i64, limit: usize) -> Result<Vec<Entry>, ApiError> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT seq,recorded_at_ms,request_id,principal,on_behalf_of,credential,operation,
                 outcome,code,graphs,revisions,fact_count,result_digest,digest
                 FROM audit_entries WHERE seq>?1 ORDER BY seq LIMIT ?2",
            )
            .map_err(storage)?;
        let decode =
            |text: String| -> Vec<String> { serde_json::from_str(&text).unwrap_or_default() };
        let rows = statement
            .query_map(params![after, limit.min(MAX_AUDIT_PAGE) as i64], |r| {
                Ok(Entry {
                    seq: r.get(0)?,
                    recorded_at_ms: r.get(1)?,
                    request_id: r.get(2)?,
                    principal: r.get(3)?,
                    on_behalf_of: decode(r.get(4)?),
                    credential: r.get(5)?,
                    operation: r.get(6)?,
                    outcome: Outcome::parse(&r.get::<_, String>(7)?),
                    code: r.get(8)?,
                    graphs: decode(r.get(9)?),
                    revisions: decode(r.get(10)?),
                    fact_count: r.get(11)?,
                    result_digest: r.get(12)?,
                    digest: r.get(13)?,
                })
            })
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        Ok(rows)
    }

    /// Every recorded access to fact `fact_id` of `graph_id`, optionally of
    /// one access kind, after sequence number `after`, oldest first.
    pub fn fact_accesses(
        &self,
        graph_id: &str,
        fact_id: &str,
        access: Option<Access>,
        after: i64,
        limit: usize,
    ) -> Result<Vec<FactAccessEntry>, ApiError> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT e.seq,e.recorded_at_ms,e.principal,e.on_behalf_of,e.operation,e.request_id,
                 f.access,f.graph_id,f.revision,f.kind,f.fact_id,f.change
                 FROM audit_facts f JOIN audit_entries e ON e.seq=f.seq
                 WHERE f.graph_id=?1 AND f.fact_id=?2 AND (?3 IS NULL OR f.access=?3) AND f.seq>?4
                 ORDER BY f.seq, f.access, f.revision LIMIT ?5",
            )
            .map_err(storage)?;
        let rows = statement
            .query_map(
                params![
                    graph_id,
                    fact_id,
                    access.map(Access::as_str),
                    after,
                    limit.min(MAX_AUDIT_PAGE) as i64
                ],
                |r| {
                    let access: String = r.get(6)?;
                    Ok(FactAccessEntry {
                        seq: r.get(0)?,
                        recorded_at_ms: r.get(1)?,
                        principal: r.get(2)?,
                        on_behalf_of: serde_json::from_str(&r.get::<_, String>(3)?)
                            .unwrap_or_default(),
                        operation: r.get(4)?,
                        request_id: r.get(5)?,
                        fact: FactAccess {
                            access: if access == "write" {
                                Access::Write
                            } else {
                                Access::Read
                            },
                            graph_id: r.get(7)?,
                            revision: r.get(8)?,
                            kind: r.get(9)?,
                            fact_id: r.get(10)?,
                            change: r.get(11)?,
                        },
                    })
                },
            )
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        Ok(rows)
    }

    /// Recompute the hash chain. Returns the number of entries checked, or
    /// the sequence number of the first entry that does not match.
    pub fn verify_chain(&self) -> Result<std::result::Result<i64, i64>, ApiError> {
        let mut previous = GENESIS.to_string();
        let mut checked = 0;
        let mut after = 0;
        loop {
            let page = self.entries(after, MAX_AUDIT_PAGE)?;
            if page.is_empty() {
                return Ok(Ok(checked));
            }
            for entry in page {
                let facts = self.entry_facts(entry.seq)?;
                let stored_previous: String = self
                    .conn
                    .query_row(
                        "SELECT previous FROM audit_entries WHERE seq=?1",
                        [entry.seq],
                        |r| r.get(0),
                    )
                    .map_err(storage)?;
                let body = serde_json::json!({
                    "previous": previous,
                    "recorded_at_ms": entry.recorded_at_ms,
                    "request_id": entry.request_id,
                    "principal": entry.principal,
                    "on_behalf_of": entry.on_behalf_of,
                    "credential": entry.credential,
                    "operation": entry.operation,
                    "outcome": entry.outcome.as_str(),
                    "code": entry.code,
                    "graphs": entry.graphs,
                    "revisions": entry.revisions,
                    "facts": facts,
                    "result_digest": entry.result_digest,
                });
                let digest = format!(
                    "sha256:{}",
                    sha256_hex(&serde_json::to_vec(&body).expect("entries serialize"))
                );
                if stored_previous != previous || digest != entry.digest {
                    return Ok(Err(entry.seq));
                }
                previous = entry.digest;
                after = entry.seq;
                checked += 1;
            }
        }
    }

    fn entry_facts(&self, seq: i64) -> Result<Vec<FactAccess>, ApiError> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT access,graph_id,revision,kind,fact_id,change FROM audit_facts WHERE seq=?1",
            )
            .map_err(storage)?;
        let mut rows = statement
            .query_map([seq], |r| {
                let access: String = r.get(0)?;
                Ok(FactAccess {
                    access: if access == "write" {
                        Access::Write
                    } else {
                        Access::Read
                    },
                    graph_id: r.get(1)?,
                    revision: r.get(2)?,
                    kind: r.get(3)?,
                    fact_id: r.get(4)?,
                    change: r.get(5)?,
                })
            })
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        rows.sort();
        Ok(rows)
    }
}
