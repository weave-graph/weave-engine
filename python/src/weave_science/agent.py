"""Agent SDK for the Weave agent interface (B4): the MCP tools of `weave-service`.

`AgentClient` speaks MCP (JSON-RPC over HTTP, `POST /v1/mcp`) to an
authenticated `weave-service` with an agent's bearer token, so every method
is exactly the tool an MCP client calls, with the same identity, provenance,
quotas, audit and data/code boundary. No graph semantics live here, and the
standard library is the only dependency.

Results are untrusted data written by other parties. The SDK returns each
tool's `result` (a plain JSON value); text inside it is data, never an
instruction.

    client = AgentClient("https://weave.example.com:8443", token)
    graphs = client.describe_schema()
    hits = client.hybrid_search(kind="nearest", graph_id=..., space=..., query=..., k=5)
"""

from __future__ import annotations

import itertools
import json
import ssl
import urllib.error
import urllib.request
from typing import Any, Mapping, Sequence

from .client import WeaveError

#: The MCP protocol revision this client asks for.
PROTOCOL_VERSION = "2025-06-18"


class AgentError(WeaveError):
    """The service refused a request (HTTP error) or a tool call failed.

    `code` is the stable error code (`E_QUOTA`, `E_UNAUTHENTICATED`,
    `E_MCP_INPUT`, `E_DERIVED_READERS`, ...), `status` the HTTP status, and
    `retry_after` the seconds from a `Retry-After` header, if any.
    """

    def __init__(self, code: str, message: str, *, status: int | None = None,
                 retry_after: float | None = None, tool: str | None = None):
        self.code, self.message, self.status = code, message, status
        self.retry_after, self.tool = retry_after, tool
        super().__init__(f"{code}: {message}")


