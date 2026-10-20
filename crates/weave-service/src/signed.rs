//! Signed admission for delegated agent requests (ADR-0007 section 8),
//! reusing the `weave-policy` capability-chain verifier.
//!
//! `POST /v1/admit` with `Authorization: Weave-Signed` carries
//! `{"proof": AdmissionProof, "program": "<program JSON text>"}`. The signed
//! body is the exact UTF-8 bytes of `program`. A request is admitted only
//! when both hold:
//!
//! 1. **The chain is valid under `weave-policy`**, with the tenant's own
//!    audience, the trusted clock, the tenant's revoked keys, and a root
//!    derived from the registry: the chain's first issuer key must be bound
//!    to an active user, and the root grants that user's current graph
//!    permissions (Discover, Read and Traverse, and Publish for Write) on the
//!    scopes the root capability names.
//! 2. **The registry agrees.** The leaf subject key is bound to a principal
//!    that can act now; every key in the chain is bound to that principal or
//!    a principal of its ownership chain; and the first issuer is the chain's
//!    root user. The program then runs as that principal, so the registry
//!    authorizes it again with the agent ceiling. A capability can only
//!    narrow what the registry allows.
//!
//! A signed program is one operation: all reads of one graph branch
//! (`read`, which needs Read and Traverse in the leaf scope) or all writes of
//! one graph branch (`publish`). Each request's nonce is reserved until the
//! request expires, so a replay fails `E_REPLAY`; a failed request still
//! consumes its nonce.
use crate::error::{codes, ApiError};
use serde_json::Value;
use std::collections::BTreeSet;
use weave_contract::{Command, Program};
use weave_engine::Permission;
use weave_policy::{Action, Operation};

/// The `Authorization` scheme of signed requests.
pub const SCHEME: &str = "Weave-Signed";
/// The only route that accepts signed requests.
pub const ROUTE: &str = "/v1/admit";
/// The `policy_revision` every capability chain for this service names.
pub const POLICY_REVISION: &str = "weave-service/1";
/// The admission policy epoch. Revocation is live through the registry and
/// key bindings, so the epoch never needs to advance.
pub const POLICY_EPOCH: &str = "weave-service/1";

/// The `weave-policy` actions a registry permission grants a root.
pub fn actions(permission: Permission) -> BTreeSet<Action> {
    let mut out = BTreeSet::from([Action::Discover]);
    if permission >= Permission::Read {
        out.extend([Action::Read, Action::Traverse, Action::Delegate]);
    }
    if permission >= Permission::Write {
        out.insert(Action::Publish);
    }
    out
}

fn collect(value: &Value, key: &str, out: &mut BTreeSet<String>, budget: &mut usize) {
    if *budget == 0 {
        return;
    }
    *budget -= 1;
    match value {
        Value::Object(map) => {
            for (k, child) in map {
                if k == key {
                    if let Value::String(s) = child {
                        out.insert(s.clone());
                    }
                }
                collect(child, key, out, budget);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect(child, key, out, budget);
            }
        }
        _ => {}
    }
}

fn single(program: &str) -> ApiError {
    ApiError::new(
        codes::UNSUPPORTED,
        format!("a signed program is all reads or all writes of one graph branch: {program}"),
    )
}

/// The one operation a signed program performs. Graphs and branches are
/// read from the decoded program re-encoded, so defaulted branches appear
/// as `main` and nothing the program names escapes the check.
pub fn operation(program: &Program) -> Result<Operation, ApiError> {
    let raw = serde_json::to_value(program).map_err(|_| ApiError::internal())?;
    let raw = &raw;
    let writes = program
        .commands
        .iter()
        .filter(|c| matches!(c, Command::Write { .. }))
        .count();
    if writes > 0 && writes != program.commands.len() {
        return Err(single("reads and writes are mixed"));
    }
    let mut graphs = BTreeSet::new();
    let mut branches = BTreeSet::new();
    let mut budget = 100_000;
    collect(raw, "graph_id", &mut graphs, &mut budget);
    let mut budget = 100_000;
    collect(raw, "branch_id", &mut branches, &mut budget);
    if graphs.len() != 1 {
        return Err(single("it must name exactly one graph"));
    }
    if branches.len() != 1 {
        return Err(single("it must name exactly one branch"));
    }
    Ok(Operation {
        action: if writes > 0 {
            Action::Publish
        } else {
            Action::Read
        },
        graph_id: graphs.into_iter().next().unwrap_or_default(),
        branch_id: branches.into_iter().next().unwrap_or_default(),
    })
}
