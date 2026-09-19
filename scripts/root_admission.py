#!/usr/bin/env python3
"""Independent signed-publication process-death and durable nonce acceptance."""
import copy
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
subprocess.run(["cargo", "build", "--locked", "-p", "weave-engine", "--example", "admission_probe", "--features", "recovery-testing"], cwd=ROOT, check=True)
PROBE = ROOT / "target" / "debug" / "examples" / "admission_probe"

with tempfile.TemporaryDirectory(prefix="weave-admission-") as directory:
    tmp = Path(directory)
    database = tmp / "graph.db"
    request_file = tmp / "request.json"

    def run(request, expected=0):
        request_file.write_text(json.dumps(request))
        result = subprocess.run([str(PROBE), str(database), str(request_file)], capture_output=True, text=True)
        assert result.returncode == expected, (result.returncode, expected, result.stdout, result.stderr)
        return json.loads(result.stdout) if result.stdout else result.stderr

    def counts():
        with sqlite3.connect(database) as connection:
            return tuple(connection.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0] for table in ("heads", "revisions", "events", "admission_receipts"))

    principal = run({"op": "install"})["principal"]
    commit = {"graph_id": "recovery", "branch_id": "main", "expected_head": None, "data": {
        "nodes": [{"id": "a", "entity_id": "A", "space_id": "s", "readers": [principal]}, {"id": "b", "entity_id": "B", "space_id": "s", "readers": [principal]}],
        "edges": [{"id": "edge", "predicate": "depends", "from": "a", "to": "b", "valid_time": {"start": 0}, "readers": [principal]}]}}
    request = {"op": "publish", "commit": commit, "nonce": "aa" * 32}
    run({**request, "crash_before_commit": True}, 80)
    assert counts() == (0, 0, 0, 0), counts()
    first = run(request)
    assert first["duplicate"] is False
    assert counts() == (1, 1, 1, 1), counts()
    duplicate = run(request)
    assert duplicate["duplicate"] is True and duplicate["result"] == first["result"]
    assert counts() == (1, 1, 1, 1), counts()

    next_commit = copy.deepcopy(commit)
    next_commit["expected_head"] = first["result"]["revision"]
    next_commit["data"]["edges"][0]["properties"] = {"observation": "after restart"}
    next_request = {"op": "publish", "commit": next_commit, "nonce": "bb" * 32}
    run({**next_request, "crash_after_commit": True}, 81)
    assert counts() == (1, 2, 2, 2), counts()
    acknowledged = run(next_request)
    assert acknowledged["duplicate"] is True
    with sqlite3.connect(database) as connection:
        assert connection.execute("SELECT revision FROM heads WHERE graph_id='recovery'").fetchone()[0] == acknowledged["result"]["revision"]
        assert connection.execute("SELECT event_id FROM events ORDER BY sequence DESC LIMIT 1").fetchone()[0] == acknowledged["result"]["event_id"]
    assert counts() == (1, 2, 2, 2), counts()

    changed = copy.deepcopy(next_request)
    changed["commit"]["data"]["edges"][0]["properties"]["observation"] = "different body"
    assert "E_REPLAY" in run(changed, 1)
    assert counts() == (1, 2, 2, 2), counts()

    public = copy.deepcopy(next_request)
    public["nonce"] = "cc" * 32
    public["commit"]["expected_head"] = acknowledged["result"]["revision"]
    public["commit"]["data"]["edges"][0]["readers"] = []
    assert "E_EGRESS" in run(public, 1)
    assert counts() == (1, 2, 2, 2), counts()

print(json.dumps({"suite": "signed admission process recovery", "checks": 6, "result": "passed", "boundaries": ["process exit before SQLite COMMIT", "process exit after COMMIT before response", "durable exact retry", "nonce conflict rejection", "private egress rollback"]}))
