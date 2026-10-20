//! The principal registry (B1, ADR-0007): users and groups, group
//! membership, graph ACLs, protected branches and the Discover permission.
//!
//! The registry is stored as ordinary facts in the reserved graph
//! `weave:registry` (branch `main`), written only through
//! [`Engine::apply_registry`]. Its history, audit and as-of reconstruction are
//! the A2 fact log and A3 as-of queries. A store without a registry keeps the
//! local trusted-host profile exactly: `HostContext.principal` matches literal
//! `readers`, and `HostContext.writable_graphs` grants writes.
//!
//! Every request resolves its principal into an [`Access`] from the registry
//! head it reads. Requests run inside one SQLite read transaction, so the
//! head is pinned at the request's first read: a change committed before a
//! request starts is visible to it, and a request already running keeps its
//! snapshot. Resolution is cached per registry revision only, so no cache
//! outlives a change (ADR-0007 open question 6).
use super::*;
use crate::fact_store::FactBackend;
use crate::visibility::{Access, Clause};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::Mutex;
use weave_contract::{Fact, FactChange, FactKind, FactTarget};

/// The reserved registry graph.
pub const REGISTRY_GRAPH: &str = "weave:registry";
/// The registry's only branch.
pub const REGISTRY_BRANCH: &str = "main";
/// The built-in group of tenant administrators. Its members hold Administer
/// on every graph, and only they change principals and statuses.
pub const ADMINISTRATORS: &str = "weave:administrators";
/// Most groups on any chain of nested groups (ADR-0007 section 1).
pub const MAX_GROUP_DEPTH: usize = 4;
/// Most groups in any principal's transitive closure (ADR-0007 section 1).
pub const MAX_GROUP_CLOSURE: usize = 1024;
/// Most changes in one registry write.
pub const MAX_REGISTRY_CHANGES: usize = 1000;
/// Most records the registry holds.
pub const MAX_REGISTRY_RECORDS: usize = 100_000;
const SPACE: &str = "weave:registry";
const MEMBER: &str = "weave:member";
const GRANT: &str = "weave:grant";
const BRANCH_WRITER: &str = "weave:branch-writer";

/// True for the registry graph, which raw commits, fact writes, capsule
/// receipt and branch acceptance reject.
pub(crate) fn reserved(graph: &str) -> bool {
    graph == REGISTRY_GRAPH
}

/// A principal's kind. A team is a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalKind {
    User,
    Group,
}

/// A principal's status. Retirement is final and the ID is never reissued.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalStatus {
    Active,
    Suspended,
    Retired,
}

/// Ordered permissions: each implies those below it (ADR-0007 section 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    Discover,
    Read,
    Write,
    Administer,
}

/// One registered principal as the registry API reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Principal {
    pub id: String,
    pub kind: PrincipalKind,
    pub status: PrincipalStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Groups only: principals (or groups) that may change the members.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub administrators: Vec<String>,
}

/// One graph ACL entry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphGrant {
    pub subject: String,
    pub permission: Permission,
}

/// A graph's ACL as its administrators see it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphAcl {
    pub graph_id: String,
    pub grants: Vec<GraphGrant>,
    /// Protected branches and the principals or groups that may write them.
    pub protected_branches: BTreeMap<String, Vec<String>>,
}

/// One graph a principal can discover, with its branches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphListing {
    pub graph_id: String,
    pub branches: Vec<String>,
    pub permission: Permission,
}

/// One change of a registry write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case", deny_unknown_fields)]
pub enum RegistryChange {
    /// Register a user or group (tenant administrators only).
    AddPrincipal {
        id: String,
        kind: PrincipalKind,
        #[serde(default)]
        display_name: Option<String>,
    },
    /// Suspend, reactivate or retire a principal (tenant administrators only).
    SetStatus { id: String, status: PrincipalStatus },
    /// Add a member (user or group) to a group.
    AddMember { group: String, member: String },
    /// Remove a member from a group. Effective on the next request.
    RemoveMember { group: String, member: String },
    /// Replace a group's administrators (tenant administrators only).
    SetGroupAdministrators {
        group: String,
        administrators: Vec<String>,
    },
    /// Set a subject's permission on a graph, replacing any earlier entry.
    Grant {
        graph_id: String,
        subject: String,
        permission: Permission,
    },
    /// Remove a subject's entry from a graph's ACL.
    Revoke { graph_id: String, subject: String },
    /// Protect a branch: writing it then needs a branch entry or Administer.
    ProtectBranch { graph_id: String, branch_id: String },
    /// Unprotect a branch: graph Write applies to it again.
    UnprotectBranch { graph_id: String, branch_id: String },
    /// Allow a subject to write a protected branch.
    GrantBranchWrite {
        graph_id: String,
        branch_id: String,
        subject: String,
    },
    /// Remove a subject's write entry from a protected branch.
    RevokeBranchWrite {
        graph_id: String,
        branch_id: String,
        subject: String,
    },
}

