# Agent interface (MCP)

Agents write and query shared Weave knowledge through a standard
[Model Context Protocol](https://modelcontextprotocol.io) tool interface
(B4, [#40](https://github.com/weave-graph/weave-engine/issues/40)).
`weave-service` serves it at `POST /v1/mcp`, behind the same
authentication, tenancy and audit as the rest of its API
([SERVICE.md](SERVICE.md)). It extends the read-only local
[preview](MCP_PREVIEW.md) (A7) with writes, identity and a schema tool.

**Status.** Part 1 of B4: the MCP endpoint with all nine tools, agent
identity and provenance on every write, the data/code boundary, revocation
on the next call, and audit. Per-agent quotas and rate limits and the
Python SDK follow in later parts of #40.

## Quickstart: connect an MCP client

Start a service and give an agent a token. This development setup serves
plaintext on loopback; production serves TLS (`--tls-cert`, `--tls-key`,
see [SERVICE.md](SERVICE.md#run-it)).

```sh
cargo build --release --locked -p weave-service
S=target/release/weave-service

# The tenant and its first administrator; prints the administrator's token.
$S tenant create --data weave-data --tenant acme --admin root
export ROOT=wvs_...   # the "token" printed above

$S serve --data weave-data --listen 127.0.0.1:8080 --plaintext-loopback &
W=http://127.0.0.1:8080
post() { curl -s -H "Authorization: Bearer $1" -H 'content-type: application/json' -d "$3" "$W$2"; }

# A user in a team, and a graph the user writes.
post "$ROOT" /v1/registry '{"changes": [
  {"change": "add_principal", "id": "alice", "kind": "user"},
  {"change": "add_principal", "id": "team-a", "kind": "group"},
  {"change": "add_member", "group": "team-a", "member": "alice"}]}'
post "$ROOT" /v1/graphs '{"graph_id": "maintenance"}'
post "$ROOT" /v1/registry '{"changes": [
  {"change": "grant", "graph_id": "maintenance", "subject": "alice", "permission": "write"}]}'
export ALICE=$(post "$ROOT" /v1/tokens '{"principal": "alice"}' | jq -r .token)

# alice registers her agent and issues its token.
post "$ALICE" /v1/registry '{"changes": [{"change": "add_agent", "id": "alice-bot", "owner": "alice",
  "delegation": {"scopes": [{"graph_id": "maintenance", "permission": "write"}],
                 "exercise": {"kind": "owner_all"}}}]}'
export WEAVE_AGENT_TOKEN=$(post "$ALICE" /v1/tokens '{"principal": "alice-bot"}' | jq -r .token)
```

Then register the endpoint with the MCP client as a Streamable HTTP server
with the token in an `Authorization` header. With Claude Code:

```sh
claude mcp add --transport http weave "$W/v1/mcp" \
  --header "Authorization: Bearer $WEAVE_AGENT_TOKEN"
```

Clients configured with JSON (Cursor, VS Code and others) take the same
URL and header:

```json
{"mcpServers": {"weave": {
  "type": "http",
  "url": "https://weave.example.com:8443/v1/mcp",
  "headers": {"Authorization": "Bearer ${WEAVE_AGENT_TOKEN}"}}}}
```

To check the endpoint by hand, POST one JSON-RPC message per request:

```sh
mcp() { curl -s -H "Authorization: Bearer $WEAVE_AGENT_TOKEN" -H 'content-type: application/json' \
  -H 'accept: application/json, text/event-stream' -d "$1" "$W/v1/mcp"; }
mcp '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"curl","version":"0"}}}'
mcp '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
mcp '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"describe_schema","arguments":{}}}'
```

Treat an agent token like a password: it is shown once, and anyone holding
it acts as the agent. Keep it in the client's secret store or environment,
never in a prompt. Revoke it with `DELETE /v1/tokens/{token_id}`.

## Identity and authority

- **The principal comes from the credential.** Every message is an
  authenticated request (API token or OIDC bearer token). The token decides
  the tenant and the principal; no header, path or tool argument names
  either. An agent token authenticates the agent, with at most its owner's
  current authority narrowed by its delegation ([REGISTRY.md](REGISTRY.md#agents)).
- **Checked on every call.** Nothing is cached between messages. Revoking
  the token, suspending or retiring the agent or its owner, expiring its
  delegation or removing a membership applies to the next tool call: a lost
  credential fails with the uniform `401 E_UNAUTHENTICATED`, and lost
  read access makes the records absent.
- **Writes go through the service path.** `assert_fact` and
  `retract_fact` run as one fact `write` through the same path as
  `POST /v1/execute`: the engine authorizes them (graph Write, branch
  protection, Read on retracted facts, nameable reader groups), records the
  writer, its `on_behalf_of` chain and the recorded time, and the service
  audits every fact changed. There is no local-store write path.
- **No tool changes permissions.** The interface has no registry, ACL,
  token, key or graph-creation tool.

## Tools

`tools/list` gives each tool's JSON Schema. The read tools take the same
arguments as in the [preview](MCP_PREVIEW.md#tools) and their `result` is
exactly the `POST /v1/execute` response (`{"results": [...]}`) for the same
one-command program, as the same principal.

| Tool | Does | Annotations |
|---|---|---|
| `describe_schema` | What the caller can see: its graphs, or one graph's schema (below) | read-only |
| `query` | A `QueryPlan`: edges of one branch by predicate, endpoint and valid time | read-only |
| `expand` | A k-hop neighborhood (`seeds` and/or `seeds_from`) | read-only |
| `as_of` | A query as the store knew it at a recorded time | read-only |
| `what_changed` | The A3 change set between two states of a branch | read-only |
| `hybrid_search` | A4 `nearest` or `similar_candidates` over embedding facts | read-only |
| `explain` | A derivation graph with the source facts and their provenance | read-only |
| `assert_fact` | Record new facts, with service-minted IDs (below) | write |
| `retract_fact` | End live facts; history keeps them | write, destructive |

### `assert_fact`

```json
{"graph_id": "maintenance",
 "provenance": {
   "method": {"kind": "model", "model": "my-extractor", "version": "2026-10"},
   "source": "maintenance log 42",
   "evidence": [{"kind": "artifact", "uri": "doc://maintenance/log-42", "digest": "sha256:..."}]},
 "visibility": {"reader_groups": ["team-a"]},
 "facts": [
   {"kind": "node", "ref": "finding", "space_id": "finding",
    "properties": {"text": "Bearing on unit 7 replaced after the vibration alarm"}},
   {"kind": "edge", "predicate": "maint:about", "from": {"ref": "finding"}, "to": "unit-7",
    "valid_time": {"start": 1760000000000}},
   {"kind": "embedding", "space": {"id": "notes", "revision": "1"},
    "subject": {"kind": "node", "ref": "finding"}, "vector": [0.12, -0.4, 0.88],
    "valid_time": {"start": 1760000000000}}]}
```

- **IDs are minted.** The service mints every record ID for the caller
  ([SERVICE.md](SERVICE.md#record-ids-are-minted-by-the-service)), so no
  argument chooses one. Facts of one call refer to each other with
  `{"ref": "label"}` (an edge endpoint) or `"ref"` in a `host` or
  `subject`; existing records are named by ID. A node's `entity_id`
  defaults to its minted ID.
- **Kinds.** `node` (`space_id`, optional `entity_id`, `type_id`,
  `properties`, `valid_time`), `edge` (`predicate`, `from`, `to`,
  `valid_time`, `properties`, `polarity`), `attachment` (`host` of kind
  `node`, `edge` or `entity`, `key`, any JSON `value` as a literal,
  `valid_time`) and `embedding` (`space`, `subject`, `vector`,
  `valid_time`) in an existing embedding space. Spaces themselves are
  created through `POST /v1/execute`.
- **Provenance is required.** `provenance.method` states how the facts were
  produced: `model` (with model and version), `rule`, `import` or, for a
  person, `human`; `unspecified` is refused (`E_MCP_CLAIM`), and so is
  `human` from an agent. `evidence` cites a document (`artifact`, a URI and
  digest) or exact facts; `source` is a free label. The method and evidence
  are claims; the writer, `on_behalf_of` and recorded time are recorded by
  the engine and are what `explain`, every result's `fact_provenance` and
  the audit log report.
- **Visibility is required.** Every fact of the call gets the same
  `readers` and `reader_groups`, and at least one of them must be named;
  a public write says `{"public": true}`. Nothing becomes public by
  omission. Groups must be ones the caller may name (`describe_schema`
  lists them); any other group fails `E_UNAVAILABLE`, as for a group that
  does not exist.
- **Not accepted.** Writer, recorded-time and `on_behalf_of` fields, record
  IDs, per-record `readers`, derivation gates (`derived_from`,
  `derived_nodes`, `derived_snapshots`, `derivations`) and context scopes
  are not fields of the tool, so they fail `E_MCP_INPUT`.
- The result lists `facts` (`{ref?, kind, id}` in order), the committed
  `revision` and the `write` command result.

Facts are immutable. To correct one, retract it and assert the
replacement; `as_of` and `what_changed` show both.

### `retract_fact`

`{"graph_id", "branch_id"?, "facts": [{"kind", "id"}]}` (1 to 256 targets)
retracts in one write. The caller needs Write on the graph and must be able
to read each fact; a fact it cannot read fails exactly as a fact that does
not exist.

### `describe_schema`

Without `graph_id` it lists the graphs the caller can discover, with its
permission and branches, and the `reader_groups` it may name in a
`visibility`. With `graph_id` (and optional `branch_id`) it summarizes the
facts the caller can see at the branch head:

| Field | Contents |
|---|---|
| `facts` | Visible facts by kind |
| `predicates` | Edge predicates with their count, property keys and an `example` `query` call |
| `node_spaces`, `node_types` | Node `space_id` and `type_id` values with counts |
| `node_properties` | Node property keys with counts and JSON types |
| `attachment_keys` | Attachment keys with counts, host kinds and value types |
| `embedding_spaces` | Each visible space's descriptor (dimensions, metric, encoder) with its number of embeddings and an `example` `hybrid_search` call |
| `example` | An `expand` call from a visible node |
| `permission`, `reader_groups`, `revision`, `coverage` | The caller's permission on the graph, the groups it may name, the head read, and its coverage |
| `omitted` | For a list longer than 1,000 entries, how many were left out (the most frequent are kept) |

Every example is a complete, valid call for that tool. The description is
computed from one A3 change set from the empty state to the head, read as
the caller, so it contains only what the caller can see; hidden facts never
change an entry or a count.

## Results are data, not instructions

Store content can be written by anyone with Write on a graph, including
text crafted to look like instructions, tool calls or protocol messages.
The interface keeps it on the data side:

- Tool names, descriptions, schemas and the server `instructions` are fixed
  text compiled into `weave-mcp`; nothing read from a store is put in them,
  and they are identical for every caller and store.
- Store content reaches the client only as JSON values under `result` (or
  in `error.message`) in an envelope marked `"trust": "untrusted_data"`
  with a fixed notice, as `structuredContent` and as the same JSON in one
  text block.
- Reading never writes. Each request is one JSON-RPC message with one
  response; a message never carries another, and no content is parsed as a
  message, a program or a tool call.
- Tool arguments are closed types: there is no field for a principal, a
  writer, a grant, a record ID or a reader list on a record, and
  visibility must be stated. Text an agent mistakes for an instruction can
  at most make the agent call a tool with its own, already limited,
  authority, which the registry checks again and the audit log records.

## Audit

Each message is one audited request ([SERVICE.md](SERVICE.md#audit)). Tool
calls record the operation `mcp.<tool>` (for example `mcp.assert_fact`),
the facts written or read, and a failed tool call's error code as a
`denied` or `error` outcome. Other messages record `mcp.initialize`,
`mcp.tools_list`, `mcp.ping`, `mcp.notification`, `mcp.unknown_tool`,
`mcp.unknown_method` or `mcp.invalid`. `GET /v1/audit/facts` therefore
shows which agent wrote a fact and which agents read it.

## Transport

The endpoint implements MCP Streamable HTTP without sessions or
server-sent events:

- Each `POST /v1/mcp` carries one JSON-RPC 2.0 message (no batches, at most
  1 MiB, duplicate keys rejected) and gets one `application/json` response;
  a notification gets `202` with no body. No `Mcp-Session-Id` is issued.
- `GET` and `DELETE` are `405` (no event stream, no session).
- A request with an `Origin` header is refused with `403 E_MCP_ORIGIN`:
  browser pages may not call the endpoint (DNS-rebinding protection).
- An `MCP-Protocol-Version` header must name a supported revision
  (2025-11-25, 2025-06-18, 2025-03-26 or 2024-11-05), else `400
  E_MCP_PROTOCOL_VERSION`.
- Malformed JSON-RPC is a JSON-RPC error with status `400`; an unknown
  tool or method is a JSON-RPC error with status `200`.
- Authentication failures are the service's uniform `401`. There is no
  OAuth authorization-server discovery: clients send a configured token.

## Errors

Tool failures are `isError` results with `error: {code, message}`. Engine
and service codes pass through (`E_UNAVAILABLE`, `E_FORBIDDEN`,
`E_CONFLICT`, `E_BUDGET`, `E_SPACE`, ...). The interface adds:

| Code | Meaning |
|---|---|
| `E_MCP_INPUT` | Arguments are not an object, have an unknown field or a wrong type, exceed the depth or string bound, nest a disallowed expression, or have a bad `ref` |
| `E_MCP_CLAIM` | `assert_fact` provenance or visibility is not acceptable (unspecified method, an agent claiming `human`, no readers and no `public`, or both) |
| `E_MCP_UNKNOWN_TOOL` | JSON-RPC `-32602`: no such tool |
| `E_MCP_ORIGIN`, `E_MCP_PROTOCOL_VERSION` | HTTP `403` and `400` (above) |

## Limits and non-guarantees

- One call writes at most 256 facts (`E_BUDGET`); strings in arguments are
  at most 4,096 bytes, so long document text belongs in an artifact cited
  as evidence or split across facts. `describe_schema` summarizes at most
  200,000 visible facts (`E_BUDGET` beyond, never truncated) and keeps at
  most 1,000 entries per list.
- The service runs one request per tenant at a time
  ([SERVICE.md](SERVICE.md#limits-and-non-guarantees)); calls are not
  cancellable. Engine budgets apply inside each call.
- There is no `correct` tool: retract and assert. Embedding spaces are
  created through `POST /v1/execute`. The service supplies no encoder: an
  agent passes vectors from the space's encoder.
- `explain` explains edges and assertions. A `nearest` result has no edges,
  so explaining it alone is an empty derivation; explain an `expand` with the
  search as `seeds_from`, or read the hits' `fact_provenance`.
- Stdio-only MCP clients need an HTTP bridge; the local `weave-mcp` binary
  stays the read-only preview over a local store.
- Revision IDs hash every fact, so hidden writes change the revision IDs a
  caller sees (a declared channel, as in every interface), never its facts,
  counts, rankings or errors.
- Derived facts are restricted as the engine propagates restrictions today
  (an embedding is restricted by its own readers). Write-time propagation
  and release policies are B2
  ([#5](https://github.com/weave-graph/weave-engine/issues/5)).
- Not yet in this part: per-agent quotas and rate limits, the Python SDK
  and a compile tool.

Evidence: `crates/weave-service/tests/agent_mcp.rs` (agent X asserts from a
document, agent Y finds it with `hybrid_search` and `explain` shows X;
`describe_schema` alone builds valid calls; revocation on the next call;
history after `retract_fact`; read tools equal `POST /v1/execute`; the
transport), `tests/agent_hidden.rs` (paired worlds that differ only in
team A's facts give agent Z of team B and agent W without access identical
responses from every tool, writes included) and `tests/agent_injection.rs`
(injected text changes no permission, triggers no write and never leaves
`result`; arguments cannot name a writer, grant or authority).
