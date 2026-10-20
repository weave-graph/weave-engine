//! HTTP routes. Each handler authenticates (middleware), then runs one
//! closure on the caller's tenant that re-checks the principal against the
//! registry, does the engine work, and appends the audit entry before the
//! response leaves.
use crate::audit::{Access, FactAccess, NewEntry, Outcome, MAX_AUDIT_PAGE};
use crate::control::{TokenBinding, DEFAULT_TOKEN_TTL_MS};
use crate::error::{codes, ApiError, ErrorCode};
use crate::ids::{hex, mint_record_id, minted_for, random};
use crate::tenant::{Tenant, TenantState};
use crate::{standing, Service};
use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, QueryRejection};
use axum::extract::{DefaultBodyLimit, MatchedPath, Path, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, MethodRouter};
use axum::{Extension, Router};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;
use weave_contract::provenance::FactRef;
use weave_contract::{Command, CommandResult, FactChange, Program};
use weave_engine::{
    Engine, HostContext, Permission, PrincipalKind, RegistryChange, ADMINISTRATORS, REGISTRY_GRAPH,
};

/// The authenticated caller of one request.
#[derive(Clone)]
pub(crate) struct Caller {
    tenant: Arc<Tenant>,
    binding: TokenBinding,
    request_id: String,
}

/// Request ID, generated per request and returned as `x-request-id`.
#[derive(Clone)]
struct RequestId(String);

/// Identity of an authenticated response, for the request log.
#[derive(Clone)]
struct LogIdentity {
    tenant: String,
    principal: String,
}

/// What one handler's tenant work saw: the principal and its standing.
pub(crate) struct Ctx {
    principal: String,
    tenant_id: String,
    id_key: [u8; 32],
    host: HostContext,
    administrator: bool,
    /// The principal is an agent (B1): it acts with at most its owner's
    /// authority, and `on_behalf_of` is its ownership chain.
    agent: bool,
    on_behalf_of: Vec<String>,
    token_id: String,
    expires_at_ms: i64,
}

/// A successful operation and what it touched.
#[derive(Default)]
pub(crate) struct Done {
    status: Option<StatusCode>,
    body: Value,
    graphs: Vec<String>,
    revisions: Vec<String>,
    facts: Vec<FactAccess>,
}

impl Done {
    fn json(body: Value) -> Self {
        Done {
            body,
            ..Done::default()
        }
    }
}

/// A failed operation and the graph IDs the request itself named.
pub(crate) struct Failed {
    error: ApiError,
    graphs: Vec<String>,
}

impl From<ApiError> for Failed {
    fn from(error: ApiError) -> Self {
        Failed {
            error,
            graphs: vec![],
        }
    }
}
impl From<weave_engine::Error> for Failed {
    fn from(error: weave_engine::Error) -> Self {
        ApiError::from(error).into()
    }
}

type Work = dyn FnOnce(&mut TenantState, &Ctx) -> Result<Done, Failed> + Send;

struct Route {
    method: Method,
    path: &'static str,
    handler: MethodRouter<Service>,
}

fn routes() -> Vec<Route> {
    let route = |method: Method, path, handler| Route {
        method,
        path,
        handler,
    };
    vec![
        route(Method::GET, "/v1/health", get(health)),
        route(Method::GET, "/v1/whoami", get(whoami)),
        route(Method::POST, "/v1/execute", post(execute)),
        route(Method::GET, "/v1/graphs", get(list_graphs)),
        route(Method::POST, "/v1/graphs", post(create_graph)),
        route(Method::GET, "/v1/graphs/{graph_id}/acl", get(graph_acl)),
        route(Method::POST, "/v1/ids", post(mint_ids)),
        route(Method::POST, "/v1/registry", post(apply_registry)),
        route(
            Method::GET,
            "/v1/registry/principals/{id}",
            get(registry_principal),
        ),
        route(
            Method::GET,
            "/v1/registry/principals/{id}/memberships",
            get(registry_memberships),
        ),
        route(
            Method::GET,
            "/v1/registry/groups/{id}/members",
            get(registry_members),
        ),
        route(Method::GET, "/v1/tokens", get(list_tokens)),
        route(Method::POST, "/v1/tokens", post(issue_token)),
        route(
            Method::DELETE,
            "/v1/tokens/{token_id}",
            delete(revoke_token),
        ),
        route(Method::GET, "/v1/audit", get(audit_entries)),
        route(Method::GET, "/v1/audit/facts", get(audit_facts)),
    ]
}