impl RegistryChange {
    /// True when the change can only add authority. Every other change
    /// advances the admission policy epoch (ADR-0007 section 5).
    fn additive(&self) -> bool {
        matches!(
            self,
            RegistryChange::AddPrincipal { .. }
                | RegistryChange::AddMember { .. }
                | RegistryChange::GrantBranchWrite { .. }
                | RegistryChange::UnprotectBranch { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    kind: PrincipalKind,
    status: PrincipalStatus,
    display_name: Option<String>,
    administrators: Vec<String>,
}

/// The registry at one revision.
#[derive(Debug, Default)]
pub(crate) struct State {
    pub(crate) revision: String,
    principals: BTreeMap<String, Entry>,
    /// group -> members
    members: BTreeMap<String, BTreeSet<String>>,
    /// member -> groups it is directly in
    parents: HashMap<String, BTreeSet<String>>,
    /// graph -> subject -> permission
    grants: BTreeMap<String, BTreeMap<String, Permission>>,
    /// (graph, branch) -> subjects allowed to write
    protected: BTreeMap<(String, String), BTreeSet<String>>,
    resolved: Mutex<HashMap<String, Arc<Resolved>>>,
}

impl Clone for State {
    fn clone(&self) -> Self {
        Self {
            revision: self.revision.clone(),
            principals: self.principals.clone(),
            members: self.members.clone(),
            parents: self.parents.clone(),
            grants: self.grants.clone(),
            protected: self.protected.clone(),
            resolved: Mutex::default(),
        }
    }
}

/// One principal resolved against one registry revision.
#[derive(Debug)]
pub(crate) struct Resolved {
    pub(crate) access: Arc<Access>,
    /// The principal and its active groups, for ACL entries.
    subjects: BTreeSet<String>,
    active_user: bool,
    administrator: bool,
}

/// Cache of the registry state, keyed by its head revision.
#[derive(Default)]
pub(crate) struct Cache(Mutex<Option<Arc<State>>>);

fn registry_error(code: &str, message: &str) -> Error {
    err(code, message)
}
fn hash(parts: &[&str]) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(parts).expect("strings serialize"))
    )
}
fn graph_node(graph: &str) -> String {
    format!("graph:{}", hash(&[graph]))
}
fn branch_node(graph: &str, branch: &str) -> String {
    format!("branch:{}", hash(&[graph, branch]))
}
fn restricted(readers: Vec<String>, groups: Vec<&str>) -> serde_json::Value {
    let mut reader_groups: Vec<String> = groups.into_iter().map(String::from).collect();
    reader_groups.push(ADMINISTRATORS.into());
    serde_json::json!({"readers": readers, "reader_groups": reader_groups})
}

impl State {
    fn entry(&self, id: &str) -> Option<&Entry> {
        self.principals.get(id)
    }
    fn active_group(&self, id: &str) -> bool {
        self.entry(id)
            .is_some_and(|e| e.status == PrincipalStatus::Active && e.kind == PrincipalKind::Group)
    }
    /// Active groups that contain `id`, transitively, through active groups only.
    fn closure(&self, id: &str) -> Option<BTreeSet<String>> {
        let mut out = BTreeSet::new();
        let mut queue = VecDeque::from([id.to_owned()]);
        while let Some(next) = queue.pop_front() {
            for group in self.parents.get(&next).into_iter().flatten() {
                if self.active_group(group) && out.insert(group.clone()) {
                    if out.len() > MAX_GROUP_CLOSURE {
                        return None;
                    }
                    queue.push_back(group.clone());
                }
            }
        }
        Some(out)
    }
    /// Every group that contains `id`, transitively, whatever its status.
    fn ancestors(&self, id: &str, limit: usize) -> Option<BTreeSet<String>> {
        let mut out = BTreeSet::new();
        let mut queue = VecDeque::from([id.to_owned()]);
        while let Some(next) = queue.pop_front() {
            for group in self.parents.get(&next).into_iter().flatten() {
                if out.insert(group.clone()) {
                    if out.len() > limit {
                        return None;
                    }
                    queue.push_back(group.clone());
                }
            }
        }
        Some(out)
    }
    /// Longest chain of groups from `group` upwards, counting `group`.
    fn height(&self, group: &str, memo: &mut HashMap<String, usize>, guard: usize) -> usize {
        if let Some(h) = memo.get(group) {
            return *h;
        }
        if guard > MAX_GROUP_DEPTH + 1 {
            return guard;
        }
        let h = 1 + self
            .parents
            .get(group)
            .into_iter()
            .flatten()
            .map(|p| self.height(p, memo, guard + 1))
            .max()
            .unwrap_or(0);
        memo.insert(group.to_owned(), h);
        h
    }
    fn resolve(&self, principal: &str) -> Arc<Resolved> {
        if let Some(hit) = self
            .resolved
            .lock()
            .ok()
            .and_then(|m| m.get(principal).cloned())
        {
            return hit;
        }
        let resolved = Arc::new(match self.entry(principal) {
            Some(entry)
                if entry.status == PrincipalStatus::Active && entry.kind == PrincipalKind::User =>
            {
                match self.closure(principal) {
                    Some(groups) => {
                        let mut subjects = groups.clone();
                        subjects.insert(principal.to_owned());
                        Resolved {
                            administrator: groups.contains(ADMINISTRATORS),
                            access: Access::from_clauses(vec![Clause {
                                principals: BTreeSet::from([principal.to_owned()]),
                                groups,
                            }]),
                            subjects,
                            active_user: true,
                        }
                    }
                    None => Resolved::nobody(),
                }
            }
            // Unregistered, suspended, retired and group principals fail closed.
            _ => Resolved::nobody(),
        });
        if let Ok(mut map) = self.resolved.lock() {
            if map.len() >= 4096 {
                map.clear();
            }
            map.insert(principal.to_owned(), resolved.clone());
        }
        resolved
    }
    /// The permission `who` holds on `graph`, if any.
    fn permission(&self, who: &Resolved, graph: &str) -> Option<Permission> {
        if !who.active_user {
            return None;
        }
        if who.administrator {
            return Some(Permission::Administer);
        }
        if reserved(graph) {
            // Every active principal reads its own registry records.
            return Some(Permission::Read);
        }
        if graph.starts_with("weave:") {
            // Other engine graphs keep their own policy checks.
            return Some(Permission::Read);
        }
        self.grants.get(graph).and_then(|acl| {
            acl.iter()
                .filter(|(subject, _)| who.subjects.contains(*subject))
                .map(|(_, p)| *p)
                .max()
        })
    }
    fn can_write_branch(&self, who: &Resolved, graph: &str, branch: &str) -> bool {
        let Some(permission) = self.permission(who, graph) else {
            return false;
        };
        if permission == Permission::Administer {
            return true;
        }
        match self.protected.get(&(graph.to_owned(), branch.to_owned())) {
            None => permission >= Permission::Write,
            Some(writers) => {
                permission >= Permission::Read && writers.iter().any(|w| who.subjects.contains(w))
            }
        }
    }
    fn may_administer_group(&self, who: &Resolved, group: &str) -> bool {
        who.administrator
            || (group != ADMINISTRATORS
                && self
                    .entry(group)
                    .is_some_and(|e| e.administrators.iter().any(|a| who.subjects.contains(a))))
    }

