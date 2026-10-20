"""The agent SDK against a real `weave-service`; set WEAVE_SERVICE_BINARY to enable.

The same operations as the MCP tools (B4, #40): agent X asserts a finding
from a document, agent Y of the same team finds it with hybrid search and
the explanation names X; agent Z of another team sees none of it; quotas,
revocation and the data/code boundary reach Python as stable error codes.
"""

import json
import os
import socket
import subprocess
import tempfile
import time
import unittest
import urllib.request
from pathlib import Path

from weave_science import AgentClient, AgentError

BINARY = os.environ.get("WEAVE_SERVICE_BINARY")
GRAPH, SPACE = "maintenance", {"id": "notes", "revision": "1"}
FINDING = "Bearing on unit 7 replaced after the vibration alarm"


class ClientShape(unittest.TestCase):
    def test_tokens_are_required_and_never_printed(self):
        client = AgentClient("http://127.0.0.1:1/", "wvs_secret")
        self.assertNotIn("wvs_secret", repr(client))
        self.assertEqual(client.url, "http://127.0.0.1:1")
        for bad in ["", "a\nb"]:
            with self.assertRaises(ValueError):
                AgentClient("http://127.0.0.1:1", bad)


@unittest.skipUnless(BINARY, "set WEAVE_SERVICE_BINARY for service integration")
class AgentService(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.data = str(Path(cls.tmp.name) / "data")
        created = subprocess.run([BINARY, "tenant", "create", "--data", cls.data, "--tenant", "acme",
                                  "--admin", "root"], capture_output=True, check=True)
        cls.root = json.loads(created.stdout)["token"]["token"]
        with socket.socket() as s:
            s.bind(("127.0.0.1", 0))
            port = s.getsockname()[1]
        cls.url = f"http://127.0.0.1:{port}"
        cls.server = subprocess.Popen([BINARY, "serve", "--data", cls.data, "--listen", f"127.0.0.1:{port}",
                                       "--plaintext-loopback"], stdout=subprocess.DEVNULL,
                                      stderr=subprocess.DEVNULL)
        for _ in range(600):
            try:
                if cls.http("GET", "/v1/health", cls.root)[0] == 200:
                    break
            except OSError:
                time.sleep(0.05)
        else:
            raise RuntimeError("weave-service did not start")
        cls.setup_tenant()

    @classmethod
    def tearDownClass(cls):
        cls.server.terminate()
        cls.server.wait(timeout=30)
        cls.tmp.cleanup()

    @classmethod
    def http(cls, method, path, token, body=None):
        data = None if body is None else json.dumps(body).encode()
        request = urllib.request.Request(cls.url + path, data=data, method=method, headers={
            "Authorization": f"Bearer {token}", "Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                return response.status, json.loads(response.read() or b"null")
        except urllib.error.HTTPError as error:
            return error.code, json.loads(error.read() or b"null")

    @classmethod
    def setup_tenant(cls):
        root = cls.root
        assert cls.http("POST", "/v1/graphs", root, {"graph_id": GRAPH})[0] == 201
        status, out = cls.http("POST", "/v1/registry", root, {"changes": [
            {"change": "add_principal", "id": "alice", "kind": "user"},
            {"change": "add_principal", "id": "carol", "kind": "user"},
            {"change": "add_principal", "id": "team-a", "kind": "group"},
            {"change": "add_member", "group": "team-a", "member": "alice"},
            {"change": "grant", "graph_id": GRAPH, "subject": "alice", "permission": "write"},
            {"change": "grant", "graph_id": GRAPH, "subject": "carol", "permission": "write"}]})
        assert status == 200, out
        unit, note, vec = cls.http("POST", "/v1/ids", root, {"count": 3})[1]["ids"]
        cls.unit = unit
        status, out = cls.http("POST", "/v1/execute", root, {"version": "1.2.0", "commands": [{
            "op": "write", "graph_id": GRAPH, "changes": [
                {"change": "assert", "fact": {"kind": "embedding_space", **SPACE, "encoder": "test",
                 "encoder_version": "1", "preprocessing": "none", "dimensions": 3,
                 "metric": "cosine_distance"}},
                {"change": "assert", "fact": {"kind": "node", "id": unit, "entity_id": unit,
                 "space_id": "unit", "properties": {"serial": "U-7"}}},
                {"change": "assert", "fact": {"kind": "node", "id": note, "entity_id": note,
                 "space_id": "note", "properties": {"text": "Routine inspection"}}},
                {"change": "assert", "fact": {"kind": "embedding", "id": vec, "space": SPACE,
                 "subject": {"kind": "node", "node_id": note}, "vector": [0.0, 1.0, 0.0],
                 "valid_time": {"start": 0}}}]}]})
        assert status == 200, out
        cls.owners = {owner: cls.http("POST", "/v1/tokens", root, {"principal": owner})[1]["token"]
                      for owner in ["alice", "carol"]}
        cls.x = cls.new_agent("alice", "x-bot")[0]
        cls.y = cls.new_agent("alice", "y-bot")[0]
        cls.z = cls.new_agent("carol", "z-bot")[0]

    @classmethod
    def new_agent(cls, owner, agent):
        """Register an agent of `owner` and return (client, token record)."""
        owner_token = cls.owners[owner]
        status, out = cls.http("POST", "/v1/registry", owner_token, {"changes": [
            {"change": "add_agent", "id": agent, "owner": owner, "delegation": {
                "scopes": [{"graph_id": GRAPH, "permission": "write"}], "exercise": {"kind": "owner_all"}}}]})
        assert status == 200, out
        issued = cls.http("POST", "/v1/tokens", owner_token, {"principal": agent})[1]
        return AgentClient(cls.url, issued["token"]), issued

    def assert_finding(self, client):
        return client.assert_fact(GRAPH, [
            {"kind": "node", "ref": "finding", "space_id": "finding", "properties": {"text": FINDING}},
            {"kind": "edge", "predicate": "maint:about", "from": {"ref": "finding"}, "to": self.unit,
             "valid_time": {"start": 1000}},
            {"kind": "embedding", "space": SPACE, "subject": {"kind": "node", "ref": "finding"},
             "vector": [1.0, 0.0, 0.1], "valid_time": {"start": 1000}}],
            method={"kind": "model", "model": "x-extractor", "version": "2026-10"},
            evidence=[{"kind": "artifact", "uri": "doc://maintenance/log-42", "digest": "sha256:00"}],
            source="maintenance log 42", visibility={"reader_groups": ["team-a"]})

    def test_the_sdk_is_the_mcp_tool_surface(self):
        info = self.y.initialize()
        self.assertEqual(info["serverInfo"]["name"], "weave-service")
        names = [t["name"] for t in self.y.tools()]
        self.assertEqual(names, ["assert_fact", "retract_fact", "query", "expand", "as_of", "what_changed",
                                 "hybrid_search", "explain", "describe_schema", "compile"])
        for method in names:
            self.assertTrue(callable(getattr(AgentClient, method)), method)

    def test_x_asserts_y_finds_and_explains_and_z_sees_nothing(self):
        asserted = self.assert_finding(self.x)
        node, edge = asserted["facts"][0]["id"], asserted["facts"][1]["id"]
        search = {"kind": "nearest", "graph_id": GRAPH, "space": SPACE,
                  "query": {"kind": "vector", "vector": [0.9, 0.0, 0.2]}, "k": 1}
        found = self.y.hybrid_search(**search)
        self.assertEqual(found["results"][0]["result"]["neighbors"][0]["subject"]["node_id"], node)
        explained = self.y.explain({"kind": "expand", "graph_id": GRAPH, "hops": 1, "direction": "out",
                                    "seeds_from": search})
        sources = explained["results"][0]["result"]["fact_provenance"]
        self.assertEqual([s["fact"]["assertion_id"] for s in sources], [edge])
        self.assertEqual(sources[0]["provenance"]["writer"], "x-bot")
        self.assertEqual(sources[0]["provenance"]["on_behalf_of"], ["alice"])
        self.assertEqual(sources[0]["provenance"]["method"]["model"], "x-extractor")
        # Z: the same search and schema show no trace of the finding.
        hidden = self.z.hybrid_search(**search)
        self.assertNotEqual(hidden["results"][0]["result"]["neighbors"][0]["subject"]["node_id"], node)
        self.assertNotIn(FINDING, json.dumps(self.z.query(GRAPH)))
        schema = self.z.describe_schema(GRAPH)
        self.assertNotIn("finding", [s["space_id"] for s in schema["node_spaces"]])
        with self.assertRaises(AgentError) as caught:
            self.z.retract_fact(GRAPH, [{"kind": "edge", "id": edge}])
        self.assertTrue(caught.exception.code.startswith("E_"))
        # describe_schema is enough to build a query.
        schema = self.y.describe_schema(GRAPH)
        example = next(p for p in schema["predicates"] if p["predicate"] == "maint:about")["example"]
        self.assertEqual(example["tool"], "query")
        self.assertTrue(self.y.query(**example["arguments"])["results"][0]["result"]["graph"]["edges"])

    def test_boundary_quota_compile_and_revocation_errors_are_codes(self):
        with self.assertRaises(AgentError) as caught:
            self.x.call("assert_fact", {"graph_id": GRAPH, "facts": [{"kind": "node", "space_id": "s"}],
                                        "provenance": {"method": {"kind": "human"}, "writer": "root"},
                                        "visibility": {"public": True}})
        self.assertEqual(caught.exception.code, "E_MCP_INPUT")
        with self.assertRaises(AgentError) as caught:
            self.x.compile('use G graph "x";')
        self.assertEqual(caught.exception.code, "E_UNSUPPORTED")
        with self.assertRaises(AgentError) as caught:
            self.x.call("no_such_tool")
        self.assertEqual(caught.exception.code, "E_MCP_UNKNOWN_TOOL")
        # An operator quota applies on the next call.
        q, issued = self.new_agent("carol", "q-bot")
        subprocess.run([BINARY, "quota", "set", "--data", self.data, "--tenant", "acme", "--principal", "q-bot",
                        "--requests-per-minute", "unlimited", "--writes-per-hour", "0"],
                       capture_output=True, check=True)
        with self.assertRaises(AgentError) as caught:
            self.assert_finding(q)
        self.assertEqual(caught.exception.code, "E_QUOTA")
        self.assertEqual(q.quota()["writes_per_hour"], 0)
        # Revoking the token fails the next call with the uniform 401.
        self.assertEqual(self.http("DELETE", f"/v1/tokens/{issued['token_id']}", self.root)[0], 200)
        with self.assertRaises(AgentError) as caught:
            q.describe_schema()
        self.assertEqual((caught.exception.code, caught.exception.status), ("E_UNAUTHENTICATED", 401))


if __name__ == "__main__":
    unittest.main()
