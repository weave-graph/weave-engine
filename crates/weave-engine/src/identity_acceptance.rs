//! Native trusted-host identity acceptance. Plans cannot install policies or approve mappings.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;
const PREFIX: &str = "weave:identity:";
const LIMIT: usize = 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IdentityPolicy {
    pub reference: IdentityPolicyRef,
    pub proposers: Vec<String>,
    pub approvers: Vec<String>,
    pub readers: Vec<String>,
    pub allowed_spaces: Vec<String>,
    pub max_members: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IdentityCandidate {
    pub id: String,
    pub mapping_id: String,
    pub policy: IdentityPolicyRef,
    pub groups: Vec<Vec<NodeRef>>,
    pub evidence: Vec<AssertionRef>,
    pub valid_time: Interval,
    pub context: Option<GraphRef>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IdentityDecisionRequest {
    pub candidate_id: String,
    pub expected_head: Option<String>,
    pub nonce: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IdentityDecisionReceipt {
    pub mapping_id: String,
    pub reference: GraphRef,
    pub event_id: Option<String>,
    pub changed: bool,
    pub duplicate: bool,
}
pub(crate) fn reserved(graph: &str) -> bool {
    graph.starts_with(PREFIX)
}
pub(crate) fn require_external_graph(graph: &str) -> Result<()> {
    if reserved(graph) {
        Err(err(
            "E_RESERVED_NAMESPACE",
            "identity graph storage is reserved for validated acceptance",
        ))
    } else {
        Ok(())
    }
}
pub(crate) fn require_external_schema(data: &GraphData) -> Result<()> {
    if let Some(schema) = &data.schema {
        if schema.id.starts_with("weave:identity") && schema != &membership_schema() {
            return Err(err(
                "E_RESERVED_NAMESPACE",
                "identity schema descriptor is reserved",
            ));
        }
    }
    Ok(())
}
fn key(value: &impl Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
fn graph_id(mapping: &str) -> Result<String> {
    Ok(format!("{PREFIX}{}", key(&mapping)?))
}
fn valid_strings(values: &[String]) -> bool {
    values.len() <= 128
        && values.iter().all(|s| valid_id(s))
        && values.iter().collect::<BTreeSet<_>>().len() == values.len()
}
fn reference_ok(r: &IdentityPolicyRef) -> bool {
    valid_id(&r.id) && valid_id(&r.revision)
}
fn source_ref(n: &Node) -> Result<NodeRef> {
    let field = |key: &str| {
        n.properties
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .ok_or_else(|| err("E_INTEGRITY", "invalid stored identity member"))
    };
    Ok(NodeRef {
        graph_id: field("source_graph")?,
        revision: field("source_revision")?,
        node_id: field("source_node")?,
    })
}
impl Engine {
    fn commit_identity_inner(
        &self,
        graph: &str,
        expected: Option<&str>,
        data: &GraphData,
        host: &HostContext,
    ) -> Result<(String, Option<String>)> {
        if !reserved(graph) {
            return Err(err(
                "E_RESERVED_NAMESPACE",
                "internal identity commit requires reserved graph",
            ));
        }
        self.commit_storage_inner(graph, "main", expected, data, host)
    }
    pub(crate) fn initialize_identity(&self) -> Result<()> {
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS identity_policies(id TEXT NOT NULL,revision TEXT NOT NULL,body TEXT NOT NULL,revoked INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(id,revision));
CREATE TABLE IF NOT EXISTS identity_candidates(id TEXT PRIMARY KEY,mapping_id TEXT NOT NULL,policy_id TEXT NOT NULL,policy_revision TEXT NOT NULL,body TEXT NOT NULL,proposer TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS identity_decisions(mapping_id TEXT NOT NULL,revision TEXT NOT NULL,graph_id TEXT NOT NULL,policy_id TEXT NOT NULL,policy_revision TEXT NOT NULL,candidate_id TEXT NOT NULL,actor TEXT NOT NULL,PRIMARY KEY(mapping_id,revision));
CREATE TABLE IF NOT EXISTS identity_memberships(mapping_id TEXT NOT NULL,revision TEXT NOT NULL,member_id TEXT NOT NULL,partition_id INTEGER NOT NULL,PRIMARY KEY(mapping_id,revision,member_id));
CREATE TABLE IF NOT EXISTS identity_mapping_heads(mapping_id TEXT PRIMARY KEY,revision TEXT NOT NULL,policy_id TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS identity_receipts(actor TEXT NOT NULL,nonce TEXT NOT NULL,body_hash TEXT NOT NULL,receipt TEXT NOT NULL,PRIMARY KEY(actor,nonce));")?;
        Ok(())
    }
    /// Trusted administrator API; deliberately absent from graph commands and signed proposal ingress.
    pub fn install_identity_policy(&self, policy: &IdentityPolicy) -> Result<()> {
        if !reference_ok(&policy.reference)
            || policy.proposers.is_empty()
            || policy.approvers.is_empty()
            || policy.allowed_spaces.is_empty()
            || policy.max_members == 0
            || policy.max_members > 128
            || [
                &policy.proposers,
                &policy.approvers,
                &policy.readers,
                &policy.allowed_spaces,
            ]
            .iter()
            .any(|v| !valid_strings(v))
        {
            return Err(err("E_IDENTITY_POLICY", "invalid bounded identity policy"));
        }
        json_size(policy, LIMIT)?;
        let body = serde_json::to_string(policy)?;
        let prior: Option<bool> = self
            .conn
            .query_row(
                "SELECT body=?3 FROM identity_policies WHERE id=?1 AND revision=?2",
                params![policy.reference.id, policy.reference.revision, body],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(prior) = prior {
            if !prior {
                return Err(err(
                    "E_POLICY_REVISION",
                    "identity policy revision is immutable",
                ));
            }
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO identity_policies(id,revision,body) VALUES (?1,?2,?3)",
            params![policy.reference.id, policy.reference.revision, body],
        )?;
        Ok(())
    }
    /// Revocation is irreversible for this exact policy revision, including historical resolution.
    pub fn revoke_identity_policy(&self, reference: &IdentityPolicyRef) -> Result<()> {
        if self.conn.execute(
            "UPDATE identity_policies SET revoked=1 WHERE id=?1 AND revision=?2",
            params![reference.id, reference.revision],
        )? != 1
        {
            return Err(err("E_IDENTITY_UNAVAILABLE", "identity policy unavailable"));
        }
        Ok(())
    }
    fn identity_policy(&self, reference: &IdentityPolicyRef) -> Result<IdentityPolicy> {
        self.read_budget.request()?;
        let limit = self.read_budget.remaining().min(LIMIT) as i64;
        let row:Option<(Option<String>,bool)>=self.conn.query_row("SELECT CASE WHEN length(CAST(body AS BLOB))<=?3 THEN body END,revoked FROM identity_policies WHERE id=?1 AND revision=?2",params![reference.id,reference.revision,limit],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let body = match row {
            Some((None, _)) => return Err(err("E_BUDGET", "identity policy read exceeds budget")),
            Some((Some(body), false)) => body,
            _ => return Err(err("E_IDENTITY_UNAVAILABLE", "identity policy unavailable")),
        };
        self.read_budget.charge(body.len())?;
        Ok(serde_json::from_str(&body)?)
    }
    fn identity_candidate(&self, id: &str) -> Result<IdentityCandidate> {
        self.read_budget.request()?;
        let limit = self.read_budget.remaining().min(LIMIT) as i64;
        let body:Option<Option<String>>=self.conn.query_row("SELECT CASE WHEN length(CAST(body AS BLOB))<=?2 THEN body END FROM identity_candidates WHERE id=?1",params![id,limit],|r|r.get(0)).optional()?;
        let body = match body {
            Some(None) => return Err(err("E_BUDGET", "identity candidate read exceeds budget")),
            Some(Some(body)) => body,
            None => {
                return Err(err(
                    "E_IDENTITY_UNAVAILABLE",
                    "identity candidate unavailable",
                ))
            }
        };
        self.read_budget.charge(body.len())?;
        Ok(serde_json::from_str(&body)?)
    }
    fn identity_sources(
        &self,
        candidate: &IdentityCandidate,
        policy: &IdentityPolicy,
        host: &HostContext,
    ) -> Result<Vec<(usize, NodeRef, Node)>> {
        if !valid_id(&candidate.id)
            || !valid_id(&candidate.mapping_id)
            || !reference_ok(&candidate.policy)
            || candidate.groups.is_empty()
            || candidate.groups.len() > 128
            || candidate.groups.iter().any(Vec::is_empty)
            || candidate.groups.iter().map(Vec::len).sum::<usize>() > policy.max_members
            || candidate.evidence.len() > 128
            || candidate
                .valid_time
                .end
                .is_some_and(|end| end <= candidate.valid_time.start)
        {
            return Err(err(
                "E_IDENTITY_CANDIDATE",
                "invalid bounded membership partition",
            ));
        }
        json_size(candidate, LIMIT)?;
        if candidate
            .context
            .as_ref()
            .is_some_and(|r| !valid_id(&r.graph_id) || !valid_id(&r.revision))
        {
            return Err(err("E_REFERENCE", "invalid identity context pin"));
        }
        let selection = candidate
            .context
            .clone()
            .map(|reference| ContextSelection::Pinned { reference })
            .unwrap_or_default();
        let mut seen = BTreeSet::new();
        let mut sources = Vec::new();
        for (group, members) in candidate.groups.iter().enumerate() {
            for reference in members {
                if !valid_id(&reference.graph_id)
                    || !valid_id(&reference.revision)
                    || !valid_id(&reference.node_id)
                    || reserved(&reference.graph_id)
                    || !seen.insert((reference.graph_id.clone(), reference.node_id.clone()))
                {
                    return Err(err(
                        "E_IDENTITY_CANDIDATE",
                        "source membership must be unique and independently pinned",
                    ));
                }
                let data = self
                    .load(&reference.graph_id, &reference.revision)?
                    .ok_or_else(|| err("E_IDENTITY_UNAVAILABLE", "identity source unavailable"))?;
                let (data, _) = self.authorized(data, host)?;
                if let Some(selected) = data
                    .context_typing
                    .as_ref()
                    .and_then(|t| t.selected.as_ref())
                {
                    context::compatible_context(
                        Some(&selection),
                        Some(&ContextSelection::Pinned {
                            reference: selected.clone(),
                        }),
                    )
                    .map_err(|d| err(&d.code, &d.message))?;
                }
                let node = data
                    .nodes
                    .into_iter()
                    .find(|n| n.id == reference.node_id)
                    .ok_or_else(|| err("E_IDENTITY_UNAVAILABLE", "identity source unavailable"))?;
                if !policy.allowed_spaces.contains(&node.space_id) {
                    return Err(err(
                        "E_IDENTITY_SPACE",
                        "source space is outside installed identity policy",
                    ));
                }
                if let Some(scope) = &node.context_scope {
                    context::compatible_context(Some(&selection), Some(scope))
                        .map_err(|d| err(&d.code, &d.message))?;
                }
                sources.push((group, reference.clone(), node));
            }
        }
        if candidate
            .evidence
            .iter()
            .any(|p| !valid_id(&p.graph_id) || !valid_id(&p.revision) || !valid_id(&p.assertion_id))
            || !self.premises_visible(
                &candidate.evidence,
                host,
                &mut HashSet::new(),
                &mut 1000,
                0,
            )?
        {
            return Err(err(
                "E_IDENTITY_UNAVAILABLE",
                "identity evidence unavailable",
            ));
        }
        for reference in &candidate.evidence {
            let data = self
                .load(&reference.graph_id, &reference.revision)?
                .ok_or_else(|| err("E_IDENTITY_UNAVAILABLE", "identity evidence unavailable"))?;
            let (data, _) = self.authorized(data, host)?;
            let (data, _) = materialize(data, &reference.graph_id, &reference.revision)?;
            let edge = data
                .edges
                .iter()
                .find(|e| e.id == reference.assertion_id)
                .ok_or_else(|| err("E_IDENTITY_UNAVAILABLE", "identity evidence unavailable"))?;
            context::ensure_consumable(Some(&selection), edge.assertion_context.as_ref())
                .map_err(|d| err(&d.code, &d.message))?;
            if edge.polarity != Polarity::Positive
                || intersect(&candidate.valid_time, &edge.valid_time).as_ref()
                    != Some(&candidate.valid_time)
            {
                return Err(err(
                    "E_IDENTITY_EVIDENCE",
                    "identity support must be positive and cover the candidate interval",
                ));
            }
        }
        Ok(sources)
    }
    /// Stores a proposal only. It cannot create a graph head, accepted mapping or event.
    pub fn submit_identity_candidate(
        &mut self,
        candidate: &IdentityCandidate,
        host: &HostContext,
    ) -> Result<bool> {
        let _scope = self.read_budget.enter();
        let policy = self.identity_policy(&candidate.policy)?;
        if !policy.proposers.contains(&host.principal) {
            return Err(err(
                "E_FORBIDDEN",
                "principal cannot propose under this identity policy",
            ));
        }
        self.identity_sources(candidate, &policy, host)?;
        let body = serde_json::to_string(candidate)?;
        let prior: Option<bool> = self
            .conn
            .query_row(
                "SELECT body=?2 FROM identity_candidates WHERE id=?1",
                params![candidate.id, body],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(prior) = prior {
            if !prior {
                return Err(err(
                    "E_IDENTITY_CANDIDATE",
                    "candidate ID already has another immutable body",
                ));
            }
            return Ok(false);
        }
        let (count,bytes):(i64,i64)=self.conn.query_row("SELECT COUNT(*),COALESCE(SUM(length(CAST(body AS BLOB))),0) FROM identity_candidates WHERE proposer=?1",[&host.principal],|r|Ok((r.get(0)?,r.get(1)?)))?;
        if count >= 1000 || bytes < 0 || bytes as usize + body.len() > 64 * 1024 * 1024 {
            return Err(err("E_BUDGET", "identity proposal capacity reached"));
        }
        self.conn.execute(
            "INSERT INTO identity_candidates VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                candidate.id,
                candidate.mapping_id,
                candidate.policy.id,
                candidate.policy.revision,
                body,
                host.principal
            ],
        )?;
        Ok(true)
    }
    /// Trusted host administration only; this head lookup is not a discoverability endpoint.
    pub fn identity_head(&self, mapping_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT revision FROM identity_mapping_heads WHERE mapping_id=?1",
                [mapping_id],
                |r| r.get(0),
            )
            .optional()?)
    }
    /// Approves one explicit replacement partition; split/supersession is a new accepted revision.
    pub fn accept_identity_candidate(
        &mut self,
        request: &IdentityDecisionRequest,
        host: &HostContext,
    ) -> Result<IdentityDecisionReceipt> {
        self.accept_identity_boundary(request, host, || {})
    }
    /// Test-only crash observer after all acceptance SQL and before the outer savepoint commits.
    #[cfg(feature = "recovery-testing")]
    pub fn accept_identity_test_before_commit(
        &mut self,
        request: &IdentityDecisionRequest,
        host: &HostContext,
        before_commit: impl FnOnce(),
    ) -> Result<IdentityDecisionReceipt> {
        self.accept_identity_boundary(request, host, before_commit)
    }
    fn accept_identity_boundary(
        &mut self,
        request: &IdentityDecisionRequest,
        host: &HostContext,
        before_commit: impl FnOnce(),
    ) -> Result<IdentityDecisionReceipt> {
        let _scope = self.read_budget.enter();
        if !valid_id(&request.candidate_id)
            || !valid_id(&request.nonce)
            || !valid_id(&host.principal)
            || request.expected_head.as_ref().is_some_and(|r| !valid_id(r))
        {
            return Err(err("E_ID", "invalid identity decision request"));
        }
        self.conn.execute_batch("SAVEPOINT identity_acceptance")?;
        let result = (|| {
            let candidate = self.identity_candidate(&request.candidate_id)?;
            let policy = self.identity_policy(&candidate.policy)?;
            if !policy.approvers.contains(&host.principal) {
                return Err(err(
                    "E_FORBIDDEN",
                    "principal cannot approve this identity policy",
                ));
            }
            let sources = self.identity_sources(&candidate, &policy, host)?;
            let body_hash = key(request)?;
            self.read_budget.request()?;
            let limit = self.read_budget.remaining().min(LIMIT) as i64;
            let prior: Option<(String, Option<String>)> = self
                .conn
                .query_row(
                    "SELECT substr(body_hash,1,65), CASE WHEN length(CAST(receipt AS BLOB))<=?3 THEN receipt END FROM identity_receipts WHERE actor=?1 AND nonce=?2",
                    params![host.principal, request.nonce, limit],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((old, receipt)) = prior {
                if old != body_hash {
                    return Err(err(
                        "E_REPLAY",
                        "identity decision nonce has a different body",
                    ));
                }
                let receipt = receipt
                    .ok_or_else(|| err("E_BUDGET", "identity receipt read exceeds budget"))?;
                self.read_budget.charge(receipt.len())?;
                let mut receipt: IdentityDecisionReceipt = serde_json::from_str(&receipt)?;
                receipt.duplicate = true;
                return Ok(receipt);
            }
            let count: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM identity_receipts WHERE actor=?1",
                [&host.principal],
                |r| r.get(0),
            )?;
            if count >= 10000 {
                return Err(err(
                    "E_BUDGET",
                    "identity decision receipt capacity reached",
                ));
            }
            let head: Option<(String, String)> = self
                .conn
                .query_row(
                    "SELECT revision,policy_id FROM identity_mapping_heads WHERE mapping_id=?1",
                    [&candidate.mapping_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if head.as_ref().map(|(revision, _)| revision) != request.expected_head.as_ref() {
                return Err(err("E_CONFLICT", "identity expected head differs"));
            }
            if head
                .as_ref()
                .is_some_and(|(_, policy)| policy != &candidate.policy.id)
            {
                return Err(err(
                    "E_IDENTITY_POLICY",
                    "mapping cannot change policy authority",
                ));
            }
            let graph = graph_id(&candidate.mapping_id)?;
            if self.head(&graph, "main")? != request.expected_head {
                return Err(err(
                    "E_INTEGRITY",
                    "identity registry and graph head disagree",
                ));
            }
            let data = membership_graph(&candidate, &policy, &sources, &host.principal)?;
            let internal = HostContext::new(&host.principal, [graph.clone()]);
            let (revision, event_id) = self.commit_identity_inner(
                &graph,
                request.expected_head.as_deref(),
                &data,
                &internal,
            )?;
            let receipt = IdentityDecisionReceipt {
                mapping_id: candidate.mapping_id.clone(),
                reference: GraphRef {
                    graph_id: graph.clone(),
                    revision: revision.clone(),
                },
                changed: event_id.is_some(),
                event_id,
                duplicate: false,
            };
            self.conn.execute(
                "INSERT OR IGNORE INTO identity_decisions VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    candidate.mapping_id,
                    revision,
                    graph,
                    candidate.policy.id,
                    candidate.policy.revision,
                    candidate.id,
                    host.principal
                ],
            )?;
            for (partition, reference, _) in &sources {
                let member_id = format!("member:{}", key(reference)?);
                self.conn.execute(
                    "INSERT OR IGNORE INTO identity_memberships VALUES (?1,?2,?3,?4)",
                    params![candidate.mapping_id, revision, member_id, *partition as i64],
                )?;
            }
            self.conn.execute("INSERT INTO identity_mapping_heads VALUES (?1,?2,?3) ON CONFLICT(mapping_id) DO UPDATE SET revision=excluded.revision",params![candidate.mapping_id,revision,candidate.policy.id])?;
            self.conn.execute(
                "INSERT INTO identity_receipts VALUES (?1,?2,?3,?4)",
                params![
                    host.principal,
                    request.nonce,
                    body_hash,
                    serde_json::to_string(&receipt)?
                ],
            )?;
            Ok(receipt)
        })();
        match result {
            Ok(value) => {
                before_commit();
                self.conn.execute_batch("RELEASE identity_acceptance")?;
                Ok(value)
            }
            Err(error) => {
                self.conn.execute_batch(
                    "ROLLBACK TO identity_acceptance; RELEASE identity_acceptance",
                )?;
                Err(error)
            }
        }
    }
    /// Resolves an explicitly accepted partition under current policy and source visibility.
    /// This native host API is not a remote acceptance capability or a global directory.
    pub fn resolve_identity(
        &self,
        request: &IdentityResolve,
        host: &HostContext,
    ) -> Result<QueryResult> {
        let _scope = self.read_budget.enter();
        if !valid_id(&request.mapping_id)
            || !valid_id(&request.revision)
            || !valid_id(&request.target_space)
            || !valid_id(&request.source.graph_id)
            || !valid_id(&request.source.revision)
            || !valid_id(&request.source.node_id)
            || !reference_ok(&request.policy)
        {
            return Err(err("E_ID", "invalid identity resolution selection"));
        }
        context::validate_selection(&request.context).map_err(|d| err(&d.code, &d.message))?;
        let graph = graph_id(&request.mapping_id)?;
        let policy_ref:Option<IdentityPolicyRef>=self.conn.query_row("SELECT policy_id,policy_revision FROM identity_decisions WHERE mapping_id=?1 AND revision=?2 AND graph_id=?3",params![request.mapping_id,request.revision,graph],|r|Ok(IdentityPolicyRef{id:r.get(0)?,revision:r.get(1)?})).optional()?;
        if policy_ref.as_ref() != Some(&request.policy)
            || !self.identity_reference_allowed(&graph, &request.revision, host)?
        {
            return Err(err(
                "E_IDENTITY_UNAVAILABLE",
                "accepted mapping unavailable",
            ));
        }
        let mut value=self.query(&serde_json::from_value(json!({"graph_id":graph,"revision":request.revision,"predicate":"weave:identity-member","valid_at":request.valid_at}))?,host)?;
        value = context::select(value, &request.context, &algebra_context(host))
            .map_err(|d| err(&d.code, &d.message))?;
        let nodes: BTreeMap<_, _> = value
            .graph
            .nodes
            .iter()
            .map(|n| (n.id.clone(), n.clone()))
            .collect();
        let source = value
            .graph
            .nodes
            .iter()
            .find(|n| source_ref(n).is_ok_and(|r| r == request.source))
            .cloned();
        let mut output_nodes = BTreeMap::new();
        let mut output_edges = Vec::new();
        let mut origins = BTreeMap::new();
        let mut node_origins = BTreeMap::new();
        let mut provenance = Vec::new();
        let mut bytes = 0;
        if let Some(source) = source {
            let source_edge = value
                .graph
                .edges
                .iter()
                .find(|e| e.from == source.id)
                .ok_or_else(|| err("E_INTEGRITY", "accepted membership claim missing"))?;
            let partition:i64=self.conn.query_row("SELECT partition_id FROM identity_memberships WHERE mapping_id=?1 AND revision=?2 AND member_id=?3",params![request.mapping_id,request.revision,source.id],|r|r.get(0))?;
            for target_edge in &value.graph.edges {
                let Some(target) = nodes.get(&target_edge.from) else {
                    return Err(err("E_INTEGRITY", "accepted member missing"));
                };
                if target.id == source.id
                    || target.space_id != request.target_space
                    || target.space_id == source.space_id
                {
                    continue;
                }
                let target_partition:i64=self.conn.query_row("SELECT partition_id FROM identity_memberships WHERE mapping_id=?1 AND revision=?2 AND member_id=?3",params![request.mapping_id,request.revision,target.id],|r|r.get(0))?;
                if target_partition != partition {
                    continue;
                }
                let target_ref = source_ref(target)?;
                let references = vec![
                    AssertionRef {
                        graph_id: graph.clone(),
                        revision: request.revision.clone(),
                        assertion_id: source_edge.id.clone(),
                    },
                    AssertionRef {
                        graph_id: graph.clone(),
                        revision: request.revision.clone(),
                        assertion_id: target_edge.id.clone(),
                    },
                ];
                let window = intersect(&source_edge.valid_time, &target_edge.valid_time)
                    .ok_or_else(|| {
                        err("E_IDENTITY_UNAVAILABLE", "accepted interval unavailable")
                    })?;
                let id = format!(
                    "counterpart:{}",
                    key(&(
                        &request.mapping_id,
                        &source.id,
                        &target.id,
                        &request.context
                    ))?
                );
                let mut from = source.clone();
                let mut to = target.clone();
                from.id = format!(
                    "resolved-member:{}",
                    key(&(
                        &request.mapping_id,
                        &source.id,
                        &source.id,
                        &request.target_space,
                        &request.context
                    ))?
                );
                to.id = format!(
                    "resolved-member:{}",
                    key(&(
                        &request.mapping_id,
                        &source.id,
                        &target.id,
                        &request.target_space,
                        &request.context
                    ))?
                );
                from.readers = vec![host.principal.clone()];
                to.readers = from.readers.clone();
                for node in [&mut from, &mut to] {
                    for reference in &references {
                        if !node.derived_from.contains(reference) {
                            node.derived_from.push(reference.clone());
                        }
                    }
                }
                let edge: Edge = serde_json::from_value(
                    json!({"id":id,"type_id":"Counterpart","predicate":"weave:accepted-counterpart","from":from.id,"to":to.id,"valid_time":window,"assertion_context":request.context.reference(),"derived_from":references,"derivations":[{"operator":"weave:accepted-identity:v1","premises":references,"parameters":{"policy":request.policy,"mapping":request.mapping_id,"mapping_revision":request.revision,"valid_at":request.valid_at,"context":request.context},"input_snapshots":[{"graph_id":graph,"revision":request.revision}]}],"readers":[host.principal]}),
                )?;
                bytes += json_size(
                    &(&from, &to, &edge),
                    MATERIALIZED_LIMIT.saturating_sub(bytes),
                )?;
                node_origins.insert(from.id.clone(), vec![]);
                node_origins.insert(to.id.clone(), vec![]);
                if let Some(previous) = output_nodes.get(&from.id) {
                    let previous: &Node = previous;
                    for reference in &previous.derived_from {
                        if !from.derived_from.contains(reference) {
                            from.derived_from.push(reference.clone());
                        }
                    }
                }
                for source in [&request.source, &target_ref] {
                    let pin = GraphRef {
                        graph_id: source.graph_id.clone(),
                        revision: source.revision.clone(),
                    };
                    if !value.input_snapshots.contains(&pin) {
                        value.input_snapshots.push(pin.clone());
                    }
                    if let Some(source_data) = self.load(&pin.graph_id, &pin.revision)? {
                        let (source_data, _) = self.authorized(source_data, host)?;
                        value.graph.influence = weave_contract::influence::merge(
                            value.graph.influence.as_ref(),
                            source_data.influence.as_ref(),
                        )
                        .map_err(|d| err(&d.code, &d.message))?;
                        if let Some(typing) = source_data.context_typing.as_ref() {
                            if let Some(selected) = &typing.selected {
                                context::compatible_context(
                                    Some(&request.context),
                                    Some(&ContextSelection::Pinned {
                                        reference: selected.clone(),
                                    }),
                                )
                                .map_err(|d| err(&d.code, &d.message))?;
                            }
                            value.graph.context_typing = context_typing::merge(
                                value.graph.context_typing.as_ref(),
                                Some(typing),
                            )
                            .map_err(|d| err(&d.code, &d.message))?;
                        }
                    }
                    value.snapshots.entry(pin.graph_id).or_insert(pin.revision);
                }
                output_nodes.insert(from.id.clone(), from);
                output_nodes.insert(to.id.clone(), to);
                origins.insert(id, references.clone());
                for reference in references {
                    if !provenance.contains(&reference) {
                        provenance.push(reference);
                    }
                }
                output_edges.push(edge);
            }
        }
        value.graph.nodes = output_nodes.into_values().collect();
        value.graph.edges = output_edges;
        value.graph.attachments.clear();
        value.metadata_graphs.clear();
        value.edge_origins = origins;
        value.node_origins = node_origins;
        value.attachment_origins.clear();
        value.provenance = provenance;
        context_typing::protect_result_generated(&mut value)
            .map_err(|d| err(&d.code, &d.message))?;
        weave_contract::influence::protect_generated_result(&mut value, MATERIALIZED_LIMIT)
            .map_err(|d| err(&d.code, &d.message))?;
        validate_graph(&value.graph)?;
        json_size(&value, MATERIALIZED_LIMIT)?;
        Ok(value)
    }
    /// Current policy authorization for reserved graph dependencies, including pinned history.
    pub(crate) fn identity_reference_allowed(
        &self,
        graph: &str,
        revision: &str,
        host: &HostContext,
    ) -> Result<bool> {
        if !reserved(graph) {
            return Ok(true);
        }
        let reference:Option<IdentityPolicyRef>=self.conn.query_row("SELECT policy_id,policy_revision FROM identity_decisions WHERE graph_id=?1 AND revision=?2",params![graph,revision],|r|Ok(IdentityPolicyRef{id:r.get(0)?,revision:r.get(1)?})).optional()?;
        let Some(reference) = reference else {
            return Ok(false);
        };
        match self.identity_policy(&reference) {
            Ok(policy) => Ok(allowed(&policy.readers, &host.principal)),
            Err(error) if error.code == "E_IDENTITY_UNAVAILABLE" => Ok(false),
            Err(error) => Err(error),
        }
    }
}
fn membership_graph(
    candidate: &IdentityCandidate,
    _policy: &IdentityPolicy,
    sources: &[(usize, NodeRef, Node)],
    actor: &str,
) -> Result<GraphData> {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for (_, reference, source) in sources {
        let id = format!("member:{}", key(reference)?);
        nodes.push(json!({"id":id,"entity_id":source.entity_id,"space_id":source.space_id,"type_id":"Member","context_scope":candidate.context.as_ref().map(|reference|ContextSelection::Pinned{reference:reference.clone()}).unwrap_or_default(),"properties":{"source_graph":reference.graph_id,"source_revision":reference.revision,"source_node":reference.node_id},"derived_nodes":[reference],"derived_from":candidate.evidence,"readers":source.readers}));
        edges.push(json!({"id":format!("membership:{}",key(reference)?),"type_id":"Membership","predicate":"weave:identity-member","from":id,"to":id,"valid_time":candidate.valid_time,"assertion_context":candidate.context,"assertion_source":actor,"assertion_properties":{"candidate":candidate.id,"policy":candidate.policy},"derived_from":candidate.evidence,"readers":source.readers}));
    }
    Ok(serde_json::from_value(
        json!({"schema":membership_schema(),"nodes":nodes,"edges":edges}),
    )?)
}
fn membership_schema() -> GraphSchema {
    serde_json::from_value(json!({"id":"weave:identity-membership","revision":"1","nodes":{"Member":{"properties":{"source_graph":{"value_type":"string","required":true},"source_revision":{"value_type":"string","required":true},"source_node":{"value_type":"string","required":true}}}},"edges":{"Membership":{"from_type":"Member","to_type":"Member"},"Counterpart":{"from_type":"Member","to_type":"Member","allow_cross_space":true}}})).expect("static identity schema")
}

fn intersect(a: &Interval, b: &Interval) -> Option<Interval> {
    let start = a.start.max(b.start);
    let end = match (a.end, b.end) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, None) => x,
        (None, y) => y,
    };
    if end.is_some_and(|end| start >= end) {
        None
    } else {
        Some(Interval { start, end })
    }
}