    /// Every registry fact, keyed for diffing.
    fn render(&self) -> Result<BTreeMap<(FactKind, String), Fact>> {
        let mut out = BTreeMap::new();
        let mut put = |fact: serde_json::Value| -> Result<()> {
            let fact: Fact = serde_json::from_value(fact)?;
            out.insert((fact.kind(), fact.key_id().into_owned()), fact);
            Ok(())
        };
        for (id, e) in &self.principals {
            let mut properties = serde_json::json!({"kind": e.kind, "status": e.status});
            if let Some(name) = &e.display_name {
                properties["display_name"] = name.clone().into();
            }
            if !e.administrators.is_empty() {
                properties["administrators"] = serde_json::json!(e.administrators);
            }
            let readers = match e.kind {
                PrincipalKind::Group => restricted(vec![], vec![id.as_str()]),
                PrincipalKind::User => restricted(vec![id.clone()], vec![]),
            };
            put(
                serde_json::json!({"kind": "node", "id": id, "entity_id": id, "space_id": SPACE,
                "properties": properties, "readers": readers["readers"],
                "reader_groups": readers["reader_groups"]}),
            )?;
        }
        for (group, members) in &self.members {
            for member in members {
                let readers = restricted(vec![member.clone()], vec![]);
                put(
                    serde_json::json!({"kind": "edge", "id": format!("member:{}", hash(&[group, member])),
                    "predicate": MEMBER, "from": member, "to": group, "valid_time": {"start": 0},
                    "readers": readers["readers"], "reader_groups": readers["reader_groups"]}),
                )?;
            }
        }
        let admins = restricted(vec![], vec![]);
        let mut graphs: BTreeSet<&str> = self.grants.keys().map(String::as_str).collect();
        graphs.extend(self.protected.keys().map(|(g, _)| g.as_str()));
        for graph in graphs {
            put(
                serde_json::json!({"kind": "node", "id": graph_node(graph), "entity_id": graph_node(graph),
                "space_id": SPACE, "properties": {"graph_id": graph},
                "readers": admins["readers"], "reader_groups": admins["reader_groups"]}),
            )?;
        }
        for (graph, acl) in &self.grants {
            for (subject, permission) in acl {
                put(
                    serde_json::json!({"kind": "edge", "id": format!("grant:{}", hash(&[graph, subject])),
                    "predicate": GRANT, "from": subject, "to": graph_node(graph), "valid_time": {"start": 0},
                    "properties": {"permission": permission},
                    "readers": admins["readers"], "reader_groups": admins["reader_groups"]}),
                )?;
            }
        }
        for ((graph, branch), writers) in &self.protected {
            let node = branch_node(graph, branch);
            put(
                serde_json::json!({"kind": "node", "id": node, "entity_id": node, "space_id": SPACE,
                "properties": {"graph_id": graph, "branch_id": branch, "protected": true},
                "readers": admins["readers"], "reader_groups": admins["reader_groups"]}),
            )?;
            for subject in writers {
                put(serde_json::json!({"kind": "edge",
                    "id": format!("branch-writer:{}", hash(&[graph, branch, subject])),
                    "predicate": BRANCH_WRITER, "from": subject, "to": node, "valid_time": {"start": 0},
                    "readers": admins["readers"], "reader_groups": admins["reader_groups"]}))?;
            }
        }
        Ok(out)
    }

