//! The agent interface (B4, #40): MCP over the authenticated service.
//!
//! `POST /v1/mcp` is an MCP Streamable HTTP endpoint without sessions or
//! server-sent events: each POST carries one JSON-RPC message and gets one
//! JSON response (or `202` for a notification). The request is
//! authenticated like every other route, so the principal comes from its
//! credential and the tenant from the control store, and each message is
//! one audited request through the same driver as the rest of the API.
//! Nothing is cached between messages, so revoking a token, suspending an
//! agent or removing a membership applies to the next tool call.
//!
//! Every tool runs through [`run_program`], the path `POST /v1/execute`
//! takes: writes are admitted with minted IDs, run as the caller and
//! audited with their facts; reads are audited with the facts they cite.
//! The tool catalog, its text and the untrusted-data envelope come from
//! `weave-mcp`; nothing read from a store is placed outside `result`.
use super::{
    body_bytes, call, error_response, named_graphs, run_program, visible_manifest, Caller, Ctx,
    Done, Service,
};
use crate::compile::Compiler;
use crate::error::{codes, ApiError};
use crate::ids::mint_record_id;
use crate::tenant::TenantState;
use crate::Limits;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use axum::Extension;
use serde_json::{json, Value};
use weave_contract::CommandResult;
use weave_mcp::agent::{
    compile_request, describe_program, parse_compile, parse_describe, plan_assert, plan_retract,
    summarize, visible_spaces, MAX_SPACE_GRAPHS,
};
use weave_mcp::{Message, Tool, AGENT, PROTOCOL_VERSIONS};

/// The audit operation of one message.
fn operation(message: &Message) -> &'static str {
    match message {
        Message::Request { method, params, .. } => {
            if let Some(tool) = weave_mcp::requested_tool(&AGENT, method, params) {
                return tool.operation();
            }
            match method.as_str() {
                "initialize" => "mcp.initialize",
                "tools/list" => "mcp.tools_list",
                "ping" => "mcp.ping",
                "tools/call" => "mcp.unknown_tool",
                _ => "mcp.unknown_method",
            }
        }
        Message::Notification => "mcp.notification",
        Message::Invalid(_) => "mcp.invalid",
    }
}

pub(super) async fn post(
    State(service): State<Service>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    // A browser page can reach a service on the user's network; MCP asks
    // servers to refuse cross-origin requests. Agents are not browsers.
    if headers.contains_key(header::ORIGIN) {
        return error_response(ApiError::new(
            codes::MCP_ORIGIN,
            "requests with an Origin header are refused",
        ));
    }
    if let Some(version) = headers.get("mcp-protocol-version") {
        let known = version
            .to_str()
            .is_ok_and(|v| PROTOCOL_VERSIONS.contains(&v));
        if !known {
            return error_response(ApiError::new(
                codes::MCP_PROTOCOL,
                format!(
                    "unsupported MCP protocol version; supported: {}",
                    PROTOCOL_VERSIONS.join(", ")
                ),
            ));
        }
    }
    let bytes = match body_bytes(body) {
        Ok(bytes) => bytes,
        Err(error) => return error_response(error),
    };
    let message = weave_mcp::parse_message(&bytes);
    let limits = service.limits().clone();
    call(
        &service,
        caller,
        operation(&message),
        true,
        Box::new(move |state, ctx| {
            let mut done = Done::default();
            match message {
                Message::Request { id, method, params } => {
                    let response =
                        weave_mcp::respond(&AGENT, id, &method, &params, |tool, args| {
                            call_tool(state, ctx, tool, args, &limits, &mut done)
                        });
                    if response.get("error").is_some() {
                        done.failure = Some(codes::INPUT.into());
                    }
                    done.body = response;
                }
                Message::Notification => {
                    done.status = Some(StatusCode::ACCEPTED);
                    done.empty = true;
                }
                Message::Invalid(response) => {
                    done.status = Some(StatusCode::BAD_REQUEST);
                    done.failure = Some(codes::INPUT.into());
                    done.body = response;
                }
            }
            Ok(done)
        }),
    )
    .await
}

/// Run one tool call as the caller and return its `CallToolResult`. A
/// failure is an `isError` result whose code the audit entry records.
fn call_tool(
    state: &mut TenantState,
    ctx: &Ctx,
    tool: Tool,
    arguments: Value,
    limits: &Limits,
    done: &mut Done,
) -> Value {
    let mut named = Vec::new();
    named_graphs(&arguments, &mut named, &mut 100_000);
    let outcome = run_tool(state, ctx, tool, arguments, limits, done);
    if let Err(error) = &outcome {
        done.failure = Some(error.code.clone());
        // As for `POST /v1/execute`: a failure records the graphs the
        // request named, never what the engine knows.
        done.graphs = named;
        done.revisions.clear();
        done.facts.clear();
    }
    weave_mcp::envelope(
        tool,
        outcome.map_err(|error| weave_engine::Error {
            code: error.code,
            message: error.message,
        }),
    )
}

fn input(error: weave_engine::Error) -> ApiError {
    ApiError::new(&error.code, error.message)
}

/// The revision a write command committed (or found unchanged).
fn written_revision(results: &[CommandResult]) -> Value {
    results
        .first()
        .map(|result| match result {
            CommandResult::Committed { revision, .. } | CommandResult::Unchanged { revision } => {
                json!(revision)
            }
            _ => Value::Null,
        })
        .unwrap_or(Value::Null)
}

