#!/usr/bin/env python3
"""Typed governance acknowledgment recovery against a real restarted native host."""
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
subprocess.run(["cargo", "build", "--locked", "-p", "weave-engine", "--example", "governance_delivery_probe", "--features", "recovery-testing"], cwd=ROOT, check=True)
PROBE = ROOT / "target/debug/examples/governance_delivery_probe"
with tempfile.TemporaryDirectory(prefix="weave-governance-delivery-") as directory:
    db = Path(directory) / "delivery.db"

    def run(mode, event="", lease="", expected=0):
        result = subprocess.run([str(PROBE), str(db), mode, event, lease], text=True, capture_output=True)
        assert result.returncode == expected, (mode, result.returncode, result.stderr)
        return json.loads(result.stdout) if result.stdout else None

    delivery = run("prepare")
    event, lease = delivery["event"]["id"], delivery["lease"]
    assert delivery["ordinal"] == 1
    run("before", event, lease, 90)
    with sqlite3.connect(db) as c:
        assert c.execute("SELECT count(*) FROM governance_delivery_receipts").fetchone()[0] == 0
        assert c.execute("SELECT count(*) FROM governance_delivery_pending").fetchone()[0] == 1
        assert c.execute("SELECT checkpoint FROM governance_subscriptions").fetchone()[0] == 0
    run("after", event, lease, 91)
    receipt = run("retry", event, lease)
    assert receipt == {"ordinal": 1, "duplicate": True}
    assert run("retry", event, lease) == receipt
    assert run("changed", event, lease)["error"] == "E_LEASE"
    with sqlite3.connect(db) as c:
        assert c.execute("SELECT count(*) FROM governance_delivery_receipts").fetchone()[0] == 1
        assert c.execute("SELECT count(*) FROM governance_delivery_pending").fetchone()[0] == 0
        assert c.execute("SELECT checkpoint FROM governance_subscriptions").fetchone()[0] == 1
        assert c.execute("SELECT count(*) FROM governance_events").fetchone()[0] == 1
        assert c.execute("SELECT count(*) FROM events").fetchone()[0] == 2
    assert run("revoke", event, lease)["error"] == "E_GOV_UNAVAILABLE"
print("governance delivery process-death acceptance: 6 checks passed")
