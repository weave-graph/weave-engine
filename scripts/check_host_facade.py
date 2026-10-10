#!/usr/bin/env python3
"""Actual arbitrary-source SDK -> strict artifact helper -> native trusted facade.
No builds, network, keys, or fixed runtime recipes. Python JSON retains exact integers.
"""
import argparse
import ctypes
import hashlib
import json
from pathlib import Path
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument('--library', type=Path, required=True)
parser.add_argument('--compiler-sdk', type=Path, required=True)
parser.add_argument('--report', type=Path)
parser.add_argument('--sdk-fixture-dir', type=Path)
parser.add_argument('--lifecycle', action='store_true', help='execute request2 source-compiled reconstruction and version transfer')
a = parser.parse_args()
runtime = ctypes.CDLL(str(a.library.resolve()))
compiler = ctypes.CDLL(str(a.compiler_sdk.resolve()))

def sdk_fn(name, count):
    f = getattr(compiler, 'weave_compiler_' + name)
    f.argtypes = [ctypes.c_uint32] * count
    f.restype = ctypes.c_int32
    return f

new, write, compile_, length, kind, read, drop = [sdk_fn(n, c) for n, c in [
    ('input_new', 1), ('input_write', 3), ('compile', 1), ('output_len', 1),
    ('output_kind', 1), ('output_read', 2), ('drop', 1)]]
assert sdk_fn('abi_version', 0)() == 1

def encode(value):
    return json.dumps(value, separators=(',', ':'), ensure_ascii=False).encode()

def compile_source(source, modules):
    request = encode({'format': 'weave-compiler-request/1', 'entry_id': 'facade-test', 'source': source, 'modules': modules})
    h = new(len(request))
    assert h > 0
    try:
        for i in range(0, len(request), 2):
            assert write(h, int.from_bytes(request[i:i+2], 'little'), min(2, len(request)-i)) == 0
        output = compile_(h)
        assert output > 0
    finally:
        drop(h)  # Consumed by accepted compile; harmless stale-handle return.
    try:
        count = length(output)
        assert 0 <= count <= 16 * 1024 * 1024 + 4096
        result = bytearray()
        for i in range(0, count, 2):
            word = read(output, i)
            assert word >= 0
            result += word.to_bytes(2, 'little')[:min(2, count-i)]
        assert kind(output) == 0, bytes(result)
        return bytes(result)
    finally:
        assert drop(output) == 0

free = runtime.weave_native_free
free.argtypes = [ctypes.c_void_p]
free.restype = None

def call(name, *parts):
    f = getattr(runtime, 'weave_host_' + name)
    f.argtypes = [item for _ in parts for item in (ctypes.c_void_p, ctypes.c_size_t)]
    f.restype = ctypes.c_void_p
    buffers = [ctypes.create_string_buffer(part) for part in parts]
    args = [v for b, part in zip(buffers, parts) for v in (ctypes.cast(b, ctypes.c_void_p), len(part))]
    pointer = f(*args)
    assert pointer
    try:
        raw = ctypes.string_at(pointer)
        return json.loads(raw), raw
    finally:
        free(pointer)

def operation(token, value, format='weave-host-request/1'):
    return call('call', token, encode({'format': format, 'operation': value}))[0]

def open_store(path, principal='owner', graphs=('Input', 'Output')):
    result, _ = call('open', str(path).encode(), encode({'principal': principal, 'writable_graphs': list(graphs)}))
    assert result['ok'], result
    assert isinstance(result['value']['handle'], str)
    return result['value']['handle'].encode()

module = 'module "helpers" revision "1"; function Next revision "1" (integer input) returns integer { return integer_add(param input, 1); }'
pin = hashlib.sha256(module.encode()).hexdigest()
reports = []
external_fixtures = []
if a.sdk_fixture_dir:
    for fixture in sorted(a.sdk_fixture_dir.glob('*.response.json')):
        raw = fixture.read_bytes()
        result, echoed = call('artifact_select', raw, encode({'kind': 'original'}))
        assert result['ok'] and raw in echoed, result
        inventory = result['value']['inventory']
        assert inventory['handler_templates'] and inventory['values']
        for name in inventory['handler_templates']:
            selected, _ = call('artifact_select', raw, encode({'kind': 'handler', 'name': name}))
            assert selected['ok']
        external_fixtures.append({'file': fixture.name, 'sha256': hashlib.sha256(raw).hexdigest(), 'inventory': inventory})
    assert len(external_fixtures) >= 2
