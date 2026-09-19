//! Sparse overlap of two authorized navigation frontiers, not entity identity or truth.
//! The host must have current authority for BOTH inputs, including the older snapshot.
use super::*;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RecordRef {
    pub member: Member,
    pub revision: String,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Overlap {
    pub before: RecordRef,
    pub after: RecordRef,
    pub shared_leaves: usize,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Lineage {
    pub algorithm: &'static str,
    pub before_snapshot: String,
    pub after_snapshot: String,
    pub before_sources: Vec<GraphRef>,
    pub after_sources: Vec<GraphRef>,
    pub before_valid_at: i64,
    pub after_valid_at: i64,
    pub before_level: usize,
    pub after_level: usize,
    pub partial_input: bool,
    /// These are navigational overlaps, never accepted identity equivalences.
    pub overlaps: Vec<Overlap>,
    /// Before records with more than one intersecting after record.
    pub splits: Vec<RecordRef>,
    /// After records with more than one intersecting before record.
    pub merges: Vec<RecordRef>,
    pub retired: Vec<RecordRef>,
    pub created: Vec<RecordRef>,
    pub removed_leaves: Vec<String>,
    pub added_leaves: Vec<String>,
}
// Charge each bounded item before retaining it; reserve the small fixed envelope.
fn retain<T: Serialize>(items: &mut Vec<T>, value: T, bytes: &mut usize) -> Result<()> {
    let length = size(&value, MAX_OUTPUT.saturating_sub(*bytes))?;
    *bytes = bytes.saturating_add(length).saturating_add(1);
    if *bytes > MAX_OUTPUT {
        return Err(Error("E_CLUSTER_BUDGET"));
    }
    items.push(value);
    Ok(())
}
impl Hierarchy {
    fn record_ref(&self, member: &Member) -> RecordRef {
        RecordRef {
            member: member.clone(),
            revision: match member {
                Member::Cluster(id) => self.clusters[id].revision.clone(),
                Member::Leaf(id) => format!(
                    "leaf-record:{:x}",
                    Sha256::digest(
                        serde_json::to_vec(&(&self.snapshot_revision, id))
                            .expect("string tuple is serializable")
                    )
                ),
            },
        }
    }
    /// Compare complete frontiers without an all-pairs cluster scan. At most one sparse
    /// overlap entry per common leaf, bounded by MAX_NODES. Does not mutate either input.
    /// This first profile permits one source graph domain; plain leaf IDs cannot
    /// identify objects across multiple graph domains.
    /// Changed authorization must be applied by the host before calling, even for history.
    pub fn lineage_to(&self, next: &Self) -> Result<Lineage> {
        let source_ids = |h: &Hierarchy| {
            h.source
                .sources
                .iter()
                .map(|r| &r.graph_id)
                .cloned()
                .collect::<BTreeSet<_>>()
        };
        let old_domains = source_ids(self);
        if old_domains.len() != 1
            || self.source.perspective != next.source.perspective
            || self.source.context != next.source.context
            || old_domains != source_ids(next)
        {
            return Err(Error("E_CLUSTER_SCOPE"));
        }
        self.lineage_to_in_domain(next, old_domains.iter().next().unwrap())
    }
    /// Compare frontiers whose leaves the caller attests belong to one exact graph-ID
    /// domain. Additional source pins may be descriptor/proof dependencies, not leaf
    /// namespaces. This pure API cannot establish that attestation; hosts must obtain
    /// both inputs from that same graph domain and recheck current authority for both.
    /// Use `lineage_to` when this distinction cannot be established by the caller.
    pub fn lineage_to_in_domain(&self, next: &Self, domain: &str) -> Result<Lineage> {
        if !valid_id(domain)
            || !self.source.sources.iter().any(|r| r.graph_id == domain)
            || !next.source.sources.iter().any(|r| r.graph_id == domain)
            || self.source.perspective != next.source.perspective
            || self.source.context != next.source.context
        {
            return Err(Error("E_CLUSTER_SCOPE"));
        }
        let before = self.owners();
        let after = next.owners();
        let mut bytes = 4096;
        let mut before_sources = Vec::new();
        let mut after_sources = Vec::new();
        for source in &self.source.sources {
            retain(&mut before_sources, source.clone(), &mut bytes)?;
        }
        for source in &next.source.sources {
            retain(&mut after_sources, source.clone(), &mut bytes)?;
        }
        let mut counts = BTreeMap::<(&Member, &Member), usize>::new();
        let mut removed = Vec::new();
        let mut added = Vec::new();
        for (leaf, owner) in &before {
            if let Some(new_owner) = after.get(leaf) {
                *counts.entry((owner, new_owner)).or_default() += 1;
            } else {
                retain(&mut removed, leaf.clone(), &mut bytes)?;
            }
        }
        for leaf in after.keys().filter(|leaf| !before.contains_key(*leaf)) {
            retain(&mut added, leaf.clone(), &mut bytes)?;
        }
        let mut out_degree = BTreeMap::<&Member, usize>::new();
        let mut in_degree = BTreeMap::<&Member, usize>::new();
        let mut overlaps = Vec::new();
        for ((old, new), shared_leaves) in counts {
            *out_degree.entry(old).or_default() += 1;
            *in_degree.entry(new).or_default() += 1;
            retain(
                &mut overlaps,
                Overlap {
                    before: self.record_ref(old),
                    after: next.record_ref(new),
                    shared_leaves,
                },
                &mut bytes,
            )?;
        }
        let mut splits = Vec::new();
        let mut merges = Vec::new();
        let mut retired = Vec::new();
        let mut created = Vec::new();
        for member in &self.frontier {
            match out_degree.get(member).copied().unwrap_or(0) {
                0 => retain(&mut retired, self.record_ref(member), &mut bytes)?,
                1 => (),
                _ => retain(&mut splits, self.record_ref(member), &mut bytes)?,
            }
        }
        for member in &next.frontier {
            match in_degree.get(member).copied().unwrap_or(0) {
                0 => retain(&mut created, next.record_ref(member), &mut bytes)?,
                1 => (),
                _ => retain(&mut merges, next.record_ref(member), &mut bytes)?,
            }
        }
        let result = Lineage {
            algorithm: "weave:frontier-overlap:1",
            before_snapshot: self.snapshot_revision.clone(),
            after_snapshot: next.snapshot_revision.clone(),
            before_sources,
            after_sources,
            before_valid_at: self.source.valid_at,
            after_valid_at: next.source.valid_at,
            before_level: self.level,
            after_level: next.level,
            partial_input: self.source.partial || next.source.partial,
            overlaps,
            splits,
            merges,
            retired,
            created,
            removed_leaves: removed,
            added_leaves: added,
        };
        size(&result, MAX_OUTPUT)?;
        Ok(result)
    }
}