pub(crate) fn endpoints() -> Vec<(Method, &'static str)> {
    routes().into_iter().map(|r| (r.method, r.path)).collect()
}

pub(crate) fn router(service: Service) -> Router {
    let mut router: Router<Service> = Router::new();
    let mut merged: Vec<(&'static str, MethodRouter<Service>)> = Vec::new();
    for r in routes() {
        match merged.iter_mut().find(|(p, _)| *p == r.path) {
            Some((_, existing)) => {
                let current = std::mem::take(existing);
                *existing = current.merge(r.handler);
            }
            None => merged.push((r.path, r.handler)),
        }
    }
    for (path, handler) in merged {
        router = router.route(path, handler.fallback(method_not_allowed));
    }
    let max_body = service.limits().max_body_bytes;
    router
        .fallback(not_found)
        .layer(DefaultBodyLimit::max(max_body))
        .layer(middleware::from_fn_with_state(
            service.clone(),
            authenticate,
        ))
        .layer(middleware::from_fn_with_state(service.clone(), log_request))
        .with_state(service)
}

// ---------------------------------------------------------------- middleware

async fn log_request(State(service): State<Service>, mut request: Request, next: Next) -> Response {
    let started = Instant::now();
    let request_id = RequestId(hex(&random::<8>()));
    let method = request.method().to_string();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| "unmatched".into());
    request.extensions_mut().insert(request_id.clone());
    let mut response = next.run(request).await;
    if let Ok(value) = HeaderValue::from_str(&request_id.0) {
        response.headers_mut().insert("x-request-id", value);
    }
    let identity = response.extensions().get::<LogIdentity>().cloned();
    let code = response
        .extensions()
        .get::<ErrorCode>()
        .map(|c| c.0.clone());
    service.log().event(
        "info",
        "request",
        json!({
            "request_id": request_id.0,
            "method": method,
            "route": route,
            "status": response.status().as_u16(),
            "code": code,
            "tenant": identity.as_ref().map(|i| i.tenant.clone()),
            "principal": identity.as_ref().map(|i| i.principal.clone()),
            "duration_ms": started.elapsed().as_millis() as u64,
        }),
    );
    response
}

/// The bearer token of a request: exactly one `Authorization` header with
/// the `Bearer` scheme. Anything else is no credential.
fn bearer(headers: &HeaderMap) -> Option<String> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    let value = value.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") || token.len() > 256 {
        return None;
    }
    Some(token.trim().to_string())
}

async fn authenticate(
    State(service): State<Service>,
    mut request: Request,
    next: Next,
) -> Response {
    let binding = match bearer(request.headers()) {
        None => None,
        Some(token) => match service
            .now()
            .and_then(|now| service.control(|c| c.authenticate(&token, now)))
        {
            Ok(binding) => binding,
            Err(error) => return error.into_response(),
        },
    };
    let Some(binding) = binding else {
        return ApiError::unauthenticated().into_response();
    };
    let tenant = match service.tenant(&binding.tenant_id) {
        Ok(tenant) => tenant,
        Err(_) => return ApiError::unauthenticated().into_response(),
    };
    let request_id = request
        .extensions()
        .get::<RequestId>()
        .map(|r| r.0.clone())
        .unwrap_or_default();
    let identity = LogIdentity {
        tenant: binding.tenant_id.clone(),
        principal: binding.principal.clone(),
    };
    request.extensions_mut().insert(Caller {
        tenant,
        binding,
        request_id,
    });
    let mut response = next.run(request).await;
    response.extensions_mut().insert(identity);
    response
}

async fn not_found() -> ApiError {
    ApiError::new(codes::NOT_FOUND, "no such route")
}