fn run_tool(
    state: &mut TenantState,
    ctx: &Ctx,
    tool: Tool,
    arguments: Value,
    limits: &Limits,
    done: &mut Done,
) -> Result<Value, ApiError> {
    let max_commands = limits.max_commands;
    match tool {
        Tool::Compile => compile(state, ctx, arguments, limits.compiler.as_ref(), done),
        Tool::AssertFact => {
            let human = arguments["provenance"]["method"]["kind"] == "human";
            let planned = plan_assert(arguments, || {
                mint_record_id(&ctx.id_key, &ctx.tenant_id, &ctx.principal)
            })
            .map_err(input)?;
            // A method is a claim, not an attestation; still, an agent's
            // own facts never claim to be a person's. Checked after the
            // arguments parse, so malformed input is always E_MCP_INPUT.
            if ctx.agent && human {
                return Err(ApiError::new(
                    weave_mcp::codes::CLAIM,
                    "an agent states a model, rule or import method",
                ));
            }
            let results = run_program(state, ctx, &planned.program(), max_commands, done)?;
            Ok(json!({
                "revision": written_revision(&results),
                "facts": planned.facts,
                "results": results,
            }))
        }
        Tool::RetractFact => {
            let planned = plan_retract(arguments).map_err(input)?;
            let results = run_program(state, ctx, &planned.program(), max_commands, done)?;
            Ok(json!({
                "revision": written_revision(&results),
                "facts": planned.facts,
                "results": results,
            }))
        }
        Tool::DescribeSchema => {
            let arguments = parse_describe(arguments).map_err(input)?;
            let graphs = state.engine.list_graphs(&ctx.host)?;
            let groups = state.engine.registry_nameable_groups(&ctx.host)?;
            let Some(graph_id) = arguments.graph_id else {
                done.graphs = graphs.iter().map(|g| g.graph_id.clone()).collect();
                let listed: Vec<Value> = graphs
                    .iter()
                    .map(|g| {
                        json!({"graph_id": g.graph_id, "permission": g.permission,
                            "branches": g.branches})
                    })
                    .collect();
                return Ok(json!({"graphs": listed, "reader_groups": groups}));
            };
            let permission = graphs
                .iter()
                .find(|g| g.graph_id == graph_id)
                .map(|g| json!(g.permission))
                .unwrap_or(Value::Null);
            let program = describe_program(&graph_id, &arguments.branch_id);
            let results = run_program(state, ctx, &program, max_commands, done)?;
            let first = results
                .first()
                .map(|r| serde_json::to_value(r).map_err(|_| ApiError::internal()))
                .transpose()?
                .unwrap_or(Value::Null);
            Ok(summarize(
                &graph_id,
                &arguments.branch_id,
                &first["result"],
                permission,
                json!(groups),
            ))
        }
        Tool::Query
        | Tool::Expand
        | Tool::AsOf
        | Tool::WhatChanged
        | Tool::HybridSearch
        | Tool::Explain => {
            let expression = weave_mcp::expression(tool, arguments).map_err(input)?;
            let program = weave_mcp::program(expression);
            let results = run_program(state, ctx, &program, max_commands, done)?;
            Ok(json!({"results": results}))
        }
    }
}

/// The `compile` tool: the caller's source compiled by the configured
/// compiler under the untrusted policy (and read-only when the caller can
/// write no graph), against its visible manifest with the embedding spaces
/// of up to [`MAX_SPACE_GRAPHS`] readable graphs. It returns the compiler's
/// response and runs nothing.
fn compile(
    state: &mut TenantState,
    ctx: &Ctx,
    arguments: Value,
    compiler: Option<&Compiler>,
    done: &mut Done,
) -> Result<Value, ApiError> {
    let arguments = parse_compile(arguments).map_err(input)?;
    let Some(compiler) = compiler else {
        return Err(ApiError::new(
            codes::UNSUPPORTED,
            "this service has no compiler configured",
        ));
    };
    let mut manifest = visible_manifest(state, ctx, done)?;
    let readable = |entry: &Value| {
        matches!(
            entry["permission"].as_str(),
            Some("read" | "write" | "administer")
        )
    };
    let wanted: Vec<String> = match &arguments.graphs {
        Some(graphs) => graphs.clone(),
        None => manifest["graphs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|g| readable(g))
            .filter_map(|g| g["graph_id"].as_str().map(String::from))
            .take(MAX_SPACE_GRAPHS)
            .collect(),
    };
    let can_write = manifest["graphs"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|g| matches!(g["permission"].as_str(), Some("write" | "administer")));
    for entry in manifest["graphs"].as_array_mut().into_iter().flatten() {
        let Some(graph_id) = entry["graph_id"].as_str().map(String::from) else {
            continue;
        };
        if !readable(entry) || !wanted.contains(&graph_id) {
            continue;
        }
        // Space descriptors carry no fact provenance, so this read cites
        // no facts; a graph whose spaces cannot be read is left unknown.
        let program = describe_program(&graph_id, "main");
        if let Ok(results) = state.engine.execute(&program, &ctx.host) {
            if let Some(CommandResult::Queried { result }) = results.first() {
                let result = serde_json::to_value(result).map_err(|_| ApiError::internal())?;
                entry["embedding_spaces"] = Value::Array(visible_spaces(&result));
            }
        }
    }
    let read_only = !can_write;
    let request = compile_request(&arguments, read_only, manifest);
    let bytes = serde_json::to_vec(&request).map_err(|_| ApiError::internal())?;
    let response = compiler.compile(&bytes, read_only)?;
    Ok(json!({
        "policy": {"untrusted": true, "read_only": read_only},
        "response": response,
    }))
}
