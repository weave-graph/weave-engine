//! Graph creation with an engine-assigned ID (ADR-0007 section 4, B3 #39).
//!
//! A non-administrator cannot choose a graph ID, because a collision would
//! reveal a hidden graph. [`Engine::create_graph`] derives an opaque ID from
//! host-supplied randomness and grants the creating user Administer on it, in
//! one registry revision written by that user.
use super::*;

/// Prefix of every engine-assigned graph ID.
pub const CREATED_GRAPH_PREFIX: &str = "g-";

impl Engine {
    /// Create a graph whose ID the engine derives from `entropy`, and grant
    /// `host.principal` Administer on it. The host must supply 32 bytes of
    /// fresh randomness; the ID is `g-` and 32 hex digits of their SHA-256
    /// digest, so a caller cannot pick it to land on an existing graph. Any
    /// active user may create a graph. Groups, inactive principals and
    /// unregistered principals get `E_FORBIDDEN`, because Administer is not
    /// delegable (ADR-0007 section 3). The change is additive, so it leaves
    /// the admission epoch alone.
    pub fn create_graph(&self, entropy: &[u8; 32], host: &HostContext) -> Result<String> {
        if self.registry_head()?.is_none() {
            return Err(err("E_REGISTRY_DISABLED", "the store has no registry"));
        }
        let digest = format!(
            "{:x}",
            Sha256::digest([b"weave:created-graph\0".as_slice(), entropy].concat())
        );
        let graph = format!("{CREATED_GRAPH_PREFIX}{}", &digest[..32]);
        let exists: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM heads WHERE graph_id=?1)",
            [&graph],
            |r| r.get(0),
        )?;
        self.registry_write(host, |state| {
            if !state.resolve(&host.principal).active_user {
                return Err(forbidden());
            }
            if exists
                || state.grants.contains_key(&graph)
                || state.protected.keys().any(|(g, _)| g == &graph)
            {
                return Err(err("E_CONFLICT", "graph identifier collision"));
            }
            state
                .grants
                .entry(graph.clone())
                .or_default()
                .insert(host.principal.clone(), Permission::Administer);
            Ok(false)
        })?;
        Ok(graph)
    }
}
