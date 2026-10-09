//! Trusted-host retained cluster computations. No compiler-origin certification.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub const RETAINED_LIMIT: usize = 2 * 1024 * 1024;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RetainedClusterCompletion {
    pub format: String,
    pub runtime_source: String,
    pub adapter: String,
    pub event_id: String,
    pub event: GraphRef,
    pub event_branch: String,
    pub adapter_manifest_digest: String,
    pub recipe: Program,
    pub result: QueryResult,
    pub completion: Program,
    pub closure: Vec<GraphRef>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum HandlerSlotState {
    Pending,
    Completed { request_hash: String },
}
fn invalid() -> Error {
    err("E_RETAINED_CLUSTER", "retained cluster binding unavailable")
}
fn unavailable() -> Error {
    err("E_UNAVAILABLE", "retained cluster input unavailable")
}
fn scoped_navigation(result: &QueryResult) -> bool {
    result.coverage == Coverage::Partial
        && result.diagnostics.len() == 1
        && result.diagnostics[0].code == "I_CLUSTER_SCOPED"
}
fn digest(value: &impl Serialize) -> Result<String> {
    weave_contract::identity::source_fingerprint(&("weave-retained-cluster-manifest-v1", value))
        .map_err(|d| err(&d.code, &d.message))
}
fn selection(program: &Program) -> Result<&ClusterRequest> {
    if ![VERSION, "0.19.0"].contains(&program.version.as_str()) {
        return Err(invalid());
    }
    let expression = match program.commands.as_slice() {
        [Command::Bind { value, .. }] | [Command::Evaluate { value }] => value,
        _ => return Err(invalid()),
    };
    match expression {
        GraphExpression::Cluster { selection } => Ok(selection),
        _ => Err(invalid()),
    }
}
fn restrict(graph: &mut GraphData, principal: &str) {
    for n in &mut graph.nodes {
        n.readers = vec![principal.into()];
    }
    for e in &mut graph.edges {
        e.readers = vec![principal.into()];
    }
    for e in &mut graph.structural_edges {
        e.readers = vec![principal.into()];
    }
    for a in &mut graph.assertions {
        a.readers = vec![principal.into()];
    }
    for a in &mut graph.attachments {
        a.readers = vec![principal.into()];
    }
}
impl Engine {
    /// Actual persisted runtime identity, never a request-selected replica label.
    pub fn runtime_source_identity(&self) -> Result<String> {
        let value: String = self.conn.query_row(
            "SELECT substr(source,1,513) FROM engine_identity WHERE id=1",
            [],
            |r| r.get(0),
        )?;
        if !valid_id(&value) || !value.starts_with("urn:weave:replica:") {
            return Err(invalid());
        }
        Ok(value)
    }
    fn retained_event(
        &self,
        adapter: &str,
        event: &str,
        host: &HostContext,
    ) -> Result<(AdapterManifest, GraphRef, String, i64, i64)> {
        self.require_adapter_host(adapter, host)?;
        self.reject_governed_effect_adapter(adapter)?;
        if self.is_compiled_handler(adapter)? {
            return Err(err(
                "E_HANDLER_BOUND",
                "compiled handlers require prepared completion",
            ));
        }
        let (manifest, state, checkpoint) = self.dispatch_manifest(adapter)?;
        if state != "running" && state != "draining" {
            return Err(err("E_PAUSED", "adapter unavailable"));
        }
        if manifest.output_graphs.len() != 1 || !manifest.effect_destinations.is_empty() {
            return Err(invalid());
        }
        let (graph_id, branch, revision, sequence) = self
            .scoped_event(&manifest, event)?
            .ok_or_else(unavailable)?;
        Ok((
            manifest,
            GraphRef { graph_id, revision },
            branch,
            sequence,
            checkpoint,
        ))
    }
    fn retained_slot(
        &self,
        adapter: &str,
        event: &str,
        sequence: i64,
        checkpoint: i64,
    ) -> Result<HandlerSlotState> {
        self.read_budget.request()?;
        let hash: Option<String> = self.conn.query_row("SELECT substr(request_hash,1,65) FROM handler_receipts WHERE adapter=?1 AND event_id=?2", params![adapter,event], |r| r.get(0)).optional()?;
        if let Some(request_hash) = hash {
            if request_hash.len() != 64 || !request_hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(invalid());
            }
            Ok(HandlerSlotState::Completed { request_hash })
        } else if checkpoint >= sequence {
            Err(invalid())
        } else {
            Ok(HandlerSlotState::Pending)
        }
    }
    /// Owner-only status without receipt payload; a lost host journal must not trigger recomputation.
    pub fn inspect_handler_slot_for(
        &self,
        adapter: &str,
        event: &str,
        host: &HostContext,
    ) -> Result<HandlerSlotState> {
        let tx = self.conn.unchecked_transaction()?;
        let _scope = self.operation_scope()?;
        let (_, _, _, sequence, checkpoint) = self.retained_event(adapter, event, host)?;
        let result = self.retained_slot(adapter, event, sequence, checkpoint)?;
        tx.commit()?;
        Ok(result)
    }
    /// Evaluate once under trusted host configuration. Returned data must be durably journaled before completion.
    pub fn capture_retained_cluster_for(
        &mut self,
        adapter: &str,
        event: &str,
        lease: &str,
        recipe: &Program,
        output_branch: &str,
        host: &HostContext,
    ) -> Result<RetainedClusterCompletion> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _scope = self.operation_write_scope()?;
            json_size(recipe, RETAINED_LIMIT)?;
            let request = selection(recipe)?;
            let (manifest, source, branch, seq, checkpoint) =
                self.retained_event(adapter, event, host)?;
            if self.retained_slot(adapter, event, seq, checkpoint)? != HandlerSlotState::Pending {
                return Err(err(
                    "E_HOST_JOURNAL_MISSING",
                    "completed event requires its retained journal",
                ));
            }
            self.check_lease(adapter, event, lease)?;
            if request.source != source || !valid_id(output_branch) {
                return Err(invalid());
            }
            let results = self.execute(recipe, host)?;
            let [CommandResult::Queried { result }] = results.as_slice() else {
                return Err(invalid());
            };
            // Navigation deliberately reports scoped Partial even for a fully visible
            // local snapshot. Whole-input availability is checked independently below;
            // retaining a cluster must never promote navigation to global completeness.
            if !scoped_navigation(result) {
                return Err(unavailable());
            }
            json_size(result, RETAINED_LIMIT)?;
            let mut result = (**result).clone();
            // Whole selected occurrence consumption is explicit, including an empty input.
            let gate = GraphInfluence {
                snapshots: vec![source.clone()],
                ..GraphInfluence::default()
            };
            result.graph.influence =
                weave_contract::influence::merge(result.graph.influence.as_ref(), Some(&gate))
                    .map_err(|d| err(&d.code, &d.message))?;
            weave_contract::influence::protect_generated_result(&mut result, RETAINED_LIMIT)
                .map_err(|d| err(&d.code, &d.message))?;
            let mut data = result.graph.clone();
            restrict(&mut data, &manifest.principal);
            let mut sources = recipe.source_revisions.clone();
            merge_sources(&mut sources, &result.source_revisions)?;
            let completion = Program {
                version: VERSION.into(),
                source_revisions: sources,
                commands: vec![Command::Commit {
                    graph_id: manifest.output_graphs[0].clone(),
                    branch_id: output_branch.into(),
                    expected_head: self.head(&manifest.output_graphs[0], output_branch)?,
                    data,
                }],
            };
            let closure = self.retained_closure(&source, &result, host)?;
            let captured = RetainedClusterCompletion {
                format: "weave-retained-cluster/1".into(),
                runtime_source: self.runtime_source_identity()?,
                adapter: adapter.into(),
                event_id: event.into(),
                event: source,
                event_branch: branch,
                adapter_manifest_digest: digest(&manifest)?,
                recipe: recipe.clone(),
                result,
                completion,
                closure,
            };
            json_size(&captured, RETAINED_LIMIT)?;
            Ok(captured)
        }));
        self.finish_retained_transaction(outcome)
    }
    fn finish_retained_transaction<T>(&self, outcome: std::thread::Result<Result<T>>) -> Result<T> {
        match operation_clock::rollback_unwind(outcome, &self.conn, "ROLLBACK") {
            Ok(value) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(value)
            }
            Err(error) => {
                self.conn.execute_batch("ROLLBACK")?;
                Err(error)
            }
        }
    }
    fn retained_closure(
        &self,
        source: &GraphRef,
        result: &QueryResult,
        host: &HostContext,
    ) -> Result<Vec<GraphRef>> {
        let mut seen = BTreeSet::new();
        let mut queue = vec![source.clone()];
        seen.insert((source.graph_id.clone(), source.revision.clone()));
        let mut work = 0;
        let enqueue = |g: &str,
                       r: &str,
                       seen: &mut BTreeSet<(String, String)>,
                       queue: &mut Vec<GraphRef>|
         -> Result<()> {
            if !seen.contains(&(g.into(), r.into())) {
                if seen.len() >= 1000 {
                    return Err(err("E_BUDGET", "retained closure limit"));
                }
                seen.insert((g.into(), r.into()));
                queue.push(GraphRef {
                    graph_id: g.into(),
                    revision: r.into(),
                });
            }
            Ok(())
        };
        for data in
            std::iter::once(&result.graph).chain(result.metadata_graphs.iter().map(|g| &g.graph))
        {
            capsule_export::visit_semantic_dependencies(data, &mut work, |g, r| {
                enqueue(g, r, &mut seen, &mut queue)
            })?;
        }
        let mut index = 0;
        while index < queue.len() {
            let reference = &queue[index];
            if !self.protected_reference_allowed(&reference.graph_id, &reference.revision, host)? {
                return Err(unavailable());
            }
            let raw = self
                .load(&reference.graph_id, &reference.revision)?
                .ok_or_else(unavailable)?;
            let (visible, partial) = self.authorized(raw.clone(), host)?;
            if !whole_graph_visible(&raw, visible, partial) {
                return Err(unavailable());
            }
            capsule_export::visit_semantic_dependencies(&raw, &mut work, |g, r| {
                enqueue(g, r, &mut seen, &mut queue)
            })?;
            index += 1;
        }
        self.require_current_result_authority(result, host)?;
        Ok(seen
            .into_iter()
            .map(|(graph_id, revision)| GraphRef { graph_id, revision })
            .collect())
    }
    fn validate_retained(
        &self,
        adapter: &str,
        event: &str,
        captured: &RetainedClusterCompletion,
        host: &HostContext,
    ) -> Result<()> {
        json_size(captured, RETAINED_LIMIT)?;
        let (manifest, source, branch, _, _) = self.retained_event(adapter, event, host)?;
        if captured.format != "weave-retained-cluster/1"
            || captured.runtime_source != self.runtime_source_identity()?
            || captured.adapter != adapter
            || captured.event_id != event
            || captured.event != source
            || captured.event_branch != branch
            || captured.adapter_manifest_digest != digest(&manifest)?
            || selection(&captured.recipe)?.source != source
            || !scoped_navigation(&captured.result)
        {
            return Err(invalid());
        }
        let [Command::Commit {
            graph_id,
            branch_id,
            data,
            ..
        }] = captured.completion.commands.as_slice()
        else {
            return Err(invalid());
        };
        if graph_id != &manifest.output_graphs[0]
            || !valid_id(branch_id)
            || ![VERSION, "0.19.0"].contains(&captured.completion.version.as_str())
        {
            return Err(invalid());
        }
        let mut expected = captured.result.graph.clone();
        restrict(&mut expected, &manifest.principal);
        let mut sources = captured.recipe.source_revisions.clone();
        merge_sources(&mut sources, &captured.result.source_revisions)?;
        if &expected != data
            || sources != captured.completion.source_revisions
            || !captured
                .result
                .graph
                .influence
                .as_ref()
                .is_some_and(|i| i.snapshots.contains(&source))
        {
            return Err(invalid());
        }
        if self.retained_closure(&source, &captured.result, host)? != captured.closure {
            return Err(invalid());
        }
        Ok(())
    }
    /// Revalidate a retained host record without completing or returning its payload.
    pub fn validate_retained_cluster_for(
        &self,
        captured: &RetainedClusterCompletion,
        host: &HostContext,
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let _scope = self.operation_scope()?;
        self.validate_retained(&captured.adapter, &captured.event_id, captured, host)?;
        tx.commit()?;
        Ok(())
    }
    /// Trusted host completion. Validates retained binding/current authority before the existing receipt fast path.
    pub fn complete_retained_cluster_for(
        &mut self,
        adapter: &str,
        event: &str,
        lease: &str,
        captured: &RetainedClusterCompletion,
        host: &HostContext,
    ) -> Result<HandlerReceipt> {
        self.retained_complete_boundary(adapter, event, lease, captured, host, || {})
    }
    #[cfg(feature = "recovery-testing")]
    pub fn complete_retained_cluster_test_before_commit(
        &mut self,
        adapter: &str,
        event: &str,
        lease: &str,
        captured: &RetainedClusterCompletion,
        host: &HostContext,
        before: impl FnOnce(),
    ) -> Result<HandlerReceipt> {
        self.retained_complete_boundary(adapter, event, lease, captured, host, before)
    }
    fn retained_complete_boundary(
        &mut self,
        adapter: &str,
        event: &str,
        lease: &str,
        captured: &RetainedClusterCompletion,
        host: &HostContext,
        before: impl FnOnce(),
    ) -> Result<HandlerReceipt> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _scope = self.operation_write_scope()?;
            self.validate_retained(adapter, event, captured, host)?;
            let receipt =
                self.complete_handler_in_transaction(adapter, event, lease, &captured.completion)?;
            before();
            Ok(receipt)
        }));
        self.finish_retained_transaction(outcome)
    }
}
