//! Canonical host view artifacts. Identities are descriptive; they never install authority.
use crate::*;
use serde::{Deserialize, Serialize};

pub const VIEW_TEMPLATE_FORMAT: &str = "weave-view-registration/1";
pub const VIEW_TEMPLATE_PROTOCOL: &str = "0.17.0";
const LIMIT: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AcceptedGraphSelection {
    pub view_id: String,
    pub decision_id: String,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ViewReadTime {
    Fixed,
    Tick { valid_at: i64 },
}
impl<'de> Deserialize<'de> for ViewReadTime {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
        enum Wire {
            Fixed {},
            Tick { valid_at: i64 },
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::Fixed {} => Self::Fixed,
            Wire::Tick { valid_at } => Self::Tick { valid_at },
        })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CurrentViewSelection {
    pub view_id: String,
    pub definition_digest: String,
    pub time: ViewReadTime,
}
/// Serialization is byte-compatible with the existing native definition clock.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ViewClock {
    Fixed,
    Tick,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CompiledViewTemplate {
    pub format: String,
    pub protocol: String,
    pub name: String,
    pub revision: String,
    pub expression: GraphExpression,
    pub clock: ViewClock,
    pub source_revisions: Vec<SourceRevision>,
    pub definition_digest: String,
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
pub fn is_definition_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|h| {
        h.len() == 64
            && h.bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    })
}
fn fields(template: &CompiledViewTemplate) -> Result<(), Diagnostic> {
    identity::digest("weave-view-template-bound", template, LIMIT)?;
    if template.format != VIEW_TEMPLATE_FORMAT
        || ![VIEW_TEMPLATE_PROTOCOL, "0.16.0"].contains(&template.protocol.as_str())
    {
        return Err(fail(
            "E_VIEW_TEMPLATE",
            "unsupported registration artifact format or protocol",
        ));
    }
    if !id(&template.name) || !id(&template.revision) {
        return Err(fail(
            "E_VIEW_TEMPLATE",
            "template name/revision must be bounded identifiers",
        ));
    }
    if template.source_revisions.len() > 1000 {
        return Err(fail("E_BUDGET", "template source manifest count exceeded"));
    }
    for source in &template.source_revisions {
        if !id(&source.name) || !id(&source.revision) || !is_definition_digest(&source.digest) {
            return Err(fail(
                "E_SOURCE_REVISION",
                "template sources require bounded labels and canonical SHA-256 digests",
            ));
        }
    }
    let mut expression = &template.expression;
    for _ in 0..=32 {
        match expression {
            GraphExpression::Query { query } => {
                if !id(&query.graph_id)
                    || !id(&query.branch_id)
                    || query.revision.is_some()
                    || query.include_metadata
                    || query.max_depth > 32
                {
                    return Err(fail("E_VIEW_TEMPLATE","template Query must select one bounded live handle without metadata expansion"));
                }
                return Ok(());
            }
            GraphExpression::Filter { input, .. } => expression = input,
            _ => {
                return Err(fail(
                    "E_VIEW_TEMPLATE",
                    "first template profile accepts only standalone live Query/Filter recipes",
                ))
            }
        }
    }
    Err(fail("E_BUDGET", "template expression depth exceeded"))
}
/// Own source identity uses the normalized compiled recipe, not source formatting.
/// The framed kind/name hash gives templates a disjoint fixed-size source-label namespace.
pub fn template_source_revision(
    template: &CompiledViewTemplate,
) -> Result<SourceRevision, Diagnostic> {
    fields(template)?;
    let label = identity::digest(
        "weave-source-label-v1",
        &("view_template", &template.name),
        LIMIT,
    )?;
    Ok(SourceRevision {
        name: format!("weave:view-template:{}", &label[7..]),
        revision: template.revision.clone(),
        digest: identity::digest(
            "weave-view-template-source-v1",
            &(
                &template.protocol,
                &template.name,
                &template.revision,
                &template.expression,
                &template.clock,
            ),
            LIMIT,
        )?,
    })
}
/// Hashes normalized expression order/literals and canonical conflict-checked source identities.
/// The host-selected instance ID and the digest field itself are excluded.
pub fn definition_digest(template: &CompiledViewTemplate) -> Result<String, Diagnostic> {
    fields(template)?;
    let sources = algebra::merge_source_revisions(&template.source_revisions, &[])?;
    identity::digest(
        "weave-view-definition-v1",
        &(
            &template.format,
            &template.protocol,
            &template.name,
            &template.revision,
            &template.expression,
            &template.clock,
            sources,
        ),
        LIMIT,
    )
}
/// Compiler construction helper. Adds the recipe's own source manifest and seals identity.
pub fn seal_template(
    mut template: CompiledViewTemplate,
) -> Result<CompiledViewTemplate, Diagnostic> {
    let source = template_source_revision(&template)?;
    template.source_revisions =
        algebra::merge_source_revisions(&template.source_revisions, &[source])?;
    template.definition_digest = definition_digest(&template)?;
    fields(&template)?;
    Ok(template)
}
/// Validate before host installation. Canonical source order/dedup is part of the artifact.
pub fn validate_template(template: &CompiledViewTemplate) -> Result<(), Diagnostic> {
    fields(template)?;
    if !is_definition_digest(&template.definition_digest) {
        return Err(fail(
            "E_VIEW_TEMPLATE",
            "canonical definition digest required",
        ));
    }
    let sealed = seal_template(template.clone())?;
    if sealed != *template {
        return Err(fail(
            "E_VIEW_TEMPLATE",
            "registration identity or canonical source manifest mismatch",
        ));
    }
    Ok(())
}
