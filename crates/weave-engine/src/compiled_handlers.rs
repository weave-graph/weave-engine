//! Trusted compiled projection handlers. No caller Program or QueryResult is authority.
use super::*;
use serde::{Deserialize, Serialize};
const REGISTRATION_LIMIT: usize = 2 * 1024 * 1024;

/// Trusted host mapping of an inert artifact slot to one writable destination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HandlerOutputBinding {
    pub slot: String,
    pub graph_id: String,
    pub branch_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreparedHandlerReceipt {
    pub duplicate: bool,
    pub preparation_id: String,
    pub definition_digest: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Output {
    slot: String,
    graph_id: String,
    branch_id: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    template: CompiledHandlerTemplate,
    output: Output,
    manifest: AdapterManifest,
}
fn digest(value: &impl Serialize) -> Result<String> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value)?)
    ))
}
fn invalid() -> Error {
    err(
        "E_HANDLER_INTEGRITY",
        "compiled handler binding unavailable",
    )
}
impl Engine {
    pub(crate) fn initialize_compiled_handlers(&self) -> Result<()> {
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS compiled_handlers(adapter TEXT PRIMARY KEY REFERENCES dispatch_adapters(id),principal TEXT NOT NULL,registration TEXT NOT NULL,binding_digest TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS handler_preparations(adapter TEXT NOT NULL REFERENCES compiled_handlers(adapter),event_id TEXT NOT NULL,principal TEXT NOT NULL,preparation_id TEXT UNIQUE NOT NULL,body TEXT NOT NULL,body_digest TEXT NOT NULL,PRIMARY KEY(adapter,event_id));")?;
        Ok(())
    }
    pub(crate) fn is_compiled_handler(&self, adapter: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM compiled_handlers WHERE adapter=?1)",
            [adapter],
            |r| r.get(0),
        )?)
    }
    /// Installation is explicit trusted-host authority, never a serialized command.
    pub fn install_compiled_handler(
        &self,
        manifest: &AdapterManifest,
        template: &CompiledHandlerTemplate,
        output: &HandlerOutputBinding,
        authority: &HostContext,
    ) -> Result<()> {
        handler_registration::validate_handler_template(template)
            .map_err(|d| err(&d.code, &d.message))?;
        if !valid_id(&authority.principal)
            || !valid_id(&output.graph_id)
            || !valid_id(&output.branch_id)
            || output.slot != template.output_slot
            || manifest.principal != authority.principal
            || manifest.artifact_digest != template.definition_digest
            || manifest.subscriptions
                != [SubscriptionScope {
                    graph_id: template.input.graph_id.clone(),
                    branch_id: template.input.branch_id.clone(),
                }]
            || manifest.output_graphs != [output.graph_id.clone()]
            || !manifest.effect_destinations.is_empty()
            || !manifest.projection_replay
            || !authority.writable_graphs.contains(&output.graph_id)
            || (output.graph_id == template.input.graph_id
                && output.branch_id == template.input.branch_id)
        {
            return Err(err(
                "E_HANDLER_INSTALL",
                "invalid or unauthorized handler installation",
            ));
        }
        identity_acceptance::require_external_graph(&output.graph_id)?;
        json_size(manifest, 256 * 1024)?;
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let _clock = self.operation_write_scope()?;
        let registration = Registration {
            template: template.clone(),
            output: Output {
                slot: output.slot.clone(),
                graph_id: output.graph_id.clone(),
                branch_id: output.branch_id.clone(),
            },
            manifest: manifest.clone(),
        };
        json_size(&registration, REGISTRATION_LIMIT)?;
        let encoded = serde_json::to_string(&registration)?;
        if self.is_compiled_handler(&manifest.id)? {
            let prior = self.handler_registration(&manifest.id)?;
            if prior != registration {
                return Err(err(
                    "E_ADAPTER_VERSION",
                    "compiled installation is immutable",
                ));
            }
            tx.commit()?;
            return Ok(());
        }
        let exists: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM dispatch_adapters WHERE id=?1)",
            [&manifest.id],
            |r| r.get(0),
        )?;
        if exists {
            return Err(err(
                "E_HANDLER_INSTALL",
                "legacy adapter cannot become compiled",
            ));
        }
        let (count, bytes): (i64, i64) = self.conn.query_row("SELECT COUNT(*),COALESCE(SUM(length(CAST(registration AS BLOB))),0) FROM compiled_handlers WHERE principal=?1", [&authority.principal], |r| Ok((r.get(0)?,r.get(1)?)))?;
        if count >= 128 || bytes.saturating_add(encoded.len() as i64) > 16 * 1024 * 1024 {
            return Err(err("E_BUDGET", "compiled registration quota exceeded"));
        }
        self.install_adapter(manifest, authority)?;
        self.conn.execute(
            "INSERT INTO compiled_handlers VALUES (?1,?2,?3,?4)",
            params![
                manifest.id,
                authority.principal,
                encoded,
                digest(&registration)?
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
    fn handler_registration(&self, adapter: &str) -> Result<Registration> {
        self.read_budget.request()?;
        let limit = self.read_budget.remaining().min(REGISTRATION_LIMIT);
        let (encoded, expected, principal): (Option<String>, String, String) = self.conn.query_row("SELECT CASE WHEN length(CAST(registration AS BLOB))<=?2 THEN registration END,substr(binding_digest,1,72),substr(principal,1,513) FROM compiled_handlers WHERE adapter=?1", params![adapter,limit as i64], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?.ok_or_else(invalid)?;
        let encoded =
            encoded.ok_or_else(|| err("E_BUDGET", "compiled registration exceeds read budget"))?;
        self.read_budget.charge(encoded.len())?;
        let registration: Registration = serde_json::from_str(&encoded)?;
        handler_registration::validate_handler_template(&registration.template)
            .map_err(|_| invalid())?;
        let (manifest, _, _) = self.dispatch_manifest(adapter)?;
        if expected != digest(&registration)?
            || registration.manifest != manifest
            || manifest.id != adapter
            || manifest.principal != principal
            || manifest.artifact_digest != registration.template.definition_digest
        {
            return Err(invalid());
        }
        Ok(registration)
    }
}