    /// Rebuild the state from the registry's live facts.
    fn parse(revision: String, facts: Vec<Fact>) -> Result<Self> {
        let corrupt = || registry_error("E_INTEGRITY", "registry record is malformed");
        let mut state = State {
            revision,
            ..State::default()
        };
        let mut graph_nodes: HashMap<String, String> = HashMap::new();
        let mut branch_nodes: HashMap<String, (String, String)> = HashMap::new();
        let mut edges = Vec::new();
        for fact in facts {
            match fact {
                Fact::Node(node) => {
                    let p = &node.properties;
                    let text = |k: &str| p.get(k).and_then(|v| v.as_str()).map(String::from);
                    if let Some(graph) = text("graph_id") {
                        match text("branch_id") {
                            Some(branch) => {
                                branch_nodes.insert(node.id.clone(), (graph, branch));
                            }
                            None => {
                                graph_nodes.insert(node.id.clone(), graph);
                            }
                        }
                        continue;
                    }
                    let entry = Entry {
                        kind: serde_json::from_value(p.get("kind").cloned().ok_or_else(corrupt)?)
                            .map_err(|_| corrupt())?,
                        status: serde_json::from_value(
                            p.get("status").cloned().ok_or_else(corrupt)?,
                        )
                        .map_err(|_| corrupt())?,
                        display_name: text("display_name"),
                        administrators: match p.get("administrators") {
                            None => vec![],
                            Some(v) => serde_json::from_value(v.clone()).map_err(|_| corrupt())?,
                        },
                    };
                    state.principals.insert(node.id, entry);
                }
                Fact::Edge(edge) => edges.push(edge),
                _ => return Err(corrupt()),
            }
        }
        for edge in edges {
            match edge.predicate.as_str() {
                MEMBER => {
                    state
                        .members
                        .entry(edge.to.clone())
                        .or_default()
                        .insert(edge.from.clone());
                    state.parents.entry(edge.from).or_default().insert(edge.to);
                }
                GRANT => {
                    let graph = graph_nodes.get(&edge.to).ok_or_else(corrupt)?;
                    let permission: Permission = serde_json::from_value(
                        edge.properties
                            .get("permission")
                            .cloned()
                            .ok_or_else(corrupt)?,
                    )
                    .map_err(|_| corrupt())?;
                    state
                        .grants
                        .entry(graph.clone())
                        .or_default()
                        .insert(edge.from, permission);
                }
                BRANCH_WRITER => {
                    let key = branch_nodes.get(&edge.to).ok_or_else(corrupt)?;
                    state
                        .protected
                        .entry(key.clone())
                        .or_default()
                        .insert(edge.from);
                }
                _ => return Err(corrupt()),
            }
        }
        for key in branch_nodes.into_values() {
            state.protected.entry(key).or_default();
        }
        Ok(state)
    }
}

impl Resolved {
    fn nobody() -> Self {
        Resolved {
            access: Access::nobody(),
            subjects: BTreeSet::new(),
            active_user: false,
            administrator: false,
        }
    }
}

fn check_id(id: &str) -> Result<()> {
    if !valid_id(id) {
        return Err(registry_error(
            "E_ID",
            "registry identifiers require 1 to 512 bytes",
        ));
    }
    Ok(())
}
fn forbidden() -> Error {
    registry_error(
        "E_FORBIDDEN",
        "the principal may not make this registry change",
    )
}
fn unknown() -> Error {
    registry_error("E_UNAVAILABLE", "principal unavailable")
}

