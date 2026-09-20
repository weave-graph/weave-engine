//! Comparison-only normalization for releasing an entire immutable snapshot.
use super::*;
use std::collections::BTreeSet;

/// Authorization rebuilds the flat assertion index from retained alternatives.
/// Its order and duplicates do not change the proof; all other fields must remain
/// exact, including alternative boundaries and their input snapshot lists.
/// Only the temporary authorized copy is normalized. Stored bytes are untouched.
pub(super) fn whole_graph_visible(
    original: &GraphData,
    mut authorized: GraphData,
    incomplete: bool,
) -> bool {
    if incomplete
        || original.edges.len() != authorized.edges.len()
        || original.assertions.len() != authorized.assertions.len()
    {
        return false;
    }
    for (before, after) in original.edges.iter().zip(&mut authorized.edges) {
        if before.id != after.id {
            return false;
        }
        if !before.derivations.is_empty()
            && !restore_flat_index(&before.derived_from, &mut after.derived_from)
        {
            return false;
        }
    }
    for (before, after) in original.assertions.iter().zip(&mut authorized.assertions) {
        if before.id != after.id {
            return false;
        }
        if !before.derivations.is_empty()
            && !restore_flat_index(&before.derived_from, &mut after.derived_from)
        {
            return false;
        }
    }
    original == &authorized
}

fn restore_flat_index(original: &[AssertionRef], authorized: &mut Vec<AssertionRef>) -> bool {
    if original == authorized {
        return true;
    }
    // Match the existing per-assertion dependency budget before allocating sets.
    if original.len() > 1000 || authorized.len() > 1000 {
        return false;
    }
    fn keys(refs: &[AssertionRef]) -> BTreeSet<(&str, &str, &str)> {
        refs.iter()
            .map(|r| {
                (
                    r.graph_id.as_str(),
                    r.revision.as_str(),
                    r.assertion_id.as_str(),
                )
            })
            .collect::<BTreeSet<_>>()
    }
    if keys(original) != keys(authorized) {
        return false;
    }
    authorized.clear();
    authorized.extend_from_slice(original);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_group_backed_flat_indices_are_normalized() {
        let original: GraphData = serde_json::from_value(json!({
            "nodes":[{"id":"n","entity_id":"n","space_id":"s"}],
            "edges":[{"id":"e","predicate":"p","from":"n","to":"n",
                "valid_time":{"start":0},
                "derived_from":[
                    {"graph_id":"g","revision":"r","assertion_id":"z"},
                    {"graph_id":"g","revision":"r","assertion_id":"a"}],
                "derivations":[{"operator":"fixture","premises":[
                    {"graph_id":"g","revision":"r","assertion_id":"a"},
                    {"graph_id":"g","revision":"r","assertion_id":"z"}],
                    "parameters":{"ordered":[1,2]},
                    "input_snapshots":[{"graph_id":"g","revision":"r"}]}]}]
        }))
        .unwrap();
        let mut reordered = original.clone();
        reordered.edges[0].derived_from.reverse();
        assert!(whole_graph_visible(&original, reordered.clone(), false));
        assert!(!whole_graph_visible(&original, reordered.clone(), true));
        let mut altered = reordered.clone();
        altered.edges[0].derivations[0].premises.reverse();
        assert!(!whole_graph_visible(&original, altered, false));
        let mut altered = reordered.clone();
        altered.edges[0].derivations[0].input_snapshots.clear();
        assert!(!whole_graph_visible(&original, altered, false));
        let mut altered = reordered.clone();
        altered.nodes.clear();
        assert!(!whole_graph_visible(&original, altered, false));
        let mut ungrouped = original;
        ungrouped.edges[0].derivations.clear();
        reordered.edges[0].derivations.clear();
        assert!(!whole_graph_visible(&ungrouped, reordered, false));
    }
}
