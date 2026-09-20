#!/usr/bin/env python3
"""Independent raw-client acceptance for named metadata and logical snapshots."""
import argparse
import copy
import json
import sqlite3
import subprocess
import tempfile
from pathlib import Path


def node(name='n', **extra):
    return dict(id=name, entity_id=name, space_id='s', **extra)


def attachment(name, graph, revision, readers=None):
    return {'id': name, 'host': {'kind': 'graph'}, 'key': 'evidence',
            'value': {'kind': 'graph', 'reference': {'graph_id': graph, 'revision': revision}},
            'valid_time': {'start': 0, 'end': None}, 'readers': readers or []}


def query(graph, metadata=True):
    return {'kind': 'query', 'query': {'graph_id': graph, 'include_metadata': metadata}}


def extract(value):
    return {'kind': 'metadata', 'input': value, 'host': {'kind': 'graph'}, 'key': 'evidence'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine', type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    engine = args.engine.resolve()
    with tempfile.TemporaryDirectory(prefix='weave-metadata-review-') as tmp:
        root = Path(tmp)

        def execute(commands, database, actor='bob', writes=(), error=None):
            plan = root/'plan.json'
            plan.write_text(json.dumps({'version': '0.4.0', 'commands': commands}))
            command = ['cargo', 'run', '--locked', '--quiet', '-p', 'weave-engine', '--',
                       'run', '--db', str(database), '--actor', actor]
            for graph in writes:
                command += ['--write', graph]
            result = subprocess.run(command+[str(plan)], cwd=engine, text=True, capture_output=True)
            if error:
                assert result.returncode != 0, result.stdout
                diagnostic = json.loads(result.stderr)
                assert diagnostic['code'] == error, diagnostic
                return diagnostic
            assert result.returncode == 0, result.stderr
            return json.loads(result.stdout)

        def counts(database):
            with sqlite3.connect(database) as connection:
                return tuple(connection.execute('SELECT COUNT(*) FROM '+table).fetchone()[0]
                             for table in ('revisions', 'events'))

        # A new A->B->C->A cycle is constructed atomically, without recursive hashes.
        database = root/'cycle.db'
        commits = []
        for graph, target in [('A', 'B'), ('B', 'C'), ('C', 'A')]:
            commits.append({'graph_id': graph, 'data': {'nodes': [node(graph)],
                'attachments': [attachment('proof-'+graph, target, 'logical:cycle:'+target,
                                           ['bob'] if graph == 'A' else [])]}})
        batch = {'op': 'commit_batch', 'batch_id': 'cycle', 'commits': commits}
        result = execute([batch, {'op': 'evaluate', 'value': extract(extract(query('A')))}],
                         database, writes=['A', 'B', 'C'])[-1]['result']
        assert result['coverage'] == 'complete', result
        # Traversal adds path restrictions, so this is a derived wrapper rather
        # than an unchanged source record. Its exact source remains a proof gate.
        assert len(result['graph']['nodes']) == 1, result
        wrapped = result['graph']['nodes'][0]
        assert wrapped['entity_id'] == 'C' and wrapped['space_id'] == 's', result
        assert wrapped['id'].startswith('metadata-node:'), result
        assert result['node_origins'][wrapped['id']] == [], result
        assert {'graph_id': 'C', 'revision': result['snapshots']['C'],
                'node_id': 'C'} in wrapped['derived_nodes'], result
        premises = {(p['graph_id'], p['assertion_id']) for p in result['provenance']}
        assert {('A', 'proof-A'), ('B', 'proof-B')} <= premises, premises
        assert result['graph']['nodes'][0]['readers'] == ['bob'], result
        assert counts(database) == (3, 3)
        # A private attachment does not make independent public target access private.
        public = execute([{'op': 'evaluate', 'value': query('C', False)}], database,
                         actor='alice')[-1]['result']
        assert [n['id'] for n in public['graph']['nodes']] == ['C'], public
        hidden = execute([{'op': 'evaluate', 'value': extract(query('A'))}], database,
                         actor='alice')[-1]['result']
        assert not hidden['graph']['nodes'] and hidden['coverage'] == 'partial', hidden
        # Exact redelivery is an idempotent receipt, with no new logical event.
        execute([batch], database, writes=['A', 'B', 'C'])
        assert counts(database) == (3, 3)

        # A single batch must not assign two meanings to one schema revision.
        conflicting = []
        for graph, kind, value in [('X', 'integer', 1), ('Y', 'string', 'one')]:
            schema = {'id': 'shared', 'revision': 'r1', 'nodes': {'T': {
                'properties': {'value': {'value_type': kind, 'required': True}}}}}
            conflicting.append({'graph_id': graph, 'data': {'schema': schema,
                'nodes': [node(type_id='T', properties={'value': value})]}})
        bad = root/'schemas.db'
        execute([{'op': 'commit_batch', 'batch_id': 'conflict', 'commits': conflicting}],
                bad, writes=['X', 'Y'], error='E_SCHEMA_REVISION')
        assert counts(bad) == (0, 0)

        # An attachment must not alias an edge when used as an assertion premise.
        collision = {'nodes': [node('a'), node('b')], 'edges': [{
            'id': 'ambiguous', 'predicate': 'p', 'from': 'a', 'to': 'b',
            'valid_time': {'start': 0, 'end': None}}],
            'attachments': [attachment('ambiguous', 'absent', 'r1')]}
        bad = root/'collision.db'
        execute([{'op': 'commit', 'graph_id': 'G', 'data': collision}], bad,
                writes=['G'], error='E_ATTACHMENT')
        assert counts(bad) == (0, 0)

        # No-op commits retain the revision, while retargeting a deleted edge is rejected.
        database = root/'identity.db'
        data = copy.deepcopy(collision)
        data.pop('attachments')
        first = execute([{'op': 'commit', 'graph_id': 'G', 'data': data}], database,
                        writes=['G'])[0]['revision']
        unchanged = execute([{'op': 'commit', 'graph_id': 'G', 'expected_head': first,
                              'data': data}], database, writes=['G'])[0]['revision']
        assert unchanged == first and counts(database) == (1, 1)
        removed = execute([{'op': 'commit', 'graph_id': 'G', 'expected_head': first,
                            'data': {'nodes': data['nodes']}}], database,
                          writes=['G'])[0]['revision']
        data['edges'][0]['to'] = 'a'
        execute([{'op': 'commit', 'graph_id': 'G', 'expected_head': removed,
                  'data': data}], database, writes=['G'], error='E_EDGE_IDENTITY')
        assert counts(database) == (2, 2)

        # Evidence reached through an edge attachment inherits both temporal scopes.
        for label, host_start, host_end, expected in [('overlap', 12, 18, {'start': 15, 'end': 18}),
                                                       ('disjoint', 0, 5, None)]:
            proof = attachment('binding', 'Proof', f'logical:{label}:Proof')
            proof['host'] = {'kind': 'edge', 'id': 'host-edge'}
            proof['valid_time'] = {'start': 10, 'end': 20}
            host_graph = {'nodes': [node('a'), node('b')], 'edges': [{
                'id': 'host-edge', 'predicate': 'has_proof', 'from': 'a', 'to': 'b',
                'valid_time': {'start': host_start, 'end': host_end}}], 'attachments': [proof]}
            target = {'nodes': [node('c'), node('d')], 'edges': [{
                'id': 'observation', 'predicate': 'measured', 'from': 'c', 'to': 'd',
                'valid_time': {'start': 15, 'end': 30}}]}
            program = [{'op': 'commit_batch', 'batch_id': label, 'commits': [
                {'graph_id': 'Host', 'data': host_graph}, {'graph_id': 'Proof', 'data': target}]},
                {'op': 'evaluate', 'value': {'kind': 'metadata', 'input': query('Host'),
                  'host': {'kind': 'edge', 'id': 'host-edge'}, 'key': 'evidence'}}]
            temporal = execute(program, root/(label+'.db'), writes=['Host', 'Proof'])[-1]['result']
            if expected:
                assert len(temporal['graph']['edges']) == 1, temporal
                assert temporal['graph']['edges'][0]['valid_time'] == expected, temporal
            else:
                assert not temporal['graph']['edges'], temporal
        print(json.dumps({'status': 'passed', 'contract': '0.4.0', 'checks': [
            'atomic three-graph metadata cycle', 'nested attachment path provenance',
            'private path and independent public target', 'batch redelivery deduplication',
            'same-batch schema equivocation rollback', 'unambiguous assertion identities',
            'no-op revision/event suppression', 'edge identity survives deletion',
            'attachment-host-evidence temporal intersection', 'disjoint metadata path emits no edge']}, indent=2))


if __name__ == '__main__':
    main()
