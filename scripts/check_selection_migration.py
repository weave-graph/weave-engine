#!/usr/bin/env python3
"""Marker12 ->13 atomic auxiliary-state migration using existing binaries (no build)."""
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
with tempfile.TemporaryDirectory(prefix='weave-selection-migration-') as tmp:
    db = Path(tmp) / 'db.sqlite'
    plan = Path(tmp) / 'empty.json'
    plan.write_text(json.dumps({'version': '0.15.0', 'commands': []}))
    def run(binary):
        return subprocess.run([str(binary.resolve()), 'run', str(plan), '--db', str(db), '--actor', 'host'], capture_output=True, text=True)
    assert run(a.older_engine).returncode == 0
    with sqlite3.connect(db) as c:
        assert c.execute('PRAGMA user_version').fetchone()[0] == 12
        assert c.execute("SELECT count(*) FROM sqlite_master WHERE name IN ('view_selection','view_schedules','view_schedule_cursors')").fetchone()[0] == 0
    killed = subprocess.run([str(a.storage_probe.resolve()), str(db), 'crash'], capture_output=True, text=True)
    assert killed.returncode == 82, (killed.returncode, killed.stderr)
    with sqlite3.connect(db) as c:
        assert c.execute('PRAGMA user_version').fetchone()[0] == 12
        assert c.execute("SELECT count(*) FROM sqlite_master WHERE name IN ('view_selection','view_schedules','view_schedule_cursors')").fetchone()[0] == 0
    new = run(a.engine)
    assert new.returncode == 0, new.stderr
    with sqlite3.connect(db) as c:
        assert c.execute('PRAGMA user_version').fetchone()[0] == 13
        assert c.execute("SELECT count(*) FROM sqlite_master WHERE name IN ('view_selection','view_schedules','view_schedule_cursors')").fetchone()[0] == 3
        assert c.execute('SELECT count(*) FROM events').fetchone()[0] == 0
    old = run(a.older_engine)
    assert old.returncode != 0 and 'E_STORAGE_VERSION' in old.stderr, old.stderr
    assert run(a.engine).returncode == 0
print('selection migration: precommit rollback, restart, marker12 binary refusal, idempotence passed')
