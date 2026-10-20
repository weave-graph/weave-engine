//! B2 part 3 (ADR-0008 sections 5 and 6): persisted results that carry
//! their inputs' restrictions, and release policies.
//!
//! A persisted analysis or ranking becomes attachment facts whose own clause
//! is the clause-wise meet of every restricted input clause (never broader
//! than any input, never resolving membership) and whose influence carriers
//! name every input record, so reads enforce each input exactly and under
//! current authority.
//!
//! A release policy is the only way a derived value is readable by
//! principals who cannot read all of its inputs: a named, versioned,
//! audited declassification held in the reserved graph `weave:release`.
use super::*;
use crate::fact_store::FactBackend;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use weave_contract::provenance::EvidenceRef;

/// The reserved graph that holds release policies and their audit records.
pub const RELEASE_GRAPH: &str = "weave:release";
const RELEASE_BRANCH: &str = "main";
/// Most records one persisted result may write.
pub const MAX_PERSISTED_RECORDS: usize = 10_000;

/// What to persist: values computed from one authorized result.
#[derive(Debug, Clone)]
pub struct PersistRequest<'a> {
    /// The authorized result the values were computed from.
    pub input: &'a QueryResult,
    pub graph_id: &'a str,
    pub branch_id: &'a str,
    /// The attachment key of every persisted record.
    pub key: &'a str,
    /// How the values were computed (ADR-0005 claimed method).
    pub method: weave_contract::provenance::Method,
    pub values: &'a [PersistedValue],
}

/// One value of a persisted result, attached to a node of the analysed graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedValue {
    pub node_id: String,
    pub value: serde_json::Value,
}

/// A declassification (ADR-0008 section 6). Policies are installed by a
/// tenant administrator (by the trusted host in a store without a registry)
/// and never through programs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleasePolicy {
    pub id: String,
    pub revision: String,
    pub kind: ReleaseKind,
    pub source: ReleaseSource,
    /// Who may read a released value: a reader clause like a record's.
    pub audience: ReleaseAudience,
    /// The principal whose authority evaluates the source. It is the
    /// authority being declassified.
    pub evaluator: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReleaseKind {
    /// The number of distinct entities among the source's nodes, released
    /// only when it is at least `min_k`.
    AggregateCount { min_k: u64 },
}

/// What a policy counts: the nodes of one graph's head as the evaluator
/// reads it, optionally only those in one space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseSource {
    pub graph_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseAudience {
    #[serde(default)]
    pub readers: Vec<String>,
    #[serde(default)]
    pub reader_groups: Vec<String>,
}

/// The outcome of one release request. `released` is the value when the
/// policy allows it, and `None` when it is withheld (below `min_k`): the
/// requester learns only that, which is what the policy allows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseOutcome {
    pub policy_id: String,
    pub policy_revision: String,
    pub released: Option<u64>,
    /// The revision of `weave:release` that recorded the audit entry.
    pub audit_revision: String,
}

/// One audit entry: who asked, under which policy revision, when, and
/// whether the value was released.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseAuditEntry {
    pub policy_id: String,
    pub policy_revision: String,
    pub requester: String,
    pub released: bool,
    pub recorded_at_ms: i64,
}

fn unavailable_policy() -> Error {
    err("E_UNAVAILABLE", "release policy unavailable")
}

/// A record ID within the 512-byte bound.
fn bounded_id(prefix: &str, parts: &[&str]) -> String {
    let plain = format!("{prefix}:{}", parts.join(":"));
    if plain.len() <= 512 {
        plain
    } else {
        format!(
            "{prefix}:{:x}",
            Sha256::digest(serde_json::to_vec(parts).unwrap_or_default())
        )
    }
}

