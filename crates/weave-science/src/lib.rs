//! Native experiments use the runtime's authorized query boundary and immutable revisions.
//! Analysis never accepts a caller-supplied graph as a substitute for an authorized query.
mod algorithms;
mod input;
pub use input::parse_request;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use weave_contract::{Command, CommandResult, Coverage, GraphData, Program, VERSION};
use weave_engine::{Engine, Error, HostContext, Result};

pub const SCIENCE_VERSION: &str = "0.1.0";
pub const MAX_INPUT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024 * 1024;

fn main_branch() -> String {
    "main".into()
}
fn directed() -> bool {
    true
}
fn damping() -> f64 {
    0.85
}
fn tolerance() -> f64 {
    1e-10
}
fn iterations() -> usize {
    100
}
fn vector_property() -> String {
    "vector".into()
}
fn neighbors() -> usize {
    10
}

/// The JSON host interface is versioned independently from the compiler/runtime contract.
#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Import {
        graph_id: String,
        #[serde(default = "main_branch")]
        branch_id: String,
        #[serde(default)]
        expected_head: Option<String>,
        data: Box<GraphData>,
    },
    Execute {
        program: Program,
    },
    Analyze {
        program: Program,
        #[serde(default)]
        result_index: usize,
        analysis: Analysis,
        #[serde(default)]
        allow_partial: bool,
        #[serde(default)]
        limits: Limits,
    },
    Capabilities,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentMode {
    Weak,
    Strong,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    Cosine,
    Euclidean,
}

/// Topology samples positive assertions only, with parallel assertions kept as parallel edges.
/// An absent valid_at intentionally analyzes the union of the selected time intervals.
#[derive(Debug, Deserialize)]
#[serde(tag = "algorithm", rename_all = "snake_case", deny_unknown_fields)]
pub enum Analysis {
    Degree {
        #[serde(default)]
        valid_at: Option<i64>,
    },
    Components {
        mode: ComponentMode,
        #[serde(default)]
        valid_at: Option<i64>,
    },
    ShortestPaths {
        source: String,
        #[serde(default = "directed")]
        directed: bool,
        #[serde(default)]
        target: Option<String>,
        #[serde(default)]
        valid_at: Option<i64>,
    },
    Pagerank {
        #[serde(default = "damping")]
        damping: f64,
        #[serde(default = "tolerance")]
        tolerance: f64,
        #[serde(default = "iterations")]
        max_iterations: usize,
        #[serde(default)]
        valid_at: Option<i64>,
    },
    NearestVectors {
        #[serde(default = "vector_property")]
        property: String,
        space_id: String,
        query: Vec<f64>,
        metric: Metric,
        #[serde(default = "neighbors")]
        k: usize,
    },
}

impl Analysis {
    pub(crate) fn valid_at(&self) -> Option<i64> {
        match self {
            Self::Degree { valid_at }
            | Self::Components { valid_at, .. }
            | Self::ShortestPaths { valid_at, .. }
            | Self::Pagerank { valid_at, .. } => *valid_at,
            Self::NearestVectors { .. } => None,
        }
    }
}

