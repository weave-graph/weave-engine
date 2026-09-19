//! Principal-owned pinned routes. Route lifecycle is separate from graph acceptance.
use super::*;
use serde::{Deserialize, Serialize};
const MOUNT_LIMIT: usize = 1000;
const EVENT_LIMIT: i64 = 10000;
const STORAGE_LIMIT: usize = 64 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MountSpec {
    pub id: String,
    pub reference: GraphRef,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MountReceipt {
    pub id: String,
    pub generation: u64,
    pub active: bool,
    pub event_id: String,
    pub duplicate: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MountEvent {
    pub cursor: u64,
    pub id: String,
    pub generation: u64,
    pub active: bool,
    pub event_id: String,
}
fn digest(value: &impl Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
impl Engine {
    pub(crate) fn initialize_mounts(&self) -> Result<()> {
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS mounts(principal TEXT NOT NULL,id TEXT NOT NULL,spec TEXT NOT NULL,generation INTEGER NOT NULL,active INTEGER NOT NULL,authorization TEXT NOT NULL,PRIMARY KEY(principal,id));
CREATE TABLE IF NOT EXISTS mount_events(principal TEXT NOT NULL,cursor INTEGER NOT NULL,id TEXT NOT NULL,generation INTEGER NOT NULL,active INTEGER NOT NULL,event_id TEXT NOT NULL,PRIMARY KEY(principal,cursor),UNIQUE(event_id));
CREATE TABLE IF NOT EXISTS mount_receipts(principal TEXT NOT NULL,nonce TEXT NOT NULL,body_hash TEXT NOT NULL,receipt TEXT NOT NULL,PRIMARY KEY(principal,nonce));")?;
        Ok(())
    }
    fn mount_row(
        &self,
        id: &str,
        host: &HostContext,
    ) -> Result<Option<(MountSpec, u64, bool, QueryResult)>> {
        self.read_budget.request()?;
        let limit = self.read_budget.remaining().min(MATERIALIZED_LIMIT) as i64;
        type Row = (Option<String>, i64, bool, Option<String>);
        let row:Option<Row>=self.conn.query_row("SELECT CASE WHEN length(CAST(spec AS BLOB))<=4096 THEN spec END,generation,active,CASE WHEN length(CAST(authorization AS BLOB))<=?3 THEN authorization END FROM mounts WHERE principal=?1 AND id=?2",params![host.principal,id,limit],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let Some((spec, generation, active, authorization)) = row else {
            return Ok(None);
        };
        let spec = spec.ok_or_else(|| err("E_BUDGET", "mount descriptor exceeds budget"))?;
        let authorization =
            authorization.ok_or_else(|| err("E_BUDGET", "mount authorization exceeds budget"))?;
        self.read_budget.charge(spec.len() + authorization.len())?;
        let generation = u64::try_from(generation)
            .map_err(|_| err("E_INTEGRITY", "invalid mount generation"))?;
        Ok(Some((
            serde_json::from_str(&spec)?,
            generation,
            active,
            serde_json::from_str(&authorization)?,
        )))
    }
    fn mount_prior(
        &self,
        nonce: &str,
        body: &str,
        host: &HostContext,
    ) -> Result<Option<MountReceipt>> {
        let prior:Option<(String,Option<String>)>=self.conn.query_row("SELECT substr(body_hash,1,65),CASE WHEN length(CAST(receipt AS BLOB))<=4096 THEN receipt END FROM mount_receipts WHERE principal=?1 AND nonce=?2",params![host.principal,nonce],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        match prior {
            None => Ok(None),
            Some((hash, receipt)) => {
                if hash != body {
                    return Err(err("E_REPLAY", "mount nonce binds another request"));
                }
                let mut receipt: MountReceipt = serde_json::from_str(
                    &receipt.ok_or_else(|| err("E_BUDGET", "mount receipt exceeds budget"))?,
                )?;
                receipt.duplicate = true;
                Ok(Some(receipt))
            }
        }
    }
    fn mount_occurrence(
        &self,
        id: &str,
        generation: u64,
        active: bool,
        nonce: &str,
        body: &str,
        host: &HostContext,
    ) -> Result<MountReceipt> {
        let last: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(cursor),0) FROM mount_events WHERE principal=?1",
            [&host.principal],
            |r| r.get(0),
        )?;
        if last >= EVENT_LIMIT {
            return Err(err("E_BACKPRESSURE", "mount event capacity reached"));
        }
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM mount_receipts WHERE principal=?1",
            [&host.principal],
            |r| r.get(0),
        )?;
        if count >= EVENT_LIMIT {
            return Err(err("E_BACKPRESSURE", "mount receipt capacity reached"));
        }
        let event_id = format!(
            "mount:{}",
            digest(&(&host.principal, id, generation, active, last + 1))?
        );
        let receipt = MountReceipt {
            id: id.into(),
            generation,
            active,
            event_id: event_id.clone(),
            duplicate: false,
        };
        self.conn.execute(
            "INSERT INTO mount_events VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                host.principal,
                last + 1,
                id,
                generation as i64,
                active,
                event_id
            ],
        )?;
        self.conn.execute(
            "INSERT INTO mount_receipts VALUES (?1,?2,?3,?4)",
            params![
                host.principal,
                nonce,
                body,
                serde_json::to_string(&receipt)?
            ],
        )?;
        Ok(receipt)
    }
    /// A route never grants graph authority or advances an accepted branch.
    pub fn attach_mount(
        &mut self,
        spec: &MountSpec,
        expected_generation: Option<u64>,
        nonce: &str,
        host: &HostContext,
    ) -> Result<MountReceipt> {
        let _scope = self.read_budget.enter();
        if [
            &spec.id,
            &spec.reference.graph_id,
            &spec.reference.revision,
            &host.principal,
        ]
        .iter()
        .any(|s| !valid_id(s))
            || !valid_id(nonce)
        {
            return Err(err("E_ID", "invalid mount request"));
        }
        json_size(spec, 4096)?;
        let tx = self.conn.unchecked_transaction()?;
        let _clock_scope = self.operation_write_scope()?;
        let query: QueryPlan = serde_json::from_value(
            serde_json::json!({"graph_id":spec.reference.graph_id,"revision":spec.reference.revision}),
        )?;
        let authorization = self.query(&query, host)?;
        let body = digest(&("attach", spec, expected_generation))?;
        if let Some(prior) = self.mount_prior(nonce, &body, host)? {
            let (_, _, _, guard) = self
                .mount_row(&spec.id, host)?
                .ok_or_else(|| err("E_INTEGRITY", "mount receipt has no route"))?;
            self.require_current_result_authority(&guard, host)?;
            tx.commit()?;
            return Ok(prior);
        }
        let prior = self.mount_row(&spec.id, host)?;
        if prior.as_ref().map(|(_, generation, _, _)| *generation) != expected_generation {
            return Err(err("E_CONFLICT", "mount generation differs"));
        }
        if prior.as_ref().is_some_and(|(old, _, _, _)| old != spec) {
            return Err(err("E_MOUNT_IDENTITY", "mount source is immutable"));
        }
        if prior.as_ref().is_some_and(|(_, _, active, _)| *active) {
            return Err(err(
                "E_MOUNT_ACTIVE",
                "mount already active; retry the original nonce",
            ));
        }
        let (count,bytes):(i64,i64)=self.conn.query_row("SELECT COUNT(*),COALESCE(SUM(length(CAST(authorization AS BLOB))),0) FROM mounts WHERE principal=?1",[&host.principal],|r|Ok((r.get(0)?,r.get(1)?)))?;
        let encoded = serde_json::to_string(&authorization)?;
        let previous_bytes = prior
            .as_ref()
            .map(|(_, _, _, value)| json_size(value, MATERIALIZED_LIMIT))
            .transpose()?
            .unwrap_or(0);
        let final_bytes = usize::try_from(bytes)
            .ok()
            .and_then(|used| used.checked_sub(previous_bytes))
            .and_then(|used| used.checked_add(encoded.len()));
        if (prior.is_none() && count >= MOUNT_LIMIT as i64)
            || final_bytes.is_none_or(|bytes| bytes > STORAGE_LIMIT)
        {
            return Err(err("E_BACKPRESSURE", "mount storage capacity reached"));
        }
        let generation = expected_generation
            .unwrap_or(0)
            .checked_add(1)
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or_else(|| err("E_BUDGET", "mount generation exhausted"))?;
        self.conn.execute("INSERT INTO mounts VALUES (?1,?2,?3,?4,1,?5) ON CONFLICT(principal,id) DO UPDATE SET generation=excluded.generation,active=1,authorization=excluded.authorization",params![host.principal,spec.id,serde_json::to_string(spec)?,generation as i64,encoded])?;
        let receipt = self.mount_occurrence(&spec.id, generation, true, nonce, &body, host)?;
        tx.commit()?;
        Ok(receipt)
    }
    /// Owner cleanup remains available after source revocation; receipt discloses no graph content.
    pub fn detach_mount(
        &mut self,
        id: &str,
        expected_generation: u64,
        nonce: &str,
        host: &HostContext,
    ) -> Result<MountReceipt> {
        let _scope = self.read_budget.enter();
        if !valid_id(id) || !valid_id(nonce) || !valid_id(&host.principal) {
            return Err(err("E_ID", "invalid mount request"));
        }
        let tx = self.conn.unchecked_transaction()?;
        let _clock_scope = self.operation_write_scope()?;
        let body = digest(&("detach", id, expected_generation))?;
        if let Some(prior) = self.mount_prior(nonce, &body, host)? {
            tx.commit()?;
            return Ok(prior);
        }
        let (_, generation, active, _) = self
            .mount_row(id, host)?
            .ok_or_else(|| err("E_UNAVAILABLE", "mount unavailable"))?;
        if generation != expected_generation {
            return Err(err("E_CONFLICT", "mount generation differs"));
        }
        if !active {
            return Err(err("E_MOUNT_DETACHED", "mount already detached"));
        }
        let next = generation
            .checked_add(1)
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or_else(|| err("E_BUDGET", "mount generation exhausted"))?;
        self.conn.execute(
            "UPDATE mounts SET active=0,generation=?3 WHERE principal=?1 AND id=?2",
            params![host.principal, id, next as i64],
        )?;
        let receipt = self.mount_occurrence(id, next, false, nonce, &body, host)?;
        tx.commit()?;
        Ok(receipt)
    }
    pub fn query_mount(&self, id: &str, host: &HostContext) -> Result<QueryResult> {
        let _scope = self.read_budget.enter();
        let tx = self.conn.unchecked_transaction()?;
        let _clock_scope = self.operation_scope()?;
        let (spec, _, active, prior) = self
            .mount_row(id, host)?
            .ok_or_else(|| err("E_UNAVAILABLE", "mount unavailable"))?;
        if !active {
            return Err(err("E_UNAVAILABLE", "mount unavailable"));
        }
        self.require_current_result_authority(&prior, host)?;
        let result=self.query(&serde_json::from_value(serde_json::json!({"graph_id":spec.reference.graph_id,"revision":spec.reference.revision}))?,host)?;
        tx.commit()?;
        Ok(result)
    }
    /// Native principal-local lifecycle stream. It is not the generic graph adapter bus.
    pub fn mount_changes(
        &self,
        after: u64,
        limit: usize,
        host: &HostContext,
    ) -> Result<Vec<MountEvent>> {
        let _scope = self.read_budget.enter();
        if limit == 0 || limit > 100 || after > i64::MAX as u64 {
            return Err(err("E_BUDGET", "invalid mount stream bounds"));
        }
        let mut statement=self.conn.prepare("SELECT cursor,id,generation,active,event_id FROM mount_events WHERE principal=?1 AND cursor>?2 ORDER BY cursor LIMIT ?3")?;
        let events = statement
            .query_map(params![host.principal, after as i64, limit as i64], |r| {
                Ok(MountEvent {
                    cursor: r.get::<_, i64>(0)? as u64,
                    id: r.get(1)?,
                    generation: r.get::<_, i64>(2)? as u64,
                    active: r.get(3)?,
                    event_id: r.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        // This stream contains only actions performed by this owner and no source graph IDs/content.
        Ok(events)
    }
}