async fn method_not_allowed() -> ApiError {
    ApiError::new(codes::METHOD, "method not allowed on this route")
}

// ------------------------------------------------------------------- driver

/// Run `work` for `caller` on its tenant: re-check the principal against
/// the registry, run the work, append the audit entry (unless `audited` is
/// false) and build the response.
async fn call(
    service: &Service,
    caller: Caller,
    operation: &'static str,
    audited: bool,
    work: Box<Work>,
) -> Response {
    let clock = service.clone();
    let wait = service.limits().queue_timeout;
    let tenant = caller.tenant.clone();
    let log = service.log().clone();
    let outcome = tenant
        .run(wait, move |state| {
            let binding = &caller.binding;
            let host = HostContext::new(binding.principal.clone(), Vec::<String>::new());
            let mut entry = NewEntry {
                request_id: caller.request_id.clone(),
                principal: binding.principal.clone(),
                credential: binding.token_id.clone(),
                operation: operation.into(),
                ..NewEntry::default()
            };
            let standing = clock.now().map_err(Failed::from).and_then(|now| {
                standing(&state.engine, &binding.principal, now).map_err(Failed::from)
            });
            let result = match standing {
                Ok(Some(standing)) => {
                    entry.on_behalf_of = standing.on_behalf_of.clone();
                    let agent = standing.kind == PrincipalKind::Agent;
                    let administrator = if agent {
                        // Administer is not delegable: agents are never tenant
                        // administrators.
                        Ok(false)
                    } else {
                        is_administrator(&state.engine, &host)
                    };
                    match administrator {
                        Ok(administrator) => {
                            let ctx = Ctx {
                                principal: binding.principal.clone(),
                                tenant_id: binding.tenant_id.clone(),
                                id_key: caller.tenant.record.id_key,
                                host,
                                administrator,
                                agent,
                                on_behalf_of: standing.on_behalf_of,
                                token_id: binding.token_id.clone(),
                                expires_at_ms: binding.expires_at_ms,
                            };
                            work(state, &ctx)
                        }
                        Err(error) => Err(Failed::from(error)),
                    }
                }
                Ok(None) => Err(Failed::from(ApiError::unauthenticated())),
                Err(failed) => Err(failed),
            };
            let (status, body, code) = match result {
                Ok(done) => {
                    entry.outcome = Some(Outcome::Ok);
                    entry.graphs = done.graphs;
                    entry.revisions = done.revisions;
                    entry.facts = done.facts;
                    let body = serde_json::to_vec(&done.body).map_err(|_| ApiError::internal());
                    match body {
                        Ok(body) => (done.status.unwrap_or(StatusCode::OK), body, None),
                        Err(error) => (error.status, error.body(), Some(error.code)),
                    }
                }
                Err(failed) => {
                    let error = failed.error;
                    entry.outcome = Some(match error.code.as_str() {
                        "E_FORBIDDEN" | "E_UNAVAILABLE" | codes::UNAUTHENTICATED => Outcome::Denied,
                        _ => Outcome::Error,
                    });
                    entry.code = Some(error.code.clone());
                    entry.graphs = failed.graphs;
                    (error.status, error.body(), Some(error.code))
                }
            };
            if audited {
                let now = clock.now();
                let appended = now.and_then(|now| state.audit.append(&entry, &body, now));
                if appended.is_err() {
                    log.event(
                        "error",
                        "audit_append_failed",
                        json!({"request_id": entry.request_id}),
                    );
                    let error = ApiError::new(codes::AUDIT, "the request could not be audited");
                    return (error.status, error.body(), Some(error.code));
                }
            }
            (status, body, code)
        })
        .await;
    match outcome {
        Ok((status, body, code)) => {
            let mut response = (status, body).into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
            if status == StatusCode::UNAUTHORIZED {
                response
                    .headers_mut()
                    .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
            }
            if let Some(code) = code {
                response.extensions_mut().insert(ErrorCode(code));
            }
            response
        }
        Err(error) => error.into_response(),
    }
}

fn is_administrator(engine: &Engine, host: &HostContext) -> weave_engine::Result<bool> {
    Ok(engine
        .registry_memberships(&host.principal, host)?
        .iter()
        .any(|g| g == ADMINISTRATORS))
}

