#!/usr/bin/env python3
"""Actual marker13 ->14 death/restart/refusal; uses preserved binaries, never builds."""
import argparse
import json
import sqlite3
import subprocess
import tempfile
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument('--older-engine', type=Path, required=True)
p.add_argument('--engine', type=Path, default=Path('target/debug/weave-engine'))
p.add_argument('--storage-probe', type=Path, default=Path('target/debug/examples/storage_probe'))
a = p.parse_args()
with tempfile.TemporaryDirectory(prefix='weave-compiled-migration-') as tmp:
    db = Path(tmp) / 'db.sqlite'
    plan = Path(tmp) / 'plan.json'
    def run(binary):
        return subprocess.run([str(binary.resolve()), 'run', str(plan), '--db', str(db), '--actor', 'host', '--write', 'source'], capture_output=True, text=True)
    plan.write_text(json.dumps({'version': '0.15.0', 'commands': [{'op':'commit', 'graph_id':'source', 'data':{'nodes':[{'id':'n','entity_id':'E','space_id':'s'}]}}]}))
    seeded = run(a.older_engine)
    assert seeded.returncode == 0, seeded.stderr
    def state():
        with sqlite3.connect(db) as c:
            return (c.execute('PRAGMA user_version').fetchone()[0], c.execute("SELECT count(*) FROM sqlite_master WHERE name='view_sources'").fetchone()[0], c.execute("SELECT count(*) FROM pragma_table_info('live_views') WHERE name='source_digest'").fetchone()[0], c.execute('SELECT revision,data FROM revisions ORDER BY revision').fetchall(), c.execute('SELECT count(*) FROM events').fetchone()[0])
    before = state()
    assert before[:3] == (13,0,0), before
    killed = subprocess.run([str(a.storage_probe.resolve()), str(db), 'crash'], capture_output=True, text=True)
    assert killed.returncode == 82, (killed.returncode, killed.stderr)
    assert state() == before
    plan.write_text(json.dumps({'version':'0.15.0','commands':[]}))
    upgraded = run(a.engine)
    assert upgraded.returncode == 0, upgraded.stderr
    after = state()
    assert after[:3] == (14,1,1) and after[3:] == before[3:], (before,after)
    refused = run(a.older_engine)
    assert refused.returncode != 0 and 'E_STORAGE_VERSION' in refused.stderr, refused.stderr
    assert run(a.engine).returncode == 0 and state() == after
print('compiled view migration: precommit death rollback, restart/preserved rows, marker13 refusal, idempotence passed')
