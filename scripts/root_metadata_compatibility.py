#!/usr/bin/env python3
"""Compare source-local metadata selectors and current-result projection IDs on two runtimes."""
import argparse
import copy
import json
from pathlib import Path
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--baseline", type=Path, required=True, help="0.14 engine executable")
parser.add_argument("--candidate", type=Path, required=True, help="0.15 engine executable")
args = parser.parse_args()
def attachment(name, host, key, target):
    return {"id": name, "host": host, "key": key,
            "value": {"kind": "graph", "reference": {"graph_id": target, "revision": "logical:selectors:" + target}},
            "valid_time": {"start": 0, "end": 10}}
def node(id, entity):
    return {"id": id, "entity_id": entity, "space_id": "s"}
def query(graph):
    return {"kind": "query", "query": {"graph_id": graph, "include_metadata": True}}
def metadata(value, kind="graph", id=None, key="next"):
    host = {"kind": kind}
    if id is not None:
        host["id"] = id
    return {"kind": "metadata", "input": value, "host": host, "key": key}
def project(value, nodes=(), edges=()):
    return {"kind": "project", "input": value, "node_ids": list(nodes), "edge_ids": list(edges)}
seed = {"op": "commit_batch", "batch_id": "selectors", "commits": [
    {"graph_id": "A", "data": {"nodes": [node("a", "A")], "attachments": [attachment("ab", {"kind": "graph"}, "next", "B")]}},
    {"graph_id": "B", "data": {"profile": "explicit", "nodes": [node("b", "Entity-B"), node("b2", "Entity-B2")],
        "structural_edges": [{"id": "relation", "predicate": "p", "from": "b", "to": "b2"}],
        "assertions": [{"id": "claim", "edge_id": "relation", "source": "observer", "valid_time": {"start": 0, "end": 10}}],
        "attachments": [attachment("node-next", {"kind": "node", "id": "b"}, "node", "C"),
            attachment("edge-next", {"kind": "edge", "id": "relation"}, "edge", "C"),
            attachment("assertion-next", {"kind": "assertion", "id": "claim"}, "assertion", "C"),
            attachment("graph-next", {"kind": "graph"}, "graph", "C"),
            attachment("entity-next", {"kind": "entity", "id": "Entity-B"}, "entity", "C")]}},
    {"graph_id": "C", "data": {"nodes": [node("c", "Entity-C")]}}
]}
summary = {}
with tempfile.TemporaryDirectory(prefix="weave-metadata-compatibility-") as directory:
    root = Path(directory)
    for label, binary in [("baseline", args.baseline), ("candidate", args.candidate)]:
        database = root / label
        def execute(commands, error=None):
            path = root / "plan.json"
            path.write_text(json.dumps({"version": "0.14.0", "commands": commands}))
            result = subprocess.run([str(binary.resolve()), "run", "--db", str(database), "--actor", "reader",
                "--write", "A", "--write", "B", "--write", "C", "--write", "Marker", str(path)], text=True, capture_output=True)
            if error:
                assert result.returncode != 0, result.stdout
                actual = json.loads(result.stderr)
                assert actual["code"] == error, actual
                return actual
            assert result.returncode == 0, result.stderr
            return json.loads(result.stdout)
        def evaluate(value):
            return execute([{"op": "evaluate", "value": value}])[-1]["result"]
        execute([copy.deepcopy(seed)])
        value = metadata(query("A"))
        direct = evaluate(query("B"))
        reached = evaluate(value)
        assert {n["id"] for n in direct["graph"]["nodes"]} == {"b", "b2"}
        assert {(n["entity_id"], n["space_id"]) for n in reached["graph"]["nodes"]} == {("Entity-B", "s"), ("Entity-B2", "s")}
        current = next(n["id"] for n in reached["graph"]["nodes"] if n["entity_id"] == "Entity-B")
        if label == "candidate":
            assert current.startswith("metadata-node:")
        else:
            assert current == "b"
        current_projection = evaluate(project(value, [current]))
        assert [n["entity_id"] for n in current_projection["graph"]["nodes"]] == ["Entity-B"]
        edge_projection = evaluate(project(value, edges=["claim"]))
        assert {n["id"] for n in edge_projection["graph"]["nodes"]} == {n["id"] for n in reached["graph"]["nodes"]}
        assert [e["id"] for e in edge_projection["graph"]["edges"]] == ["claim"]
        assert [n["id"] for n in evaluate(project(query("B"), ["b"]))["graph"]["nodes"]] == ["b"]
        if label == "candidate":
            execute([{"op": "commit", "graph_id": "Marker", "data": {}},
                {"op": "evaluate", "value": project(value, ["b"])}], error="E_PROJECT_MEMBER")
            execute([{"op": "query", "query": {"graph_id": "Marker"}}], error="E_UNAVAILABLE")
        else:
            assert evaluate(project(value, ["b"]))["graph"]["nodes"][0]["id"] == "b"
        # Entity and space identify semantics, not a current local node ID alias.
        execute([{"op": "evaluate", "value": project(value, ["Entity-B"])}], error="E_PROJECT_MEMBER")
        outcomes = {}
        for kind, host_id in [("graph", None), ("edge", "claim"), ("assertion", "claim"), ("entity", "Entity-B"), ("node", "b")]:
            nested = evaluate(metadata(value, kind, host_id, kind))
            outcomes[kind] = {"coverage": nested["coverage"], "entities": [n["entity_id"] for n in nested["graph"]["nodes"]]}
            assert outcomes[kind] == {"coverage": "complete", "entities": ["Entity-C"]}, (label, kind, nested)
        exact = evaluate(metadata(value, "node", current, "node"))
        assert exact["coverage"] == "complete" and exact["graph"]["nodes"][0]["entity_id"] == "Entity-C"
        summary[label] = {"hosts": outcomes, "projection_ids": "current result IDs; entity identity is separate",
            "original_metadata_node_projection": "E_PROJECT_MEMBER with atomic rollback" if label == "candidate" else "original local ID retained"}
print(json.dumps({"status": "passed", "matrix": summary}, indent=2))
