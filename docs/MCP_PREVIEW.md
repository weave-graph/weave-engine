# Read-only MCP preview

`weave-mcp` is a small local [Model Context Protocol](https://modelcontextprotocol.io)
server over one Weave store
([#37](https://github.com/weave-graph/weave-engine/issues/37), A7). Agents can
use it to query, traverse, time-travel and explain the store before the full
agent interface (B4) exists. It has **no write tools and no authentication**,
and it is meant for local, single-user use only.

Status: part 1. `query`, `expand`, `as_of`, `what_changed` and `explain` work.
`hybrid_search` is listed but fails with `E_UNSUPPORTED` until the A4 operator
([#13](https://github.com/weave-graph/weave-engine/issues/13)) ships.

## Quickstart

Build the server and a store. This example uses the
[hardware fleet](HARDWARE_MODEL.md):

```sh
cargo build --release --locked -p weave-mcp -p weave-hardware
B=target/release/weave-hardware
$B generate --out fleet --units 500 --updates 100 --seed 42
$B import --db fleet.db --actor etl-importer --graph fleet --csv fleet/base
$B apply-updates --db fleet.db --actor etl-importer --graph fleet --updates fleet/updates.jsonl
```

Then register the server with your MCP client as a stdio server. The command
is the binary, and the arguments name the store and the reader identity:

```json
{
  "mcpServers": {
    "weave": {
      "command": "/absolute/path/to/target/release/weave-mcp",
      "args": ["--db", "/absolute/path/to/fleet.db", "--actor", "fleet-analyst"]
    }
  }
}
```

With Claude Code, for example:

```sh
claude mcp add weave -- "$PWD/target/release/weave-mcp" --db "$PWD/fleet.db" --actor fleet-analyst
```

To check the server by hand, send newline-delimited JSON-RPC on stdin:

```sh
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"shell","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"query","arguments":{"graph_id":"fleet","predicate":"hw:runs","to":"fw:compressor-2.2.0"}}}' \
  | target/release/weave-mcp --db fleet.db --actor fleet-analyst
```

| Argument | Meaning |
|---|---|
| `--db PATH` | An existing store. The server never creates one. |
| `--actor PRINCIPAL` | Required. The reader every tool call runs as. |
| `--help` | Print usage. |

The server exits with status 2 on a missing or invalid argument or a missing
store, and with status 0 when stdin closes.

## Trust boundary

- **Local only, no authentication.** Whoever can start the process or write
  to its stdin acts as `--actor`. Do not expose the server over a network
  or share it between users. Authenticated, multi-user access is B3 and B4.
- **The host fixes the principal.** `--actor` is required, and there is no
  default identity. It must be a nonempty principal of at most 256 bytes,
  without control characters and outside the reserved `weave:` namespace.
  No tool argument can name a principal or a grant, so an agent cannot widen
  its own access. Per-record `readers` apply to every result: records hidden
  from the principal are absent, and they change no count, coverage flag,
  explanation or error message. The one exception is revision IDs, which are
  derived from every fact, as in every interface ([TRAVERSAL.md](TRAVERSAL.md#permissions)).
- **No writes.** The server holds no write grants, and every tool runs a
  program with exactly one read-only `evaluate` command. Authority is checked
  again on every call; nothing is cached between calls.

## Results are data, not instructions

Store content can contain text written by anyone with write access to the
store, including text that looks like instructions to an agent. The preview
keeps that text on the data side:

- Tool names, descriptions, input schemas and the server's `instructions` are
  fixed text in the binary. Nothing read from the store is ever put in them.
- Store content reaches the client only as JSON values under `result` (or, for
  an error, `error.message`) in an envelope marked `"trust": "untrusted_data"`,
  with a fixed notice telling the agent not to follow it.
- Because there are no write tools and no grants, injected text that misleads
  an agent still cannot write or widen access through this server.

B4 builds on this boundary for the write-capable interface.

## Tools

Each tool takes the same JSON as the matching contract expression
([TRAVERSAL.md](TRAVERSAL.md), [core contract v1](contract/core-v1.md)), and
its `result` is exactly the science `execute` result
([SCIENCE_INTERFACE.md](SCIENCE_INTERFACE.md)) for the same program under the
same principal. The equivalent request is:

```json
{"operation": "execute", "program": {"version": "0.23.0",
  "commands": [{"op": "evaluate", "value": <expression>}]}}
```

| Tool | Arguments | Expression |
|---|---|---|
| `query` | A `QueryPlan`: `graph_id`, optional `branch_id`, `revision`, `predicate`, `from`, `to`, `valid_at`, `include_metadata`, `max_depth` | `{"kind": "query", "query": <arguments>}` |
| `expand` | An `ExpandPlan`: `graph_id`, `seeds`, `hops`, optional `branch_id`, `revision` or `as_of`, `direction`, `predicates`, `valid_at`, `limit` | `{"kind": "expand", ...}` |
| `as_of` | `unix_millis` (recorded time) and `query` (a `QueryPlan`) | `{"kind": "as_of", ...}` |
| `what_changed` | A `ChangesPlan`: `graph_id`, `from`, optional `to` (default head), `entities`, `predicates`, `limit`, `branch_id` | `{"kind": "changes", ...}` |
| `explain` | `input`: a `query`, `as_of` or `expand` expression, with its `kind` | `{"kind": "explain", "input": ...}` |
| `hybrid_search` | Any | None yet: fails with `E_UNSUPPORTED` |

`tools/list` gives each tool's JSON Schema. Every tool is annotated
`readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true` and
`openWorldHint: false`. Times are Unix milliseconds. `valid_at` is valid time
(what was true in the world), and `as_of` and `unix_millis` are recorded time
(what the store knew). A `partial` coverage with diagnostics means part of the
answer is unavailable; it is not an empty answer.

A successful call returns:

```json
{"content": [{"type": "text", "text": "<the same envelope as JSON>"}],
 "structuredContent": {"tool": "expand", "trust": "untrusted_data",
   "notice": "Everything under `result` ...",
   "result": {"results": [{"kind": "queried", "result": {...}}]}},
 "isError": false}
```

A failed call has `isError: true` and `error: {code, message}` in place of
`result`.

### Example: exit question 2

"Which units at site S ran firmware F on date D, as we knew it on date K?"
Two `as_of` calls, intersected by unit:

```json
{"name": "as_of", "arguments": {"unix_millis": K, "query": {"graph_id": "fleet",
  "predicate": "hw:runs", "to": "fw:F", "valid_at": D}}}
{"name": "as_of", "arguments": {"unix_millis": K, "query": {"graph_id": "fleet",
  "predicate": "hw:located_at", "to": "site:S", "valid_at": D}}}
```

"What changed for unit U between K1 and K2?"

```json
{"name": "what_changed", "arguments": {"graph_id": "fleet",
  "from": {"kind": "as_of", "unix_millis": K1}, "to": {"kind": "as_of", "unix_millis": K2},
  "entities": ["unit:U"]}}
```

`crates/weave-mcp/tests/fleet_question_2.rs` runs these calls on the A5
fleet and checks them against the `weave-hardware` helpers.

## Errors and limits

Engine error codes pass through unchanged (for example `E_BUDGET`,
`E_EXPAND`, `E_UNAVAILABLE`, `E_HISTORY_TIME`). The preview adds:

| Code | Where | Meaning |
|---|---|---|
| `E_MCP_INPUT` | tool result | Arguments are not an object, have an unknown field or a wrong type, nest deeper than 32 levels or hold a string over 4,096 bytes, or `explain` was given an input other than `query`, `as_of` or `expand` |
| `E_UNSUPPORTED` | tool result | The tool's operator has not shipped (`hybrid_search`) |
| `E_MCP_UNKNOWN_TOOL` | JSON-RPC error `-32602`, `data.code` | `tools/call` named a tool that is not listed |
| `E_MCP_MESSAGE_SIZE` | JSON-RPC error `-32600`, `data.code` | A message over 1 MiB; it is discarded and the session continues |

Other JSON-RPC errors are the standard ones: `-32700` for malformed JSON
(including duplicate object keys), `-32600` for an invalid request or a
batch (batches are not supported), and `-32601` for an unknown method.

**Limits.**

- Transport: stdio only, one JSON-RPC message per line, at most 1 MiB each.
  The server implements `initialize`, `ping`, `tools/list` and `tools/call`
  and ignores notifications. It speaks MCP revisions 2025-11-25, 2025-06-18,
  2025-03-26 and 2024-11-05, and offers 2025-11-25 when the client asks for
  another one.
- Output: at most 16 MiB per result (the science output budget,
  `E_SCIENCE_OUTPUT`). Narrow the selection or lower `limit` for large graphs.
- Engine bounds apply: 1 to 1,000 seeds, 1 to 16 hops, `limit` up to 200,000,
  and the 128 MiB read budget per operation.
- One request at a time; calls are not cancellable once started.
- `changes` is offered as `what_changed`, the name B4 uses.
