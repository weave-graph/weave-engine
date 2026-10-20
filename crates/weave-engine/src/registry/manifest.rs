//! The groups a principal may name in `reader_groups`, for hosts that serve
//! a visible manifest (weave-language `weave-visible/1`; B3 #39).
use super::*;

impl Engine {
    /// Every group `host.principal` may name in `reader_groups` now: the
    /// active groups it is in through every clause of its identity, and the
    /// groups it administers (all active groups for a tenant administrator).
    /// Exactly the groups a write by it would accept. Sorted; empty for a
    /// store without a registry and for a principal that cannot act.
    pub fn registry_nameable_groups(&self, host: &HostContext) -> Result<Vec<String>> {
        let _transaction = self.optional_read_transaction()?;
        let Some((state, who)) = self.resolved(host)? else {
            return Ok(vec![]);
        };
        Ok(state
            .principals
            .keys()
            .filter(|id| state.nameable_group(&who, id))
            .cloned()
            .collect())
    }
}