impl Engine {
    pub(crate) fn registry_head(&self) -> Result<Option<String>> {
        self.head(REGISTRY_GRAPH, REGISTRY_BRANCH)
    }
    /// The registry state at its head, or `None` for a store without one.
    pub(crate) fn registry_state(&self) -> Result<Option<Arc<State>>> {
        let Some(head) = self.registry_head()? else {
            return Ok(None);
        };
        if let Some(state) = self
            .registry_cache
            .0
            .lock()
            .map_err(|_| unknown())?
            .as_ref()
        {
            if state.revision == head {
                return Ok(Some(state.clone()));
            }
        }
        let stored = self.fact_store().select(
            &head,
            &crate::fact_store::FactFilter::default(),
            MAX_REGISTRY_RECORDS,
        )?;
        if stored.len() > MAX_REGISTRY_RECORDS {
            return Err(registry_error(
                "E_INTEGRITY",
                "registry exceeds its record bound",
            ));
        }
        let mut facts = Vec::with_capacity(stored.len());
        for record in stored {
            facts.push(facts::parse(&record.version)?);
        }
        let state = Arc::new(State::parse(head, facts)?);
        *self.registry_cache.0.lock().map_err(|_| unknown())? = Some(state.clone());
        Ok(Some(state))
    }
    /// True once [`Engine::enable_registry`] has run for this store.
    pub fn registry_enabled(&self) -> Result<bool> {
        Ok(self.registry_head()?.is_some())
    }
    fn resolved(&self, host: &HostContext) -> Result<Option<(Arc<State>, Arc<Resolved>)>> {
        Ok(self.registry_state()?.map(|state| {
            let who = state.resolve(&host.principal);
            (state, who)
        }))
    }
    /// The request's identity (ADR-0007 section 2): the literal principal in
    /// a store without a registry, otherwise its resolved identity set.
    pub(crate) fn access(&self, host: &HostContext) -> Result<Arc<Access>> {
        Ok(match self.resolved(host)? {
            None => Access::literal(&host.principal),
            Some((_, who)) => who.access.clone(),
        })
    }
    /// Graph-level authorization for an operation that names `graph`
    /// (ADR-0007 section 4). Without Discover the error is exactly the one a
    /// nonexistent graph gives; with Discover but not `need` it is
    /// `E_FORBIDDEN`. A store without a registry is not checked here.
    pub(crate) fn require_graph(
        &self,
        host: &HostContext,
        graph: &str,
        need: Permission,
    ) -> Result<()> {
        let Some((state, who)) = self.resolved(host)? else {
            return Ok(());
        };
        match state.permission(&who, graph) {
            None => Err(err("E_UNAVAILABLE", "graph unavailable")),
            Some(held) if held < need => Err(err(
                "E_FORBIDDEN",
                "the principal lacks the graph permission",
            )),
            Some(_) => Ok(()),
        }
    }
    /// [`Engine::require_graph`] for Read, opening its own read snapshot
    /// when the caller has none (operators that check before their scope).
    pub(crate) fn require_graph_snapshot(&self, host: &HostContext, graph: &str) -> Result<()> {
        let _transaction = self.optional_read_transaction()?;
        self.require_graph(host, graph, Permission::Read)
    }
    /// Graph Read for a nested reference (metadata, evidence, derivation
    /// pins): a graph the reader cannot read behaves like an unavailable
    /// revision.
    pub(crate) fn graph_readable(&self, host: &HostContext, graph: &str) -> Result<bool> {
        Ok(match self.resolved(host)? {
            None => true,
            Some((state, who)) => state
                .permission(&who, graph)
                .is_some_and(|p| p >= Permission::Read),
        })
    }
    /// Write authority on a graph branch: the host's `writable_graphs` in a
    /// store without a registry, otherwise graph Write (or a branch entry on
    /// a protected branch) under the current registry. `branch` `None` asks
    /// for an unprotected branch.
    pub(crate) fn graph_writable(
        &self,
        host: &HostContext,
        graph: &str,
        branch: Option<&str>,
    ) -> Result<bool> {
        Ok(match self.resolved(host)? {
            None => host.writable_graphs.contains(graph),
            Some((state, who)) => match branch {
                Some(branch) => state.can_write_branch(&who, graph, branch),
                None => state
                    .permission(&who, graph)
                    .is_some_and(|p| p >= Permission::Write),
            },
        })
    }
    /// [`Engine::graph_writable`] for every graph of an adapter or handler.
    pub(crate) fn all_writable(&self, host: &HostContext, graphs: &[String]) -> Result<bool> {
        for graph in graphs {
            if !self.graph_writable(host, graph, None)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    /// [`Engine::require_graph`] for a write of one branch: `E_UNAVAILABLE`
    /// without Discover, `E_FORBIDDEN` without write authority.
    pub(crate) fn require_branch_write(
        &self,
        host: &HostContext,
        graph: &str,
        branch: &str,
    ) -> Result<()> {
        if self.registry_head()?.is_some() {
            self.require_graph(host, graph, Permission::Discover)?;
        }
        if !self.graph_writable(host, graph, Some(branch))? {
            return Err(err(
                "E_FORBIDDEN",
                "host has not granted graph write authority",
            ));
        }
        Ok(())
    }
    /// Groups named in `reader_groups` must exist and be usable by the writer
    /// (ADR-0007 section 2). Any other value fails with `E_UNAVAILABLE`, the
    /// code of a nonexistent group, so a write cannot probe for groups.
    pub(crate) fn validate_reader_groups<'a>(
        &self,
        groups: impl IntoIterator<Item = &'a String>,
        host: &HostContext,
    ) -> Result<()> {
        let mut groups = groups.into_iter().peekable();
        if groups.peek().is_none() {
            return Ok(());
        }
        let unavailable = || err("E_UNAVAILABLE", "reader group unavailable");
        let Some((state, who)) = self.resolved(host)? else {
            return Err(unavailable());
        };
        for group in groups {
            if !valid_id(group) {
                return Err(err(
                    "E_ID",
                    "reader group identifiers require 1 to 512 bytes",
                ));
            }
            // A group is discoverable by its members (through every clause of
            // the request's identity), its administrators and tenant administrators.
            let discoverable = who.access.has_group(group)
                || (who.active_user && state.may_administer_group(&who, group));
            if !state.active_group(group) || !discoverable {
                return Err(unavailable());
            }
        }
        Ok(())
    }

    /// Enable the registry for this store. One-way and recorded: the host's
    /// principal becomes the first user and tenant administrator. From then
    /// on graph ACLs, Discover and groups apply to every request.
    pub fn enable_registry(&self, host: &HostContext) -> Result<String> {
        check_id(&host.principal)?;
        if host.principal.starts_with("weave:") {
            return Err(err(
                "E_RESERVED_NAMESPACE",
                "weave: principal IDs are reserved",
            ));
        }
        self.registry_write(host, |state| {
            if !state.principals.is_empty() {
                return Err(registry_error(
                    "E_CONFLICT",
                    "the registry is already enabled",
                ));
            }
            let active = |kind| Entry {
                kind,
                status: PrincipalStatus::Active,
                display_name: None,
                administrators: vec![],
            };
            state
                .principals
                .insert(ADMINISTRATORS.into(), active(PrincipalKind::Group));
            state
                .principals
                .insert(host.principal.clone(), active(PrincipalKind::User));
            add_member(state, ADMINISTRATORS, &host.principal)?;
            Ok(false)
        })
    }

    /// Apply registry changes atomically, as one fact revision of
    /// `weave:registry` written by `host.principal`. Changes are checked in
    /// order against the state the earlier changes produced. Returns the new
    /// registry revision (or the current one when nothing changed).
    pub fn apply_registry(&self, changes: &[RegistryChange], host: &HostContext) -> Result<String> {
        if changes.is_empty() {
            return Err(err("E_REGISTRY", "a registry write needs a change"));
        }
        if changes.len() > MAX_REGISTRY_CHANGES {
            return Err(err(
                "E_BUDGET",
                "a registry write carries at most 1000 changes",
            ));
        }
        if self.registry_head()?.is_none() {
            return Err(err("E_REGISTRY_DISABLED", "the store has no registry"));
        }
        self.registry_write(host, |state| {
            let mut reducing = false;
            for change in changes {
                apply_change(state, change, host)?;
                reducing |= !change.additive();
            }
            Ok(reducing)
        })
    }

    fn registry_write(
        &self,
        host: &HostContext,
        change: impl FnOnce(&mut State) -> Result<bool>,
    ) -> Result<String> {
        self.conn.execute_batch("SAVEPOINT weave_registry")?;
        let result = (|| {
            let _clock = self.operation_write_scope()?;
            let current = self.registry_state()?;
            let before = current.as_deref().cloned().unwrap_or_default();
            let mut after = before.clone();
            // Changes see the actor as resolved before this write.
            after.resolved = Mutex::default();
            let reducing = change(&mut after)?;
            let old = before.render()?;
            let new = after.render()?;
            if new.len() > MAX_REGISTRY_RECORDS {
                return Err(err("E_BUDGET", "the registry holds at most 100000 records"));
            }
            let mut writes = Vec::new();
            for (key, fact) in &new {
                match old.get(key) {
                    None => writes.push(FactChange::Assert { fact: fact.clone() }),
                    Some(previous) if previous != fact => writes.push(FactChange::Correct {
                        fact: fact.clone(),
                        expected: None,
                    }),
                    Some(_) => {}
                }
            }
            for key in old.keys().filter(|k| !new.contains_key(*k)) {
                writes.push(FactChange::Retract {
                    target: FactTarget {
                        kind: key.0,
                        id: key.1.clone(),
                    },
                    expected: None,
                });
            }
            let head = self.registry_head()?;
            if writes.is_empty() {
                return head.ok_or_else(|| err("E_INTEGRITY", "no-op registry write without head"));
            }
            let (revision, _) =
                self.append_facts(REGISTRY_GRAPH, REGISTRY_BRANCH, &Some(head), &writes, host)?;
            if reducing {
                self.advance_admission_epoch(&revision)?;
            }
            Ok(revision)
        })();
        match result {
            Ok(revision) => {
                self.conn.execute_batch("RELEASE weave_registry")?;
                Ok(revision)
            }
            Err(error) => {
                self.conn
                    .execute_batch("ROLLBACK TO weave_registry; RELEASE weave_registry")?;
                Err(error)
            }
        }
    }

    /// A principal's registry record. Readable by tenant administrators and
    /// by the principal itself; any other request fails `E_UNAVAILABLE`.
    pub fn registry_principal(&self, id: &str, host: &HostContext) -> Result<Principal> {
        let _transaction = self.optional_read_transaction()?;
        let (state, who) = self.resolved(host)?.ok_or_else(unknown)?;
        if !(who.administrator || (who.active_user && id == host.principal)) {
            return Err(unknown());
        }
        let entry = state.entry(id).ok_or_else(unknown)?;
        Ok(Principal {
            id: id.into(),
            kind: entry.kind,
            status: entry.status,
            display_name: entry.display_name.clone(),
            administrators: entry.administrators.clone(),
        })
    }
    /// The groups a principal is directly in. Readable by tenant
    /// administrators and by the principal itself.
    pub fn registry_memberships(&self, id: &str, host: &HostContext) -> Result<Vec<String>> {
        let _transaction = self.optional_read_transaction()?;
        let (state, who) = self.resolved(host)?.ok_or_else(unknown)?;
        if !(who.administrator || (who.active_user && id == host.principal)) {
            return Err(unknown());
        }
        state.entry(id).ok_or_else(unknown)?;
        Ok(state
            .parents
            .get(id)
            .map(|g| g.iter().cloned().collect())
            .unwrap_or_default())
    }
    /// A group's direct members. Readable by tenant administrators and the
    /// group's administrators.
    pub fn registry_members(&self, group: &str, host: &HostContext) -> Result<Vec<String>> {
        let _transaction = self.optional_read_transaction()?;
        let (state, who) = self.resolved(host)?.ok_or_else(unknown)?;
        if !who.active_user || !state.may_administer_group(&who, group) {
            return Err(unknown());
        }
        Ok(state
            .members
            .get(group)
            .map(|m| m.iter().cloned().collect())
            .unwrap_or_default())
    }
    /// A graph's ACL. Requires Administer on the graph.
    pub fn registry_graph_acl(&self, graph: &str, host: &HostContext) -> Result<GraphAcl> {
        let _transaction = self.optional_read_transaction()?;
        self.require_graph(host, graph, Permission::Administer)?;
        let state = self.registry_state()?.ok_or_else(unknown)?;
        Ok(GraphAcl {
            graph_id: graph.into(),
            grants: state
                .grants
                .get(graph)
                .into_iter()
                .flatten()
                .map(|(subject, permission)| GraphGrant {
                    subject: subject.clone(),
                    permission: *permission,
                })
                .collect(),
            protected_branches: state
                .protected
                .iter()
                .filter(|((g, _), _)| g == graph)
                .map(|((_, b), w)| (b.clone(), w.iter().cloned().collect()))
                .collect(),
        })
    }
    /// The graphs this principal can discover, with their branches. Graphs
    /// it cannot discover are absent, and no total is reported. In a store
    /// without a registry, every external graph with a branch is listed with
    /// the permission the host grants (Write for `writable_graphs`, else Read).
    pub fn list_graphs(&self, host: &HostContext) -> Result<Vec<GraphListing>> {
        let _transaction = self.optional_read_transaction()?;
        let mut branches: BTreeMap<String, Vec<String>> = BTreeMap::new();
        {
            let mut statement = self.conn.prepare(
                "SELECT substr(graph_id,1,513),substr(branch_id,1,513) FROM heads ORDER BY graph_id,branch_id",
            )?;
            let rows = statement.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get(1)?)))?;
            for row in rows {
                let (graph, branch) = row?;
                branches.entry(graph).or_default().push(branch);
            }
        }
        let resolved = self.resolved(host)?;
        let mut graphs: BTreeSet<String> = branches
            .keys()
            .filter(|g| !g.starts_with("weave:"))
            .cloned()
            .collect();
        if let Some((state, _)) = &resolved {
            // Graphs with ACL entries but no data yet are discoverable too.
            graphs.extend(state.grants.keys().cloned());
        }
        let mut out = Vec::new();
        for graph in graphs {
            let permission = match &resolved {
                None if host.writable_graphs.contains(&graph) => Some(Permission::Write),
                None => Some(Permission::Read),
                Some((state, who)) => state.permission(who, &graph),
            };
            if let Some(permission) = permission {
                out.push(GraphListing {
                    branches: branches.get(&graph).cloned().unwrap_or_default(),
                    graph_id: graph,
                    permission,
                });
            }
        }
        Ok(out)
    }
}

