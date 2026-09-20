#!/usr/bin/env python3
"""Native threshold acceptance across process death; deterministic local test keys only."""
import argparse
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument("--older-engine", type=Path, help="optional marker11 native engine binary")
args = parser.parse_args()
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
        for table in ["governance_decisions", "governance_receipts", "governance_events", "governance_graphs", "governance_exposure_decisions"]:
            assert connection.execute(f"SELECT count(*) FROM {table}").fetchone()[0] == 0
        assert connection.execute("SELECT count(*) FROM revisions WHERE graph_id LIKE 'weave:governance:%'").fetchone()[0] == 0
        assert connection.execute("SELECT count(*) FROM heads WHERE graph_id LIKE 'weave:governance:%'").fetchone()[0] == 0
        assert connection.execute("SELECT count(*) FROM schema_registry WHERE id LIKE 'weave:governance:%'").fetchone()[0] == 0
        assert connection.execute("SELECT count(*) FROM events").fetchone()[0] == 1
        assert connection.execute("SELECT decision_id,source FROM governance_views").fetchone() == (None, None)
        assert connection.execute("SELECT count(*) FROM governance_proposals").fetchone()[0] == 2
        assert connection.execute("SELECT count(*) FROM governance_approvals").fetchone()[0] == 4
    run("after", 89)
    receipt = run("retry")
    assert receipt["duplicate"] is True
    assert run("retry") == receipt
    assert run("inspect")["decision_id"] == receipt["decision_id"]
    accepted = run("accepted")
    assert len(accepted["graph"]["nodes"]) == 1
    assert len(accepted["graph"]["influence"]["assertions"]) == 1
    assert accepted["graph"]["influence"]["assertions"][0]["assertion_id"] == "accepted"
    assert run("expired_read")["error"] == "E_GOV_UNAVAILABLE"
    if args.older_engine:
        empty_plan = Path(directory) / "empty.json"
        empty_plan.write_text(json.dumps({"version":"0.15.0","commands":[]}))
        old = subprocess.run([str(args.older_engine.resolve()), "run", str(empty_plan), "--db", str(db), "--actor", "reader"], text=True, capture_output=True)
        assert old.returncode != 0 and "E_STORAGE_VERSION" in old.stderr + old.stdout, (old.returncode, old.stdout, old.stderr)
    assert run("changed")["error"] == "E_GOV_REPLAY"
    assert run("expired")["error"] == "E_GOV_QUORUM"
    with sqlite3.connect(db) as connection:
        for table in ["governance_decisions", "governance_receipts", "governance_events", "governance_graphs", "governance_exposure_decisions"]:
            assert connection.execute(f"SELECT count(*) FROM {table}").fetchone()[0] == 1
        assert connection.execute("SELECT count(*) FROM events").fetchone()[0] == 2
        assert connection.execute("SELECT count(*) FROM revisions WHERE graph_id LIKE 'weave:governance:%'").fetchone()[0] == 1
        assert connection.execute("SELECT count(*) FROM heads WHERE graph_id LIKE 'weave:governance:%'").fetchone()[0] == 1
        assert connection.execute("PRAGMA user_version").fetchone()[0] == 16
print("governance process-death acceptance: 8 checks passed (plus old-binary refusal when supplied)")
