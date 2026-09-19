#!/usr/bin/env python3
"""Process-death acceptance for signed isolated-proposal promotion; fixed local test keys only."""
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile
ROOT=Path(__file__).resolve().parents[1]
subprocess.run(["cargo","build","--locked","-p","weave-engine","--example","integration_probe","--features","recovery-testing"],cwd=ROOT,check=True)
PROBE=ROOT/"target"/"debug"/"examples"/"integration_probe"
with tempfile.TemporaryDirectory(prefix="weave-integration-") as directory:
    db=Path(directory)/"receiver.db"
    def run(operation,expected=0):
        result=subprocess.run([str(PROBE),str(db),operation],text=True,capture_output=True)
        assert result.returncode==expected,(operation,result.returncode,result.stderr)
        return json.loads(result.stdout) if result.stdout else None
    assert run("prepare")["events"]==0
    run("before",86)
    with sqlite3.connect(db) as c:
        for table in ["revisions","heads","events","edge_structures","schema_registry","integration_receipts"]:
            assert c.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]==0,table
        assert c.execute("SELECT COUNT(*) FROM isolated_proposals").fetchone()[0]==1
    run("after",87)
    result=run("retry")
    assert result["receipt"]["duplicate"] and result["events"]==1 and result["edges"]==1
    again=run("retry")
    assert again==result
    with sqlite3.connect(db) as c:
        assert c.execute("SELECT COUNT(*) FROM integration_receipts").fetchone()[0]==1
        assert c.execute("SELECT COUNT(*) FROM heads WHERE branch_id='offline'").fetchone()[0]==1
        assert c.execute("SELECT COUNT(*) FROM heads WHERE branch_id='main'").fetchone()[0]==0
        assert c.execute("SELECT COUNT(*) FROM isolated_proposals").fetchone()[0]==1
    assert run("changed")["error"]=="E_REPLAY"
    assert run("revoke")["error"]
print(json.dumps({"suite":"signed integration process death","result":"passed","checks":["before outer commit all promotion effects roll back","isolated proposal survives failed decision","after commit lost response retries exact receipt","one accepted occurrence and independent offline branch","changed nonce body rejected","current revocation blocks cached decision retry"]}))
