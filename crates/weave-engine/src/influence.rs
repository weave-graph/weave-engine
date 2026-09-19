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
impl Engine {
    pub(crate) fn graph_influence_visible(
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
        Ok(
            self.premises_visible(&influence.assertions, host, visiting, budget, depth)?
                && self.node_refs_visible(&influence.nodes, host, visiting, budget, depth)?,
        )
    }
}