fn json_body<T: DeserializeOwned>(body: Result<Bytes, BytesRejection>) -> Result<T, ApiError> {
    let bytes = body_bytes(body)?;
    let bytes: &[u8] = if bytes.iter().all(u8::is_ascii_whitespace) {
        b"{}"
    } else {
        &bytes
    };
    weave_contract::decode(bytes)
        .map_err(|e| ApiError::input(format!("invalid request body: {}", e.message)))
}

fn body_bytes(body: Result<Bytes, BytesRejection>) -> Result<Bytes, ApiError> {
    body.map_err(|rejection| {
        if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            ApiError::new(codes::BODY_LIMIT, "the request body exceeds the limit")
        } else {
            ApiError::input("unreadable request body")
        }
    })
}

fn query<T>(query: Result<Query<T>, QueryRejection>) -> Result<T, ApiError> {
    query
        .map(|Query(q)| q)
        .map_err(|_| ApiError::input("invalid query parameters"))
}

fn check_id(id: &str, what: &str) -> Result<(), ApiError> {
    if id.is_empty() || id.len() > 512 {
        return Err(ApiError::input(format!(
            "{what} identifiers require 1 to 512 bytes"
        )));
    }
    Ok(())
}

fn error_response(error: ApiError) -> Response {
    error.into_response()
}

// ------------------------------------------------------------------ handlers

async fn health(State(service): State<Service>, Extension(caller): Extension<Caller>) -> Response {
    call(
        &service,
        caller,
        "health",
        false,
        Box::new(|_, _| {
            Ok(Done::json(json!({
                "status": "ok",
                "service": "weave-service",
                "version": env!("CARGO_PKG_VERSION"),
                "protocol": weave_contract::VERSION,
            })))
        }),
    )
    .await
}

async fn whoami(State(service): State<Service>, Extension(caller): Extension<Caller>) -> Response {
    call(
        &service,
        caller,
        "whoami",
        true,
        Box::new(|_, ctx| {
            Ok(Done::json(json!({
                "tenant": ctx.tenant_id,
                "principal": ctx.principal,
                "administrator": ctx.administrator,
                "agent": ctx.agent,
                "on_behalf_of": ctx.on_behalf_of,
                "credential": ctx.token_id,
                "expires_at_ms": ctx.expires_at_ms,
            })))
        }),
    )
    .await
}

/// Graph IDs a request names anywhere in its JSON: client input, recorded
/// on failure in place of anything the engine knows.
fn named_graphs(value: &Value, out: &mut Vec<String>, budget: &mut usize) {
    if *budget == 0 {
        return;
    }
    *budget -= 1;
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if key == "graph_id" {
                    if let Value::String(graph) = child {
                        if out.len() < 64 && !out.contains(graph) {
                            out.push(graph.clone());
                        }
                    }
                }
                named_graphs(child, out, budget);
            }
        }
        Value::Array(items) => {
            for child in items {
                named_graphs(child, out, budget);
            }
        }
        _ => {}
    }
}

fn kind_name<T: serde::Serialize>(kind: &T) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default()
}

