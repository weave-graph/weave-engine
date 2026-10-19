//! The one record-visibility predicate and the one reader intersection
//! (ADR-0007 section 13, items 4 and 6). B1 extends these two functions for
//! `reader_groups`; no other code re-implements the literal-reader rule.

/// True when a record with these `readers` is visible to `principal`:
/// empty means public, otherwise the principal must be listed.
pub(crate) fn readers_admit(readers: &[String], principal: &str) -> bool {
    readers.is_empty() || readers.iter().any(|r| r == principal)
}

/// Readers of a record derived from two records, as the explicit-assertion
/// materialization has always computed them: an empty side defers to the
/// other side, and two restricted sides keep their common principals.
pub(crate) fn intersect_readers(left: &[String], right: &[String]) -> Vec<String> {
    if left.is_empty() {
        right.to_vec()
    } else if right.is_empty() {
        left.to_vec()
    } else {
        left.iter().filter(|p| right.contains(p)).cloned().collect()
    }
}
