//! Exact typed context interpretation; witnesses are data that must be revalidated.
use super::*;
use weave_contract::context_axes::{ContextDefinition, ContextSchema};

fn unavailable() -> Error {
    err("E_CONTEXT_UNAVAILABLE", "typed context unavailable")
}
impl Engine {
    pub(crate) fn context_typing_visible(
        &self,
        data: &GraphData,
        host: &HostContext,
        visiting: &mut HashSet<(u8, String, String, String)>,
        budget: &mut usize,
        depth: u32,
    ) -> Result<bool> {
        let Some(typing) = &data.context_typing else {
            return Ok(true);
        };
        if depth > 32 || context_typing::validate_graph(data).is_err() {
            return Ok(false);
        }
        for witness in &typing.witnesses {
            let key = (
                2,
                witness.context.graph_id.clone(),
                witness.context.revision.clone(),
                "definition".into(),
            );
            if *budget == 0 || !visiting.insert(key.clone()) {
                return Ok(false);
            }
            *budget -= 1;
            let resolved = self.resolve_context_witness(
                &witness.context,
                &witness.schema,
                host,
                visiting,
                budget,
                depth + 1,
            )?;
            visiting.remove(&key);
            if resolved.as_ref().is_none_or(|r| {
                r.context != witness.context
                    || r.definition != witness.definition
                    || r.anchor_nodes != witness.anchor_nodes
            }) {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn resolve_context_witness(
        &self,
        reference: &GraphRef,
        expected: &ContextSchema,
        host: &HostContext,
        visiting: &mut HashSet<(u8, String, String, String)>,
        budget: &mut usize,
        depth: u32,
    ) -> Result<Option<TypedContextWitness>> {
        if depth > 32
            || *budget == 0
            || expected.validate().is_err()
            || !valid_id(&reference.graph_id)
            || !valid_id(&reference.revision)
            || !self.protected_reference_allowed(&reference.graph_id, &reference.revision, host)?
        {
            return Ok(None);
        }
        *budget -= 1;
        let Some(data) = self.load(&reference.graph_id, &reference.revision)? else {
            return Ok(None);
        };
        let data = visible(data, &host.principal);
        if data.profile != GraphProfile::Explicit
            || !self.context_typing_visible(&data, host, visiting, budget, depth + 1)?
            || !self.graph_influence_visible(&data, host, visiting, budget, depth + 1)?
        {
            return Ok(None);
        }
        let mut definitions = data.assertions.iter().filter_map(|a| {
            data.structural_edges
                .iter()
                .find(|e| e.id == a.edge_id && e.predicate == "weave:context:definition")
                .map(|e| (a, e))
        });
        let Some((assertion, structure)) = definitions.next() else {
            return Ok(None);
        };
        if definitions.next().is_some()
            || assertion.id != "definition"
            || assertion.polarity != Polarity::Positive
            || assertion.context.is_some()
            || assertion.valid_time.start != i64::MIN
            || assertion.valid_time.end.is_some()
            || structure.from != structure.to
        {
            return Ok(None);
        }
        let Some(payload) = assertion.properties.get("weave.context") else {
            return Ok(None);
        };
        if json_size(payload, context_axes::MAX_BYTES).is_err() {
            return Ok(None);
        }
        let Ok(definition) = ContextDefinition::from_json(&serde_json::to_vec(payload)?) else {
            return Ok(None);
        };
        if definition.schema.canonical_bytes().ok() != expected.canonical_bytes().ok() {
            return Ok(None);
        }
        let assertion_ref = AssertionRef {
            graph_id: reference.graph_id.clone(),
            revision: reference.revision.clone(),
            assertion_id: "definition".into(),
        };
        let anchor = NodeRef {
            graph_id: reference.graph_id.clone(),
            revision: reference.revision.clone(),
            node_id: structure.from.clone(),
        };
        if !self.premises_visible(
            std::slice::from_ref(&assertion_ref),
            host,
            visiting,
            budget,
            depth + 1,
        )? || !self.node_refs_visible(
            std::slice::from_ref(&anchor),
            host,
            visiting,
            budget,
            depth + 1,
        )? {
            return Ok(None);
        }
        let schema =
            ContextSchema::from_json(&expected.canonical_bytes().map_err(|_| unavailable())?)
                .map_err(|_| unavailable())?;
        Ok(Some(TypedContextWitness {
            context: reference.clone(),
            schema,
            definition: assertion_ref,
            anchor_nodes: vec![anchor],
        }))
    }
    pub(crate) fn select_typed_context(
        &self,
        input: QueryResult,
        reference: &GraphRef,
        expected: &ContextSchema,
        host: &HostContext,
    ) -> Result<QueryResult> {
        let _scope = self.read_budget.enter();
        self.require_current_result_authority(&input, host)
            .map_err(|_| unavailable())?;
        let witness = self
            .resolve_context_witness(reference, expected, host, &mut HashSet::new(), &mut 1000, 0)
            .map_err(|e| {
                if e.code == "E_BUDGET" {
                    e
                } else {
                    unavailable()
                }
            })?
            .ok_or_else(unavailable)?;
        let mut value = context::select(
            input,
            &ContextSelection::Pinned {
                reference: reference.clone(),
            },
            &algebra_context(host),
        )
        .map_err(|d| err(&d.code, &d.message))?;
        let added = ContextTyping {
            selected: Some(reference.clone()),
            witnesses: vec![witness],
        };
        let mut typing = context_typing::merge(value.graph.context_typing.as_ref(), Some(&added))
            .map_err(|d| err(&d.code, &d.message))?
            .ok_or_else(unavailable)?;
        typing.selected = Some(reference.clone());
        value.graph.context_typing = Some(typing);
        if !value.input_snapshots.contains(reference) {
            value.input_snapshots.push(reference.clone());
        }
        context_typing::validate_result(&value).map_err(|d| err(&d.code, &d.message))?;
        json_size(&value, MATERIALIZED_LIMIT)?;
        Ok(value)
    }
}
