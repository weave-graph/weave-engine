//! Genuine local acceptance records. Serialized labels never install governance authority.
use super::*;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

const PREFIX: &str = "weave:governance:decision:";
const SCHEMA_ID: &str = "weave:governance:decision-schema";
const LIMIT: usize = 64 * 1024;
const REGISTRY: &str = "CREATE TABLE IF NOT EXISTS governance_graphs(graph_id TEXT PRIMARY KEY,revision TEXT NOT NULL UNIQUE,decision_id TEXT NOT NULL UNIQUE,view_id TEXT NOT NULL,body TEXT NOT NULL); CREATE TABLE IF NOT EXISTS governance_exposure_decisions(decision_id TEXT PRIMARY KEY);";

/// Native host selection, never an installation or acceptance capability.
#[derive(Clone, Debug)]
pub struct AcceptedViewSelection {
    pub view_id: String,
    /// None captures the current head once; Some requests an immutable historical occurrence.
    pub decision_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    decision: String,
    view: String,
    action: String,
    accepted_at: i64,
    source: GraphRef,
    branch: String,
    publication: String,
    content_digest: String,
}
#[derive(Default)]
pub(crate) struct ProtectedReads(Arc<Mutex<HashSet<(String, String, String)>>>);
struct ProtectedScope(
    Arc<Mutex<HashSet<(String, String, String)>>>,
    (String, String, String),
);
impl Drop for ProtectedScope {
    fn drop(&mut self) {
        if let Ok(mut active) = self.0.lock() {
            active.remove(&self.1);
        }
    }
}
fn unavailable() -> Error {
    err("E_GOV_UNAVAILABLE", "accepted graph unavailable")
}
pub(crate) fn reserved(graph: &str) -> bool {
    graph.starts_with("weave:governance:")
}
pub(crate) fn require_external_graph(graph: &str) -> Result<()> {
    if reserved(graph) {
        Err(err(
            "E_RESERVED_NAMESPACE",
            "governance graph storage is reserved",
        ))
    } else {
        Ok(())
    }
}
pub(crate) fn require_external_schema(data: &GraphData) -> Result<()> {
    // Exact canonical schema can travel with an ordinary materialized copy; a new label
    // or altered descriptor cannot preempt the kernel's immutable registry association.
    if data
        .schema
        .as_ref()
        .is_some_and(|s| s.id.starts_with("weave:governance:") && s != &decision_schema())
    {
        return Err(err(
            "E_RESERVED_NAMESPACE",
            "governance schema descriptor is reserved",
        ));
    }
    Ok(())
}
fn graph_id(decision: &str) -> String {
    format!("{PREFIX}{decision}")
}
fn decision_schema() -> GraphSchema {
    serde_json::from_value(serde_json::json!({
        "id":SCHEMA_ID,"revision":"1",
        "nodes":{"Decision":{"properties":{
            "view":{"value_type":"string","required":true},
            "occurrence":{"value_type":"string","required":true},
            "action":{"value_type":"string","required":true},
            "accepted_at_ms":{"value_type":"integer","required":true}
        }}},
        "edges":{"Accepted":{"from_type":"Decision","to_type":"Decision","properties":{}}}
    }))
    .expect("fixed governance schema")
}
fn decision_data(binding: &Binding) -> GraphData {
    let mut data: GraphData = serde_json::from_value(serde_json::json!({
        "profile":"explicit", "nodes":[{"id":"decision","entity_id":binding.decision,
            "space_id":"weave:governance","type_id":"Decision",
            "context_scope":{"kind":"default"},
            "properties":{"view":binding.view,"occurrence":binding.decision,"action":binding.action,"accepted_at_ms":binding.accepted_at}}],
        "structural_edges":[{"id":"acceptance","from":"decision","to":"decision","predicate":"weave:accepted-view-decision","type_id":"Accepted"}],
        "assertions":[{"id":"accepted","edge_id":"acceptance","source":"weave:local-governance",
            "valid_time":{"start":binding.accepted_at}}],
        "attachments":[{"id":"source","host":{"kind":"assertion","id":"accepted"},"key":"source",
            "value":{"kind":"graph","reference":binding.source},"valid_time":{"start":binding.accepted_at}}]
    })).expect("fixed governance graph");
    data.schema = Some(decision_schema());
    data
}
fn digest(data: &GraphData) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(data)?)))
}

