#!/usr/bin/env python3
"""Real process death during legacy schema/backfill upgrade."""
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile

ROOT=Path(__file__).resolve().parents[1]
subprocess.run(["cargo","build","--locked","-p","weave-engine","--example","storage_probe","--features","recovery-testing"],cwd=ROOT,check=True)
PROBE=ROOT/"target"/"debug"/"examples"/"storage_probe"
with tempfile.TemporaryDirectory(prefix="weave-migration-") as directory:
    db=Path(directory)/"legacy.db"
    def run(operation,expected=0):
        result=subprocess.run([str(PROBE),str(db),operation],text=True,capture_output=True)
        assert result.returncode==expected,(result.returncode,result.stderr)
        return json.loads(result.stdout) if result.stdout else None
    before=run("seed")
    with sqlite3.connect(db) as c:
        c.execute("DELETE FROM edge_structures")
        c.execute("PRAGMA user_version=5")
        c.execute("DROP TABLE admission_epochs")
    run("crash",82)
    with sqlite3.connect(db) as c:
        assert c.execute("PRAGMA user_version").fetchone()[0]==5
        assert c.execute("SELECT COUNT(*) FROM edge_structures").fetchone()[0]==0
        assert c.execute("SELECT COUNT(*) FROM sqlite_master WHERE name='admission_epochs'").fetchone()[0]==0
        assert c.execute("SELECT COUNT(*) FROM revisions").fetchone()[0]==1
        assert c.execute("SELECT COUNT(*) FROM events").fetchone()[0]==1
    assert run("open")==before
    with sqlite3.connect(db) as c:
        assert c.execute("PRAGMA user_version").fetchone()[0]==18
        assert c.execute("SELECT COUNT(*) FROM edge_structures").fetchone()[0]==1
        assert c.execute("SELECT COUNT(*) FROM sqlite_master WHERE name='admission_epochs'").fetchone()[0]==1
    assert run("open")==before
print(json.dumps({"suite":"migration process death","result":"passed","checks":["pre-COMMIT termination rolls back schema, identities and marker","restart completes upgrade without changing evidence or events","second reopen is idempotent"]}))
