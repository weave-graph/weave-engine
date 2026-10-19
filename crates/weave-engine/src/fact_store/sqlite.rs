//! SQLite [`FactBackend`]. This module is the only engine code that issues SQL
//! against the fact log, its indexes and checkpoints; CI enforces that with
//! `scripts/check_fact_store_boundary.py`.
//!
//! The backend borrows the engine's connection, so fact rows commit atomically
//! with revisions, heads, events and head observations. `begin`/`commit` use a
//! savepoint, which nests inside an outer engine transaction and commits on
//! its own in autocommit mode.
use super::{
    order, validate_new, BaseState, FactBackend, FactFilter, FactKind, FactOp, FactVersion,
    IndexKeys, LoggedOp, Namespace, NewRevision, Recording, RevisionInfo, StoredFact, ValidTime,
    MAX_CHECKPOINT_BYTES,
};
use crate::{err, Result};
use rusqlite::{params, types::Value, Connection, OptionalExtension};

/// Every storage object this backend owns.
pub const TABLES: &[&str] = &[
    "fact_lines",
    "fact_revisions",
    "fact_log",
    "fact_index",
    "fact_checkpoints",
];

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS fact_lines(line INTEGER PRIMARY KEY,graph_id TEXT NOT NULL,header TEXT NOT NULL,base_parent TEXT);
CREATE TABLE IF NOT EXISTS fact_revisions(revision TEXT PRIMARY KEY,graph_id TEXT NOT NULL,branch_id TEXT NOT NULL,parent TEXT,line INTEGER NOT NULL REFERENCES fact_lines(line),seq INTEGER NOT NULL CHECK(seq>0),first_position INTEGER NOT NULL,op_count INTEGER NOT NULL CHECK(op_count>0),writer TEXT NOT NULL,recorded_at_ms INTEGER NOT NULL CHECK(recorded_at_ms>=0),UNIQUE(line,seq));
CREATE TABLE IF NOT EXISTS fact_log(position INTEGER PRIMARY KEY,line INTEGER NOT NULL REFERENCES fact_lines(line),seq INTEGER NOT NULL,op TEXT NOT NULL CHECK(op IN ('base','assert','retract')),kind TEXT NOT NULL,namespace TEXT NOT NULL,fact_id TEXT NOT NULL,body TEXT,writer TEXT NOT NULL,recorded_at_ms INTEGER NOT NULL,recorded_in TEXT NOT NULL,CHECK((op='retract')=(body IS NULL)));
CREATE TABLE IF NOT EXISTS fact_index(position INTEGER PRIMARY KEY REFERENCES fact_log(position),line INTEGER NOT NULL,namespace TEXT NOT NULL,fact_id TEXT NOT NULL,kind TEXT NOT NULL,from_seq INTEGER NOT NULL,until_seq INTEGER,entity TEXT,subject TEXT,object TEXT,predicate TEXT,valid_from INTEGER,valid_to INTEGER,recorded_at_ms INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS fact_checkpoints(revision TEXT PRIMARY KEY REFERENCES fact_revisions(revision),body TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS fact_revisions_line ON fact_revisions(line,seq);
CREATE INDEX IF NOT EXISTS fact_index_key ON fact_index(line,namespace,fact_id,from_seq);
CREATE INDEX IF NOT EXISTS fact_index_kind ON fact_index(line,kind,fact_id);
CREATE INDEX IF NOT EXISTS fact_index_entity ON fact_index(line,entity);
CREATE INDEX IF NOT EXISTS fact_index_subject ON fact_index(line,subject);
CREATE INDEX IF NOT EXISTS fact_index_object ON fact_index(line,object);
CREATE INDEX IF NOT EXISTS fact_index_predicate ON fact_index(line,predicate);
CREATE INDEX IF NOT EXISTS fact_index_valid ON fact_index(line,valid_from);
CREATE INDEX IF NOT EXISTS fact_index_recorded ON fact_index(line,recorded_at_ms);
";

const FACT_COLUMNS: &str = "l.position,l.kind,l.fact_id,l.body,l.writer,l.recorded_at_ms,l.recorded_in,i.entity,i.subject,i.object,i.predicate,i.valid_from,i.valid_to";
const LIVE: &str = "i.line=?1 AND i.from_seq<=?2 AND (i.until_seq IS NULL OR i.until_seq>?2)";

struct Entry<'a> {
    line: i64,
    seq: i64,
    op: &'a str,
    kind: FactKind,
    id: &'a str,
    body: Option<&'a str>,
}

/// The SQLite fact store over a borrowed connection.
pub struct SqliteFactStore<'c> {
    conn: &'c Connection,
    open: bool,
}