impl Engine {
    pub(crate) fn initialize_governance_graphs(&self) -> Result<()> {
        self.conn.execute_batch(REGISTRY)?;
        Ok(())
    }
    /// Shared gate for every native graph/proof reference, including cached results.
    pub(crate) fn protected_reference_allowed(
        &self,
        graph: &str,
        revision: &str,
        host: &HostContext,
    ) -> Result<bool> {
        let _authorization = self.authorization.enter();
        if !self.identity_reference_allowed(graph, revision, host)? {
            return Ok(false);
        }
        if !reserved(graph) {
            return Ok(true);
        }
        let key = (
            graph.to_owned(),
            revision.to_owned(),
            host.principal.clone(),
        );
        {
            let mut active = self.protected_reads.0.lock().map_err(|_| unavailable())?;
            if active.len() >= 32 || !active.insert(key.clone()) {
                return Ok(false);
            }
        }
        let _scope = ProtectedScope(self.protected_reads.0.clone(), key);
        match self.authorize_governance_record(graph, revision, host) {
            Ok(_) => Ok(true),
            Err(e)
                if matches!(
                    e.code.as_str(),
                    "E_GOV_UNAVAILABLE"
                        | "E_GOV_POLICY"
                        | "E_GOV_SOURCE"
                        | "E_IDENTITY_UNAVAILABLE"
                        | "E_UNAVAILABLE"
                        | "E_DEPENDENCY_UNAVAILABLE"
                ) =>
            {
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }
    fn governance_binding(&self, decision: &str) -> Result<Option<(GraphRef, Binding)>> {
        self.read_budget.request()?;
        let limit = self.read_budget.remaining().min(LIMIT) as i64;
        let row: Option<(String,String,Option<String>)> = self.conn.query_row(
            "SELECT substr(graph_id,1,513),substr(revision,1,513),CASE WHEN length(CAST(body AS BLOB))<=?2 THEN body END FROM governance_graphs WHERE decision_id=?1",
            params![decision,limit], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let Some((graph_id, revision, body)) = row else {
            return Ok(None);
        };
        let body = body.ok_or_else(|| err("E_BUDGET", "governance record bound"))?;
        self.read_budget.charge(body.len())?;
        let binding: Binding = serde_json::from_str(&body).map_err(|_| unavailable())?;
        if binding.decision != decision
            || graph_id != self::graph_id(decision)
            || !valid_id(&revision)
        {
            return Err(unavailable());
        }
        Ok(Some((GraphRef { graph_id, revision }, binding)))
    }
    fn authorize_governance_record(
        &self,
        graph: &str,
        revision: &str,
        host: &HostContext,
    ) -> Result<Binding> {
        let decision = graph.strip_prefix(PREFIX).ok_or_else(unavailable)?;
        let (reference, binding) = self.governance_binding(decision)?.ok_or_else(unavailable)?;
        if reference.graph_id != graph || reference.revision != revision {
            return Err(unavailable());
        }
        let row: Option<(String, i64)> = self
            .conn
            .query_row(
                "SELECT substr(view_id,1,513),accepted_at_ms FROM governance_decisions WHERE id=?1",
                [decision],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if row != Some((binding.view.clone(), binding.accepted_at)) {
            return Err(unavailable());
        }
        let expected = decision_data(&binding);
        let stored = self.load(graph, revision)?.ok_or_else(unavailable)?;
        if digest(&expected)? != binding.content_digest || stored != expected {
            return Err(unavailable());
        }
        let head = self.gov_head(&binding.view)?;
        let policy = self.gov_policy(&binding.view, &head.policy)?;
        if !policy.readers.is_empty()
            && !policy.readers.contains(&host.principal)
            && !policy.proposers.contains(&host.principal)
        {
            return Err(unavailable());
        }
        self.gov_source(&binding.source, &binding.branch, &policy, host)?;
        self.require_fixed_governance_closure(&binding.source, host)?;
        if binding.publication != binding.decision {
            let (publication, original) = self
                .governance_binding(&binding.publication)?
                .ok_or_else(unavailable)?;
            if original.action != "publish"
                || original.view != binding.view
                || original.source != binding.source
                || original.branch != binding.branch
                || !self.protected_reference_allowed(
                    &publication.graph_id,
                    &publication.revision,
                    host,
                )?
            {
                return Err(unavailable());
            }
        }
        Ok(binding)
    }
    /// Entire pinned influence and metadata closure must be readable and have no live handles.
    /// No claim is made that accepting a source body approves future live metadata targets.
    fn require_fixed_governance_closure(&self, root: &GraphRef, host: &HostContext) -> Result<()> {
        let mut pending = vec![root.clone()];
        let mut seen = HashSet::new();
        while let Some(reference) = pending.pop() {
            if !seen.insert((reference.graph_id.clone(), reference.revision.clone())) {
                continue;
            }
            if seen.len() > 1000 {
                return Err(err("E_BUDGET", "governance dependency bound"));
            }
            if !self.protected_reference_allowed(&reference.graph_id, &reference.revision, host)? {
                return Err(unavailable());
            }
            let data = self
                .load(&reference.graph_id, &reference.revision)?
                .ok_or_else(unavailable)?;
            if data
                .attachments
                .iter()
                .any(|a| matches!(a.value, MetadataValue::LiveGraph { .. }))
            {
                return Err(err(
                    "E_GOV_LIVE",
                    "accepted graph exposure requires fixed dependencies",
                ));
            }
            let (visible, incomplete) = self.authorized(data.clone(), host)?;
            if !whole_graph_visible(&data, visible, incomplete) {
                return Err(unavailable());
            }
            let mut dependencies = refs(&data);
            for premises in data
                .edges
                .iter()
                .map(|e| &e.derived_from)
                .chain(data.assertions.iter().map(|a| &a.derived_from))
            {
                dependencies.extend(premises.iter().map(|r| GraphRef {
                    graph_id: r.graph_id.clone(),
                    revision: r.revision.clone(),
                }));
            }
            dependencies.extend(
                data.attachments
                    .iter()
                    .filter_map(|a| a.origin.as_ref())
                    .map(|r| GraphRef {
                        graph_id: r.graph_id.clone(),
                        revision: r.revision.clone(),
                    }),
            );
            if pending.len().saturating_add(dependencies.len()) > 10000 {
                return Err(err("E_BUDGET", "governance dependency queue bound"));
            }
            pending.extend(dependencies);
        }
        Ok(())
    }
    pub(crate) fn governance_decision_current(
        &self,
        decision: &str,
        host: &HostContext,
    ) -> Result<bool> {
        match self.governance_binding(decision)? {
            Some((reference, _)) => {
                self.protected_reference_allowed(&reference.graph_id, &reference.revision, host)
            }
            None => {
                // A missing binding for a newly exposed occurrence is corruption/denial,
                // never an implicit downgrade to the legacy SQL-only profile.
                let required: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM governance_exposure_decisions WHERE decision_id=?1)",[decision],|r|r.get(0))?;
                Ok(!required)
            }
        }
    }
    pub(crate) fn record_governance_graph(
        &self,
        decision: &str,
        view: &str,
        action: &GovernanceAction,
        parent: Option<&str>,
        host: &HostContext,
    ) -> Result<()> {
        let (source, branch, publication, kind) = match action {
            GovernanceAction::Publish { source, branch_id } => (
                source.clone(),
                branch_id.clone(),
                decision.to_owned(),
                "publish",
            ),
            GovernanceAction::ReplacePolicy { .. } => {
                let Some(parent) = parent else {
                    return Ok(());
                };
                let Some((_, binding)) = self.governance_binding(parent)? else {
                    return Ok(());
                };
                (
                    binding.source,
                    binding.branch,
                    binding.publication,
                    "policy_transition",
                )
            }
        };
        self.require_fixed_governance_closure(&source, host)?;
        let mut binding = Binding {
            decision: decision.into(),
            view: view.into(),
            action: kind.into(),
            accepted_at: self.operation_time()?,
            source,
            branch,
            publication,
            content_digest: String::new(),
        };
        let data = decision_data(&binding);
        binding.content_digest = digest(&data)?;
        let graph = graph_id(decision);
        let internal = HostContext::new(&host.principal, [graph.clone()]);
        let (revision, _) = self.commit_storage_inner(&graph, "main", None, &data, &internal)?;
        self.conn.execute(
            "INSERT INTO governance_exposure_decisions VALUES (?1)",
            [decision],
        )?;
        self.conn.execute(
            "INSERT INTO governance_graphs VALUES (?1,?2,?3,?4,?5)",
            params![
                graph,
                revision,
                decision,
                view,
                serde_json::to_string(&binding)?
            ],
        )?;
        Ok(())
    }
    /// Returns real source records with an acceptance influence carrier, including empty values.
    /// No DTO supplied by a caller can install a decision graph or registry binding.
    pub fn query_accepted_view(
        &self,
        selection: &AcceptedViewSelection,
        host: &HostContext,
    ) -> Result<QueryResult> {
        let _transaction = self.optional_read_transaction()?;
        let _clock_scope = self.operation_scope()?;
        if !valid_id(&selection.view_id)
            || !valid_id(&host.principal)
            || selection
                .decision_id
                .as_ref()
                .is_some_and(|id| !valid_id(id))
        {
            return Err(unavailable());
        }
        let decision = match &selection.decision_id {
            Some(id) => id.clone(),
            None => self
                .gov_head(&selection.view_id)
                .map_err(|error| {
                    if error.code == "E_GOV_UNAVAILABLE" {
                        unavailable()
                    } else {
                        error
                    }
                })?
                .decision_id
                .ok_or_else(unavailable)?,
        };
        let (reference, binding) = self
            .governance_binding(&decision)?
            .ok_or_else(unavailable)?;
        if binding.view != selection.view_id
            || !self.protected_reference_allowed(&reference.graph_id, &reference.revision, host)?
        {
            return Err(unavailable());
        }
        let query = QueryPlan {
            graph_id: binding.source.graph_id,
            branch_id: binding.branch,
            revision: Some(binding.source.revision),
            predicate: None,
            from: None,
            to: None,
            valid_at: None,
            include_metadata: false,
            max_depth: 8,
        };
        let mut result = self.query(&query, host)?;
        let mut obligations = vec![reference];
        if binding.publication != decision {
            obligations.push(
                self.governance_binding(&binding.publication)?
                    .ok_or_else(unavailable)?
                    .0,
            );
        }
        for reference in obligations {
            let premise = AssertionRef {
                graph_id: reference.graph_id.clone(),
                revision: reference.revision.clone(),
                assertion_id: "accepted".into(),
            };
            let influence = result
                .graph
                .influence
                .get_or_insert_with(GraphInfluence::default);
            if !influence.assertions.contains(&premise) {
                influence.assertions.push(premise);
            }
            if !result.input_snapshots.contains(&reference) {
                result.input_snapshots.push(reference);
            }
        }
        weave_contract::influence::validate_graph(&result.graph)
            .map_err(|d| err(&d.code, &d.message))?;
        json_size(&result, MATERIALIZED_LIMIT)?;
        Ok(result)
    }
}

/// Authenticated publication details for the native effect bridge. Never inferred from metadata.
pub(crate) struct EffectPublication {
    pub reference: GraphRef,
    pub source: GraphRef,
    pub branch: String,
    pub current: bool,
}
impl Engine {
    pub(crate) fn effect_publication(
        &self,
        view: &str,
        decision: &str,
        host: &HostContext,
    ) -> Result<EffectPublication> {
        let head = self.inspect_governance_head(view, host)?;
        let (reference, binding) = self.governance_binding(decision)?.ok_or_else(unavailable)?;
        if binding.view != view
            || binding.action != "publish"
            || binding.publication != decision
            || !self.protected_reference_allowed(&reference.graph_id, &reference.revision, host)?
        {
            return Err(unavailable());
        }
        let current_decision = head.decision_id.ok_or_else(unavailable)?;
        let (current_ref, current) = self
            .governance_binding(&current_decision)?
            .ok_or_else(unavailable)?;
        if current.view != view
            || !self.protected_reference_allowed(
                &current_ref.graph_id,
                &current_ref.revision,
                host,
            )?
        {
            return Err(unavailable());
        }
        Ok(EffectPublication {
            reference,
            source: binding.source,
            branch: binding.branch,
            current: current.publication == decision,
        })
    }
}