/// The facts and inputs of a program's results, for the audit log.
fn touched(program: &Program, results: &[CommandResult], done: &mut Done) {
    for (command, result) in program.commands.iter().zip(results) {
        match (command, result) {
            (
                Command::Write {
                    graph_id, changes, ..
                },
                CommandResult::Committed { revision, .. },
            ) => {
                done.graphs.push(graph_id.clone());
                done.revisions.push(revision.clone());
                for change in changes {
                    let (kind, id, name) = match change {
                        FactChange::Assert { fact } => (
                            kind_name(&fact.kind()),
                            fact.key_id().into_owned(),
                            "assert",
                        ),
                        FactChange::Correct { fact, .. } => (
                            kind_name(&fact.kind()),
                            fact.key_id().into_owned(),
                            "correct",
                        ),
                        FactChange::Retract { target, .. } => {
                            (kind_name(&target.kind), target.id.clone(), "retract")
                        }
                    };
                    done.facts.push(FactAccess {
                        access: Access::Write,
                        graph_id: graph_id.clone(),
                        revision: revision.clone(),
                        kind,
                        fact_id: id,
                        change: Some(name.into()),
                    });
                }
            }
            (Command::Write { graph_id, .. }, CommandResult::Unchanged { revision }) => {
                done.graphs.push(graph_id.clone());
                done.revisions.push(revision.clone());
            }
            (_, CommandResult::Queried { result }) => {
                for input in &result.input_snapshots {
                    done.graphs.push(input.graph_id.clone());
                    done.revisions.push(input.revision.clone());
                }
                for entry in &result.fact_provenance {
                    let (graph_id, revision, kind, fact_id) = match &entry.fact {
                        FactRef::Assertion {
                            graph_id,
                            revision,
                            assertion_id,
                        } => (graph_id, revision, "assertion", assertion_id),
                        FactRef::Node {
                            graph_id,
                            revision,
                            node_id,
                        } => (graph_id, revision, "node", node_id),
                        FactRef::Embedding {
                            graph_id,
                            revision,
                            embedding_id,
                        } => (graph_id, revision, "embedding", embedding_id),
                    };
                    done.facts.push(FactAccess {
                        access: Access::Read,
                        graph_id: graph_id.clone(),
                        revision: revision.clone(),
                        kind: kind.into(),
                        fact_id: fact_id.clone(),
                        change: None,
                    });
                }
            }
            _ => {}
        }
    }
}

/// Check that a program is one the service runs: protocol 1.0.0 or later
/// (so results carry `fact_provenance` for the audit log), only fact
/// writes, queries and graph expressions, and for non-administrators only
/// minted record IDs in `assert` changes.
fn admit_program(program: &Program, ctx: &Ctx, max_commands: usize) -> Result<(), ApiError> {
    if !weave_contract::version_at_least(&program.version, "1.0.0") {
        return Err(ApiError::new(
            "E_VERSION",
            "the service accepts programs of protocol 1.0.0 and later",
        ));
    }
    if program.commands.is_empty() || program.commands.len() > max_commands {
        return Err(ApiError::new(
            "E_BUDGET",
            format!("a program carries 1 to {max_commands} commands"),
        ));
    }
    for command in &program.commands {
        match command {
            Command::Write { changes, .. } => {
                if ctx.administrator {
                    continue;
                }
                for change in changes {
                    if let FactChange::Assert { fact } = change {
                        if !minted_for(&ctx.id_key, &ctx.tenant_id, &ctx.principal, fact.id()) {
                            return Err(ApiError::new(
                                codes::ID_NOT_MINTED,
                                "asserted record IDs must be minted for the caller by POST /v1/ids",
                            ));
                        }
                    }
                }
            }
            Command::Query { .. } | Command::Evaluate { .. } | Command::Bind { .. } => {}
            _ => {
                return Err(ApiError::new(
                    codes::UNSUPPORTED,
                    "the service runs write, query, evaluate and bind commands only",
                ))
            }
        }
    }
    Ok(())
}

async fn execute(
    State(service): State<Service>,
    Extension(caller): Extension<Caller>,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let bytes = match body_bytes(body) {
        Ok(bytes) => bytes,
        Err(error) => return error_response(error),
    };
    let max_commands = service.limits().max_commands;
    call(
        &service,
        caller,
        "execute",
        true,
        Box::new(move |state, ctx| {
            let named = || {
                let mut out = Vec::new();
                if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                    named_graphs(&value, &mut out, &mut 100_000);
                }
                out
            };
            let fail = |error: ApiError| Failed {
                error,
                graphs: named(),
            };
            let program = weave_contract::decode_program(&bytes)
                .map_err(|e| fail(ApiError::new(&e.code, e.message)))?;
            admit_program(&program, ctx, max_commands).map_err(fail)?;
            let results = state
                .engine
                .execute(&program, &ctx.host)
                .map_err(|e| fail(e.into()))?;
            let mut done = Done::json(json!({"results": results}));
            touched(&program, &results, &mut done);
            Ok(done)
        }),
    )
    .await
}

