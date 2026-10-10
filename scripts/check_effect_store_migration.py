#!/usr/bin/env python3
"""Preserve a real historical unknown effect and its separate sink across migration."""
from retention_migration import CAUSAL_DISPATCH_TABLES,ACTOR_DISPOSITION_TABLES,ACTOR_LIFECYCLE_TABLES,RETENTION_TABLES,LIFECYCLE_TABLES,COMPILED_LIFECYCLE_TABLES,COMPILED_REBUILD_TABLES,RECORDED_ACTOR_TABLES,retention_tables,assert_retention_baseline
from version_profile import store_marker
import argparse
from contextlib import closing
import hashlib
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile
from migration_history import assert_recorded_baselines, recorded_history


def invoke(binary, *args, code=0):
    result = subprocess.run(
        [str(binary.resolve()), *map(str, args)],
        capture_output=True, text=True, timeout=30,
    )
    assert result.returncode == code, (binary, result.returncode, result.stdout, result.stderr)
    return json.loads(result.stdout) if code == 0 else result.stderr


def snapshot(path, ignore=()):
    with closing(sqlite3.connect(path)) as connection:
        tables = connection.execute(
            "SELECT name,sql FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
        ).fetchall()
        rows = {
            table: (sql, sorted(connection.execute(
                'SELECT * FROM "' + table.replace('"', '""') + '"'
            ).fetchall(), key=repr))
            for table, sql in tables if table not in ignore
        }
        return connection.execute('PRAGMA user_version').fetchone()[0], rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('old-compiler', 'old-probe'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--probe', type=Path)
    parser.add_argument('--storage', type=Path)
    parser.add_argument('--new-marker', type=int, default=store_marker())
    parser.add_argument('--old-marker', type=int, default=17, choices=range(17,store_marker()))
    parser.add_argument('--old-protocol', default='0.18.0', choices=['0.18.0','0.19.0','0.20.0','0.21.0'])
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
        assert template['protocol'] == args.old_protocol

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
        old_history = recorded_history(db)
        assert before[0] == args.old_marker
        assert len(sink_before[1]['actions'][1]) == 1
        assert len(sink_before[1]['receipts'][1]) == 1
        for table in ('governed_effect_bindings', 'governed_effect_receipts',
                      'governed_effect_context', 'handler_preparations',
                      'handler_receipts', 'governance_delivery_receipts', 'effect_intents'):
            assert before[1][table][1], table
        checks = ['actual historical source compiler and accepted governed occurrence',
                  'unknown immutable effect with one independent committed sink action']
        if not args.prepare_only:
            invoke(args.storage, db, 'crash', 10, code=82)
            assert snapshot(db) == before and snapshot(sink) == sink_before
            assert recorded_history(db) == old_history
            invoke(args.storage, db, 'after_commit', 10, code=83)
            migrated = (args.new_marker, before[1])
            added = ('head_observations',) if args.old_marker < 19 <= args.new_marker else ()
            if args.old_marker<22<=args.new_marker: added+=tuple(RETENTION_TABLES)
            if args.old_marker<23<=args.new_marker: added+=tuple(LIFECYCLE_TABLES)
            if args.old_marker<24<=args.new_marker: added+=tuple(COMPILED_LIFECYCLE_TABLES)
            if args.old_marker<25<=args.new_marker: added+=tuple(COMPILED_REBUILD_TABLES)
            if args.old_marker<26<=args.new_marker: added+=tuple(RECORDED_ACTOR_TABLES)
            if args.old_marker<27<=args.new_marker: added+=tuple(ACTOR_LIFECYCLE_TABLES)
            if args.old_marker<28<=args.new_marker: added+=tuple(ACTOR_DISPOSITION_TABLES)
            if args.old_marker<29<=args.new_marker: added+=tuple(CAUSAL_DISPATCH_TABLES)
            assert_retention_baseline(db,args.new_marker)
            assert snapshot(db, added) == migrated and snapshot(sink) == sink_before
            history = assert_recorded_baselines(db, 10) if 'head_observations' in added else old_history
            replay = call(args.probe, db, 'enqueue', **delivery)
            assert replay == {**original, 'duplicate': True}
            assert call(args.probe, db, 'status', intent=intent) == unknown
            assert 'E_EFFECT_UNKNOWN' in call(args.probe, db, 'begin', code=1, intent=intent)
            assert snapshot(db, added) == migrated and recorded_history(db) == history
            assert 'E_STORAGE_VERSION' in call(args.old_probe, db, 'status', code=1, intent=intent)
            assert snapshot(db, added) == migrated and recorded_history(db) == history
            checks += ['precommit migration death preserves all historical rows',
                       'postcommit migration preserves history and adds baselines only before store19',
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
                  'old_marker': args.old_marker, 'new_marker': None if args.prepare_only else args.new_marker,
                  'old_protocol': args.old_protocol, 'checks': checks,
                  'historical_tables': len(before[1]),
                  'ticket_payload_sha256': hashlib.sha256(bytes(ticket['payload'])).hexdigest()}
        if args.report:
            args.report.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
        print(json.dumps(report))


if __name__ == '__main__':
    main()