with tempfile.TemporaryDirectory(prefix='weave-host-facade-') as temporary:
    for index, body in enumerate(['return input;', 'explain Explanation from input; return Explanation;']):
        source = f'''import h module "helpers" revision "1" sha256 "{pin}";
apply Exact from h::Next {{ integer input {9007199254740993 + index}; }}
schema Model revision "1" {{ node Item space "s" {{ property "count" integer required; }} }}
graph Input schema Model {{ node "n" type Item entity "entity" space "s" property "count" value Exact; }}
function Transform revision "1" (graph input) {{ {body} }}
handler Chosen revision "1" using Transform {{ input event graph "Input" branch "main" metadata depth 0;
 on "graph.accepted", "graph.committed"; output slot "result"; replay pinned; }}
live_handle Head graph "Input" branch "main";
view_template Visible revision "1" from Head clock fixed {{}}
'''
        sdk = compile_source(source, [{'id': 'helpers', 'revision': '1', 'source': module}])
        selected, raw = call('artifact_select', sdk, encode({'kind': 'original'}))
        assert selected['ok'] and sdk in raw
        inventory = selected['value']['inventory']
        assert inventory['values'] == ['Exact'] and inventory['handler_templates'] == ['Chosen']
        assert inventory['view_templates'] == ['Visible']
        artifacts = selected['value']['selected']['artifacts']
        program, _ = call('artifact_select', sdk, encode({'kind': 'program'}))
        assert program['ok'] and program['value']['selected'] == artifacts['program']
        path = Path(temporary) / f'peer-{index}.db'
        token = open_store(path)
        handles = [token]
        try:
            result = operation(token, {'kind': 'execute', 'program': program['value']['selected']})
            assert result['ok'] and result['requires_fence'], result
            template = artifacts['handler_templates']['Chosen']
            manifest = {'id': 'projection', 'version': '1', 'artifact_digest': template['definition_digest'], 'config_revision': '1', 'principal': 'owner',
                        'subscriptions': [{'graph_id': 'Input', 'branch_id': 'main'}], 'output_graphs': ['Output'], 'effect_destinations': [],
                        'max_attempts': 5, 'lease_ms': 60000, 'max_pending_events': 10, 'projection_replay': True}
            config = {'name': 'Chosen', 'manifest': manifest, 'output': {'slot': 'result', 'graph_id': 'Output', 'branch_id': 'main'}}
            assert call('install_handler', token, sdk, encode(config))[0]['ok']
            assert call('set_adapter_state', token, encode({'adapter': 'projection', 'state': 'running'}))[0]['ok']
            event = operation(token, {'kind': 'poll', 'adapter': 'projection'})['value']
            prep = operation(token, {'kind': 'prepare', 'adapter': 'projection', 'event': event['id'], 'lease': event['lease']})
            assert prep['ok'], prep
            complete = {'kind': 'complete', 'adapter': 'projection', 'event': event['id'], 'lease': event['lease'], 'preparation': prep['value']['preparation_id']}
            assert operation(token, complete)['value']['duplicate'] is False
            other = open_store(path, 'other'); handles.append(other)
            assert operation(other, complete)['error']['code'] == 'E_HOST_AUTH'
            assert call('close', token)[0]['ok']; handles.remove(token)
            assert operation(token, {'kind': 'poll', 'adapter': 'projection'})['error']['code'] == 'E_HOST_HANDLE'
            previous = token
            token = open_store(path)
            assert token != previous; handles.append(token)
            assert operation(token, complete)['value']['duplicate'] is True
            query = {'version': '0.18.0', 'commands': [{'op': 'query', 'query': {'graph_id': 'Input'}}]}
            result = operation(token, {'kind': 'execute', 'program': query})
            assert result['ok'], result
            # Check exact number anywhere in the actual serialized graph result without a float conversion.
            def integers(value):
                if isinstance(value, dict):
                    return [x for v in value.values() for x in integers(v)]
                if isinstance(value, list):
                    return [x for v in value for x in integers(v)]
                return [value] if isinstance(value, int) else []
            assert 9007199254740994 + index in integers(result)
            if a.lifecycle:
                output_query = {'kind':'execute','program':{'version':'0.21.0','commands':[{'op':'query','query':{'graph_id':'Output'}}]}}
                original_output = operation(token, output_query)
                assert original_output['ok'], original_output
                original_graph = original_output['value'][0]['result']['graph']
                def owner(value):
                    result = operation(token, value, 'weave-host-request/2')
                    assert result['ok'] and result['requires_fence'] and not result['poisoned'], result
                    return result['value']
                capability = owner({'kind':'capabilities'})
                assert capability['store_marker'] == 29 and len(capability['operations']) == 31
                owner({'kind':'lifecycle','adapter':'projection','state':'paused'})
                inputs = owner({'kind':'compiled_rebuild_inputs','adapter':'projection'})
                reconstruction = {'kind':'compiled_rebuild','request':{'inputs':inputs,'nonce':'source-rebuild'}}
                rebuilt = owner(reconstruction)
                assert owner(reconstruction)['duplicate']
                # Compile a genuinely different revision; the kernel receives the actual
                # sealed compiler template, not a hand-reconstructed fingerprint.
                upgraded_source = source.replace('function Transform revision "1"', 'function Transform revision "2"').replace('handler Chosen revision "1"', 'handler Chosen revision "2"')
                upgraded_sdk = compile_source(upgraded_source, [{'id':'helpers','revision':'1','source':module}])
                selected, _ = call('artifact_select', upgraded_sdk, encode({'kind':'original'}))
                assert selected['ok'] and selected['value']['inventory']['values'] == ['Exact']
                upgraded_template = selected['value']['selected']['artifacts']['handler_templates']['Chosen']
                assert upgraded_template['definition_digest'] != template['definition_digest']
                destination = {**manifest,'id':'projection-v2','version':'2','config_revision':'2','artifact_digest':upgraded_template['definition_digest']}
                inputs = owner({'kind':'compiled_migration_inputs','adapter':'projection'})
                upgrade = {'kind':'compiled_migrate','request':{'inputs':inputs,'destination':destination,'template':upgraded_template,'output':config['output'],'nonce':'source-upgrade','disposition':{'kind':'upgrade'}}}
                owner(upgrade); assert owner(upgrade)['duplicate']
                owner({'kind':'lifecycle','adapter':'projection-v2','state':'paused'})
                inputs = owner({'kind':'compiled_migration_inputs','adapter':'projection-v2'})
                restored = {**manifest,'id':'projection-restored'}
                rollback = {'kind':'compiled_migrate','request':{'inputs':inputs,'destination':restored,'template':template,'output':config['output'],'nonce':'source-rollback','disposition':{'kind':'rollback','restore_from':'projection'}}}
                owner(rollback)
                assert call('close', token)[0]['ok']; handles.remove(token)
                token = open_store(path); handles.append(token)
                assert owner(rollback)['duplicate']
                assert owner(reconstruction)['duplicate']
                owner({'kind':'lifecycle','adapter':'projection-restored','state':'running'})
                assert owner({'kind':'poll','adapter':'projection-restored'}) is None
                assert owner({'kind':'lag','adapter':'projection-restored'})['visible_backlog_lower_bound'] == 0
                query = operation(token, output_query)
                # Reconstruction creates a new owned occurrence namespace. Entity
                # identity, content and external premises stay stable in this profile;
                # manifestation/attribution IDs and the output occurrence are new.
                def content(graph, revision):
                    def visit(value):
                        if isinstance(value, list):
                            return [visit(item) for item in value]
                        if not isinstance(value, dict):
                            return value
                        result = {}
                        for key, item in value.items():
                            if key in ('derived_nodes','derived_edges','derived_attachments'):
                                for origin in item:
                                    if origin['graph_id'] == 'Output':
                                        assert origin['revision'] == revision
                                item = [origin for origin in item if origin['graph_id'] != 'Output']
                                if not item:
                                    continue
                            result[key] = visit(item)
                        return result
                    return visit(graph)
                assert query['ok'], query
                original_revision = original_output['value'][0]['result']['snapshots']['Output']
                current_revision = query['value'][0]['result']['snapshots']['Output']
                original_content = content(original_graph, original_revision)
                current_content = content(query['value'][0]['result']['graph'], current_revision)
                assert len(original_content['nodes']) == len(current_content['nodes']) == 1
                assert len(original_content['attachments']) == len(current_content['attachments']) == 1
                assert original_content['edges'] == current_content['edges'] == []
                for collection in ('nodes','attachments'):
                    old_id = original_content[collection][0].pop('id')
                    new_id = current_content[collection][0].pop('id')
                    assert old_id != new_id and old_id.startswith('handler:') and new_id.startswith('handler:')
                assert current_content == original_content, (original_content,current_content)
                assert query['value'][0]['result']['snapshots']['Output'] == rebuilt['output']['revision']
                if index == 0:
                    assert 9007199254740994 in integers(query)
                else:
                    graph = query['value'][0]['result']['graph']
                    assert graph['schema']['id'] == 'weave:explanation'
                    assert len(graph['nodes']) == 1 and graph['nodes'][0]['type_id'] == 'Snapshot'
                    assert graph['nodes'][0]['properties']['graph_id'] == 'Input'
            reports.append({'source_sha256': hashlib.sha256(source.encode()).hexdigest(), 'artifact_fingerprint': inventory['artifact_fingerprint'],
                            'handler_digest': template['definition_digest'], 'sdk_bytes': len(sdk), 'exact_integer': str(9007199254740994 + index),
                            'reopen_duplicate': True, 'foreign_completion_denied': True, 'complete_inventory_retained': True,
                            'request2_source_rebuild_upgrade_rollback': a.lifecycle})
        finally:
            for handle in handles:
                call('close', handle)
assert reports[0]['handler_digest'] != reports[1]['handler_digest']
report = {'profile': 'native-trusted-host-source-lifecycle/2' if a.lifecycle else 'native-trusted-host-facade-A', 'source_compiler_sdk_sha256': hashlib.sha256(a.compiler_sdk.read_bytes()).hexdigest(),
          'native_library_sha256': hashlib.sha256(a.library.read_bytes()).hexdigest(), 'cases': reports, 'independent_sdk_fixtures': external_fixtures,
          'browser_durability_tested': False, 'mobile_execution_tested': False}
if a.report:
    a.report.write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
