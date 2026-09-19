//! Typed native governance delivery. No graph mutation or external effect is executed.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernanceEvent {
    pub id: String,
    pub view_id: String,
    pub decision_id: String,
    pub event_type: String,
    pub recorded_at_ms: i64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernanceDelivery {
    pub event: GovernanceEvent,
    pub source: String,
    /// Recipient-local delivered occurrence ordinal, never a global event offset.
    pub ordinal: u64,
    pub lease: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernanceAcknowledgment {
    pub ordinal: u64,
    pub duplicate: bool,
}
struct Subscription {
    checkpoint: i64,
    ordinal: i64,
    epoch: i64,
}
struct Pending {
    event: String,
    sequence: i64,
    ordinal: i64,
    lease: String,
    expires: i64,
    attempts: u32,
    dead: bool,
}
fn unavailable() -> Error {
    err("E_GOV_UNAVAILABLE", "governance delivery unavailable")
}
impl Engine {
    pub(crate) fn initialize_governance_delivery(&self) -> Result<()> {
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS governance_subscriptions(adapter TEXT NOT NULL,view_id TEXT NOT NULL,state TEXT NOT NULL,checkpoint INTEGER NOT NULL DEFAULT 0,ordinal INTEGER NOT NULL DEFAULT 0,epoch INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(adapter,view_id));
CREATE TABLE IF NOT EXISTS governance_delivery_pending(adapter TEXT NOT NULL,view_id TEXT NOT NULL,event_id TEXT NOT NULL,sequence INTEGER NOT NULL,ordinal INTEGER NOT NULL,lease TEXT NOT NULL,expires INTEGER NOT NULL,attempts INTEGER NOT NULL,status TEXT NOT NULL,PRIMARY KEY(adapter,view_id));
CREATE TABLE IF NOT EXISTS governance_delivery_receipts(adapter TEXT NOT NULL,view_id TEXT NOT NULL,event_id TEXT NOT NULL,lease TEXT NOT NULL,ordinal INTEGER NOT NULL,epoch INTEGER NOT NULL,PRIMARY KEY(adapter,view_id,event_id));")?;
        Ok(())
    }
    fn governance_delivery_atomic<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        self.conn.execute_batch("SAVEPOINT governance_delivery")?;
        match f() {
            Ok(value) => {
                if let Err(error) = self.conn.execute_batch("RELEASE governance_delivery") {
                    self.conn.execute_batch(
                        "ROLLBACK TO governance_delivery; RELEASE governance_delivery",
                    )?;
                    return Err(error.into());
                }
                Ok(value)
            }
            Err(error) => {
                self.conn.execute_batch(
                    "ROLLBACK TO governance_delivery; RELEASE governance_delivery",
                )?;
                Err(error)
            }
        }
    }
    fn governance_adapter(
        &self,
        adapter: &str,
        host: &HostContext,
    ) -> Result<(AdapterManifest, String)> {
        if !valid_id(adapter) || !valid_id(&host.principal) {
            return Err(unavailable());
        }
        self.read_budget.request()?;
        let row: Option<(Option<String>,String)> = self.conn.query_row(
            "SELECT CASE WHEN length(CAST(manifest AS BLOB))<=65536 THEN manifest ELSE NULL END,substr(state,1,16) FROM dispatch_adapters WHERE id=?1",
            [adapter],|r|Ok((r.get(0)?,r.get(1)?)),
        ).optional()?;
        let (body, state) = row.ok_or_else(unavailable)?;
        let body =
            body.ok_or_else(|| err("E_BUDGET", "governance adapter manifest exceeds limit"))?;
        self.read_budget.charge(body.len())?;
        let manifest: AdapterManifest = serde_json::from_str(&body)?;
        if manifest.id != adapter
            || manifest.principal != host.principal
            || state == "removed"
            || !(1..=20).contains(&manifest.max_attempts)
            || !(100..=3_600_000).contains(&manifest.lease_ms)
            || !(1..=100_000).contains(&manifest.max_pending_events)
        {
            return Err(unavailable());
        }
        Ok((manifest, state))
    }
    fn governance_subscription(&self, adapter: &str, view: &str) -> Result<Subscription> {
        if !valid_id(view) {
            return Err(unavailable());
        }
        let row:Option<(String,i64,i64,i64)>=self.conn.query_row(
            "SELECT substr(state,1,16),checkpoint,ordinal,epoch FROM governance_subscriptions WHERE adapter=?1 AND view_id=?2",
            params![adapter,view],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
        ).optional()?;
        let Some((state, checkpoint, ordinal, epoch)) = row else {
            return Err(unavailable());
        };
        if state != "active" || checkpoint < 0 || ordinal < 0 || epoch < 0 {
            return Err(unavailable());
        }
        Ok(Subscription {
            checkpoint,
            ordinal,
            epoch,
        })
    }
    fn governance_pending(&self, adapter: &str, view: &str) -> Result<Option<Pending>> {
        self.conn.query_row(
            "SELECT substr(event_id,1,513),sequence,ordinal,substr(lease,1,49),expires,attempts,status='dead_letter' FROM governance_delivery_pending WHERE adapter=?1 AND view_id=?2",
            params![adapter,view],|r|Ok(Pending{event:r.get(0)?,sequence:r.get(1)?,ordinal:r.get(2)?,lease:r.get(3)?,expires:r.get(4)?,attempts:r.get(5)?,dead:r.get(6)?}),
        ).optional().map_err(Into::into)
    }
    /// Uses an existing immutable adapter manifest and principal. A canceled subscription cannot restart.
    pub fn subscribe_governance(
        &self,
        adapter: &str,
        view: &str,
        now: i64,
        host: &HostContext,
    ) -> Result<bool> {
        let _scope = self.read_budget.enter();
        self.governance_delivery_atomic(|| {
            self.governance_adapter(adapter,host)?;
            self.inspect_governance_head(view,now,host)?;
            let prior:Option<String>=self.conn.query_row(
                "SELECT substr(state,1,16) FROM governance_subscriptions WHERE adapter=?1 AND view_id=?2",
                params![adapter,view],|r|r.get(0),
            ).optional()?;
            if let Some(state)=prior {
                if state!="active" { return Err(unavailable()); }
                return Ok(false);
            }
            let count:i64=self.conn.query_row("SELECT count(*) FROM governance_subscriptions WHERE adapter=?1",[adapter],|r|r.get(0))?;
            if count>=32 { return Err(err("E_BUDGET","governance subscription limit")); }
            self.conn.execute("INSERT INTO governance_subscriptions(adapter,view_id,state) VALUES (?1,?2,'active')",params![adapter,view])?;
            Ok(true)
        })
    }
    /// Owner cleanup remains available after view authority is revoked; no event payload is returned.
    pub fn cancel_governance_subscription(
        &self,
        adapter: &str,
        view: &str,
        host: &HostContext,
    ) -> Result<()> {
        let _scope = self.read_budget.enter();
        self.governance_delivery_atomic(|| {
            self.governance_adapter(adapter,host)?;
            self.governance_subscription(adapter,view)?;
            self.conn.execute("UPDATE governance_subscriptions SET state='canceled',epoch=epoch+1 WHERE adapter=?1 AND view_id=?2",params![adapter,view])?;
            self.conn.execute("DELETE FROM governance_delivery_pending WHERE adapter=?1 AND view_id=?2",params![adapter,view])?;
            Ok(())
        })
    }
    pub(crate) fn invalidate_governance_leases(&self, adapter: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE governance_subscriptions SET epoch=epoch+1 WHERE adapter=?1",
            [adapter],
        )?;
        self.conn.execute(
            "UPDATE governance_delivery_pending SET lease='',expires=0 WHERE adapter=?1",
            [adapter],
        )?;
        Ok(())
    }
    pub fn poll_governance(
        &self,
        adapter: &str,
        view: &str,
        now: i64,
        host: &HostContext,
    ) -> Result<Option<GovernanceDelivery>> {
        let _scope = self.read_budget.enter();
        if now < 0 {
            return Err(err("E_CLOCK", "negative delivery clock"));
        }
        self.governance_delivery_atomic(|| {
            let (manifest,state)=self.governance_adapter(adapter,host)?;
            if state!="running" && state!="draining" { return Err(err("E_PAUSED","adapter is not running")); }
            let sub=self.governance_subscription(adapter,view)?;
            self.inspect_governance_head(view,now,host)?;
            let pending=self.governance_pending(adapter,view)?;
            let (event,sequence,ordinal,attempts)=if let Some(pending)=pending {
                // Recheck original source before considering delivery timing or exposing its identity.
                let Some(event)=self.governance_delivery_event(view,&pending.event,now,host)? else { return Err(unavailable()); };
                if pending.dead || now<pending.expires { return Ok(None); }
                if pending.attempts>=manifest.max_attempts {
                    self.conn.execute("UPDATE governance_delivery_pending SET status='dead_letter' WHERE adapter=?1 AND view_id=?2",params![adapter,view])?;
                    return Ok(None);
                }
                (event,pending.sequence,pending.ordinal,pending.attempts+1)
            } else {
                if state=="draining" { return Ok(None); }
                let mut statement=self.conn.prepare("SELECT substr(id,1,513),sequence FROM governance_events WHERE view_id=?1 AND sequence>?2 ORDER BY sequence LIMIT 1001")?;
                let rows=statement.query_map(params![view,sub.checkpoint],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?)))?;
                let mut selected=None;
                let mut checkpoint=sub.checkpoint;
                let mut visible=0u32;
                for row in rows {
                    let (id,sequence)=row?;
                    if let Some(event)=self.governance_delivery_event(view,&id,now,host)? {
                        visible+=1;
                        if selected.is_none() { selected=Some((event,sequence)); }
                    } else if selected.is_none() { checkpoint=sequence; }
                }
                if visible>manifest.max_pending_events { return Err(err("E_BACKPRESSURE","governance backlog requires host review")); }
                self.conn.execute("UPDATE governance_subscriptions SET checkpoint=?3 WHERE adapter=?1 AND view_id=?2",params![adapter,view,checkpoint])?;
                let Some((event,sequence))=selected else { return Ok(None); };
                let ordinal=sub.ordinal.checked_add(1).ok_or_else(||err("E_BUDGET","delivery ordinal exhausted"))?;
                self.conn.execute("UPDATE governance_subscriptions SET ordinal=?3 WHERE adapter=?1 AND view_id=?2",params![adapter,view,ordinal])?;
                (event,sequence,ordinal,1)
            };
            let lease:String=self.conn.query_row("SELECT lower(hex(randomblob(24)))",[],|r|r.get(0))?;
            let expires=now.checked_add(manifest.lease_ms).ok_or_else(||err("E_CLOCK","delivery clock overflow"))?;
            self.conn.execute(
                "INSERT INTO governance_delivery_pending VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'leased') ON CONFLICT(adapter,view_id) DO UPDATE SET lease=excluded.lease,expires=excluded.expires,attempts=excluded.attempts,status='leased'",
                params![adapter,view,event.id,sequence,ordinal,lease,expires,attempts],
            )?;
            let source:String=self.conn.query_row("SELECT substr(source,1,513) FROM engine_identity WHERE id=1",[],|r|r.get(0))?;
            Ok(Some(GovernanceDelivery{event,source,ordinal:u64::try_from(ordinal).map_err(|_|unavailable())?,lease}))
        })
    }
    pub fn acknowledge_governance(
        &self,
        adapter: &str,
        view: &str,
        event: &str,
        lease: &str,
        now: i64,
        host: &HostContext,
    ) -> Result<GovernanceAcknowledgment> {
        self.acknowledge_governance_observed(adapter, view, event, lease, now, host, || {})
    }
    #[cfg(feature = "recovery-testing")]
    #[allow(clippy::too_many_arguments)]
    pub fn acknowledge_governance_test_before_commit(
        &self,
        adapter: &str,
        view: &str,
        event: &str,
        lease: &str,
        now: i64,
        host: &HostContext,
        hook: impl FnOnce(),
    ) -> Result<GovernanceAcknowledgment> {
        self.acknowledge_governance_observed(adapter, view, event, lease, now, host, hook)
    }
    #[allow(clippy::too_many_arguments)]
    fn acknowledge_governance_observed(
        &self,
        adapter: &str,
        view: &str,
        event: &str,
        lease: &str,
        now: i64,
        host: &HostContext,
        hook: impl FnOnce(),
    ) -> Result<GovernanceAcknowledgment> {
        let _scope = self.read_budget.enter();
        if now < 0
            || !valid_id(event)
            || lease.len() != 48
            || !lease.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(unavailable());
        }
        self.governance_delivery_atomic(|| {
            let (_,state)=self.governance_adapter(adapter,host)?;
            if state!="running" && state!="draining" { return Err(err("E_PAUSED","adapter is not running")); }
            let sub=self.governance_subscription(adapter,view)?;
            self.governance_delivery_event(view,event,now,host)?.ok_or_else(unavailable)?;
            let prior:Option<(String,i64,i64)>=self.conn.query_row(
                "SELECT substr(lease,1,49),ordinal,epoch FROM governance_delivery_receipts WHERE adapter=?1 AND view_id=?2 AND event_id=?3",
                params![adapter,view,event],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
            ).optional()?;
            if let Some((prior_lease,ordinal,epoch))=prior {
                if lease!=prior_lease || epoch!=sub.epoch { return Err(err("E_LEASE","governance lease is no longer current")); }
                return Ok(GovernanceAcknowledgment{ordinal:u64::try_from(ordinal).map_err(|_|unavailable())?,duplicate:true});
            }
            let pending=self.governance_pending(adapter,view)?.ok_or_else(unavailable)?;
            if pending.event!=event || pending.lease!=lease || pending.dead || now>=pending.expires { return Err(err("E_LEASE","governance lease is no longer current")); }
            let count:i64=self.conn.query_row("SELECT count(*) FROM governance_delivery_receipts WHERE adapter=?1 AND view_id=?2",params![adapter,view],|r|r.get(0))?;
            if count>=10000 { return Err(err("E_BUDGET","governance receipt limit")); }
            self.conn.execute("INSERT INTO governance_delivery_receipts VALUES (?1,?2,?3,?4,?5,?6)",params![adapter,view,event,lease,pending.ordinal,sub.epoch])?;
            self.conn.execute("UPDATE governance_subscriptions SET checkpoint=?3 WHERE adapter=?1 AND view_id=?2",params![adapter,view,pending.sequence])?;
            self.conn.execute("DELETE FROM governance_delivery_pending WHERE adapter=?1 AND view_id=?2",params![adapter,view])?;
            hook();
            Ok(GovernanceAcknowledgment{ordinal:u64::try_from(pending.ordinal).map_err(|_|unavailable())?,duplicate:false})
        })
    }
    pub fn replay_governance_dead_letter(
        &self,
        adapter: &str,
        view: &str,
        now: i64,
        host: &HostContext,
    ) -> Result<()> {
        let _scope = self.read_budget.enter();
        self.governance_delivery_atomic(|| {
            let (_,state)=self.governance_adapter(adapter,host)?;
            if state!="running" { return Err(err("E_PAUSED","adapter is not running")); }
            self.governance_subscription(adapter,view)?;
            let pending=self.governance_pending(adapter,view)?.ok_or_else(unavailable)?;
            self.governance_delivery_event(view,&pending.event,now,host)?.ok_or_else(unavailable)?;
            if !pending.dead { return Err(err("E_LIFECYCLE","delivery is not dead lettered")); }
            self.conn.execute("UPDATE governance_delivery_pending SET status='leased',lease='',expires=0,attempts=0 WHERE adapter=?1 AND view_id=?2",params![adapter,view])?;
            Ok(())
        })
    }
}
