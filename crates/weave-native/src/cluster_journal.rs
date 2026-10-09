//! Native trusted-host replay journal. The hash detects corruption, not compiler authority.
use crate::{
    artifacts::ArtifactBundle,
    host::{encode_value, HostError, HostReply, HostSession},
    strict_json,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    path::Path,
};
use weave_contract::Program;
use weave_engine::{Error, HandlerSlotState, RetainedClusterCompletion};
const RECORD_LIMIT: usize = 2 * 1024 * 1024;
const TOTAL_LIMIT: usize = 8 * 1024 * 1024;
const RECEIPT_LIMIT: usize = 64 * 1024;
type JournalRow = (Option<Vec<u8>>, Option<Vec<u8>>);
fn error(code: &str) -> Error {
    Error {
        code: code.into(),
        message: "trusted host journal unavailable".into(),
    }
}
fn host_error(e: impl std::fmt::Display) -> HostError {
    let _ = e;
    HostError::new("E_HOST_JOURNAL", "trusted host journal unavailable")
}
fn db<T>(r: rusqlite::Result<T>) -> weave_engine::Result<T> {
    r.map_err(|_| error("E_STORAGE"))
}
fn hash(domain: &str, value: &impl Serialize) -> weave_engine::Result<String> {
    weave_contract::identity::source_fingerprint(&(domain, value)).map_err(|d| Error {
        code: d.code,
        message: d.message,
    })
}
fn valid(v: &str) -> bool {
    !v.is_empty() && v.len() <= 512 && !v.chars().any(char::is_control)
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    format: String,
    host_store: String,
    runtime_source: String,
    sdk_sha256: String,
    artifact_fingerprint: String,
    retained: RetainedClusterCompletion,
}
/// One writer across processes; the operating system releases this lock on process death.
/// Paths/store IDs are independently installed host configuration, never operational JSON.
pub struct ClusterJournal {
    conn: Connection,
    _lock: File,
    host_store: String,
    runtime_source: String,
}
impl ClusterJournal {
    pub fn open(
        path: &Path,
        host_store: &str,
        session: &HostSession,
        create: bool,
    ) -> Result<Self, HostError> {
        if !valid(host_store) || session.is_poisoned() {
            return Err(host_error("config"));
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.with_extension("journal-lock"))
            .map_err(host_error)?;
        lock.try_lock()
            .map_err(|_| HostError::new("E_HOST_JOURNAL_LOCKED", "journal already owned"))?;
        if path.exists() == create {
            return Err(host_error("create/reopen"));
        }
        let runtime_source = session
            .engine
            .runtime_source_identity()
            .map_err(host_error)?;
        let conn = Connection::open(path).map_err(host_error)?;
        conn.execute_batch(
            "PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA busy_timeout=0;",
        )
        .map_err(host_error)?;
        if create {
            conn.execute_batch("BEGIN IMMEDIATE; CREATE TABLE journal_header(format TEXT,host_store TEXT,runtime_source TEXT); CREATE TABLE retained(id TEXT PRIMARY KEY,adapter TEXT NOT NULL,event TEXT NOT NULL,body BLOB NOT NULL,bundle BLOB NOT NULL,UNIQUE(adapter,event)); CREATE TABLE observed(id TEXT PRIMARY KEY,body BLOB NOT NULL,digest TEXT NOT NULL);").map_err(host_error)?;
            conn.execute(
                "INSERT INTO journal_header VALUES('weave-native-cluster-journal/1',?1,?2)",
                params![host_store, runtime_source],
            )
            .map_err(host_error)?;
            conn.execute_batch("COMMIT").map_err(host_error)?;
        }
        let headers: i64 = conn
            .query_row("SELECT count(*) FROM journal_header", [], |r| r.get(0))
            .map_err(host_error)?;
        let header:(String,String,String)=conn.query_row("SELECT substr(format,1,64),substr(host_store,1,513),substr(runtime_source,1,513) FROM journal_header",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(host_error)?;
        if headers != 1
            || header
                != (
                    "weave-native-cluster-journal/1".into(),
                    host_store.into(),
                    runtime_source.clone(),
                )
        {
            return Err(host_error("binding"));
        }
        Ok(Self {
            conn,
            _lock: lock,
            host_store: host_store.into(),
            runtime_source,
        })
    }
    fn bind(&self, session_source: &str) -> weave_engine::Result<()> {
        if session_source != self.runtime_source {
            Err(error("E_HOST_STORE"))
        } else {
            Ok(())
        }
    }
    fn load(&self, id: &str) -> weave_engine::Result<Record> {
        if !valid(id) {
            return Err(error("E_HOST_JOURNAL"));
        }
        let row:Option<JournalRow>=db(self.conn.query_row("SELECT CASE WHEN length(body)<=?2 THEN body END,CASE WHEN length(bundle)<=?2 THEN bundle END FROM retained WHERE id=?1",params![id,RECORD_LIMIT as i64],|r|Ok((r.get(0)?,r.get(1)?))).optional())?;
        let Some((body, bundle)) = row else {
            return Err(error("E_HOST_JOURNAL_MISSING"));
        };
        let (Some(body), Some(bundle)) = (body, bundle) else {
            return Err(error("E_HOST_JOURNAL_BUDGET"));
        };
        if body.len().saturating_add(bundle.len()) > RECORD_LIMIT {
            return Err(error("E_HOST_JOURNAL_BUDGET"));
        }
        strict_json::check(&body, RECORD_LIMIT).map_err(|_| error("E_HOST_JOURNAL_INTEGRITY"))?;
        let record: Record =
            serde_json::from_slice(&body).map_err(|_| error("E_HOST_JOURNAL_INTEGRITY"))?;
        let artifact =
            ArtifactBundle::parse(&bundle).map_err(|_| error("E_HOST_JOURNAL_INTEGRITY"))?;
        let recipe: Program = serde_json::from_slice(artifact.program_bytes())
            .map_err(|_| error("E_HOST_JOURNAL_INTEGRITY"))?;
        if hash("weave-native-cluster-record-v1", &record)? != id
            || record.format != "weave-native-cluster-record/1"
            || record.host_store != self.host_store
            || record.runtime_source != self.runtime_source
            || record.sdk_sha256 != format!("{:x}", Sha256::digest(&bundle))
            || record.artifact_fingerprint != artifact.inventory().artifact_fingerprint
            || recipe != record.retained.recipe
        {
            return Err(error("E_HOST_JOURNAL_INTEGRITY"));
        }
        Ok(record)
    }
    fn capacity(&self, additional: usize, new_record: bool) -> weave_engine::Result<()> {
        let (count,size):(i64,i64)=db(self.conn.query_row("SELECT count(*),coalesce(sum(length(body)+length(bundle)),0)+(SELECT coalesce(sum(length(body)),0) FROM observed) FROM retained",[],|r|Ok((r.get(0)?,r.get(1)?))))?;
        if count < 0
            || size < 0
            || count + i64::from(new_record) > 16
            || (size as usize).saturating_add(additional) > TOTAL_LIMIT
        {
            return Err(error("E_HOST_JOURNAL_BUDGET"));
        }
        Ok(())
    }
    /// The supplied bundle is complete SDK output; only the strict pinned Cluster Program is executed.
    pub fn prepare(
        &mut self,
        session: &mut HostSession,
        adapter: &str,
        event: &str,
        lease: &str,
        bundle: &ArtifactBundle,
        output_branch: &str,
    ) -> HostReply {
        self.prepare_boundary(
            session,
            adapter,
            event,
            lease,
            bundle,
            output_branch,
            || {},
            || {},
        )
    }
    #[cfg(feature = "recovery-testing")]
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_test_observers(
        &mut self,
        session: &mut HostSession,
        adapter: &str,
        event: &str,
        lease: &str,
        bundle: &ArtifactBundle,
        output_branch: &str,
        before: impl FnOnce(),
        after: impl FnOnce(),
    ) -> HostReply {
        self.prepare_boundary(
            session,
            adapter,
            event,
            lease,
            bundle,
            output_branch,
            before,
            after,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn prepare_boundary(
        &mut self,
        session: &mut HostSession,
        adapter: &str,
        event: &str,
        lease: &str,
        bundle: &ArtifactBundle,
        output_branch: &str,
        before: impl FnOnce(),
        after: impl FnOnce(),
    ) -> HostReply {
        session.invoke(|engine, authority| {
            self.bind(&engine.runtime_source_identity()?)?;
            let prior: Option<String> = db(self
                .conn
                .query_row(
                    "SELECT substr(id,1,100) FROM retained WHERE adapter=?1 AND event=?2",
                    params![adapter, event],
                    |r| r.get(0),
                )
                .optional())?;
            if let Some(id) = prior {
                let record = self.load(&id)?;
                if record.artifact_fingerprint != bundle.inventory().artifact_fingerprint {
                    return Err(error("E_HOST_JOURNAL_CONFLICT"));
                }
                let [weave_contract::Command::Commit { branch_id, .. }] =
                    record.retained.completion.commands.as_slice()
                else {
                    return Err(error("E_HOST_JOURNAL_INTEGRITY"));
                };
                if branch_id != output_branch {
                    return Err(error("E_HOST_JOURNAL_CONFLICT"));
                }
                engine.validate_retained_cluster_for(&record.retained, authority)?;
                return encode_value(&serde_json::json!({"record_id":id,"duplicate":true}));
            }
            if !matches!(
                engine.inspect_handler_slot_for(adapter, event, authority)?,
                HandlerSlotState::Pending
            ) {
                return Err(error("E_HOST_JOURNAL_MISSING"));
            }
            if bundle.original_bytes().len() > RECORD_LIMIT {
                return Err(error("E_HOST_JOURNAL_BUDGET"));
            }
            self.capacity(bundle.original_bytes().len(), true)?;
            let recipe: Program = serde_json::from_slice(bundle.program_bytes())
                .map_err(|_| error("E_HOST_ARTIFACT"))?;
            let retained = engine.capture_retained_cluster_for(
                adapter,
                event,
                lease,
                &recipe,
                output_branch,
                authority,
            )?;
            let record = Record {
                format: "weave-native-cluster-record/1".into(),
                host_store: self.host_store.clone(),
                runtime_source: self.runtime_source.clone(),
                sdk_sha256: format!("{:x}", Sha256::digest(bundle.original_bytes())),
                artifact_fingerprint: bundle.inventory().artifact_fingerprint.clone(),
                retained,
            };
            let body = bounded_json(
                &record,
                RECORD_LIMIT.saturating_sub(bundle.original_bytes().len()),
            )?;
            self.capacity(body.len() + bundle.original_bytes().len(), true)?;
            let id = hash("weave-native-cluster-record-v1", &record)?;
            let tx = db(self.conn.transaction())?;
            db(tx.execute(
                "INSERT INTO retained VALUES(?1,?2,?3,?4,?5)",
                params![id, adapter, event, body, bundle.original_bytes()],
            ))?;
            before();
            db(tx.commit())?;
            after();
            encode_value(&serde_json::json!({"record_id":id,"duplicate":false}))
        })
    }
    pub fn complete(&mut self, session: &mut HostSession, id: &str, lease: &str) -> HostReply {
        self.complete_boundary(session, id, lease, || {}, || {})
    }
    #[cfg(feature = "recovery-testing")]
    pub fn complete_test_observers(
        &mut self,
        session: &mut HostSession,
        id: &str,
        lease: &str,
        before: impl FnOnce(),
        after: impl FnOnce(),
    ) -> HostReply {
        self.complete_boundary(session, id, lease, before, after)
    }
    fn complete_boundary(
        &mut self,
        session: &mut HostSession,
        id: &str,
        lease: &str,
        before: impl FnOnce(),
        after: impl FnOnce(),
    ) -> HostReply {
        session.invoke(|engine,authority|{
            self.bind(&engine.runtime_source_identity()?)?;
            let record=self.load(id)?;
            let captured=&record.retained;
            #[cfg(feature="recovery-testing")]
            let receipt=engine.complete_retained_cluster_test_before_commit(&captured.adapter,&captured.event_id,lease,captured,authority,before)?;
            #[cfg(not(feature="recovery-testing"))]
            let receipt={let _=before;engine.complete_retained_cluster_for(&captured.adapter,&captured.event_id,lease,captured,authority)?};
            after();
            let bytes=bounded_json(&receipt.results,RECEIPT_LIMIT)?;
            let digest=hash("weave-native-cluster-observed-v1",&receipt.results)?;
            let prior:Option<(Option<Vec<u8>>,String)>=db(self.conn.query_row("SELECT CASE WHEN length(body)<=?2 THEN body END,substr(digest,1,100) FROM observed WHERE id=?1",params![id,RECEIPT_LIMIT as i64],|r|Ok((r.get(0)?,r.get(1)?))).optional())?;
            if let Some((old,old_digest))=prior {
                if old.as_deref()!=Some(bytes.as_slice())||old_digest!=digest{return Err(error("E_HOST_UNCERTAIN"));}
            }else{
                self.capacity(bytes.len(),false).map_err(|_|error("E_HOST_UNCERTAIN"))?;
                db(self.conn.execute("INSERT INTO observed VALUES(?1,?2,?3)",params![id,bytes,digest]))?;
            }
            encode_value(&receipt)
        })
    }
}
fn bounded_json(value: &impl Serialize, limit: usize) -> weave_engine::Result<Vec<u8>> {
    struct Writer {
        bytes: Vec<u8>,
        limit: usize,
    }
    impl std::io::Write for Writer {
        fn write(&mut self, v: &[u8]) -> std::io::Result<usize> {
            if self.bytes.len().saturating_add(v.len()) > self.limit {
                return Err(std::io::Error::other("journal limit"));
            }
            self.bytes.extend_from_slice(v);
            Ok(v.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Writer {
        bytes: vec![],
        limit,
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| error("E_HOST_JOURNAL_BUDGET"))?;
    Ok(writer.bytes)
}