/// Limits bound materialized science input and algorithm work; runtime query budgets also apply.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub max_nodes: usize,
    pub max_edges: usize,
    pub max_work: u64,
    pub max_vector_dimensions: usize,
    pub max_output_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_nodes: 100_000,
            max_edges: 1_000_000,
            max_work: 100_000_000,
            max_vector_dimensions: 16_384,
            max_output_bytes: 16 * 1024 * 1024,
        }
    }
}
impl Limits {
    fn validate(&self) -> Result<()> {
        if self.max_nodes > 1_000_000
            || self.max_edges > 5_000_000
            || self.max_work > 1_000_000_000
            || self.max_vector_dimensions > 65_536
            || self.max_output_bytes > MAX_OUTPUT_BYTES
        {
            return Err(failure(
                "E_SCIENCE_LIMIT",
                "requested limit exceeds the host maximum",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct Response {
    pub ok: bool,
    pub science_version: &'static str,
    pub contract_version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Error>,
}
impl Response {
    pub fn success(result: Value) -> Self {
        Self {
            ok: true,
            science_version: SCIENCE_VERSION,
            contract_version: VERSION,
            result: Some(result),
            error: None,
        }
    }
    pub fn error(error: Error) -> Self {
        Self {
            ok: false,
            science_version: SCIENCE_VERSION,
            contract_version: VERSION,
            result: None,
            error: Some(error),
        }
    }
}

pub(crate) fn failure(code: &str, message: &str) -> Error {
    Error {
        code: code.into(),
        message: message.into(),
    }
}

/// The embedding host supplies authority, independently of untrusted experiment requests.
pub struct ScienceSession {
    engine: Engine,
    host: HostContext,
}
impl ScienceSession {
    pub fn new(engine: Engine, host: HostContext) -> Self {
        Self { engine, host }
    }
    pub fn open(path: impl AsRef<Path>, host: HostContext) -> Result<Self> {
        Ok(Self::new(Engine::open(path)?, host))
    }
    pub fn memory(host: HostContext) -> Result<Self> {
        Ok(Self::new(Engine::memory()?, host))
    }

    pub fn handle(&mut self, request: Request) -> Result<Value> {
        let mutating_execute = matches!(&request, Request::Execute { program } if program.commands.iter().any(|command| matches!(command, Command::Commit { .. } | Command::CommitBatch { .. })));
        let output_limit = match &request {
            Request::Analyze { limits, .. } => {
                limits.validate()?;
                limits.max_output_bytes
            }
            _ => Limits::default().max_output_bytes,
        };
        let result = match request {
            Request::Capabilities => capabilities(),
            Request::Import {
                graph_id,
                branch_id,
                expected_head,
                data,
            } => {
                let program = Program {
                    version: VERSION.into(),
                    source_revisions: vec![],
                    commands: vec![Command::Commit {
                        graph_id,
                        branch_id,
                        expected_head,
                        data: *data,
                    }],
                };
                json!({"results":self.engine.execute(&program, &self.host)?})
            }
            Request::Execute { program } => {
                json!({"results":self.engine.execute(&program, &self.host)?})
            }
            Request::Analyze {
                program,
                result_index,
                analysis,
                allow_partial,
                limits,
            } => {
                // Reject every mutation before executing any prefix of the supplied program.
                if program.commands.iter().any(|command| {
                    matches!(
                        command,
                        Command::Commit { .. } | Command::CommitBatch { .. }
                    )
                }) {
                    return Err(failure(
                        "E_SCIENCE_READ_ONLY",
                        "analysis programs must contain only read operations",
                    ));
                }
                let mut results = self.engine.execute(&program, &self.host)?;
                if result_index >= results.len() {
                    return Err(failure(
                        "E_SCIENCE_RESULT",
                        "result_index is outside the command results",
                    ));
                }
                let input = match results.swap_remove(result_index) {
                    CommandResult::Queried { result } => result,
                    _ => {
                        return Err(failure(
                            "E_SCIENCE_RESULT",
                            "selected command result is not a graph query",
                        ))
                    }
                };
                if input.coverage != Coverage::Complete && !allow_partial {
                    return Err(failure("E_SCIENCE_PARTIAL", "analysis requires complete authorized input; explicitly opt into partial coverage"));
                }
                let (output, semantics) = algorithms::analyze(&input, &analysis, &limits)?;
                json!({"input":input,"analysis":output,"semantics":semantics})
            }
        };
        // Include the wire envelope in the output budget, before the caller emits any bytes.
        let envelope = Response::success(result);
        if serde_json::to_vec(&envelope)?.len() > output_limit {
            if mutating_execute {
                return Err(failure("E_SCIENCE_OUTPUT_AFTER_EXECUTION", "execution succeeded but its response exceeds the byte budget; durable commits remain; query current heads before retrying"));
            }
            return Err(failure(
                "E_SCIENCE_OUTPUT",
                "serialized response exceeds its byte budget",
            ));
        }
        Ok(envelope.result.expect("success envelope contains a result"))
    }
    pub fn respond(&mut self, request: Request) -> Response {
        match self.handle(request) {
            Ok(value) => Response::success(value),
            Err(error) => Response::error(error),
        }
    }
}

pub fn capabilities() -> Value {
    json!({
        "engine":"weave-engine", "engine_contract_version":VERSION,
        "operations":["import","execute","analyze","capabilities"],
        "algorithms":["degree","components","shortest_paths","pagerank","nearest_vectors"],
        "component_modes":["weak","strong"], "vector_metrics":["cosine","euclidean"],
        "clustering":"existing Program graph expression: cluster",
        "limits":Limits::default(), "max_input_bytes":MAX_INPUT_BYTES,
        "limit_maxima":{"max_nodes":1000000,"max_edges":5000000,"max_work":1000000000,"max_vector_dimensions":65536,"max_output_bytes":MAX_OUTPUT_BYTES},
        "snapshot_semantics":"immutable engine revisions; expected_head null creates a new branch only",
        "authorization":"host actor and graph write grants; every analysis reexecutes a runtime-authorized read-only Program",
        "edge_semantics":"positive directed multigraph; self-loops retained; unweighted hops and multiplicity-weighted PageRank",
        "time_semantics":"optional analysis.valid_at selects half-open valid intervals and retains selected nodes; absent means interval union",
        "vector_semantics":"exact numeric node properties within one explicit space; no implicit encoder or cross-space mapping",
        "determinism":"lexical node/neighbor order and ID tie-breaks; repeatable floating-point operation order; compare platform math using tolerances; no wall clock or randomness in algorithms",
        "partial_coverage":"rejected by default; allow_partial explicitly retains original coverage and diagnostics"
        ,"work_budget_semantics":"abstract node/edge visits and vector arithmetic; sorting remains bounded by node/edge limits; not a CPU instruction or wall time quota"
    })
}