fn add_member(state: &mut State, group: &str, member: &str) -> Result<()> {
    let target = state.entry(group).ok_or_else(unknown)?;
    if target.kind != PrincipalKind::Group {
        return Err(registry_error(
            "E_REGISTRY",
            "members can only be added to groups",
        ));
    }
    if target.status == PrincipalStatus::Retired {
        return Err(registry_error(
            "E_CONFLICT",
            "a retired group takes no members",
        ));
    }
    let entry = state.entry(member).ok_or_else(unknown)?;
    if group == ADMINISTRATORS && entry.kind != PrincipalKind::User {
        return Err(registry_error(
            "E_REGISTRY",
            "tenant administrators are users",
        ));
    }
    if state.members.get(group).is_some_and(|m| m.contains(member)) {
        return Ok(());
    }
    // Acyclic: the member must not already contain the group.
    let above = state
        .ancestors(group, usize::MAX)
        .expect("unbounded ancestors");
    if member == group || above.contains(member) {
        return Err(registry_error(
            "E_BUDGET",
            "group membership must be acyclic",
        ));
    }
    state
        .members
        .entry(group.into())
        .or_default()
        .insert(member.into());
    state
        .parents
        .entry(member.into())
        .or_default()
        .insert(group.into());
    // Depth and closure bounds for the member and everything below it.
    let mut below = BTreeSet::from([member.to_owned()]);
    let mut queue = VecDeque::from([member.to_owned()]);
    while let Some(next) = queue.pop_front() {
        for m in state.members.get(&next).into_iter().flatten() {
            if below.insert(m.clone()) {
                queue.push_back(m.clone());
            }
        }
    }
    let mut memo = HashMap::new();
    for principal in &below {
        let groups = state
            .ancestors(principal, MAX_GROUP_CLOSURE)
            .ok_or_else(|| {
                registry_error(
                    "E_BUDGET",
                    "a principal's group closure exceeds 1024 groups",
                )
            })?;
        let deepest = groups
            .iter()
            .chain(
                (state.entry(principal).map(|e| e.kind) == Some(PrincipalKind::Group))
                    .then_some(principal),
            )
            .map(|g| state.height(g, &mut memo, 1))
            .max()
            .unwrap_or(0);
        if deepest > MAX_GROUP_DEPTH {
            return Err(registry_error("E_BUDGET", "groups nest at most 4 deep"));
        }
    }
    Ok(())
}

