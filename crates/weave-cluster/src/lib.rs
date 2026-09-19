//! Deterministic lazy topology aggregation of an already authorized snapshot.
//! The trusted host must filter visibility, context and valid time BEFORE construction.
//! This crate cannot authenticate a caller or release a result to another principal.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use weave_contract::{AssertionRef, ContextSelection, GraphRef};

pub mod lineage;

pub const ALGORITHM: &str = "weave:topology-matching:1";
const MAX_INPUT: usize = 16 * 1024 * 1024;
const MAX_NODES: usize = 10_000;
const MAX_LINKS: usize = 100_000;
const MAX_WORK: usize = 2_000_000;
const MAX_OUTPUT: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub &'static str);
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub id: String,
    pub from: String,
    pub to: String,
    /// Contributing authorized claims. The host retains their complete proof groups.
    pub evidence: Vec<AssertionRef>,
}

/// A host-selected perspective over one authorized context/time snapshot.
/// Reader sets and policy grants are deliberately not accepted as wire authority.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub perspective: String,
    pub context: ContextSelection,
    pub valid_at: i64,
    pub sources: Vec<GraphRef>,
    pub nodes: Vec<String>,
    pub links: Vec<Link>,
    pub partial: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Member {
    Leaf(String),
    Cluster(String),
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Cluster {
    /// Stable membership identity; never a unique cache key for an entire record.
    pub id: String,
    pub revision: String,
    pub children: [Member; 2],
    pub leaves: Vec<String>,
    /// Source links wholly contained in this aggregate; IDs resolve through snapshot().
    pub contributing_links: Vec<String>,
    pub level: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AggregateLink {
    pub from: Member,
    pub to: Member,
    /// Existential relation: at least one source link, never a universal claim.
    pub contributing_links: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_offset: Option<usize>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Manifest {
    pub algorithm: &'static str,
    pub level: usize,
    pub frontier: Vec<Member>,
    pub evidence_boundary: bool,
    pub partial_input: bool,
    pub approximate_navigation: bool,
    pub work_used: usize,
}

/// Private construction state prevents callers from injecting cyclic cluster children.
pub struct Hierarchy {
    source: Snapshot,
    clusters: BTreeMap<String, Cluster>,
    frontier: Vec<Member>,
    level: usize,
    boundary: bool,
    work: usize,
    bytes: usize,
    work_limit: usize,
    snapshot_revision: String,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 1024 && !value.chars().any(char::is_control)
}
struct Counter(usize, usize);
impl std::io::Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        if self.0 > self.1 {
            return Err(std::io::Error::other("cluster byte budget"));
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn size<T: Serialize>(value: &T, limit: usize) -> Result<usize> {
    let mut counter = Counter(0, limit);
    serde_json::to_writer(&mut counter, value).map_err(|_| Error("E_CLUSTER_BUDGET"))?;
    Ok(counter.0)
}

impl Hierarchy {
    pub fn new(source: Snapshot) -> Result<Self> {
        Self::with_budget(source, MAX_WORK)
    }
    pub fn with_budget(mut source: Snapshot, work_limit: usize) -> Result<Self> {
        if work_limit == 0 || work_limit > MAX_WORK {
            return Err(Error("E_CLUSTER_BUDGET"));
        }
        if source.nodes.len() > MAX_NODES || source.links.len() > MAX_LINKS {
            return Err(Error("E_CLUSTER_BUDGET"));
        }
        size(&source, MAX_INPUT)?;
        weave_contract::context::validate_selection(&source.context)
            .map_err(|_| Error("E_CLUSTER_INPUT"))?;
        if !valid_id(&source.perspective) || source.sources.len() > 4096 {
            return Err(Error("E_CLUSTER_INPUT"));
        }
        source.nodes.sort();
        if source.nodes.iter().any(|n| !valid_id(n))
            || source.nodes.windows(2).any(|p| p[0] == p[1])
        {
            return Err(Error("E_CLUSTER_INPUT"));
        }
        let nodes: BTreeSet<_> = source.nodes.iter().collect();
        let pins: BTreeSet<_> = source
            .sources
            .iter()
            .map(|r| (&r.graph_id, &r.revision))
            .collect();
        source.links.sort_by(|a, b| a.id.cmp(&b.id));
        if source.links.windows(2).any(|p| p[0].id == p[1].id) {
            return Err(Error("E_CLUSTER_INPUT"));
        }
        for link in &source.links {
            if !valid_id(&link.id)
                || !nodes.contains(&link.from)
                || !nodes.contains(&link.to)
                || link.evidence.is_empty()
                || link.evidence.len() > 256
                || link.evidence.iter().any(|p| {
                    !valid_id(&p.graph_id)
                        || !valid_id(&p.revision)
                        || !valid_id(&p.assertion_id)
                        || !pins.contains(&(&p.graph_id, &p.revision))
                })
            {
                return Err(Error("E_CLUSTER_INPUT"));
            }
        }
        for r in &source.sources {
            if !valid_id(&r.graph_id) || !valid_id(&r.revision) {
                return Err(Error("E_CLUSTER_INPUT"));
            }
        }
        let snapshot_revision = format!(
            "snapshot:{:x}",
            Sha256::digest(serde_json::to_vec(&source).map_err(|_| Error("E_CLUSTER_INPUT"))?)
        );
        let frontier = source.nodes.iter().cloned().map(Member::Leaf).collect();
        Ok(Self {
            source,
            clusters: BTreeMap::new(),
            frontier,
            level: 0,
            boundary: false,
            work: 0,
            bytes: 0,
            work_limit,
            snapshot_revision,
        })
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.source
    }
    pub fn cluster(&self, id: &str) -> Option<&Cluster> {
        self.clusters.get(id)
    }
    pub fn manifest(&self) -> Manifest {
        Manifest {
            algorithm: ALGORITHM,
            level: self.level,
            frontier: self.frontier.clone(),
            evidence_boundary: self.boundary,
            partial_input: self.source.partial,
            approximate_navigation: true,
            work_used: self.work,
        }
    }
    fn leaves(&self, member: &Member) -> Vec<String> {
        match member {
            Member::Leaf(id) => vec![id.clone()],
            Member::Cluster(id) => self.clusters[id].leaves.clone(),
        }
    }
    fn owners(&self) -> BTreeMap<String, Member> {
        self.frontier
            .iter()
            .flat_map(|m| self.leaves(m).into_iter().map(|n| (n, m.clone())))
            .collect()
    }
    /// Advance by one maximal matching. Unmatched objects persist unchanged.
    /// Every successful level strictly reduces frontier size; no fixed semantic depth.
    /// Budget exhaustion is an error, never a claim that the evidence boundary was reached.
    pub fn advance(&mut self) -> Result<Manifest> {
        if self.boundary {
            return Ok(self.manifest());
        }
        let charge = self
            .source
            .nodes
            .len()
            .saturating_add(self.source.links.len())
            .saturating_add(self.frontier.len());
        if self.work.saturating_add(charge) > self.work_limit {
            return Err(Error("E_CLUSTER_BUDGET"));
        }
        self.work += charge;
        let owners = self.owners();
        let mut pairs = BTreeSet::new();
        for link in &self.source.links {
            let (a, b) = (&owners[&link.from], &owners[&link.to]);
            if a != b {
                pairs.insert(if a < b {
                    (a.clone(), b.clone())
                } else {
                    (b.clone(), a.clone())
                });
            }
        }
        let mut used = BTreeSet::new();
        let mut next = Vec::new();
        let mut created = Vec::new();
        let mut bytes = self.bytes;
        // Bound matching candidates as part of total work before processing them.
        if self.work.saturating_add(pairs.len()) > self.work_limit {
            return Err(Error("E_CLUSTER_BUDGET"));
        }
        self.work += pairs.len();
        for (a, b) in pairs {
            if used.contains(&a) || used.contains(&b) {
                continue;
            }
            let mut leaves = self.leaves(&a);
            leaves.extend(self.leaves(&b));
            leaves.sort();
            let id = format!(
                "cluster:{:x}",
                Sha256::digest(
                    serde_json::to_vec(&(
                        ALGORITHM,
                        &self.source.perspective,
                        &self.source.context,
                        &leaves
                    ))
                    .map_err(|_| Error("E_CLUSTER_INPUT"))?
                )
            );
            let cluster = Cluster {
                id: id.clone(),
                revision: String::new(),
                children: [a.clone(), b.clone()],
                leaves,
                contributing_links: Vec::new(),
                level: self.level + 1,
            };
            used.insert(a);
            used.insert(b);
            next.push(Member::Cluster(id));
            created.push(cluster);
        }
        if created.is_empty() {
            self.boundary = true;
            return Ok(self.manifest());
        }
        // One source pass, rather than an edge scan for every newly made cluster.
        let new_owners: BTreeMap<_, _> = created
            .iter()
            .enumerate()
            .flat_map(|(i, c)| c.leaves.iter().map(move |n| (n, i)))
            .collect();
        let mut contributions = vec![Vec::new(); created.len()];
        for link in &self.source.links {
            if let (Some(a), Some(b)) = (new_owners.get(&link.from), new_owners.get(&link.to))
                && a == b
            {
                contributions[*a].push(link.id.clone());
            }
        }
        for (c, links) in created.iter_mut().zip(contributions) {
            c.contributing_links = links;
            c.revision = format!(
                "record:{:x}",
                Sha256::digest(
                    serde_json::to_vec(&(
                        &self.snapshot_revision,
                        &c.id,
                        &c.children,
                        &c.leaves,
                        &c.contributing_links,
                        c.level
                    ))
                    .map_err(|_| Error("E_CLUSTER_INPUT"))?
                )
            );
            bytes += size(c, MAX_OUTPUT.saturating_sub(bytes))?;
        }
        next.extend(self.frontier.iter().filter(|m| !used.contains(*m)).cloned());
        next.sort();
        for c in created {
            self.clusters.insert(c.id.clone(), c);
        }
        self.frontier = next;
        self.level += 1;
        self.bytes = bytes;
        Ok(self.manifest())
    }

    /// Semantic expansion exposes the two direct children. It does not change a camera.
    pub fn zoom(&self, id: &str) -> Result<[Member; 2]> {
        self.cluster(id)
            .map(|c| c.children.clone())
            .ok_or(Error("E_CLUSTER_UNAVAILABLE"))
    }
    pub fn evidence(&self, id: &str, offset: usize, limit: usize) -> Result<Page<String>> {
        if limit == 0 || limit > 256 {
            return Err(Error("E_CLUSTER_BUDGET"));
        }
        let c = self.cluster(id).ok_or(Error("E_CLUSTER_UNAVAILABLE"))?;
        if offset > c.contributing_links.len() {
            return Err(Error("E_CLUSTER_CURSOR"));
        }
        let end = offset.saturating_add(limit).min(c.contributing_links.len());
        Ok(Page {
            items: c.contributing_links[offset..end].to_vec(),
            next_offset: (end < c.contributing_links.len()).then_some(end),
        })
    }
    /// Aggregated directed edges retain all visible contributing links. This is navigation,
    /// never an exact-query exclusion certificate. Exact queries still inspect the source.
    pub fn aggregate_links(&self) -> Result<Vec<AggregateLink>> {
        let owners = self.owners();
        let mut links: BTreeMap<(Member, Member), Vec<String>> = BTreeMap::new();
        for link in &self.source.links {
            let pair = (owners[&link.from].clone(), owners[&link.to].clone());
            if pair.0 != pair.1 {
                links.entry(pair).or_default().push(link.id.clone());
            }
        }
        let output: Vec<_> = links
            .into_iter()
            .map(|((from, to), contributing_links)| AggregateLink {
                from,
                to,
                contributing_links,
            })
            .collect();
        size(&output, MAX_OUTPUT)?;
        Ok(output)
    }
}
