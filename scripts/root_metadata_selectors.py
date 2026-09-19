#!/usr/bin/env python3
"""Independent source regression: nested local node selectors across metadata wrappers."""
import argparse
import copy
import json
from pathlib import Path
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--engine", type=Path, required=True, help="engine executable")
parser.add_argument("--language", type=Path, required=True, help="language checkout")
args = parser.parse_args()
source = '''transaction chain {
 graph A { node "a" entity "A" space "s";
 attachment "ab" on node "a" key "next" graph "B" revision "logical:chain:B" valid 0 until 10;
 attachment "cycle-a" on graph key "cycle" graph "B" revision "logical:chain:B" valid 0 until 10; }
 graph Other { node "a2" entity "Other" space "s";
 attachment "other-b" on node "a2" key "next" graph "B" revision "logical:chain:B" valid 0 until 10; }
 graph B { node "b" entity "B" space "s";
 attachment "bc" on node "b" key "next" graph "C" revision "logical:chain:C" valid 0 until 10;
 attachment "cycle-b" on graph key "cycle" graph "C" revision "logical:chain:C" valid 0 until 10; }
 graph C { node "c" entity "C" space "s";
 attachment "cycle-c" on graph key "cycle" graph "A" revision "logical:chain:A" valid 0 until 10; }
}
use Input graph "A" revision "logical:chain:A";
lens FullA from Input { metadata depth 8; }
metadata BValue from FullA on node "a" key "next";
metadata CValue from BValue on node "b" key "next";
metadata BRepeated from FullA on node "a" key "next";
use OtherInput graph "Other" revision "logical:chain:Other";
lens FullOther from OtherInput { metadata depth 8; }
metadata BOther from FullOther on node "a2" key "next";
union Distinct from BValue with BOther;
union Idempotent from BValue with BRepeated;
metadata CycleB from FullA on graph key "cycle";
metadata CycleC from CycleB on graph key "cycle";
metadata CycleA from CycleC on graph key "cycle";
'''
with tempfile.TemporaryDirectory(prefix="weave-metadata-selectors-") as directory:
    work = Path(directory)
    entry = work / "source.weave"
    entry.write_text(source)
    plan = json.loads(subprocess.check_output(["cargo", "run", "--locked", "--quiet", "--",
        "plan", str(entry)], cwd=args.language.resolve()))
    # Raw host fixture makes only the first path private; source does not mint authority.
    plan["commands"][0]["commits"][0]["data"]["attachments"][0]["readers"] = ["reader"]
    database = work / "store.db"
    def execute(program, actor="reader", writes=()):
        path = work / "plan.json"
        path.write_text(json.dumps(program))
        command = [str(args.engine.resolve()), "run", "--db", str(database), "--actor", actor]
        for graph in writes:
            command += ["--write", graph]
        result = subprocess.run(command + [str(path)], text=True, capture_output=True)
        assert result.returncode == 0, result.stderr
        return json.loads(result.stdout)
    output = execute(plan, writes=["A", "Other", "B", "C"])
    values = {command["name"]: result["result"] for command, result in zip(plan["commands"], output)
              if command["op"] == "bind"}
    result = values["CValue"]
    assert result["coverage"] == "complete", result
    assert [node["entity_id"] for node in result["graph"]["nodes"]] == ["C"], result
    assert all(not origins for origins in result["node_origins"].values()), result
    c = result["graph"]["nodes"][0]
    assert {"graph_id": "C", "revision": "logical:chain:C", "node_id": "c"} in c["derived_nodes"]
    assert {(p["graph_id"], p["assertion_id"]) for p in c["derived_from"]} >= {("A", "ab"), ("B", "bc")}
    b = values["BValue"]["graph"]["nodes"][0]
    assert b["id"].startswith("metadata-node:") and b["id"] != "b"
    assert b["id"] == values["BRepeated"]["graph"]["nodes"][0]["id"]
    assert b["id"] != values["BOther"]["graph"]["nodes"][0]["id"]
    assert len(values["Distinct"]["graph"]["nodes"]) == 2
    assert len(values["Idempotent"]["graph"]["nodes"]) == 1
    assert [n["entity_id"] for n in values["CycleA"]["graph"]["nodes"]] == ["A"]
    assert values["CycleA"]["coverage"] == "complete"
    # Copying only the wrapper node cannot discard the private original path proof.
    copied = copy.deepcopy(b)
    copied["readers"] = []
    execute({"version": result["version"], "commands": [
        {"op": "commit", "graph_id": "Saved", "data": {"nodes": [copied]}}]}, writes=["Saved"])
    saved = execute({"version": result["version"], "commands": [
        {"op": "query", "query": {"graph_id": "Saved"}}]}, actor="outsider")[-1]["result"]
    assert saved["graph"]["nodes"] == []
print("Metadata selector acceptance passed: unchanged nested source IDs, distinct/idempotent wrappers, cycle closure and node-only path privacy")