fn apply_change(state: &mut State, change: &RegistryChange, host: &HostContext) -> Result<()> {
    // Authority comes from the state the earlier changes of this write
    // produced, exactly as if each change were its own write.
    if let Ok(cache) = state.resolved.get_mut() {
        cache.clear();
    }
    let who = state.resolve(&host.principal);
    if !who.active_user {
        return Err(forbidden());
    }
    let reject_reserved = |id: &str| -> Result<()> {
        check_id(id)?;
        if id.starts_with("weave:") {
            return Err(err(
                "E_RESERVED_NAMESPACE",
                "weave: identifiers are reserved",
            ));
        }
        Ok(())
    };
    let graph_admin = |state: &State, graph: &str| -> Result<()> {
        check_id(graph)?;
        if graph.starts_with("weave:") {
            return Err(err("E_RESERVED_NAMESPACE", "engine graphs have no ACL"));
        }
        match state.permission(&who, graph) {
            None => Err(err("E_UNAVAILABLE", "graph unavailable")),
            Some(Permission::Administer) => Ok(()),
            Some(_) => Err(forbidden()),
        }
    };
    let subject = |state: &State, id: &str| -> Result<()> {
        check_id(id)?;
        // Naming a principal is an administrator's view of the registry.
        state.entry(id).ok_or_else(unknown).map(|_| ())
    };
    match change {
        RegistryChange::AddPrincipal {
            id,
            kind,
            display_name,
        } => {
            if !who.administrator {
                return Err(forbidden());
            }
            reject_reserved(id)?;
            if let Some(name) = display_name {
                if name.len() > 512 {
                    return Err(err("E_ID", "display names hold at most 512 bytes"));
                }
            }
            if state.principals.contains_key(id) {
                return Err(registry_error(
                    "E_CONFLICT",
                    "principal IDs are unique and never reused",
                ));
            }
            if state.principals.len() >= MAX_REGISTRY_RECORDS / 2 {
                return Err(err("E_BUDGET", "the registry principal bound is reached"));
            }
            state.principals.insert(
                id.clone(),
                Entry {
                    kind: *kind,
                    status: PrincipalStatus::Active,
                    display_name: display_name.clone(),
                    administrators: vec![],
                },
            );
        }
        RegistryChange::SetStatus { id, status } => {
            if !who.administrator {
                return Err(forbidden());
            }
            check_id(id)?;
            if id == ADMINISTRATORS {
                return Err(err(
                    "E_RESERVED_NAMESPACE",
                    "the administrators group is permanent",
                ));
            }
            let entry = state.principals.get_mut(id).ok_or_else(unknown)?;
            if entry.status == PrincipalStatus::Retired && *status != PrincipalStatus::Retired {
                return Err(registry_error("E_CONFLICT", "retirement is final"));
            }
            entry.status = *status;
            if !state.principals.iter().any(|(p, e)| {
                e.kind == PrincipalKind::User
                    && e.status == PrincipalStatus::Active
                    && state
                        .parents
                        .get(p)
                        .is_some_and(|g| g.contains(ADMINISTRATORS))
            }) {
                return Err(registry_error(
                    "E_CONFLICT",
                    "the tenant keeps at least one active administrator",
                ));
            }
        }
        RegistryChange::AddMember { group, member } => {
            check_id(group)?;
            check_id(member)?;
            if !state.may_administer_group(&who, group) {
                return Err(
                    if state.active_group(group) && who.access.has_group(group) {
                        forbidden()
                    } else {
                        unknown()
                    },
                );
            }
            add_member(state, group, member)?;
        }
        RegistryChange::RemoveMember { group, member } => {
            check_id(group)?;
            check_id(member)?;
            if !state.may_administer_group(&who, group) {
                return Err(
                    if state.active_group(group) && who.access.has_group(group) {
                        forbidden()
                    } else {
                        unknown()
                    },
                );
            }
            if let Some(members) = state.members.get_mut(group) {
                members.remove(member);
                if members.is_empty() {
                    state.members.remove(group);
                }
            }
            if let Some(parents) = state.parents.get_mut(member) {
                parents.remove(group);
                if parents.is_empty() {
                    state.parents.remove(member);
                }
            }
            if group == ADMINISTRATORS && !state.members.contains_key(ADMINISTRATORS) {
                return Err(registry_error(
                    "E_CONFLICT",
                    "the tenant keeps at least one administrator",
                ));
            }
        }
        RegistryChange::SetGroupAdministrators {
            group,
            administrators,
        } => {
            if !who.administrator {
                return Err(forbidden());
            }
            check_id(group)?;
            if group == ADMINISTRATORS {
                return Err(err(
                    "E_RESERVED_NAMESPACE",
                    "tenant administrators administer their own group",
                ));
            }
            for a in administrators {
                subject(state, a)?;
            }
            let mut list = administrators.clone();
            list.sort();
            list.dedup();
            if list.len() > 256 {
                return Err(err("E_BUDGET", "a group has at most 256 administrators"));
            }
            let entry = state.principals.get_mut(group).ok_or_else(unknown)?;
            if entry.kind != PrincipalKind::Group {
                return Err(registry_error(
                    "E_REGISTRY",
                    "only groups have administrators",
                ));
            }
            entry.administrators = list;
        }
        RegistryChange::Grant {
            graph_id,
            subject: who_gets,
            permission,
        } => {
            graph_admin(state, graph_id)?;
            subject(state, who_gets)?;
            let acl = state.grants.entry(graph_id.clone()).or_default();
            if acl.len() >= 4096 && !acl.contains_key(who_gets) {
                return Err(err("E_BUDGET", "a graph ACL holds at most 4096 entries"));
            }
            acl.insert(who_gets.clone(), *permission);
        }
        RegistryChange::Revoke {
            graph_id,
            subject: who_loses,
        } => {
            graph_admin(state, graph_id)?;
            check_id(who_loses)?;
            if let Some(acl) = state.grants.get_mut(graph_id) {
                acl.remove(who_loses);
                if acl.is_empty() {
                    state.grants.remove(graph_id);
                }
            }
        }
        RegistryChange::ProtectBranch {
            graph_id,
            branch_id,
        } => {
            graph_admin(state, graph_id)?;
            check_id(branch_id)?;
            state
                .protected
                .entry((graph_id.clone(), branch_id.clone()))
                .or_default();
        }
        RegistryChange::UnprotectBranch {
            graph_id,
            branch_id,
        } => {
            graph_admin(state, graph_id)?;
            check_id(branch_id)?;
            state
                .protected
                .remove(&(graph_id.clone(), branch_id.clone()));
        }
        RegistryChange::GrantBranchWrite {
            graph_id,
            branch_id,
            subject: writer,
        } => {
            graph_admin(state, graph_id)?;
            check_id(branch_id)?;
            subject(state, writer)?;
            let writers = state
                .protected
                .get_mut(&(graph_id.clone(), branch_id.clone()))
                .ok_or_else(|| registry_error("E_REGISTRY", "the branch is not protected"))?;
            if writers.len() >= 4096 {
                return Err(err("E_BUDGET", "a branch ACL holds at most 4096 entries"));
            }
            writers.insert(writer.clone());
        }
        RegistryChange::RevokeBranchWrite {
            graph_id,
            branch_id,
            subject: writer,
        } => {
            graph_admin(state, graph_id)?;
            check_id(branch_id)?;
            check_id(writer)?;
            if let Some(writers) = state
                .protected
                .get_mut(&(graph_id.clone(), branch_id.clone()))
            {
                writers.remove(writer);
            }
        }
    }
    Ok(())
}