/// Create the fact-store schema if it is absent. Idempotent.
pub fn install(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)?;
    Ok(())
}

impl<'c> SqliteFactStore<'c> {
    pub fn new(conn: &'c Connection) -> Self {
        Self { conn, open: false }
    }
    fn info(&self, revision: &str) -> Result<RevisionInfo> {
        self.revision(revision)?
            .ok_or_else(|| err("E_UNAVAILABLE", "fact revision unavailable"))
    }
    fn tip(&self, line: i64) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COALESCE(MAX(seq),0) FROM fact_revisions WHERE line=?1",
            [line],
            |r| r.get(0),
        )?)
    }
    fn live(
        &self,
        line: i64,
        seq: i64,
        namespace: Namespace,
        id: &str,
    ) -> Result<Option<(i64, String)>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT i.position,i.kind FROM fact_index i WHERE {LIVE} AND i.namespace=?3 AND i.fact_id=?4"
                ),
                params![line, seq, namespace.as_str(), id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }
    fn insert_log(&self, entry: Entry<'_>, recording: &Recording) -> Result<i64> {
        let Entry {
            line,
            seq,
            op,
            kind,
            id,
            body,
        } = entry;
        self.conn.execute(
            "INSERT INTO fact_log(line,seq,op,kind,namespace,fact_id,body,writer,recorded_at_ms,recorded_in) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                line,
                seq,
                op,
                kind.as_str(),
                kind.namespace().as_str(),
                id,
                body,
                recording.writer,
                recording.recorded_at_ms,
                recording.revision
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }
    fn insert_version(
        &self,
        line: i64,
        seq: i64,
        op: &str,
        version: &FactVersion,
        recording: &Recording,
    ) -> Result<i64> {
        let position = self.insert_log(
            Entry {
                line,
                seq,
                op,
                kind: version.kind,
                id: &version.id,
                body: Some(&version.body),
            },
            recording,
        )?;
        let keys = &version.keys;
        self.conn.execute(
            "INSERT INTO fact_index(position,line,namespace,fact_id,kind,from_seq,until_seq,entity,subject,object,predicate,valid_from,valid_to,recorded_at_ms) VALUES (?1,?2,?3,?4,?5,?6,NULL,?7,?8,?9,?10,?11,?12,?13)",
            params![
                position,
                line,
                version.kind.namespace().as_str(),
                version.id,
                version.kind.as_str(),
                seq,
                keys.entity,
                keys.subject,
                keys.object,
                keys.predicate,
                keys.valid.map(|v| v.start),
                keys.valid.and_then(|v| v.end),
                recording.recorded_at_ms
            ],
        )?;
        Ok(position)
    }
    fn append_inner(&self, revision: NewRevision, base: Option<BaseState>) -> Result<RevisionInfo> {
        validate_new(&revision, base.as_ref())?;
        if self.revision(&revision.revision)?.is_some() {
            return Err(err("E_CONFLICT", "fact revision already exists"));
        }
        let parent = match &revision.parent {
            Some(parent) => self.revision(parent)?,
            None => None,
        };
        if parent
            .as_ref()
            .is_some_and(|p| p.graph_id != revision.graph_id)
        {
            return Err(err("E_INTEGRITY", "fact parent belongs to another graph"));
        }
        let continues = match &parent {
            Some(p) => self.tip(p.line)? == p.seq,
            None => false,
        };
        let (line, seq) = match (&parent, continues) {
            (Some(parent), true) => {
                if base.is_some() {
                    return Err(err(
                        "E_INTEGRITY",
                        "a line continuation takes no base state",
                    ));
                }
                if revision.recorded_at_ms < parent.recorded_at_ms {
                    return Err(err("E_HISTORY_CLOCK", "recorded time regressed on a line"));
                }
                (parent.line, parent.seq + 1)
            }
            _ => {
                let base =
                    base.ok_or_else(|| err("E_INTEGRITY", "a new line requires a base state"))?;
                if base
                    .facts
                    .iter()
                    .any(|(_, r)| r.recorded_at_ms > revision.recorded_at_ms)
                {
                    return Err(err("E_HISTORY_CLOCK", "recorded time regressed on a line"));
                }
                self.conn.execute(
                    "INSERT INTO fact_lines(graph_id,header,base_parent) VALUES (?1,?2,?3)",
                    params![revision.graph_id, base.header, revision.parent],
                )?;
                let line = self.conn.last_insert_rowid();
                for (version, recording) in &base.facts {
                    self.insert_version(line, 0, "base", version, recording)?;
                }
                (line, 1)
            }
        };
        let recording = Recording {
            writer: revision.writer.clone(),
            recorded_at_ms: revision.recorded_at_ms,
            revision: revision.revision.clone(),
        };
        let mut first_position = None;
        for op in &revision.ops {
            let live = self.live(line, seq - 1, op.kind().namespace(), op.id())?;
            if let Some((position, kind)) = &live {
                if kind != op.kind().as_str() {
                    return Err(err("E_CONFLICT", "fact kind differs from the live fact"));
                }
                self.conn.execute(
                    "UPDATE fact_index SET until_seq=?1 WHERE position=?2",
                    params![seq, position],
                )?;
            }
            let position = match op {
                FactOp::Assert(version) => {
                    self.insert_version(line, seq, "assert", version, &recording)?
                }
                FactOp::Retract { kind, id } => {
                    if live.is_none() {
                        return Err(err("E_CONFLICT", "retracted fact is not live"));
                    }
                    let entry = Entry {
                        line,
                        seq,
                        op: "retract",
                        kind: *kind,
                        id,
                        body: None,
                    };
                    self.insert_log(entry, &recording)?
                }
            };
            first_position.get_or_insert(position);
        }
        let first_position = first_position
            .ok_or_else(|| err("E_CONFLICT", "a fact revision needs an operation"))?;
        let info = RevisionInfo {
            revision: revision.revision,
            graph_id: revision.graph_id,
            branch_id: revision.branch_id,
            parent: revision.parent,
            line,
            seq,
            first_position,
            op_count: revision.ops.len() as i64,
            writer: revision.writer,
            recorded_at_ms: revision.recorded_at_ms,
        };
        let line_graph: String = self.conn.query_row(
            "SELECT graph_id FROM fact_lines WHERE line=?1",
            [line],
            |r| r.get(0),
        )?;
        if line_graph != info.graph_id {
            return Err(err("E_INTEGRITY", "fact line belongs to another graph"));
        }
        self.conn.execute(
            "INSERT INTO fact_revisions(revision,graph_id,branch_id,parent,line,seq,first_position,op_count,writer,recorded_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                info.revision,
                info.graph_id,
                info.branch_id,
                info.parent,
                info.line,
                info.seq,
                info.first_position,
                info.op_count,
                info.writer,
                info.recorded_at_ms
            ],
        )?;
        Ok(info)
    }
}

