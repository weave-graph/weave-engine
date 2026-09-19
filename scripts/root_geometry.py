#!/usr/bin/env python3
"""Independent compiler/runtime derived-node privacy and atomic rejection checks."""
import argparse
import copy
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--language', type=Path, default=ROOT.parent / 'weave-language')
parser.add_argument('--engine', type=Path, default=ROOT)
args = parser.parse_args()
plan = json.loads(subprocess.check_output(['cargo', 'run', '--locked', '--quiet', '--', 'plan', 'examples/geometry.weave'], cwd=args.language))
subprocess.run(['cargo', 'build', '--locked', '-p', 'weave-engine'], cwd=args.engine, check=True)

with tempfile.TemporaryDirectory(prefix='weave-root-geometry-') as directory:
    tmp = Path(directory)
    def run(program, actor, name, db='evidence.db', writes=()):
        path = tmp / (name + '.json')
        path.write_text(json.dumps(program))
        command = [str(args.engine.resolve() / 'target/debug/weave-engine'), 'run', '--db', str(tmp / db), '--actor', actor]
        for graph in writes: command += ['--write', graph]
        result = subprocess.run(command + [str(path)], text=True, capture_output=True)
        return result
    def success(program, actor, name, **kwargs):
        result = run(program, actor, name, **kwargs)
        assert result.returncode == 0, result.stderr
        return json.loads(result.stdout)
    for command in plan['commands']:
        if command['op'] == 'commit':
            for key in ['nodes', 'edges', 'structural_edges', 'assertions', 'attachments']:
                for obj in command['data'].get(key, []): obj['readers'] = ['alice']
    outputs = success(plan, 'alice', 'seed', writes=['Geometry'])
    values = {command['name']: result['result'] for command, result in zip(plan['commands'], outputs) if command['op'] == 'bind'}
    assert values['Range']['graph']['nodes'][0]['properties']['value'] == 5.0
    checks = []
    for name in ['Range', 'Proof']:
        graph = copy.deepcopy(values[name]['graph'])
        graph['edges'] = []
        for node in graph['nodes']:
            node['readers'] = []
            assert node.get('derived_from'), node
        stored = 'Copied' + name
        commit = {'version': plan['version'], 'commands': [{'op': 'commit', 'graph_id': stored, 'data': graph}]}
        success(commit, 'alice', 'save-' + name, writes=[stored])
        query = {'version': plan['version'], 'commands': [{'op': 'query', 'query': {'graph_id': stored}}]}
        visible = success(query, 'alice', 'owner-' + name)[0]['result']
        denied = success(query, 'bob', 'other-' + name)[0]['result']
        assert len(visible['graph']['nodes']) == len(graph['nodes'])
        assert denied['graph']['nodes'] == [] and denied['coverage'] == 'partial', denied
        checks.append(name + ' node-only saved value retains private proof restriction across processes')
    invalid = copy.deepcopy(plan)
    invalid['commands'].append({'op': 'evaluate', 'value': {'kind': 'geometry', 'valid_at': 7, 'operation': {'kind': 'distance', 'left': {'input': {'kind': 'reference', 'name': 'Navigation'}, 'assertion_id': 'value'}, 'right': {'input': {'kind': 'reference', 'name': 'Navigation'}, 'assertion_id': 'value'}}}})
    failed = run(invalid, 'alice', 'bad-navigation', db='rollback.db', writes=['Geometry'])
    assert failed.returncode != 0 and json.loads(failed.stderr)['code'] == 'E_GEOMETRY_KIND', failed.stderr
    with sqlite3.connect(tmp / 'rollback.db') as db:
        assert db.execute('SELECT COUNT(*) FROM events').fetchone()[0] == 0
        assert db.execute('SELECT COUNT(*) FROM revisions').fetchone()[0] == 0
    checks.append('display projection rejected as metric input with all source commits rolled back')
print(json.dumps({'suite': 'geometry node privacy', 'result': 'passed', 'checks': checks}))
