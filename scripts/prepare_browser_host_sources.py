#!/usr/bin/env python3
"""Compile independent actual SDK sources; preserve original UTF-8 and inventories."""
import argparse
import ctypes
import hashlib
import json
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument('--compiler-sdk', type=Path, required=True)
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()
a.output.mkdir(parents=True, exist_ok=False)
sdk = ctypes.CDLL(str(a.compiler_sdk.resolve()))


def function(name, count):
    f = getattr(sdk, 'weave_compiler_' + name)
    f.argtypes = [ctypes.c_uint32] * count
    f.restype = ctypes.c_int32
    return f


new, write, compile_, length, kind, read, drop = [function(n, c) for n, c in [
    ('input_new', 1), ('input_write', 3), ('compile', 1), ('output_len', 1),
    ('output_kind', 1), ('output_read', 2), ('drop', 1)]]
assert function('abi_version', 0)() == 1


def encode(value):
    return json.dumps(value, separators=(',', ':'), ensure_ascii=False)


def compile_source(source, modules):
    request = encode({'format': 'weave-compiler-request/1', 'entry_id': 'browser-host',
                      'source': source, 'modules': modules}).encode()
    h = new(len(request))
    assert h > 0
    try:
        for i in range(0, len(request), 2):
            assert write(h, int.from_bytes(request[i:i+2], 'little'), min(2, len(request)-i)) == 0
        output = compile_(h)
        assert output > 0
    finally:
        drop(h)
    try:
        count = length(output)
        assert 0 <= count <= 16 * 1024 * 1024 + 4096
        result = bytearray()
        for i in range(0, count, 2):
            word = read(output, i)
            assert word >= 0
            result += word.to_bytes(2, 'little')[:min(2, count-i)]
        assert kind(output) == 0, bytes(result)
        return bytes(result).decode()
    finally:
        assert drop(output) == 0


module = 'module "helpers" revision "1"; function Next revision "1" (integer input) returns integer { return integer_add(param input, 1); }'
pin = hashlib.sha256(module.encode()).hexdigest()
modules = [{'id': 'helpers', 'revision': '1', 'source': module}]
cases = []
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
    raw = compile_source(source, modules)
    # Python's integer decoder preserves the original exact scalar values.
    result = json.loads(raw)
    assert result['ok'], result
    artifacts = result['artifacts']
    template = artifacts['handler_templates']['Chosen']
    manifest = {'id': 'projection', 'version': '1', 'artifact_digest': template['definition_digest'],
                'config_revision': '1', 'principal': 'owner',
                'subscriptions': [{'graph_id': 'Input', 'branch_id': 'main'}],
                'output_graphs': ['Output'], 'effect_destinations': [], 'max_attempts': 5,
                'lease_ms': 60000, 'max_pending_events': 10, 'projection_replay': True}
    output = {'slot': 'result', 'graph_id': 'Output', 'branch_id': 'main'}
    upgraded_source = source.replace('Transform revision "1"', 'Transform revision "2"').replace('Chosen revision "1"', 'Chosen revision "2"')
    upgraded_raw = compile_source(upgraded_source, modules)
    upgraded = json.loads(upgraded_raw)['artifacts']['handler_templates']['Chosen']
    assert upgraded['definition_digest'] != template['definition_digest']
    for name, text in [('source', source), ('sdk', raw), ('upgraded-source', upgraded_source), ('upgraded-sdk', upgraded_raw)]:
        (a.output / f'case{index}-{name}.txt').write_text(text)
    cases.append({'source_sha256': hashlib.sha256(source.encode()).hexdigest(), 'sdk_raw': raw,
                  'program_raw': encode(artifacts['program']),
                  'config_raw': encode({'name': 'Chosen', 'manifest': manifest, 'output': output}),
                  'manifest': manifest, 'output': output, 'template_raw': encode(template),
                  'upgraded_sdk_raw': upgraded_raw, 'upgraded_template_raw': encode(upgraded),
                  'upgraded_digest': upgraded['definition_digest'],
                  'exact_integer': str(9007199254740994 + index),
                  'inventory': {k: sorted(artifacts[k]) for k in ['values', 'handler_templates', 'view_templates']}})
assert cases[0]['manifest']['artifact_digest'] != cases[1]['manifest']['artifact_digest']
report = {'compiler_sdk_sha256': hashlib.sha256(a.compiler_sdk.read_bytes()).hexdigest(),
          'cases': cases, 'compilations': 4}
(a.output / 'fixtures.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps({'compilations': 4, 'case_count': 2, 'compiler_sdk_sha256': report['compiler_sdk_sha256']}))