fn integrity() -> crate::Error {
    err("E_INTEGRITY", "stored fact entry is malformed")
}

type FactRow = (
    i64,
    String,
    String,
    Option<String>,
    String,
    i64,
    String,
    [Option<String>; 4],
    Option<i64>,
    Option<i64>,
);

fn fact_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<FactRow> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        [r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?],
        r.get(11)?,
        r.get(12)?,
    ))
}

fn stored(row: FactRow) -> Result<StoredFact> {
    let (position, kind, id, body, writer, recorded_at_ms, revision, keys, from, to) = row;
    let kind = FactKind::parse(&kind).ok_or_else(integrity)?;
    let body = body.ok_or_else(integrity)?;
    let [entity, subject, object, predicate] = keys;
    let valid = match (from, to) {
        (Some(start), end) => Some(ValidTime { start, end }),
        (None, None) => None,
        (None, Some(_)) => return Err(integrity()),
    };
    Ok(StoredFact {
        position,
        version: FactVersion {
            kind,
            id,
            body,
            keys: IndexKeys {
                entity,
                subject,
                object,
                predicate,
                valid,
            },
        },
        recording: Recording {
            writer,
            recorded_at_ms,
            revision,
        },
    })
}

impl FactBackend for SqliteFactStore<'_> {
    fn durable(&self) -> bool {
        true
    }
    fn begin(&mut self) -> Result<()> {
        if self.open {
            return Err(err("E_INTEGRITY", "fact transaction already open"));
        }
        self.conn.execute_batch("SAVEPOINT weave_fact_tx")?;
        self.open = true;
        Ok(())
    }
    fn commit(&mut self) -> Result<()> {
        if !self.open {
            return Err(err("E_INTEGRITY", "no fact transaction open"));
        }
        self.conn.execute_batch("RELEASE weave_fact_tx")?;
        self.open = false;
        Ok(())
    }
    fn rollback(&mut self) -> Result<()> {
        if !self.open {
            return Err(err("E_INTEGRITY", "no fact transaction open"));
        }
        self.conn
            .execute_batch("ROLLBACK TO weave_fact_tx; RELEASE weave_fact_tx")?;
        self.open = false;
        Ok(())
    }
    fn revision(&self, revision: &str) -> Result<Option<RevisionInfo>> {
        Ok(self
            .conn
            .query_row(
                "SELECT revision,graph_id,branch_id,parent,line,seq,first_position,op_count,writer,recorded_at_ms FROM fact_revisions WHERE revision=?1",
                [revision],
                |r| {
                    Ok(RevisionInfo {
                        revision: r.get(0)?,
                        graph_id: r.get(1)?,
                        branch_id: r.get(2)?,
                        parent: r.get(3)?,
                        line: r.get(4)?,
                        seq: r.get(5)?,
                        first_position: r.get(6)?,
                        op_count: r.get(7)?,
                        writer: r.get(8)?,
                        recorded_at_ms: r.get(9)?,
                    })
                },
            )
            .optional()?)
    }
    fn is_tip(&self, revision: &str) -> Result<bool> {
        match self.revision(revision)? {
            Some(info) => Ok(self.tip(info.line)? == info.seq),
            None => Ok(false),
        }
    }
    fn append(&mut self, revision: NewRevision, base: Option<BaseState>) -> Result<RevisionInfo> {
        self.conn.execute_batch("SAVEPOINT weave_fact_append")?;
        match self.append_inner(revision, base) {
            Ok(info) => {
                self.conn.execute_batch("RELEASE weave_fact_append")?;
                Ok(info)
            }
            Err(error) => {
                self.conn
                    .execute_batch("ROLLBACK TO weave_fact_append; RELEASE weave_fact_append")?;
                Err(error)
            }
        }
    }
    fn header(&self, revision: &str) -> Result<String> {
        let info = self.info(revision)?;
        Ok(self.conn.query_row(
            "SELECT header FROM fact_lines WHERE line=?1",
            [info.line],
            |r| r.get(0),
        )?)
    }
    fn get(&self, revision: &str, namespace: Namespace, id: &str) -> Result<Option<StoredFact>> {
        let info = self.info(revision)?;
        let row = self
            .conn
            .query_row(
                &format!("SELECT {FACT_COLUMNS} FROM fact_index i JOIN fact_log l ON l.position=i.position WHERE {LIVE} AND i.namespace=?3 AND i.fact_id=?4"),
                params![info.line, info.seq, namespace.as_str(), id],
                fact_row,
            )
            .optional()?;
        row.map(stored).transpose()
    }
    fn select(&self, revision: &str, filter: &FactFilter, limit: usize) -> Result<Vec<StoredFact>> {
        let info = self.info(revision)?;
        let mut sql = format!(
            "SELECT {FACT_COLUMNS} FROM fact_index i JOIN fact_log l ON l.position=i.position WHERE {LIVE}"
        );
        let mut values: Vec<Value> = vec![info.line.into(), info.seq.into()];
        // Each `?` in a clause refers to the one value bound with it.
        let mut bind = |sql: &mut String, clause: &str, value: Value| {
            values.push(value);
            sql.push_str(&clause.replace('?', &format!("?{}", values.len())));
        };
        if let Some(kind) = filter.kind {
            bind(&mut sql, " AND i.kind=?", kind.as_str().to_string().into());
        }
        for (column, value) in [
            ("entity", &filter.entity),
            ("subject", &filter.subject),
            ("object", &filter.object),
            ("predicate", &filter.predicate),
        ] {
            if let Some(value) = value {
                bind(
                    &mut sql,
                    &format!(" AND i.{column}=?"),
                    value.clone().into(),
                );
            }
        }
        if let Some(t) = filter.valid_at {
            // No valid time means valid at every instant.
            bind(
                &mut sql,
                " AND (i.valid_from IS NULL OR (i.valid_from<=? AND (i.valid_to IS NULL OR i.valid_to>?)))",
                t.into(),
            );
        }
        if let Some((start, end)) = filter.recorded {
            bind(&mut sql, " AND i.recorded_at_ms>=?", start.into());
            bind(&mut sql, " AND i.recorded_at_ms<?", end.into());
        }
        let cap = i64::try_from(limit)
            .unwrap_or(i64::MAX - 1)
            .saturating_add(1);
        bind(&mut sql, " LIMIT ?", cap.into());
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(values.iter()), fact_row)?;
        let mut facts = Vec::new();
        for row in rows {
            if facts.len() == limit {
                return Err(err("E_BUDGET", "fact selection exceeds its limit"));
            }
            facts.push(stored(row?)?);
        }
        order(&mut facts);
        Ok(facts)
    }
    fn ops(&self, revision: &str) -> Result<Vec<LoggedOp>> {
        let info = self.info(revision)?;
        let mut statement = self.conn.prepare(
            "SELECT l.position,l.op,l.kind,l.fact_id,l.body,i.entity,i.subject,i.object,i.predicate,i.valid_from,i.valid_to FROM fact_log l LEFT JOIN fact_index i ON i.position=l.position WHERE l.position>=?1 AND l.position<?2 ORDER BY l.position",
        )?;
        let rows = statement.query_map(
            params![info.first_position, info.first_position + info.op_count],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    [r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?],
                    r.get::<_, Option<i64>>(9)?,
                    r.get::<_, Option<i64>>(10)?,
                ))
            },
        )?;
        let mut ops = Vec::new();
        for row in rows {
            let (position, op, kind, id, body, keys, from, to) = row?;
            let kind = FactKind::parse(&kind).ok_or_else(integrity)?;
            let op = match (op.as_str(), body) {
                ("retract", None) => FactOp::Retract { kind, id },
                ("assert", Some(body)) => {
                    let [entity, subject, object, predicate]: [Option<String>; 4] = keys;
                    let valid = match (from, to) {
                        (Some(start), end) => Some(ValidTime { start, end }),
                        (None, None) => None,
                        (None, Some(_)) => return Err(integrity()),
                    };
                    FactOp::Assert(FactVersion {
                        kind,
                        id,
                        body,
                        keys: IndexKeys {
                            entity,
                            subject,
                            object,
                            predicate,
                            valid,
                        },
                    })
                }
                _ => return Err(integrity()),
            };
            ops.push(LoggedOp { position, op });
        }
        if ops.len() as i64 != info.op_count {
            return Err(integrity());
        }
        Ok(ops)
    }
    fn put_checkpoint(&mut self, revision: &str, body: &str) -> Result<()> {
        self.info(revision)?;
        if body.len() > MAX_CHECKPOINT_BYTES {
            return Err(err("E_BUDGET", "checkpoint exceeds 256 MiB"));
        }
        self.conn.execute(
            "INSERT INTO fact_checkpoints(revision,body) VALUES (?1,?2) ON CONFLICT(revision) DO UPDATE SET body=excluded.body",
            params![revision, body],
        )?;
        Ok(())
    }
    fn checkpoint(&self, revision: &str) -> Result<Option<String>> {
        self.info(revision)?;
        Ok(self
            .conn
            .query_row(
                "SELECT body FROM fact_checkpoints WHERE revision=?1",
                [revision],
                |r| r.get(0),
            )
            .optional()?)
    }
    fn drop_checkpoint(&mut self, revision: &str) -> Result<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM fact_checkpoints WHERE revision=?1", [revision])?
            > 0)
    }
}