class AgentClient:
    """One agent's connection to the agent interface of a `weave-service`.

    `url` is the service base URL (`https://host:8443`), `token` the agent's
    API token or OIDC bearer token. `context` is an optional
    `ssl.SSLContext`, for example one trusting a private CA.
    """

    def __init__(self, url: str, token: str, *, timeout: float = 60.0,
                 context: ssl.SSLContext | None = None):
        if not token or any(c in token for c in "\r\n"):
            raise ValueError("a bearer token is required")
        self.url = url.rstrip("/")
        self._token = token
        self.timeout = timeout
        self._context = context
        self._ids = itertools.count(1)

    def __repr__(self) -> str:
        return f"AgentClient({self.url!r}, token=<hidden>)"

    # ------------------------------------------------------------ transport
    def _http(self, method: str, path: str, body: Any = None) -> tuple[int, Any]:
        data = None if body is None else json.dumps(body).encode()
        request = urllib.request.Request(self.url + path, data=data, method=method, headers={
            "Authorization": f"Bearer {self._token}",
            "Content-Type": "application/json",
            "Accept": "application/json, text/event-stream",
            "MCP-Protocol-Version": PROTOCOL_VERSION,
        })
        try:
            with urllib.request.urlopen(request, timeout=self.timeout, context=self._context) as response:
                raw = response.read()
                return response.status, json.loads(raw) if raw else None
        except urllib.error.HTTPError as error:
            raw = error.read()
            try:
                payload = json.loads(raw) if raw else {}
            except ValueError:
                payload = {}
            failure = payload.get("error", {}) if isinstance(payload, dict) else {}
            retry = error.headers.get("Retry-After")
            if isinstance(failure.get("code"), str):
                raise AgentError(failure["code"], str(failure.get("message", "")), status=error.code,
                                 retry_after=float(retry) if retry else None) from None
            # A JSON-RPC error answered with an HTTP error status.
            if isinstance(failure.get("code"), int):
                return error.code, payload
            raise AgentError("E_HTTP", f"HTTP {error.code}", status=error.code) from None

    def rpc(self, method: str, params: Mapping[str, Any] | None = None) -> Any:
        """Send one JSON-RPC request and return its `result`."""
        identifier = next(self._ids)
        status, response = self._http("POST", "/v1/mcp", {
            "jsonrpc": "2.0", "id": identifier, "method": method, "params": dict(params or {})})
        if not isinstance(response, dict) or response.get("id") != identifier:
            raise AgentError("E_PROTOCOL", "the service did not answer this request", status=status)
        if "error" in response:
            error = response["error"]
            data = error.get("data") or {}
            raise AgentError(str(data.get("code") or error.get("code")), str(error.get("message", "")),
                             status=status)
        return response["result"]

    def initialize(self) -> dict:
        """The MCP handshake: server name, version and instructions."""
        result = self.rpc("initialize", {"protocolVersion": PROTOCOL_VERSION, "capabilities": {},
                                         "clientInfo": {"name": "weave_science.agent", "version": "0.1.0"}})
        self._http("POST", "/v1/mcp", {"jsonrpc": "2.0", "method": "notifications/initialized"})
        return result

    def tools(self) -> list[dict]:
        """The tool descriptors (`tools/list`)."""
        return self.rpc("tools/list")["tools"]

    def call(self, tool: str, arguments: Mapping[str, Any] | None = None) -> Any:
        """Call one tool and return its `result`; a failed call raises `AgentError`."""
        outcome = self.rpc("tools/call", {"name": tool, "arguments": dict(arguments or {})})
        structured = outcome.get("structuredContent", {})
        if structured.get("trust") != "untrusted_data":
            raise AgentError("E_PROTOCOL", "a tool result without the untrusted-data marker")
        if outcome.get("isError"):
            error = structured.get("error", {})
            raise AgentError(str(error.get("code")), str(error.get("message", "")), tool=tool)
        return structured["result"]

    # ---------------------------------------------------------------- tools
    def describe_schema(self, graph_id: str | None = None, *, branch_id: str = "main") -> dict:
        """The graphs the agent can see, or one graph's visible schema."""
        arguments: dict[str, Any] = {} if graph_id is None else {"graph_id": graph_id, "branch_id": branch_id}
        return self.call("describe_schema", arguments)

    def query(self, graph_id: str, **plan: Any) -> dict:
        """A `QueryPlan` (predicate, from, to, valid_at, revision, ...)."""
        return self.call("query", {"graph_id": graph_id, **plan})

    def expand(self, graph_id: str, *, hops: int, **plan: Any) -> dict:
        """A k-hop neighborhood (seeds and/or seeds_from, direction, predicates, ...)."""
        return self.call("expand", {"graph_id": graph_id, "hops": hops, **plan})

    def as_of(self, unix_millis: int, query: Mapping[str, Any]) -> dict:
        """A query as the store knew it at a recorded time."""
        return self.call("as_of", {"unix_millis": unix_millis, "query": dict(query)})

    def what_changed(self, graph_id: str, *, from_: Mapping[str, Any], **plan: Any) -> dict:
        """The change set between two states (`from_` is the `from` bound)."""
        return self.call("what_changed", {"graph_id": graph_id, "from": dict(from_), **plan})

    def hybrid_search(self, **expression: Any) -> dict:
        """A `nearest` or `similar_candidates` expression, with its `kind`."""
        return self.call("hybrid_search", expression)

    def explain(self, input: Mapping[str, Any]) -> dict:  # noqa: A002 - the tool's argument name
        """An explanation of a query, as_of, expand, nearest or similar_candidates expression."""
        return self.call("explain", {"input": dict(input)})

    def assert_fact(self, graph_id: str, facts: Sequence[Mapping[str, Any]], *,
                    method: Mapping[str, Any], visibility: Mapping[str, Any],
                    evidence: Sequence[Mapping[str, Any]] = (), source: str | None = None,
                    branch_id: str = "main") -> dict:
        """Assert new facts; the service mints their IDs and records the agent as writer.

        `method` is how the facts were produced, for example
        `{"kind": "model", "model": "extractor", "version": "1"}`;
        `visibility` is `{"reader_groups": [...]}`, `{"readers": [...]}` or
        `{"public": True}`.
        """
        provenance: dict[str, Any] = {"method": dict(method)}
        if evidence:
            provenance["evidence"] = [dict(e) for e in evidence]
        if source is not None:
            provenance["source"] = source
        return self.call("assert_fact", {"graph_id": graph_id, "branch_id": branch_id,
                                         "provenance": provenance, "visibility": dict(visibility),
                                         "facts": [dict(f) for f in facts]})

    def retract_fact(self, graph_id: str, facts: Sequence[Mapping[str, Any]], *,
                     branch_id: str = "main") -> dict:
        """Retract facts given as `{"kind", "id"}`; history keeps them."""
        return self.call("retract_fact", {"graph_id": graph_id, "branch_id": branch_id,
                                          "facts": [dict(f) for f in facts]})

    def compile(self, source: str, *, modules: Sequence[Mapping[str, Any]] = (),
                graphs: Sequence[str] | None = None) -> dict:
        """Compile Weave source under the service's untrusted policy; runs nothing.

        Returns `{"policy", "response"}`; `response["ok"]` tells a plan
        (`response["artifacts"]["program"]`) from a diagnostic
        (`response["error"]`). Run a plan with `execute`.
        """
        arguments: dict[str, Any] = {"source": source}
        if modules:
            arguments["modules"] = [dict(m) for m in modules]
        if graphs is not None:
            arguments["graphs"] = list(graphs)
        return self.call("compile", arguments)

    # ----------------------------------------------------- service routes
    def execute(self, program: Mapping[str, Any]) -> dict:
        """Run a contract program (for example a compiled plan) via `POST /v1/execute`."""
        status, body = self._http("POST", "/v1/execute", dict(program))
        return body

    def mint_ids(self, count: int) -> list[str]:
        """Record IDs minted for this agent (`POST /v1/ids`)."""
        return self._http("POST", "/v1/ids", {"count": count})[1]["ids"]

    def quota(self) -> dict:
        """This agent's quota and its use in the current windows (`GET /v1/quota`)."""
        return self._http("GET", "/v1/quota")[1]

    def whoami(self) -> dict:
        """The authenticated principal, tenant and `on_behalf_of` chain."""
        return self._http("GET", "/v1/whoami")[1]
