#!/usr/bin/env python3
"""Actual process termination across identity acceptance SQL/commit boundaries."""
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile
ROOT=Path(__file__).resolve().parents[1]
subprocess.run(["cargo","build","--locked","-p","weave-engine","--example","identity_probe","--features","recovery-testing"],cwd=ROOT,check=True)
PROBE=ROOT/"target"/"debug"/"examples"/"identity_probe"
with tempfile.TemporaryDirectory(prefix="weave-identity-") as directory:
    db=Path(directory)/"identity.db"
    def run(op,code=0):
        result=subprocess.run([str(PROBE),str(db),op],text=True,capture_output=True)
        assert result.returncode==code,(result.returncode,result.stderr)
        return json.loads(result.stdout) if result.stdout else None
    assert run("seed")=={"events":2,"head":None}
    run("crash-before",83)
    with sqlite3.connect(db) as c:
        for table in ["identity_decisions","identity_mapping_heads","identity_memberships","identity_receipts"]:
            assert c.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]==0,table
        assert c.execute("SELECT COUNT(*) FROM revisions").fetchone()[0]==2
        assert c.execute("SELECT COUNT(*) FROM events").fetchone()[0]==2
        assert c.execute("SELECT COUNT(*) FROM identity_candidates").fetchone()[0]==1
        assert c.execute("SELECT COUNT(*) FROM schema_registry WHERE id='weave:identity-membership'").fetchone()[0]==0
    run("crash-after",84)
    with sqlite3.connect(db) as c:
        stored=json.loads(c.execute("SELECT receipt FROM identity_receipts").fetchone()[0])
        for table in ["identity_decisions","identity_mapping_heads","identity_receipts"]:
            assert c.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]==1,table
        assert c.execute("SELECT COUNT(*) FROM identity_memberships").fetchone()[0]==2
        assert c.execute("SELECT COUNT(*) FROM revisions").fetchone()[0]==3
        assert c.execute("SELECT COUNT(*) FROM events").fetchone()[0]==3
    retried=run("retry")
    assert retried["events"]==3
    expected=dict(stored,duplicate=True)
    assert retried["receipt"]==expected
    assert run("retry")==retried
    assert run("changed")=={"error":"E_REPLAY","events":3}
    assert run("resolve")=={"nodes":2,"edges":1}
    assert run("revoke")=={"events":3}
    assert run("retry")=={"error":"E_IDENTITY_UNAVAILABLE","events":3}
    assert run("resolve")=={"error":"E_IDENTITY_UNAVAILABLE"}
print(json.dumps({"suite":"identity acceptance process death","result":"passed","checks":["precommit death rolls back graph event registry membership schema and receipt","postcommit lost response retains one atomic decision","fresh retries return exact original receipt without duplicate event","changed body nonce fails","restart resolves accepted independent identifiers","revocation rejects pinned resolution and receipt retry"]}))
