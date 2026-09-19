#!/usr/bin/env python3
"""Exercise source-level edge evidence and typed schemas through both real CLIs."""
import argparse
import json
import sqlite3
import subprocess
import tempfile
from pathlib import Path


def run(command, cwd):
    result = subprocess.run(command, cwd=cwd, capture_output=True, text=True)
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--language', type=Path, default=Path(__file__).resolve().parents[2]/'weave-language')
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='weave-paper-example-') as temp:
        root = Path(temp)
        for filename, graphs in [('metadata_cycle.weave', ['Operations', 'Evidence', 'Catalog']),
                                 ('schema.weave', ['Network']),
                                 ('typed_join.weave', ['Devices', 'Issues'])]:
            plan = run(['cargo', 'run', '--locked', '--quiet', '--', 'plan', 'examples/'+filename],
                       args.language)
            source = root/(filename+'.json')
            source.write_text(json.dumps(plan))
            db = root/(filename+'.db')
            command = ['cargo', 'run', '--locked', '--quiet', '-p', 'weave-engine', '--',
                       'run', '--db', str(db), '--actor', 'reviewer']
            for graph in graphs:
                command += ['--write', graph]
            output = run(command+[str(source)], args.engine)
            result = output[-1]['result']
            assert result['coverage'] == 'complete', result
            assert len(result['graph']['edges']) == 1, result
            if filename == 'metadata_cycle.weave':
                edge = result['graph']['edges'][0]
                assert edge['predicate'] == 'reviewed_evidence', edge
                premises = {(p['graph_id'], p['assertion_id']) for p in edge['derived_from']}
                assert {('Operations', 'edge-evidence'), ('Evidence', 'source'),
                        ('Catalog', 'review')} <= premises, premises
                assert {p['graph_id'] for p in result['input_snapshots']} == set(graphs)
                assert edge['readers'] == ['reviewer'], edge
                with sqlite3.connect(db) as connection:
                    assert connection.execute('SELECT COUNT(*) FROM events').fetchone()[0] == 3
                    assert connection.execute('SELECT COUNT(*) FROM revisions').fetchone()[0] == 3
            elif filename == 'schema.weave':
                declared = next(c['data']['schema'] for c in plan['commands'] if c['op'] == 'commit')
                assert result['graph']['schema'] == declared
                assert {n['type_id'] for n in result['graph']['nodes']} == {'Device', 'Gateway'}
                assert result['graph']['edges'][0]['type_id'] == 'Connected'
            else:
                full = result['graph']
                schema = full['schema']
                assert len({n['type_id'] for n in full['nodes']}) == 2, full
                nodes = {n['id']: n for n in full['nodes']}
                for edge in full['edges']:
                    definition = schema['edges'][edge['type_id']]
                    assert definition['from_type'] == nodes[edge['from']]['type_id']
                    assert definition['to_type'] == nodes[edge['to']]['type_id']
                    assert edge['valid_time'] == {'start': 150, 'end': 200}
                # Same source schemas and operator, but a different result shape.
                empty_plan = {'version': plan['version'], 'commands': [{'op': 'join',
                    'left': {'graph_id': 'Devices', 'valid_at': 100},
                    'right': {'graph_id': 'Issues', 'valid_at': 100},
                    'output_predicate': 'exposed_to', 'match_on': 'entity_space_to_from'}]}
                source.write_text(json.dumps(empty_plan))
                empty = run(command+[str(source)], args.engine)[-1]['result']['graph']
                assert not empty['edges'], empty
                # Both typed results must be reusable without assigning contradictory
                # descriptors to the same schema identity/revision.
                source.write_text(json.dumps({'version': plan['version'], 'commands': [
                    {'op': 'commit', 'graph_id': 'FullView', 'data': full},
                    {'op': 'commit', 'graph_id': 'EmptyView', 'data': empty}]}))
                run(command+['--write', 'FullView', '--write', 'EmptyView', str(source)], args.engine)
        print(json.dumps({'status': 'passed', 'checks': [
            'source compiles atomic host/evidence cycle', 'named edge metadata becomes a graph value',
            'evidence graph joins independent catalog', 'attachment and leaf evidence survive join',
            'typed source validates at runtime', 'query retains exact schema and types',
            'cross-schema type names cannot collide', 'full and empty typed joins remain reusable']}, indent=2))


if __name__ == '__main__':
    main()
