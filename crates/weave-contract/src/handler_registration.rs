//! Inert sealed pure handler artifacts. Descriptive identity never installs authority.
use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const HANDLER_TEMPLATE_FORMAT: &str = "weave-handler-registration/1";
pub const HANDLER_TEMPLATE_PROTOCOL: &str = "0.19.0";
pub const HANDLER_EVENT_BINDING: &str = "$event";
pub const MAX_HANDLER_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CompiledHandlerTemplate {
    pub format: String,
    pub protocol: String,
    pub name: String,
    pub revision: String,
    pub input: HandlerInput,
    #[serde(deserialize_with = "events")]
    pub event_types: Vec<HandlerEventType>,
    pub recipe: HandlerRecipe,
    pub output_slot: String,
    #[serde(deserialize_with = "sources")]
    pub source_revisions: Vec<SourceRevision>,
    pub definition_digest: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HandlerInput {
    pub graph_id: String,
    pub branch_id: String,
    pub metadata_depth: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum HandlerEventType {
    #[serde(rename = "graph.accepted")]
    GraphAccepted,
    #[serde(rename = "graph.committed")]
    GraphCommitted,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HandlerRecipe {
    #[serde(deserialize_with = "bindings")]
    pub bindings: Vec<HandlerBinding>,
    pub output: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HandlerBinding {
    pub name: String,
    pub value: GraphExpression,
}
fn bounded<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>, const N: usize>(
    d: D,
) -> Result<Vec<T>, D::Error> {
    struct Visitor<T, const N: usize>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>, const N: usize> serde::de::Visitor<'de> for Visitor<T, N> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "at most {N} handler artifact members")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<T>, A::Error> {
            if seq.size_hint().is_some_and(|n| n > N) {
                return Err(serde::de::Error::custom("handler member limit"));
            }
            let mut values = Vec::new();
            while let Some(value) = seq.next_element()? {
                if values.len() == N {
                    return Err(serde::de::Error::custom("handler member limit"));
                }
                values.push(value);
            }
            Ok(values)
        }
    }
    d.deserialize_seq(Visitor::<T, N>(std::marker::PhantomData))
}
fn events<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<HandlerEventType>, D::Error> {
    bounded::<D, _, 2>(d)
}
fn sources<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<SourceRevision>, D::Error> {
    bounded::<D, _, 1000>(d)
}
fn bindings<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<HandlerBinding>, D::Error> {
    bounded::<D, _, 256>(d)
}
fn fail(code: &str, message: &str) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        message: message.into(),
    }
}
fn id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512
}

