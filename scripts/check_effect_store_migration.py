#!/usr/bin/env python3
"""Preserve a real historical unknown effect and its separate sink across migration."""
import argparse
from contextlib import closing
import hashlib
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile


def invoke(binary, *args, code=0):
    result = subprocess.run(
        [str(binary.resolve()), *map(str, args)],
        capture_output=True, text=True, timeout=30,
    )
    assert result.returncode == code, (binary, result.returncode, result.stdout, result.stderr)
    return json.loads(result.stdout) if code == 0 else result.stderr


def snapshot(path):
    with closing(sqlite3.connect(path)) as connection:
        tables = connection.execute(
            "SELECT name,sql FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
        ).fetchall()
        rows = {
            table: (sql, sorted(connection.execute(
                'SELECT * FROM "' + table.replace('"', '""') + '"'
            ).fetchall(), key=repr))
            for table, sql in tables
        }
        return connection.execute('PRAGMA user_version').fetchone()[0], rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('old-compiler', 'old-probe'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--probe', type=Path)
    parser.add_argument('--storage', type=Path)
    parser.add_argument('--new-marker', type=int, default=18)
    parser.add_argument('--prepare-only', action='store_true', help='verify historical population only; no migration claim')
    parser.add_argument('--report', type=Path)
    args = parser.parse_args()
    if not args.prepare_only and (args.probe is None or args.storage is None):
        parser.error('--probe and --storage are required for migration')
    with tempfile.TemporaryDirectory(prefix='weave-effect-migration-') as temporary:
        root = Path(temporary)
        db, sink, request = root / 'engine.db', root / 'sink.db', root / 'request.json'
        source = root / 'handler.weave'
        source.write_text('''function Identity revision "1" (graph input) { return input; }
handler ReferenceRequest revision "1" using Identity {
 input event graph "input" branch "main" metadata depth 0;
 on "graph.accepted", "graph.committed";
 output slot "request";
 replay pinned;
}
''', encoding='utf-8')
        template = invoke(args.old_compiler, 'handler-plan', source, '--handler', 'ReferenceRequest')
        assert template['protocol'] == '0.18.0'

        def call(binary, path, mode, code=0, **values):
            request.write_text(json.dumps({'mode': mode, **values}), encoding='utf-8')
            return invoke(binary, path, request, code=code)

        seeded = call(args.old_probe, db, 'seed', template=template)
        delivery = {'event': seeded['delivery']['event']['id'], 'lease': seeded['delivery']['lease']}
        original = call(args.old_probe, db, 'enqueue', **delivery)
        assert original['disposition']['kind'] == 'intent'
        intent = original['disposition']['intent_id']
        ticket = call(args.old_probe, db, 'begin', intent=intent)
        # The independent sink commits; the engine has not learned its outcome.
        evidence = call(args.old_probe, sink, 'sink', payload=ticket['payload'],
                        idempotency_key=ticket['idempotency_key'], idempotent=True)
        unknown = call(args.old_probe, db, 'status', intent=intent)
        assert unknown['state'] == 'unknown' and unknown['attempt_id'] == ticket['attempt_id']
        assert 'E_EFFECT_UNKNOWN' in call(args.old_probe, db, 'begin', code=1, intent=intent)
        before, sink_before = snapshot(db), snapshot(sink)
        assert before[0] == 17
        assert len(sink_before[1]['actions'][1]) == 1
        assert len(sink_before[1]['receipts'][1]) == 1
        for table in ('governed_effect_bindings', 'governed_effect_receipts',
                      'governed_effect_context', 'handler_preparations',
                      'handler_receipts', 'governance_delivery_receipts', 'effect_intents'):
            assert before[1][table][1], table
        checks = ['actual historical source compiler and accepted governed occurrence',
                  'unknown immutable effect with one independent committed sink action']
        if not args.prepare_only:
            invoke(args.storage, db, 'crash', code=82)
            assert snapshot(db) == before and snapshot(sink) == sink_before
            invoke(args.storage, db, 'after_commit', code=83)
            migrated = (args.new_marker, before[1])
            assert snapshot(db) == migrated and snapshot(sink) == sink_before
            replay = call(args.probe, db, 'enqueue', **delivery)
            assert replay == {**original, 'duplicate': True}
            assert call(args.probe, db, 'status', intent=intent) == unknown
            assert 'E_EFFECT_UNKNOWN' in call(args.probe, db, 'begin', code=1, intent=intent)
            assert snapshot(db) == migrated
            assert 'E_STORAGE_VERSION' in call(args.old_probe, db, 'status', code=1, intent=intent)
            assert snapshot(db) == migrated
            checks += ['precommit migration death preserves all historical rows',
                       'postcommit migration death changes only store marker',
                       'exact governed receipt and unknown attempt survive restart',
                       'no second dispatch ticket; old runtime refuses upgraded database']
            reconciliation = dict(intent=intent, attempt=ticket['attempt_id'],
                                  outcome='confirmed', evidence=evidence)
            call(args.probe, db, 'reconcile', **reconciliation)
            assert call(args.probe, db, 'status', intent=intent)['state'] == 'confirmed'
            terminal = snapshot(db)
            call(args.probe, db, 'reconcile', **reconciliation)
            assert snapshot(db) == terminal and snapshot(sink) == sink_before
            checks.append('old sink evidence reconciles once; exact replay leaves both stores unchanged')
        report = {'profile': 'historical-governed-effect-migration',
                  'status': 'historical-fixture-only' if args.prepare_only else 'passed',
                  'old_marker': 17, 'new_marker': None if args.prepare_only else args.new_marker,
                  'old_protocol': '0.18.0', 'checks': checks,
                  'historical_tables': len(before[1]),
                  'ticket_payload_sha256': hashlib.sha256(bytes(ticket['payload'])).hexdigest()}
        if args.report:
            args.report.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
        print(json.dumps(report))


if __name__ == '__main__':
    main()
