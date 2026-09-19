//! Finite, range-restricted binary rules. Signed atoms are evidence, not absence.
use crate::{Diagnostic, Polarity};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleSet {
    pub id: String,
    pub revision: String,
    pub rules: Vec<Rule>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub head: RuleAtom,
    pub body: Vec<RuleAtom>,
    #[serde(default)]
    pub allow_cross_space: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleAtom {
    pub predicate: String,
    pub from: RuleTerm,
    pub to: RuleTerm,
    #[serde(default)]
    pub polarity: Polarity,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuleTerm {
    Variable { name: String },
    Node { id: String },
}
/// Trusted runtime bounds, never authority embedded in a rule module.
#[derive(Debug, Clone)]
pub struct RuleBudget {
    pub max_steps: usize,
    pub max_rounds: usize,
    pub max_facts: usize,
    pub max_derivations: usize,
}
fn valid_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
}
pub fn validate_rule_set(set: &RuleSet) -> Vec<Diagnostic> {
    let mut errors = Vec::new();
    let mut emit = |code: &str, message: &str| {
        errors.push(Diagnostic {
            code: code.into(),
            message: message.into(),
        })
    };
    if !valid_name(&set.id) || !valid_name(&set.revision) {
        emit(
            "E_RULE_ID",
            "Rule module requires bounded nonempty ID and revision",
        );
    }
    if set.rules.is_empty() || set.rules.len() > 64 {
        emit("E_RULE_SHAPE", "Rule module must contain 1 to 64 rules");
        return errors;
    }
    let mut names = BTreeSet::new();
    for rule in &set.rules {
        if !valid_name(&rule.id) || !names.insert(&rule.id) {
            emit("E_RULE_ID", "Rule IDs must be unique, bounded and nonempty");
        }
        if rule.body.is_empty() || rule.body.len() > 8 {
            emit(
                "E_RULE_SHAPE",
                "A rule requires 1 to 8 explicit evidence atoms",
            );
            continue;
        }
        let mut variables = BTreeSet::new();
        for atom in &rule.body {
            for term in [&atom.from, &atom.to] {
                if let RuleTerm::Variable { name } = term {
                    variables.insert(name);
                }
            }
        }
        if variables.len() > 16 {
            emit("E_RULE_SHAPE", "Rule has more than 16 variables");
        }
        for term in [&rule.head.from, &rule.head.to] {
            if let RuleTerm::Variable { name } = term {
                if !variables.contains(name) {
                    emit(
                        "E_RULE_RANGE",
                        "Every head variable must occur in an evidence atom",
                    );
                }
            }
        }
        for atom in rule.body.iter().chain(std::iter::once(&rule.head)) {
            if !valid_name(&atom.predicate) {
                emit("E_RULE_ID", "Predicate must be a bounded nonempty literal");
            }
            for term in [&atom.from, &atom.to] {
                let text = match term {
                    RuleTerm::Variable { name } => name,
                    RuleTerm::Node { id } => id,
                };
                if !valid_name(text) {
                    emit(
                        "E_RULE_ID",
                        "Variables and node constants must be bounded nonempty identifiers",
                    );
                }
            }
        }
    }
    errors
}
