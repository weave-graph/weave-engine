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
                group.input_snapshots.retain(|r| {
                    group
                        .premises
                        .iter()
                        .any(|p| p.graph_id == r.graph_id && p.revision == r.revision)
                        || group
                            .node_premises
                            .iter()
                            .any(|p| p.graph_id == r.graph_id && p.revision == r.revision)
                        || group.snapshot_premises.contains(r)
                });
                kept.push(group);
            } else {
                *incomplete = true;
            }
        }
        *groups = kept;
        Ok(!groups.is_empty())
    }
}