async fn list_graphs(
    State(service): State<Service>,
    Extension(caller): Extension<Caller>,
) -> Response {
    call(
        &service,
        caller,
        "graphs.list",
        true,
        Box::new(|state, ctx| {
            let graphs = state.engine.list_graphs(&ctx.host)?;
            let mut done = Done::json(json!({"graphs": graphs}));
            done.graphs = graphs.iter().map(|g| g.graph_id.clone()).collect();
            Ok(done)
        }),
    )
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateGraph {
    #[serde(default)]
    graph_id: Option<String>,
}

async fn create_graph(
    State(service): State<Service>,
    Extension(caller): Extension<Caller>,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let request: CreateGraph = match json_body(body) {
        Ok(request) => request,
        Err(error) => return error_response(error),
    };
    call(
        &service,
        caller,
        "graphs.create",
        true,
        Box::new(move |state, ctx| {
            let graph_id = match request.graph_id {
                None => state.engine.create_graph(&random::<32>(), &ctx.host)?,
                Some(graph_id) => {
                    // Choosing an ID needs tenant Administer, which sees every
                    // graph, so the conflict below reveals nothing hidden.
                    if !ctx.administrator {
                        return Err(ApiError::forbidden(
                            "choosing a graph ID requires tenant administration",
                        )
                        .into());
                    }
                    check_id(&graph_id, "graph")?;
                    if state
                        .engine
                        .list_graphs(&ctx.host)?
                        .iter()
                        .any(|g| g.graph_id == graph_id)
                    {
                        return Err(ApiError::new("E_CONFLICT", "the graph already exists").into());
                    }
                    state.engine.apply_registry(
                        &[RegistryChange::Grant {
                            graph_id: graph_id.clone(),
                            subject: ctx.principal.clone(),
                            permission: Permission::Administer,
                        }],
                        &ctx.host,
                    )?;
                    graph_id
                }
            };
            let mut done = Done::json(json!({"graph_id": graph_id}));
            done.status = Some(StatusCode::CREATED);
            done.graphs = vec![graph_id, REGISTRY_GRAPH.into()];
            Ok(done)
        }),
    )
    .await
}

async fn graph_acl(
    State(service): State<Service>,
    Extension(caller): Extension<Caller>,
    Path(graph_id): Path<String>,
) -> Response {
    call(
        &service,
        caller,
        "registry.graph_acl",
        true,
        Box::new(move |state, ctx| {
            check_id(&graph_id, "graph")?;
            let acl = state
                .engine
                .registry_graph_acl(&graph_id, &ctx.host)
                .map_err(|e| Failed {
                    error: e.into(),
                    graphs: vec![graph_id.clone()],
                })?;
            let mut done =
                Done::json(serde_json::to_value(&acl).map_err(|_| ApiError::internal())?);
            done.graphs = vec![graph_id, REGISTRY_GRAPH.into()];
            Ok(done)
        }),
    )
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MintIds {
    #[serde(default = "one")]
    count: usize,
}
fn one() -> usize {
    1
}

async fn mint_ids(
    State(service): State<Service>,
    Extension(caller): Extension<Caller>,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let request: MintIds = match json_body(body) {
        Ok(request) => request,
        Err(error) => return error_response(error),
    };
    let max = service.limits().max_ids;
    call(
        &service,
        caller,
        "ids.mint",
        true,
        Box::new(move |_, ctx| {
            if request.count == 0 || request.count > max {
                return Err(ApiError::new("E_BUDGET", format!("mint 1 to {max} IDs")).into());
            }
            let ids: Vec<String> = (0..request.count)
                .map(|_| mint_record_id(&ctx.id_key, &ctx.tenant_id, &ctx.principal))
                .collect();
            Ok(Done::json(json!({"ids": ids})))
        }),
    )
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyRegistry {
    changes: Vec<RegistryChange>,
}

async fn apply_registry(
    State(service): State<Service>,
    Extension(caller): Extension<Caller>,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let request: ApplyRegistry = match json_body(body) {
        Ok(request) => request,
        Err(error) => return error_response(error),
    };
    call(
        &service,
        caller,
        "registry.apply",
        true,
        Box::new(move |state, ctx| {
            let revision = state.engine.apply_registry(&request.changes, &ctx.host)?;
            let mut done = Done::json(json!({"revision": revision}));
            done.graphs = vec![REGISTRY_GRAPH.into()];
            done.revisions = vec![revision];
            Ok(done)
        }),
    )
    .await
}

async fn registry_principal(
    State(service): State<Service>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Response {
    call(
        &service,
        caller,
        "registry.principal",
        true,
        Box::new(move |state, ctx| {
            check_id(&id, "principal")?;
            let principal = state.engine.registry_principal(&id, &ctx.host)?;
            let mut done =
                Done::json(serde_json::to_value(&principal).map_err(|_| ApiError::internal())?);
            done.graphs = vec![REGISTRY_GRAPH.into()];
            Ok(done)
        }),
    )
    .await
}

async fn registry_memberships(
    State(service): State<Service>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Response {
    call(
        &service,
        caller,
        "registry.memberships",
        true,
        Box::new(move |state, ctx| {
            check_id(&id, "principal")?;
            let groups = state.engine.registry_memberships(&id, &ctx.host)?;
            let mut done = Done::json(json!({"principal": id, "groups": groups}));
            done.graphs = vec![REGISTRY_GRAPH.into()];
            Ok(done)
        }),
    )
    .await
}

async fn registry_members(
    State(service): State<Service>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Response {
    call(
        &service,
        caller,
        "registry.members",
        true,
        Box::new(move |state, ctx| {
            check_id(&id, "group")?;
            let members = state.engine.registry_members(&id, &ctx.host)?;
            let mut done = Done::json(json!({"group": id, "members": members}));
            done.graphs = vec![REGISTRY_GRAPH.into()];
            Ok(done)
        }),
    )
    .await
}

async fn list_tokens(
    State(service): State<Service>,
    Extension(caller): Extension<Caller>,
) -> Response {
    let control = service.clone();
    call(
        &service,
        caller,
        "tokens.list",
        true,
        Box::new(move |_, ctx| {
            let own = (!ctx.administrator).then_some(ctx.principal.as_str());
            let tokens = control.control(|c| c.tokens(&ctx.tenant_id, own))?;
            Ok(Done::json(json!({"tokens": tokens})))
        }),
    )
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IssueToken {
    principal: String,
    #[serde(default)]
    expires_in_ms: Option<i64>,
    #[serde(default)]
    label: Option<String>,
}

async fn issue_token(
    State(service): State<Service>,
    Extension(caller): Extension<Caller>,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let request: IssueToken = match json_body(body) {
        Ok(request) => request,
        Err(error) => return error_response(error),
    };
    let control = service.clone();
    call(
        &service,
        caller,
        "tokens.issue",
        true,
        Box::new(move |state, ctx| {
            check_id(&request.principal, "principal")?;
            // Tenant administrators issue tokens for any user or agent; a
            // user issues them for agents it owns, directly or through other
            // agents. Anything else, whether or not the principal exists, is
            // the same refusal.
            if !ctx.administrator && !owns_agent(&state.engine, ctx, &request.principal)? {
                return Err(ApiError::forbidden(
                    "issuing tokens requires tenant administration or ownership of the agent",
                )
                .into());
            }
            let now = control.now()?;
            if standing(&state.engine, &request.principal, now)?.is_none() {
                return Err(ApiError::new("E_UNAVAILABLE", "principal unavailable").into());
            }
            let issued = control.control(|c| {
                c.issue_token(
                    &ctx.tenant_id,
                    &request.principal,
                    &ctx.principal,
                    request.expires_in_ms.unwrap_or(DEFAULT_TOKEN_TTL_MS),
                    request.label.as_deref(),
                    now,
                )
            })?;
            let mut done =
                Done::json(serde_json::to_value(&issued).map_err(|_| ApiError::internal())?);
            done.status = Some(StatusCode::CREATED);
            Ok(done)
        }),
    )
    .await
}

async fn revoke_token(
    State(service): State<Service>,
    Extension(caller): Extension<Caller>,
    Path(token_id): Path<String>,
) -> Response {
    let control = service.clone();
    call(
        &service,
        caller,
        "tokens.revoke",
        true,
        Box::new(move |state, ctx| {
            let unavailable = || ApiError::new("E_UNAVAILABLE", "token unavailable");
            if token_id.len() > 128 {
                return Err(unavailable().into());
            }
            let holder = control.control(|c| c.token_principal(&ctx.tenant_id, &token_id))?;
            let allowed = match &holder {
                None => false,
                Some(holder) => {
                    ctx.administrator
                        || holder == &ctx.principal
                        || owns_agent(&state.engine, ctx, holder)?
                }
            };
            if !allowed {
                return Err(unavailable().into());
            }
            let now = control.now()?;
            control.control(|c| c.revoke_token(&ctx.tenant_id, &token_id, now))?;
            Ok(Done::json(json!({"token_id": token_id, "revoked": true})))
        }),
    )
    .await
}

/// True when the caller is a user that owns `agent`, directly or through
/// other agents. The registry lets a principal read an agent's record only
/// when it is the agent itself, in its ownership chain, or a tenant
/// administrator.
fn owns_agent(engine: &Engine, ctx: &Ctx, agent: &str) -> Result<bool, Failed> {
    if ctx.agent || agent == ctx.principal {
        return Ok(false);
    }
    match engine.registry_principal(agent, &ctx.host) {
        Ok(p) => Ok(p.kind == PrincipalKind::Agent),
        Err(e) if e.code == "E_UNAVAILABLE" => Ok(false),
        Err(e) => Err(e.into()),
    }
}

fn require_auditor(ctx: &Ctx) -> Result<(), Failed> {
    if ctx.administrator {
        Ok(())
    } else {
        Err(ApiError::forbidden("reading the audit log requires tenant administration").into())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuditPage {
    #[serde(default)]
    after: i64,
    #[serde(default = "page")]
    limit: usize,
}
fn page() -> usize {
    100
}

async fn audit_entries(
    State(service): State<Service>,
    Extension(caller): Extension<Caller>,
    params: Result<Query<AuditPage>, QueryRejection>,
) -> Response {
    let params = match query(params) {
        Ok(params) => params,
        Err(error) => return error_response(error),
    };
    call(
        &service,
        caller,
        "audit.entries",
        true,
        Box::new(move |state, ctx| {
            require_auditor(ctx)?;
            if params.limit == 0 || params.limit > MAX_AUDIT_PAGE {
                return Err(ApiError::new("E_BUDGET", "audit pages hold 1 to 1000 entries").into());
            }
            let entries = state.audit.entries(params.after, params.limit)?;
            Ok(Done::json(json!({"entries": entries})))
        }),
    )
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuditFacts {
    graph_id: String,
    fact_id: String,
    #[serde(default)]
    access: Option<Access>,
    #[serde(default)]
    after: i64,
    #[serde(default = "page")]
    limit: usize,
}

async fn audit_facts(
    State(service): State<Service>,
    Extension(caller): Extension<Caller>,
    params: Result<Query<AuditFacts>, QueryRejection>,
) -> Response {
    let params = match query(params) {
        Ok(params) => params,
        Err(error) => return error_response(error),
    };
    call(
        &service,
        caller,
        "audit.facts",
        true,
        Box::new(move |state, ctx| {
            require_auditor(ctx)?;
            check_id(&params.graph_id, "graph")?;
            check_id(&params.fact_id, "fact")?;
            if params.limit == 0 || params.limit > MAX_AUDIT_PAGE {
                return Err(ApiError::new("E_BUDGET", "audit pages hold 1 to 1000 entries").into());
            }
            let accesses = state.audit.fact_accesses(
                &params.graph_id,
                &params.fact_id,
                params.access,
                params.after,
                params.limit,
            )?;
            Ok(Done::json(json!({
                "graph_id": params.graph_id,
                "fact_id": params.fact_id,
                "accesses": accesses,
            })))
        }),
    )
    .await
}
