#!/usr/bin/env python3
"""Trace actual default roots in populated migration stores without mutating their rows.

Run one existing migration controller, inspect its actual SQLite stores before
temporary cleanup, and preserve the original fixture files and plan response.
Independent destination stores and journals are excluded by their storage marker.
"""
import argparse
import hashlib
import json
from pathlib import Path
import runpy
import shutil
import sqlite3
import subprocess
import sys
import tempfile


def snapshot(path):
    with sqlite3.connect(path) as connection:
        tables = connection.execute(
            "SELECT name,sql FROM sqlite_master WHERE type='table' ORDER BY name"
        ).fetchall()
        return [(name, schema, sorted(connection.execute(
            'SELECT * FROM "' + name.replace('"', '""') + '"'
        ).fetchall(), key=repr)) for name, schema in tables]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--host', type=Path, required=True)
    parser.add_argument('--evidence-dir', type=Path, required=True)
    parser.add_argument('controller', type=Path)
    parser.add_argument('arguments', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    host = args.host.resolve()
    controller = args.controller.resolve()
    evidence = args.evidence_dir.resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    checks = []
    original_exit = tempfile.TemporaryDirectory.__exit__

    def inspect_before_cleanup(temporary, exception_type, exception, traceback):
        if exception_type is None:
            root = Path(temporary.name)
            for path in sorted(root.rglob('*')):
                if not path.is_file():
                    continue
                with path.open('rb') as source:
                    if source.read(16) != b'SQLite format 3\x00':
                        continue
                with sqlite3.connect(path) as connection:
                    marker = connection.execute('PRAGMA user_version').fetchone()[0]
                if marker != 22:
                    continue
                name = f'{len(checks):02d}'
                before = snapshot(path)
                result = subprocess.run(
                    [str(host), str(path), 'plan_existing', str(root / 'unused.json')],
                    capture_output=True, timeout=30,
                )
                (evidence / (name + '.stdout')).write_bytes(result.stdout)
                (evidence / (name + '.stderr')).write_bytes(result.stderr)
                assert result.returncode == 0, (path.name, result.stderr)
                plan = json.loads(result.stdout)
                assert plan['generation'] == 0
                assert plan['policy'] == {'history_before_ms': 0, 'replay_through_sequence': 0}
                assert plan['collect'] == []
                assert snapshot(path) == before, 'preview mutated fixture rows or schema'
                checks.append({'database': path.relative_to(root).as_posix(),
                               'retained': len(plan['retained']),
                               'response_sha256': hashlib.sha256(result.stdout).hexdigest()})
            shutil.copytree(root, evidence / f'fixture-{len(checks):02d}')
        return original_exit(temporary, exception_type, exception, traceback)

    tempfile.TemporaryDirectory.__exit__ = inspect_before_cleanup
    sys.argv = [str(controller), *args.arguments]
    sys.path.insert(0, str(controller.parent))
    try:
        runpy.run_path(str(controller), run_name='__main__')
    finally:
        tempfile.TemporaryDirectory.__exit__ = original_exit
    assert checks, 'no actual store22 fixture was inspected'
    report = {'profile': 'populated-default-retention-inventory/1', 'status': 'passed',
              'controller': controller.name, 'checks': checks,
              'scope': 'actual known native registry encodings, complete default roots, no erasure'}
    (evidence / 'inventory.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report))


if __name__ == '__main__':
    main()