pub fn validate_handler_recipe(recipe: &HandlerRecipe) -> Result<(), Diagnostic> {
    recipe_sources(recipe, true).map(|_| ())
}
fn recipe_sources(
    recipe: &HandlerRecipe,
    temporal: bool,
) -> Result<Vec<SourceRevision>, Diagnostic> {
    let mut sources = Vec::new();
    identity::digest("weave-handler-recipe-bound", recipe, MAX_HANDLER_BYTES)?;
    if recipe.bindings.len() > 256 {
        return Err(fail("E_BUDGET", "handler binding count exceeded"));
    }
    let mut names = BTreeSet::from([HANDLER_EVENT_BINDING]);
    let mut count = 0;
    for binding in &recipe.bindings {
        if !id(&binding.name)
            || names.contains(binding.name.as_str())
            || binding.name.starts_with('$')
        {
            return Err(fail(
                "E_HANDLER_BINDING",
                "handler binding must be bounded, unique and nonreserved",
            ));
        }
        let mut pending = vec![(&binding.value, 0usize)];
        while let Some((expression, depth)) = pending.pop() {
            count += 1;
            if count > 1000 || depth > 32 {
                return Err(fail("E_BUDGET", "handler expression budget exceeded"));
            }
            match expression {
                GraphExpression::Window { input, .. } if temporal => {
                    pending.push((input, depth + 1))
                }
                GraphExpression::Sequence { left, right, .. } if temporal => {
                    pending.push((left, depth + 1));
                    pending.push((right, depth + 1));
                }
                GraphExpression::Window { .. } | GraphExpression::Sequence { .. } => {
                    return Err(fail(
                        "E_HANDLER_VERSION",
                        "temporal recipe requires contract 0.19.0",
                    ));
                }
                GraphExpression::Reference { name } => {
                    if !names.contains(name.as_str()) {
                        return Err(fail(
                            "E_HANDLER_BINDING",
                            "handler reference must name the event or an earlier binding",
                        ));
                    }
                }
                GraphExpression::Filter { input, .. }
                | GraphExpression::Project { input, .. }
                | GraphExpression::Support { input, .. }
                | GraphExpression::Context { input, .. }
                | GraphExpression::Metadata { input, .. }
                | GraphExpression::Explain { input }
                | GraphExpression::Counterparts { input, .. } => pending.push((input, depth + 1)),
                GraphExpression::Reason { input, rules } => {
                    let source = SourceRevision {
                        name: rules.id.clone(),
                        revision: rules.revision.clone(),
                        digest: identity::source_fingerprint(rules)?,
                    };
                    sources = algebra::merge_source_revisions(&sources, &[source])?;
                    pending.push((input, depth + 1));
                }
                GraphExpression::Union { left, right }
                | GraphExpression::Join { left, right, .. }
                | GraphExpression::Diff {
                    before: left,
                    after: right,
                } => {
                    pending.push((left, depth + 1));
                    pending.push((right, depth + 1));
                }
                GraphExpression::Geometry { operation, .. } => pending.extend(
                    operation
                        .inputs()
                        .into_iter()
                        .map(|input| (input, depth + 1)),
                ),
                GraphExpression::Query { .. }
                | GraphExpression::TypedContext { .. }
                | GraphExpression::AcceptedGraph { .. }
                | GraphExpression::CurrentView { .. }
                | GraphExpression::ResolveIdentity { .. }
                | GraphExpression::Cluster { .. } => {
                    return Err(fail(
                        "E_HANDLER_READ",
                        "handler recipes cannot perform hidden store reads",
                    ))
                }
            }
        }
        names.insert(binding.name.as_str());
    }
    if !names.contains(recipe.output.as_str()) {
        return Err(fail(
            "E_HANDLER_BINDING",
            "handler output must name an available graph binding",
        ));
    }
    Ok(sources)
}
fn fields(template: &CompiledHandlerTemplate) -> Result<(), Diagnostic> {
    identity::digest("weave-handler-template-bound", template, MAX_HANDLER_BYTES)?;
    if template.format != HANDLER_TEMPLATE_FORMAT
        || ![HANDLER_TEMPLATE_PROTOCOL, "0.18.0"].contains(&template.protocol.as_str())
    {
        return Err(fail(
            "E_HANDLER_VERSION",
            "unsupported handler artifact format or protocol",
        ));
    }
    if !id(&template.name)
        || !id(&template.revision)
        || !id(&template.output_slot)
        || !id(&template.input.graph_id)
        || !id(&template.input.branch_id)
        || template.input.metadata_depth > 8
    {
        return Err(fail(
            "E_HANDLER_TEMPLATE",
            "handler identifiers or metadata depth invalid",
        ));
    }
    if template.event_types.len() != 2
        || !template
            .event_types
            .contains(&HandlerEventType::GraphAccepted)
        || !template
            .event_types
            .contains(&HandlerEventType::GraphCommitted)
    {
        return Err(fail(
            "E_HANDLER_TEMPLATE",
            "handler must cover both supported event types",
        ));
    }
    if template.source_revisions.len() > 1000 {
        return Err(fail("E_BUDGET", "handler source manifest count exceeded"));
    }
    for source in &template.source_revisions {
        if !id(&source.name)
            || !id(&source.revision)
            || !view_registration::is_definition_digest(&source.digest)
        {
            return Err(fail(
                "E_SOURCE_REVISION",
                "handler sources require bounded labels and SHA256 digests",
            ));
        }
    }
    recipe_sources(
        &template.recipe,
        template.protocol == HANDLER_TEMPLATE_PROTOCOL,
    )
    .map(|_| ())
}
pub fn handler_source_revision(
    template: &CompiledHandlerTemplate,
) -> Result<SourceRevision, Diagnostic> {
    fields(template)?;
    let label = identity::digest(
        "weave-source-label-v1",
        &("handler_template", &template.name),
        MAX_HANDLER_BYTES,
    )?;
    let mut events = template.event_types.clone();
    events.sort();
    Ok(SourceRevision {
        name: format!("weave:handler-template:{}", &label[7..]),
        revision: template.revision.clone(),
        digest: identity::digest(
            "weave-handler-template-source-v1",
            &(
                &template.protocol,
                &template.name,
                &template.revision,
                &template.input,
                events,
                &template.recipe,
                &template.output_slot,
            ),
            MAX_HANDLER_BYTES,
        )?,
    })
}
pub fn handler_definition_digest(template: &CompiledHandlerTemplate) -> Result<String, Diagnostic> {
    fields(template)?;
    let sources = algebra::merge_source_revisions(&template.source_revisions, &[])?;
    let mut events = template.event_types.clone();
    events.sort();
    identity::digest(
        "weave-handler-definition-v1",
        &(
            &template.format,
            &template.protocol,
            &template.name,
            &template.revision,
            &template.input,
            events,
            &template.recipe,
            &template.output_slot,
            sources,
        ),
        MAX_HANDLER_BYTES,
    )
}
pub fn seal_handler_template(
    mut template: CompiledHandlerTemplate,
) -> Result<CompiledHandlerTemplate, Diagnostic> {
    fields(&template)?;
    let own = handler_source_revision(&template)?;
    template.event_types.sort();
    let rule_sources = recipe_sources(
        &template.recipe,
        template.protocol == HANDLER_TEMPLATE_PROTOCOL,
    )?;
    template.source_revisions =
        algebra::merge_source_revisions(&template.source_revisions, &rule_sources)?;
    template.source_revisions =
        algebra::merge_source_revisions(&template.source_revisions, &[own])?;
    template.definition_digest = handler_definition_digest(&template)?;
    fields(&template)?;
    Ok(template)
}
pub fn validate_handler_template(template: &CompiledHandlerTemplate) -> Result<(), Diagnostic> {
    fields(template)?;
    if !view_registration::is_definition_digest(&template.definition_digest)
        || seal_handler_template(template.clone())? != *template
    {
        return Err(fail(
            "E_HANDLER_TEMPLATE",
            "handler canonical identity or source manifest mismatch",
        ));
    }
    Ok(())
}
