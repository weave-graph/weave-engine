//! Whole-value proof authorization shares recursion and work bounds with record proofs.
use super::*;
pub(crate) fn requires_v015(data: &GraphData) -> bool {
    data.influence.is_some()
        || data.edges.iter().any(|e| {
            !e.derived_nodes.is_empty() || e.derivations.iter().any(|g| !g.node_premises.is_empty())
        })
        || data.assertions.iter().any(|a| {
            !a.derived_nodes.is_empty() || a.derivations.iter().any(|g| !g.node_premises.is_empty())
        })
}
pub(crate) fn requires_v017(data: &GraphData) -> bool {
    data.influence
        .as_ref()
        .is_some_and(|i| !i.snapshots.is_empty())
        || data.nodes.iter().any(|r| !r.derived_snapshots.is_empty())
        || data.edges.iter().any(|r| !r.derived_snapshots.is_empty())
        || data
            .assertions
            .iter()
            .any(|r| !r.derived_snapshots.is_empty())
        || data.attachments.iter().any(|r| {
            !r.derived_snapshots.is_empty()
                || !r.derived_from.is_empty()
                || !r.derived_nodes.is_empty()
        })
}
impl Engine {
    pub(crate) fn attachment_flat_visible(
        &self,
        attachment: &MetadataAttachment,
        host: &HostContext,
        visiting: &mut HashSet<(u8, String, String, String)>,
        budget: &mut usize,
        depth: u32,
    ) -> Result<bool> {
        let _scope = self.authorization.enter();
        Ok(
            self.snapshot_refs_visible(&attachment.derived_snapshots, host)?
                && self.premises_visible(
                    &attachment.derived_from,
                    host,
                    visiting,
                    budget,
                    depth,
                )?
                && self.node_refs_visible(
                    &attachment.derived_nodes,
                    host,
                    visiting,
                    budget,
                    depth,
                )?,
        )
    }
    pub(crate) fn attachment_influence_visible(
        &self,
        attachment: &MetadataAttachment,
        host: &HostContext,
        visiting: &mut HashSet<(u8, String, String, String)>,
        budget: &mut usize,
        depth: u32,
    ) -> Result<bool> {
        Ok(
            self.attachment_flat_visible(attachment, host, visiting, budget, depth)?
                && self.groups_visible(&attachment.derivations, host, visiting, budget, depth)?,
        )
    }
    pub(crate) fn snapshot_refs_visible(
        &self,
        references: &[GraphRef],
        host: &HostContext,
    ) -> Result<bool> {
        let _scope = self.authorization.enter();
        if references.len() > 1000 {
            return Ok(false);
        }
        for reference in references {
            let Some(_proof) = self.authorization.proof(
                3,
                &reference.graph_id,
                &reference.revision,
                "",
                &host.principal,
            ) else {
                return Ok(false);
            };
            if !self.protected_reference_allowed(&reference.graph_id, &reference.revision, host)? {
                return Ok(false);
            }
            let Some(original) = self.load(&reference.graph_id, &reference.revision)? else {
                return Ok(false);
            };
            let (authorized, incomplete) = self.authorized(original.clone(), host)?;
            if !whole_graph_visible(&original, authorized, incomplete) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(crate) fn graph_flat_visible(
        &self,
        data: &GraphData,
        host: &HostContext,
        visiting: &mut HashSet<(u8, String, String, String)>,
        budget: &mut usize,
        depth: u32,
    ) -> Result<bool> {
        let Some(influence) = &data.influence else {
            return Ok(true);
        };
        weave_contract::influence::validate(influence).map_err(|d| err(&d.code, &d.message))?;
        Ok(self.snapshot_refs_visible(&influence.snapshots, host)?
            && self.premises_visible(&influence.assertions, host, visiting, budget, depth)?
            && self.node_refs_visible(&influence.nodes, host, visiting, budget, depth)?)
    }
    pub(crate) fn graph_influence_visible(
        &self,
        data: &GraphData,
        host: &HostContext,
        visiting: &mut HashSet<(u8, String, String, String)>,
        budget: &mut usize,
        depth: u32,
    ) -> Result<bool> {
        if !self.graph_flat_visible(data, host, visiting, budget, depth)? {
            return Ok(false);
        }
        match &data.influence {
            Some(i) => self.groups_visible(&i.derivations, host, visiting, budget, depth),
            None => Ok(true),
        }
    }
    /// Check a branch without leaking its failed active-path bookkeeping to siblings.
    fn group_visible(
        &self,
        group: &Derivation,
        host: &HostContext,
        visiting: &HashSet<(u8, String, String, String)>,
        budget: &mut usize,
        depth: u32,
    ) -> Result<bool> {
        if group.premises.is_empty()
            && group.node_premises.is_empty()
            && group.snapshot_premises.is_empty()
        {
            return Ok(false);
        }
        let mut branch = visiting.clone();
        Ok(self.snapshot_refs_visible(&group.snapshot_premises, host)?
            && self.premises_visible(&group.premises, host, &mut branch, budget, depth)?
            && self.node_refs_visible(&group.node_premises, host, &mut branch, budget, depth)?)
    }
    pub(crate) fn groups_visible(
        &self,
        groups: &[Derivation],
        host: &HostContext,
        visiting: &HashSet<(u8, String, String, String)>,
        budget: &mut usize,
        depth: u32,
    ) -> Result<bool> {
        if groups.is_empty() {
            return Ok(true);
        }
        for group in groups {
            if self.group_visible(group, host, visiting, budget, depth)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    /// A nonempty denied carrier is unavailable, never an unrestricted empty carrier.
    pub(crate) fn authorize_groups(
        &self,
        groups: &mut Vec<Derivation>,
        host: &HostContext,
        incomplete: &mut bool,
        budget: &mut usize,
    ) -> Result<bool> {
        if groups.is_empty() {
            return Ok(true);
        }
        let mut kept = Vec::new();
        for mut group in std::mem::take(groups) {
            if self.group_visible(&group, host, &HashSet::new(), budget, 0)? {
                // Keep explanatory pins reachable through currently authorized proof
                // ancestry. These pins never become additional authorization gates.
                let direct = |r: &GraphRef| {
                    group
                        .premises
                        .iter()
                        .any(|p| p.graph_id == r.graph_id && p.revision == r.revision)
                        || group
                            .node_premises
                            .iter()
                            .any(|p| p.graph_id == r.graph_id && p.revision == r.revision)
                        || group.snapshot_premises.contains(r)
                };
                if group.input_snapshots.iter().any(|r| !direct(r)) {
                    let pins = self.authorized_proof_pins(&group, host, budget)?;
                    group
                        .input_snapshots
                        .retain(|r| pins.contains(&r.graph_id, &r.revision));
                }
                kept.push(group);
            } else {
                *incomplete = true;
            }
        }
        *groups = kept;
        Ok(!groups.is_empty())
    }
    fn authorized_proof_pins(
        &self,
        group: &Derivation,
        host: &HostContext,
        budget: &mut usize,
    ) -> Result<ProofPins> {
        let mut walk = ProofPins::default();
        walk.group(group)?;
        while let Some((kind, graph, revision, object)) = walk.queue.pop_front() {
            if group
                .input_snapshots
                .iter()
                .all(|r| walk.contains(&r.graph_id, &r.revision))
            {
                break;
            }
            let Some(_proof) =
                self.authorization
                    .proof(4 + kind, &graph, &revision, &object, &host.principal)
            else {
                return Err(err("E_BUDGET", "explanation proof traversal limit"));
            };
            let Some(source) = self.load(&graph, &revision)? else {
                continue;
            };
            // The root group and every queued branch have already passed ordinary
            // semantic authorization in this same operation snapshot. Only actual
            // flat gates and successful alternatives are followed, never their
            // authored input_snapshots or unrelated records in the source graph.
            if let Some(i) = &source.influence {
                walk.flats(&i.assertions, &i.nodes, &i.snapshots)?;
                self.proof_pin_groups(&mut walk, &i.derivations, host, budget)?;
            }
            if kind == 1 {
                if let Some(n) = source.nodes.iter().find(|n| n.id == object) {
                    walk.flats(&n.derived_from, &n.derived_nodes, &n.derived_snapshots)?;
                    self.proof_pin_groups(&mut walk, &n.derivations, host, budget)?;
                }
            } else if let Some(e) = source.edges.iter().find(|e| e.id == object) {
                walk.flats(
                    if e.derivations.is_empty() {
                        &e.derived_from
                    } else {
                        &[]
                    },
                    &e.derived_nodes,
                    &e.derived_snapshots,
                )?;
                self.proof_pin_groups(&mut walk, &e.derivations, host, budget)?;
                walk.object(1, &graph, &revision, &e.from)?;
                walk.object(1, &graph, &revision, &e.to)?;
            } else if let Some(a) = source.assertions.iter().find(|a| a.id == object) {
                walk.flats(
                    if a.derivations.is_empty() {
                        &a.derived_from
                    } else {
                        &[]
                    },
                    &a.derived_nodes,
                    &a.derived_snapshots,
                )?;
                self.proof_pin_groups(&mut walk, &a.derivations, host, budget)?;
                if let Some(e) = source.structural_edges.iter().find(|e| e.id == a.edge_id) {
                    walk.object(1, &graph, &revision, &e.from)?;
                    walk.object(1, &graph, &revision, &e.to)?;
                }
            } else if let Some(a) = source.attachments.iter().find(|a| a.id == object) {
                walk.flats(&a.derived_from, &a.derived_nodes, &a.derived_snapshots)?;
                if let Some(origin) = &a.origin {
                    walk.object(0, &origin.graph_id, &origin.revision, &origin.assertion_id)?;
                }
                self.proof_pin_groups(&mut walk, &a.derivations, host, budget)?;
                match &a.host {
                    MetadataHost::Node { id } => walk.object(1, &graph, &revision, id)?,
                    MetadataHost::Edge { id } | MetadataHost::Assertion { id } => {
                        walk.object(0, &graph, &revision, id)?
                    }
                    MetadataHost::Entity { id } => {
                        for n in source.nodes.iter().filter(|n| &n.entity_id == id) {
                            walk.object(1, &graph, &revision, &n.id)?;
                        }
                    }
                    MetadataHost::Graph => {}
                }
            }
        }
        Ok(walk)
    }
    fn proof_pin_groups(
        &self,
        walk: &mut ProofPins,
        groups: &[Derivation],
        host: &HostContext,
        budget: &mut usize,
    ) -> Result<()> {
        for group in groups {
            if self.group_visible(group, host, &HashSet::new(), budget, 0)? {
                walk.group(group)?;
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct ProofPins {
    pins: BTreeMap<String, HashSet<String>>,
    seen: HashSet<(u8, String, String, String)>,
    queue: std::collections::VecDeque<(u8, String, String, String)>,
    work: usize,
    bytes: usize,
}
impl ProofPins {
    fn contains(&self, g: &str, r: &str) -> bool {
        self.pins.get(g).is_some_and(|v| v.contains(r))
    }
    fn charge(&mut self, g: &str, r: &str, o: &str) -> Result<()> {
        self.work += 1;
        if self.work > 10000 {
            return Err(err("E_BUDGET", "explanation proof traversal limit"));
        }
        self.bytes +=
            json_size(&(g, r, o), MATERIALIZED_LIMIT.saturating_sub(self.bytes))?.saturating_mul(3);
        if self.bytes > MATERIALIZED_LIMIT {
            return Err(err("E_BUDGET", "explanation proof byte limit"));
        }
        Ok(())
    }
    fn pin(&mut self, g: &str, r: &str) -> Result<()> {
        self.charge(g, r, "")?;
        self.pins.entry(g.into()).or_default().insert(r.into());
        Ok(())
    }
    fn object(&mut self, kind: u8, g: &str, r: &str, o: &str) -> Result<()> {
        self.charge(g, r, o)?;
        self.pins.entry(g.into()).or_default().insert(r.into());
        let key = (kind, g.into(), r.into(), o.into());
        if self.seen.insert(key.clone()) {
            self.queue.push_back(key);
        }
        Ok(())
    }
    fn flats(
        &mut self,
        assertions: &[AssertionRef],
        nodes: &[NodeRef],
        snapshots: &[GraphRef],
    ) -> Result<()> {
        for p in assertions {
            self.object(0, &p.graph_id, &p.revision, &p.assertion_id)?;
        }
        for p in nodes {
            self.object(1, &p.graph_id, &p.revision, &p.node_id)?;
        }
        for p in snapshots {
            self.pin(&p.graph_id, &p.revision)?;
        }
        Ok(())
    }
    fn group(&mut self, g: &Derivation) -> Result<()> {
        self.flats(&g.premises, &g.node_premises, &g.snapshot_premises)
    }
}
