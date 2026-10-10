#!/usr/bin/env python3
"""Independent exact-integer/content inspection of actual browser generations."""
import argparse
import hashlib
import json
import sqlite3
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument('--fixtures', type=Path, required=True)
p.add_argument('--evidence', type=Path, required=True)
a = p.parse_args()
fixtures = json.loads((a.fixtures / 'fixtures.json').read_text())
cases = []


def integers(value):
    if isinstance(value, dict):
        return [n for v in value.values() for n in integers(v)]
    if isinstance(value, list):
        return [n for v in value for n in integers(v)]
    return [value] if type(value) is int else []


def content(graph, revision):
    def visit(value):
        if isinstance(value, list):
            return [visit(v) for v in value]
        if not isinstance(value, dict):
            return value
        result = {}
        for key, item in value.items():
            if key in ('derived_nodes', 'derived_edges', 'derived_attachments'):
                for origin in item:
                    if origin['graph_id'] == 'Output':
                        assert origin['revision'] == revision
                item = [origin for origin in item if origin['graph_id'] != 'Output']
                if not item:
                    continue
            result[key] = visit(item)
        return result
    return visit(graph)


for index, f in enumerate(fixtures['cases']):
    stem = f'source-{index}'
    original = json.loads((a.evidence / f'{stem}-original-output.json').read_text())[0]['result']
    rebuilt = json.loads((a.evidence / f'{stem}-rebuilt-output.json').read_text())[0]['result']
    old = content(original['graph'], original['snapshots']['Output'])
    new = content(rebuilt['graph'], rebuilt['snapshots']['Output'])
    assert len(old['nodes']) == len(new['nodes']) == 1
    assert len(old['attachments']) == len(new['attachments']) == 1
    assert old['edges'] == new['edges'] == []
    for collection in ('nodes', 'attachments'):
        old_id, new_id = old[collection][0].pop('id'), new[collection][0].pop('id')
        assert old_id != new_id and old_id.startswith('handler:') and new_id.startswith('handler:')
    assert old == new
    if index == 0:
        assert int(f['exact_integer']) in integers(new)
    else:
        assert new['schema']['id'] == 'weave:explanation'
        assert new['nodes'][0]['type_id'] == 'Snapshot'
        assert new['nodes'][0]['properties']['graph_id'] == 'Input'
    generation = json.loads((a.evidence / f'{stem}-generation.json').read_text())
    db_path = a.evidence / f'{stem}.sqlite'
    assert hashlib.sha256(db_path.read_bytes()).hexdigest() == generation['sha256']
    assert hashlib.sha256(generation['journal_raw'].encode()).hexdigest() == generation['journal_sha256']
    entries = json.loads(generation['journal_raw'])['artifacts']
    assert entries == [{'kind': 'handler', 'config_raw': f['config_raw'], 'sdk_raw': f['sdk_raw']},
                       {'kind': 'sdk', 'sdk_raw': f['upgraded_sdk_raw']}]
    for entry in entries:
        artifacts = json.loads(entry['sdk_raw'])['artifacts']
        for kind, names in f['inventory'].items():
            assert sorted(artifacts[kind]) == names
        assert int(f['exact_integer']) in integers(artifacts['values'])
    connection = sqlite3.connect(f'file:{db_path}?mode=ro', uri=True)
    try:
        assert connection.execute('PRAGMA integrity_check').fetchone() == ('ok',)
        assert connection.execute('PRAGMA user_version').fetchone() == (29,)
        assert connection.execute("SELECT revision FROM heads WHERE graph_id='Output' AND branch_id='main'").fetchone() == (rebuilt['snapshots']['Output'],)
        assert connection.execute('SELECT count(*) FROM compiled_migrations').fetchone() == (2,)
        assert connection.execute('SELECT count(*) FROM compiled_rebuild_receipts').fetchone() == (1,)
        assert connection.execute('SELECT count(*) FROM handler_receipts').fetchone() == (1,)
        assert connection.execute('SELECT count(*) FROM dispatch_pending').fetchone() == (0,)
        assert connection.execute('SELECT count(*) FROM effect_intents').fetchone() == (0,)
    finally:
        connection.close()
    cases.append({'case': index, 'stable_content': True, 'new_owned_occurrences': True,
                  'complete_original_and_upgraded_inventories': True,
                  'exact_integer': f['exact_integer'], 'sqlite_sha256': generation['sha256']})
report = {'profile': 'independent-browser-source-image/2', 'cases': cases}
(a.evidence / 'independent-image-report.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report))
