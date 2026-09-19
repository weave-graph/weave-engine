#!/usr/bin/env python3
"""Native threshold acceptance across process death; deterministic local test keys only."""
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
subprocess.run(["cargo", "build", "--locked", "-p", "weave-engine", "--example", "governance_probe", "--features", "recovery-testing"], cwd=ROOT, check=True)
PROBE = ROOT / "target/debug/examples/governance_probe"
with tempfile.TemporaryDirectory(prefix="weave-governance-") as directory:
    db = Path(directory) / "governance.db"

    def run(mode, expected=0):
        result = subprocess.run([str(PROBE), str(db), mode], text=True, capture_output=True)
        assert result.returncode == expected, (mode, result.returncode, result.stderr)
        return json.loads(result.stdout) if result.stdout else None

    assert run("prepare")["events"] == 0
    run("before", 88)
    with sqlite3.connect(db) as connection:
        for table in ["governance_decisions", "governance_receipts", "governance_events"]:
            assert connection.execute(f"SELECT count(*) FROM {table}").fetchone()[0] == 0
        assert connection.execute("SELECT decision_id,source FROM governance_views").fetchone() == (None, None)
        assert connection.execute("SELECT count(*) FROM governance_proposals").fetchone()[0] == 2
        assert connection.execute("SELECT count(*) FROM governance_approvals").fetchone()[0] == 4
    run("after", 89)
    receipt = run("retry")
    assert receipt["duplicate"] is True
    assert run("retry") == receipt
    assert run("inspect")["decision_id"] == receipt["decision_id"]
    assert run("changed")["error"] == "E_GOV_REPLAY"
    assert run("expired")["error"] == "E_GOV_QUORUM"
    with sqlite3.connect(db) as connection:
        for table in ["governance_decisions", "governance_receipts", "governance_events"]:
            assert connection.execute(f"SELECT count(*) FROM {table}").fetchone()[0] == 1
        assert connection.execute("SELECT count(*) FROM events").fetchone()[0] == 1
print("governance process-death acceptance: 6 checks passed")
