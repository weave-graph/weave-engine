#!/usr/bin/env python3
"""Synthetic legacy backfill fixture; genuine prior binaries have separate controllers."""
import argparse
from contextlib import closing
import json
from version_profile import store_marker
from retention_migration import RETENTION_TABLES,LIFECYCLE_TABLES,assert_retention_baseline
from pathlib import Path
import sqlite3
import subprocess
import tempfile

ROOT=Path(__file__).resolve().parents[1]
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--probe',type=Path,help='use an existing exact native binary without building')
a=p.parse_args()
if a.probe is None:
    subprocess.run(["cargo","build","--locked","-p","weave-engine","--example","storage_probe","--features","recovery-testing"],cwd=ROOT,check=True)
PROBE=a.probe.resolve() if a.probe else ROOT/"target"/"debug"/"examples"/"storage_probe"
with tempfile.TemporaryDirectory(prefix="weave-migration-") as directory:
    db=Path(directory)/"legacy.db"
    def run(operation,expected=0):
        result=subprocess.run([str(PROBE),str(db),operation],text=True,capture_output=True)
        assert result.returncode==expected,(result.returncode,result.stderr)
        return json.loads(result.stdout) if result.stdout else None
    before=run("seed")
    # This fixture lowers a current seed's marker. It must remove later schema,
    # after proving the seed has only the genuine default non-erasing state.
    # Retaining modern tables under marker5 is a downgrade, not a legacy store.
    assert_retention_baseline(db,store_marker())
    with closing(sqlite3.connect(db)) as c:
        for table in sorted(RETENTION_TABLES|LIFECYCLE_TABLES):
            c.execute('DROP TABLE "'+table+'"')
        c.execute("DROP TABLE head_observations")
        c.execute("DELETE FROM edge_structures")
        c.execute("PRAGMA user_version=5")
        c.execute("DROP TABLE admission_epochs")
    run("crash",82)
    with closing(sqlite3.connect(db)) as c:
        assert c.execute("PRAGMA user_version").fetchone()[0]==5
        assert c.execute("SELECT COUNT(*) FROM edge_structures").fetchone()[0]==0
        assert c.execute("SELECT COUNT(*) FROM sqlite_master WHERE name='admission_epochs'").fetchone()[0]==0
        assert c.execute("SELECT COUNT(*) FROM revisions").fetchone()[0]==1
        assert c.execute("SELECT COUNT(*) FROM events").fetchone()[0]==1
    assert run("open")==before
    with closing(sqlite3.connect(db)) as c:
        assert c.execute("PRAGMA user_version").fetchone()[0]==store_marker()
        assert c.execute("SELECT COUNT(*) FROM edge_structures").fetchone()[0]==1
        assert c.execute("SELECT COUNT(*) FROM sqlite_master WHERE name='admission_epochs'").fetchone()[0]==1
    assert run("open")==before
print(json.dumps({"suite":"migration process death","result":"passed","checks":["pre-COMMIT termination rolls back schema, identities and marker","restart completes upgrade without changing evidence or events","second reopen is idempotent"]}))