impl Engine {
    /// Persist one engine-computed result over `input` (ADR-0008 section 5):
    /// one attachment per value, keyed `key`, on the value's node in `graph`.
    ///
    /// `input` is the authorized result the values were computed from (an
    /// analysis input, a `nearest` ranking). Every node, edge and ranked
    /// embedding it cites must be visible to `host`, else the write fails
    /// with `E_UNAVAILABLE`, as for a missing input. Each written record is
    /// readable only by readers who can read every input: its own clause is
    /// the clause-wise meet of the inputs' clauses, and its carriers name
    /// every input node and edge (or, beyond 1,000 references, every input
    /// revision as a whole-revision gate). A ranking's embeddings are cited
    /// as evidence. Returns the written revision.
    pub fn persist_result(
        &mut self,
        request: &PersistRequest<'_>,
        host: &HostContext,
    ) -> Result<String> {
        let PersistRequest {
            input,
            graph_id: graph,
            branch_id: branch,
            key,
            values,
            ..
        } = *request;
        let method = request.method.clone();
        if values.is_empty() {
            return Err(err("E_PERSIST", "a persisted result needs a value"));
        }
        if values.len() > MAX_PERSISTED_RECORDS {
            return Err(err(
                "E_BUDGET",
                "a persisted result carries at most 10000 values",
            ));
        }
        if !valid_id(key) || key.len() > 128 {
            return Err(err("E_ID", "a persisted key requires 1 to 128 bytes"));
        }
        let (readers, reader_groups, nodes, edges, snapshots, evidence) =
            self.result_restriction(input, host)?;
        let carried = nodes.len().saturating_add(edges.len()) <= 1000;
        let mut changes = Vec::with_capacity(values.len());
        for value in values {
            let mut record = serde_json::json!({
                "kind": "attachment",
                "id": bounded_id("persisted", &[key, &value.node_id]),
                "host": {"kind": "node", "id": value.node_id},
                "key": key,
                "value": {"kind": "literal", "value": value.value},
                "valid_time": {"start": i64::MIN},
                "readers": readers,
                "reader_groups": reader_groups,
                "provenance": {"method": method, "evidence": evidence},
            });
            if carried {
                record["derived_nodes"] = serde_json::to_value(&nodes)?;
                record["derived_from"] = serde_json::to_value(&edges)?;
            } else {
                record["derived_snapshots"] = serde_json::to_value(&snapshots)?;
            }
            let fact: Fact = serde_json::from_value(record)?;
            let k = facts::key(&fact);
            let live = self.fact_live(graph, branch, &k)?;
            // Engine-computed: a later persist may legitimately be readable
            // by more principals when its inputs are.
            changes.push(if live {
                FactChange::Correct {
                    fact,
                    expected: None,
                    widen_readers: true,
                }
            } else {
                FactChange::Assert { fact }
            });
        }
        let program = Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![Command::Write {
                graph_id: graph.into(),
                branch_id: branch.into(),
                expected_head: None,
                changes,
            }],
        };
        match self.execute(&program, host)?.pop() {
            Some(
                CommandResult::Committed { revision, .. } | CommandResult::Unchanged { revision },
            ) => Ok(revision),
            _ => Err(err("E_INTEGRITY", "a persisted write returned no revision")),
        }
    }

    /// The own clause, carriers and evidence of a record derived from all of
    /// `input`.
    #[allow(clippy::type_complexity)]
    fn result_restriction(
        &self,
        input: &QueryResult,
        host: &HostContext,
    ) -> Result<(
        Vec<String>,
        Vec<String>,
        Vec<NodeRef>,
        Vec<AssertionRef>,
        Vec<GraphRef>,
        Vec<EvidenceRef>,
    )> {
        let _transaction = self.optional_read_transaction()?;
        let mut nodes: BTreeMap<(String, String, String), NodeRef> = input
            .node_origins
            .values()
            .flatten()
            .map(|r| {
                (
                    (r.graph_id.clone(), r.revision.clone(), r.node_id.clone()),
                    r.clone(),
                )
            })
            .collect();
        let edges: BTreeMap<(String, String, String), AssertionRef> = input
            .edge_origins
            .values()
            .chain(input.attachment_origins.values())
            .flatten()
            .map(|r| {
                (
                    (
                        r.graph_id.clone(),
                        r.revision.clone(),
                        r.assertion_id.clone(),
                    ),
                    r.clone(),
                )
            })
            .collect();
        let mut snapshots: BTreeMap<(String, String), GraphRef> = input
            .input_snapshots
            .iter()
            .map(|r| ((r.graph_id.clone(), r.revision.clone()), r.clone()))
            .collect();
        let mut evidence = BTreeSet::new();
        // A ranking cites its embeddings and its subjects.
        for neighbor in &input.neighbors {
            let Some(pin) = input.input_snapshots.first() else {
                return Err(unavailable_input());
            };
            evidence.insert(EvidenceRef::Embedding {
                graph_id: pin.graph_id.clone(),
                revision: pin.revision.clone(),
                embedding_id: neighbor.embedding_id.clone(),
            });
            if let embedding::EmbeddingSubject::Node { node_id } = &neighbor.subject {
                nodes.insert(
                    (pin.graph_id.clone(), pin.revision.clone(), node_id.clone()),
                    NodeRef {
                        graph_id: pin.graph_id.clone(),
                        revision: pin.revision.clone(),
                        node_id: node_id.clone(),
                    },
                );
            }
        }
        if nodes.len() + edges.len() > 100_000 || evidence.len() > 1000 || snapshots.len() > 1000 {
            return Err(err("E_BUDGET", "a persisted result cites too many inputs"));
        }
        let mut clauses: Vec<(Vec<String>, Vec<String>)> = vec![];
        let push = |clauses: &mut Vec<(Vec<String>, Vec<String>)>, r: &[String], g: &[String]| {
            if !(r.is_empty() && g.is_empty()) {
                clauses.push((r.to_vec(), g.to_vec()));
            }
        };
        let mut budget = 1000;
        for node in nodes.values() {
            if !self.node_refs_visible(
                std::slice::from_ref(node),
                host,
                &mut HashSet::new(),
                &mut budget,
                0,
            )? {
                return Err(unavailable_input());
            }
            budget = 1000;
            let data = self
                .load_for_reference(
                    &node.graph_id,
                    &node.revision,
                    fact_store::Namespace::Node,
                    &node.node_id,
                )?
                .ok_or_else(unavailable_input)?;
            let record = data
                .nodes
                .iter()
                .find(|n| n.id == node.node_id)
                .ok_or_else(unavailable_input)?;
            push(&mut clauses, &record.readers, &record.reader_groups);
        }
        for edge in edges.values() {
            if !self.premises_visible(
                std::slice::from_ref(edge),
                host,
                &mut HashSet::new(),
                &mut budget,
                0,
            )? {
                return Err(unavailable_input());
            }
            budget = 1000;
            let data = self
                .load_for_reference(
                    &edge.graph_id,
                    &edge.revision,
                    fact_store::Namespace::Record,
                    &edge.assertion_id,
                )?
                .ok_or_else(unavailable_input)?;
            let id = edge.assertion_id.as_str();
            let mut found = false;
            for e in data.edges.iter().filter(|e| e.id == id) {
                push(&mut clauses, &e.readers, &e.reader_groups);
                found = true;
            }
            for a in data.assertions.iter().filter(|a| a.id == id) {
                push(&mut clauses, &a.readers, &a.reader_groups);
                if let Some(s) = data.structural_edges.iter().find(|s| s.id == a.edge_id) {
                    push(&mut clauses, &s.readers, &s.reader_groups);
                }
                found = true;
            }
            for a in data.attachments.iter().filter(|a| a.id == id) {
                push(&mut clauses, &a.readers, &a.reader_groups);
                found = true;
            }
            if !found {
                return Err(unavailable_input());
            }
        }
        // Node and edge clauses are enforced exactly by the carriers, so the
        // own clause keeps only the first of them (ADR-0008 section 2).
        // Embeddings have no carrier, so the own clause must lie within each
        // of their clauses: it is the meet of that first clause and theirs.
        clauses.truncate(1);
        for item in &evidence {
            let EvidenceRef::Embedding {
                graph_id,
                revision,
                embedding_id,
            } = item
            else {
                continue;
            };
            if !self.embedding_ref_visible(graph_id, revision, embedding_id, host)? {
                return Err(unavailable_input());
            }
            let data = self
                .load_for_reference(
                    graph_id,
                    revision,
                    fact_store::Namespace::Record,
                    embedding_id,
                )?
                .ok_or_else(unavailable_input)?;
            let record = data
                .embeddings
                .iter()
                .find(|e| &e.id == embedding_id)
                .ok_or_else(unavailable_input)?;
            push(&mut clauses, &record.readers, &record.reader_groups);
        }
        for snapshot in snapshots.values() {
            if !self.protected_reference_allowed(&snapshot.graph_id, &snapshot.revision, host)? {
                return Err(unavailable_input());
            }
        }
        let (readers, reader_groups) = meet(&clauses);
        snapshots.retain(|_, s| !s.graph_id.starts_with("weave:"));
        Ok((
            readers,
            reader_groups,
            nodes.into_values().collect(),
            edges.into_values().collect(),
            snapshots.into_values().collect(),
            evidence.into_iter().collect(),
        ))
    }

    fn fact_live(&self, graph: &str, branch: &str, key: &facts::Key) -> Result<bool> {
        let Some(head) = self.head(graph, branch)? else {
            return Ok(false);
        };
        if !facts::is_fact_revision(&head) {
            return Ok(false);
        }
        Ok(self.fact_store().get(&head, key.0, &key.1)?.is_some())
    }

    // ------------------------------------------------------------ release

    fn release_records_clause(&self) -> Result<(Vec<String>, Vec<String>)> {
        Ok(if self.registry_head()?.is_some() {
            (vec![], vec![registry::ADMINISTRATORS.to_string()])
        } else {
            (vec![visibility::MATCHES_NO_ONE.to_string()], vec![])
        })
    }

    /// Who may install policies and read the audit: tenant administrators,
    /// or the trusted host in a store without a registry.
    fn release_authority(&self, host: &HostContext) -> Result<()> {
        if self.registry_head()?.is_none() || self.tenant_administrator(host)? {
            Ok(())
        } else {
            Err(err(
                "E_FORBIDDEN",
                "release policies are managed by tenant administrators",
            ))
        }
    }

    fn release_write(&self, changes: Vec<FactChange>, host: &HostContext) -> Result<String> {
        self.conn.execute_batch("SAVEPOINT weave_release")?;
        let result = (|| {
            let _clock = self.operation_write_scope()?;
            let head = self.head(RELEASE_GRAPH, RELEASE_BRANCH)?;
            let (revision, _) =
                self.append_facts(RELEASE_GRAPH, RELEASE_BRANCH, &Some(head), &changes, host)?;
            Ok(revision)
        })();
        match result {
            Ok(revision) => {
                self.conn.execute_batch("RELEASE weave_release")?;
                Ok(revision)
            }
            Err(error) => {
                self.conn
                    .execute_batch("ROLLBACK TO weave_release; RELEASE weave_release")?;
                Err(error)
            }
        }
    }

    /// All live records of `weave:release`, unfiltered (trusted callers only).
    fn release_records(&self) -> Result<Vec<Node>> {
        let Some(head) = self.head(RELEASE_GRAPH, RELEASE_BRANCH)? else {
            return Ok(vec![]);
        };
        Ok(self
            .load(RELEASE_GRAPH, &head)?
            .map(|data| data.nodes)
            .unwrap_or_default())
    }

    fn release_policy(&self, id: &str) -> Result<Option<ReleasePolicy>> {
        let wanted = bounded_id("policy", &[id]);
        for node in self.release_records()? {
            if node.id == wanted {
                let policy =
                    node.properties.get("policy").cloned().ok_or_else(|| {
                        err("E_INTEGRITY", "release policy record lacks its policy")
                    })?;
                return Ok(Some(serde_json::from_value(policy)?));
            }
        }
        Ok(None)
    }

    /// Install or replace a release policy (ADR-0008 section 6). Each
    /// installation is a fact write to `weave:release`, so policy history is
    /// the fact log. Returns the revision.
    pub fn install_release_policy(
        &self,
        policy: &ReleasePolicy,
        host: &HostContext,
    ) -> Result<String> {
        self.release_authority(host)?;
        let ReleaseKind::AggregateCount { min_k } = policy.kind;
        if !valid_id(&policy.id)
            || !valid_id(&policy.revision)
            || !valid_id(&policy.evaluator)
            || !valid_id(&policy.source.graph_id)
            || policy
                .source
                .space_id
                .as_deref()
                .is_some_and(|s| !valid_id(s))
        {
            return Err(err(
                "E_ID",
                "release policy identifiers require 1 to 512 bytes",
            ));
        }
        if min_k == 0 {
            return Err(err("E_RELEASE", "a release threshold must be at least 1"));
        }
        if policy.audience.readers.is_empty() && policy.audience.reader_groups.is_empty() {
            return Err(err("E_RELEASE", "a release audience must name someone"));
        }
        let (readers, reader_groups) = self.release_records_clause()?;
        let record = serde_json::json!({"kind": "node", "id": bounded_id("policy", &[&policy.id]),
            "entity_id": "weave:release-policy", "space_id": "weave:release",
            "properties": {"policy": policy}, "readers": readers, "reader_groups": reader_groups});
        let fact: Fact = serde_json::from_value(record)?;
        let change = if self.release_policy(&policy.id)?.is_some() {
            FactChange::Correct {
                fact,
                expected: None,
                widen_readers: false,
            }
        } else {
            FactChange::Assert { fact }
        };
        self.release_write(vec![change], host)
    }

    /// Evaluate a release policy for `host` (ADR-0008 section 6). A
    /// requester outside the audience, like an unknown policy, gets
    /// `E_UNAVAILABLE`. The count is computed under the evaluator's
    /// authority. Every evaluation, released or withheld, appends an audit
    /// record to `weave:release` in the same transaction.
    pub fn release(&self, policy_id: &str, host: &HostContext) -> Result<ReleaseOutcome> {
        if !valid_id(policy_id) || !valid_id(&host.principal) {
            return Err(err("E_ID", "identifiers must not be empty"));
        }
        let policy = self
            .release_policy(policy_id)?
            .ok_or_else(unavailable_policy)?;
        let access = self.access(host)?;
        if !visibility::record_admits(
            &policy.audience.readers,
            &policy.audience.reader_groups,
            &access,
        ) {
            return Err(unavailable_policy());
        }
        let evaluator = HostContext::new(policy.evaluator.clone(), Vec::<String>::new());
        let ReleaseKind::AggregateCount { min_k } = policy.kind;
        let count = match self.query(
            &QueryPlan {
                graph_id: policy.source.graph_id.clone(),
                revision: None,
                branch_id: "main".into(),
                predicate: None,
                from: None,
                to: None,
                valid_at: None,
                include_metadata: false,
                max_depth: 8,
            },
            &evaluator,
        ) {
            Ok(result) => result
                .graph
                .nodes
                .iter()
                .filter(|n| {
                    policy
                        .source
                        .space_id
                        .as_ref()
                        .is_none_or(|s| s == &n.space_id)
                })
                .map(|n| n.entity_id.clone())
                .collect::<BTreeSet<_>>()
                .len() as u64,
            // An unreadable source counts nothing; the outcome is "withheld".
            Err(_) => 0,
        };
        let released = (count >= min_k).then_some(count);
        let (readers, reader_groups) = self.release_records_clause()?;
        let now = self.trusted_now()?;
        let audit = serde_json::json!({"kind": "node",
            "id": bounded_id("audit", &[&policy.id, &host.principal, &now.to_string(),
                &self.release_records()?.len().to_string()]),
            "entity_id": "weave:release-audit", "space_id": "weave:release",
            "properties": {"policy_id": policy.id, "policy_revision": policy.revision,
                "requester": host.principal, "released": released.is_some(), "recorded_at_ms": now},
            "readers": readers, "reader_groups": reader_groups});
        let audit_revision = self.release_write(
            vec![FactChange::Assert {
                fact: serde_json::from_value(audit)?,
            }],
            host,
        )?;
        Ok(ReleaseOutcome {
            policy_id: policy.id,
            policy_revision: policy.revision,
            released,
            audit_revision,
        })
    }

    /// The release audit, oldest first. Tenant administrators only (the
    /// trusted host in a store without a registry).
    pub fn release_audit(&self, host: &HostContext) -> Result<Vec<ReleaseAuditEntry>> {
        self.release_authority(host)?;
        let mut entries: Vec<ReleaseAuditEntry> = self
            .release_records()?
            .into_iter()
            .filter(|n| n.entity_id == "weave:release-audit")
            .map(|n| serde_json::from_value(serde_json::to_value(&n.properties)?))
            .collect::<std::result::Result<_, serde_json::Error>>()?;
        entries.sort_by(|a, b| {
            (a.recorded_at_ms, &a.requester).cmp(&(b.recorded_at_ms, &b.requester))
        });
        Ok(entries)
    }
}

fn unavailable_input() -> Error {
    err("E_UNAVAILABLE", "persisted input unavailable")
}

/// The clause-wise meet of restricted clauses: principals and groups every
/// clause names. It admits only readers every clause admits and never
/// resolves membership. An empty meet of restricted clauses matches no one;
/// no restricted clause at all is public.
fn meet(clauses: &[(Vec<String>, Vec<String>)]) -> (Vec<String>, Vec<String>) {
    let Some((first_readers, first_groups)) = clauses.first() else {
        return (vec![], vec![]);
    };
    let mut readers = first_readers.clone();
    let mut groups = first_groups.clone();
    for (r, g) in &clauses[1..] {
        readers.retain(|p| r.contains(p));
        groups.retain(|p| g.contains(p));
    }
    readers.sort();
    readers.dedup();
    groups.sort();
    groups.dedup();
    if readers.is_empty() && groups.is_empty() {
        return (vec![visibility::MATCHES_NO_ONE.to_string()], vec![]);
    }
    (readers, groups)
}
