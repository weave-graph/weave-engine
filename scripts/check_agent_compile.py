#!/usr/bin/env python3
"""B4 (#40) acceptance: the agent interface's `compile` tool with the real
weave-language compiler.

`weave-service serve --compiler WEAVE` runs `weave sdk --untrusted` (and
`--read-only` for a caller that can write no graph) as a sandboxed child on
the source an agent sends to the MCP tool `compile`, with the agent's own
visible manifest and embedding spaces. The scripted agent knows only its
token and an embedder; graph, space, group and property names are random.

1. a `nearest` query with the wrong vector size fails at compile time with
   `E_SPACE_MISMATCH` (the service supplied the space), is repaired from the
   diagnostic, and the compiled plan runs through `POST /v1/execute`;
2. a write without readers fails with `E_VISIBILITY_REQUIRED` (the untrusted
   floor, which the agent cannot lower); repaired with a group from
   `describe_schema`, compiling it writes nothing, and running it writes the
   finding with the agent as writer, hidden from a non-member;
3. a hidden graph fails at compile time with `E_UNAVAILABLE`;
4. a read-only agent's write fails with `E_EFFECT_DENIED`;
5. a `policy` argument is refused (`E_MCP_INPUT`).

Usage: check_agent_compile.py --compiler WEAVE --service WEAVE_SERVICE [--report FILE]
"""
import argparse, hashlib, json, math, random, re, socket, subprocess, tempfile, time
import urllib.error, urllib.request
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument('--compiler', type=Path, required=True)
p.add_argument('--service', type=Path, required=True)
p.add_argument('--seed', type=int)
p.add_argument('--report', type=Path)
a = p.parse_args()
seed = a.seed if a.seed is not None else random.SystemRandom().randrange(1 << 30)
rng = random.Random(seed)
checks = []


def check(name, condition, detail=''):
    if not condition:
        raise SystemExit(f'FAILED {name} (seed {seed}): {detail}')
    checks.append(name)


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


BASE = f'http://127.0.0.1:{free_port()}'


def http(method, path, token, body=None):
    data = None if body is None else json.dumps(body).encode()
    request = urllib.request.Request(BASE + path, data=data, method=method, headers={
        'Authorization': f'Bearer {token}', 'content-type': 'application/json',
        'accept': 'application/json, text/event-stream'})
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            return response.status, json.loads(response.read() or b'null')
    except urllib.error.HTTPError as error:
        return error.code, json.loads(error.read() or b'null')


def embed(text, dims):
    v = [0.0] * dims
    for word in re.split(r'[^0-9a-z]+', text.lower()):
        if len(word) > 2:
            h = int(hashlib.sha256(word.encode()).hexdigest(), 16)
            v[h % dims] += 1.0 if (h >> 40) & 1 == 0 else -1.0
    norm = math.sqrt(sum(x * x for x in v))
    if norm == 0:
        # Every feature cancelled: any fixed nonzero vector will do.
        v[0], norm = 1.0, 1.0
    return [x / norm for x in v]


def name(prefix):
    return f'{prefix}-{rng.randrange(16 ** 8):08x}'


GRAPH, HIDDEN, SPACE = name('kb'), name('hr'), name('space')
TEAM, OTHER, OWNER, OUTSIDER = name('team'), name('other'), name('user'), name('user')
AGENT, READER = name('agent'), name('agent')
DIMS = rng.choice([5, 6, 7, 9])

tmp = Path(tempfile.mkdtemp(prefix='weave-agent-compile-'))
data = tmp / 'data'
created = subprocess.run([str(a.service), 'tenant', 'create', '--data', str(data), '--tenant', 't1',
                          '--admin', 'root'], capture_output=True, check=True)
ROOT = json.loads(created.stdout)['token']['token']
server = subprocess.Popen([str(a.service), 'serve', '--data', str(data), '--listen', BASE[7:],
                           '--plaintext-loopback', '--compiler', str(a.compiler.resolve())],
                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
try:
    for _ in range(600):
        try:
            if http('GET', '/v1/health', ROOT)[0] == 200:
                break
        except OSError:
            time.sleep(0.05)
    else:
        raise SystemExit('weave-service did not start')

    # ------------------------------------------------------------- host setup
    for graph in [GRAPH, HIDDEN]:
        check('graph created', http('POST', '/v1/graphs', ROOT, {'graph_id': graph})[0] == 201)
    status, out = http('POST', '/v1/registry', ROOT, {'changes': [
        {'change': 'add_principal', 'id': OWNER, 'kind': 'user'},
        {'change': 'add_principal', 'id': OUTSIDER, 'kind': 'user'},
        {'change': 'add_principal', 'id': TEAM, 'kind': 'group'},
        {'change': 'add_principal', 'id': OTHER, 'kind': 'group'},
        {'change': 'add_member', 'group': TEAM, 'member': OWNER},
        {'change': 'add_member', 'group': OTHER, 'member': OUTSIDER},
        {'change': 'grant', 'graph_id': GRAPH, 'subject': OWNER, 'permission': 'write'},
        {'change': 'grant', 'graph_id': GRAPH, 'subject': OUTSIDER, 'permission': 'read'},
        {'change': 'grant', 'graph_id': HIDDEN, 'subject': OUTSIDER, 'permission': 'write'}]})
    check('registry populated', status == 200, out)
    owner = http('POST', '/v1/tokens', ROOT, {'principal': OWNER})[1]['token']
    outsider = http('POST', '/v1/tokens', ROOT, {'principal': OUTSIDER})[1]['token']
    status, out = http('POST', '/v1/registry', owner, {'changes': [
        {'change': 'add_agent', 'id': AGENT, 'owner': OWNER, 'delegation': {
            'scopes': [{'graph_id': GRAPH, 'permission': 'write'}], 'exercise': {'kind': 'owner_all'}}},
        {'change': 'add_agent', 'id': READER, 'owner': OWNER, 'delegation': {
            'scopes': [{'graph_id': GRAPH, 'permission': 'read'}], 'exercise': {'kind': 'owner_all'}}}]})
    check('agents registered', status == 200, out)
    agent = http('POST', '/v1/tokens', owner, {'principal': AGENT})[1]['token']
    reader = http('POST', '/v1/tokens', owner, {'principal': READER})[1]['token']
    notes = ['bearing vibration high replaced bearing', 'fan noise bearing worn', 'coolant leak resealed pump']
    space = {'kind': 'embedding_space', 'id': SPACE, 'revision': '1', 'encoder': 'word-hash',
             'encoder_version': '1', 'preprocessing': 'words/v1', 'dimensions': DIMS,
             'metric': 'cosine_distance'}
    ids = http('POST', '/v1/ids', ROOT, {'count': 2 * len(notes)})[1]['ids']
    changes = [{'change': 'assert', 'fact': space}]
    for i, text in enumerate(notes):
        node, emb = ids[2 * i], ids[2 * i + 1]
        changes.append({'change': 'assert', 'fact': {'kind': 'node', 'id': node, 'entity_id': node,
                        'space_id': 'notes', 'properties': {'text': text}}})
        changes.append({'change': 'assert', 'fact': {'kind': 'embedding', 'id': emb,
                        'space': {'id': SPACE, 'revision': '1'}, 'subject': {'kind': 'node', 'node_id': node},
                        'vector': embed(text, DIMS), 'valid_time': {'start': 0}}})
    status, out = http('POST', '/v1/execute', ROOT, {'version': '1.2.0', 'commands': [
        {'op': 'write', 'graph_id': GRAPH, 'changes': changes}]})
    check('host data written', status == 200, out)

    # ------------------------------------------------------------- the agent
    def tool(token, tool_name, arguments):
        status, out = http('POST', '/v1/mcp', token, {'jsonrpc': '2.0', 'id': 1, 'method': 'tools/call',
                                                      'params': {'name': tool_name, 'arguments': arguments}})
        check(f'{tool_name} answered', status == 200, out)
        content = out['result']['structuredContent']
        check(f'{tool_name} result is marked untrusted', content['trust'] == 'untrusted_data')
        return out['result']['isError'], content.get('result', content.get('error'))

    def compile_source(token, source):
        error, out = tool(token, 'compile', {'source': source})
        check('compile is not a tool error', not error, out)
        return out

    error, graphs = tool(agent, 'describe_schema', {})
    graph = graphs['graphs'][0]['graph_id']
    group = graphs['reader_groups'][0]
    error, described = tool(agent, 'describe_schema', {'graph_id': graph})
    s = described['embedding_spaces'][0]['space']
    declaration = (f'embedding_space Space id {json.dumps(s["id"])} revision {json.dumps(s["revision"])} '
                   f'encoder {json.dumps(s["encoder"])} version {json.dumps(s["encoder_version"])} '
                   f'preprocessing {json.dumps(s["preprocessing"])} dimensions {s["dimensions"]} '
                   f'metric {s["metric"]};\n')

    # 1. Wrong vector size: a compile-time diagnostic from the served spaces.
    def nearest(dims):
        vector = ', '.join(repr(x) for x in embed('bearing noise', dims))
        return (declaration + f'use G graph {json.dumps(graph)};\n'
                f'nearest Hits from G {{ space Space; vector [{vector}]; k 2; }}\n')
    out = compile_source(agent, nearest(4 if DIMS != 4 else 3))
    check('policy is untrusted and writable', out['policy'] == {'untrusted': True, 'read_only': False}, out)
    check('wrong size is E_SPACE_MISMATCH', not out['response']['ok']
          and out['response']['error']['code'] == 'E_SPACE_MISMATCH', out)
    dims = int(re.search(r'\((\d+) dimensions', out['response']['error']['message']).group(1))
    check('the diagnostic states the stored size', dims == DIMS, out['response']['error'])
    out = compile_source(agent, nearest(dims))
    check('repaired query compiles', out['response']['ok'], out)
    status, ran = http('POST', '/v1/execute', agent, out['response']['artifacts']['program'])
    check('compiled query runs through /v1/execute', status == 200
          and len(ran['results'][0]['result']['neighbors']) == 2, ran)

    # 2. A finding: the untrusted floor, then a group from describe_schema.
    record = http('POST', '/v1/ids', agent, {'count': 1})[1]['ids'][0]
    write = (f'write Finding graph {json.dumps(graph)} {{ assert node {json.dumps(record)} '
             f'entity {json.dumps(record)} space "findings" property "text" "bearing family is noisy"')
    out = compile_source(agent, write + '; }\n')
    check('implicitly public write is E_VISIBILITY_REQUIRED', not out['response']['ok']
          and out['response']['error']['code'] == 'E_VISIBILITY_REQUIRED', out)
    out = compile_source(agent, write + f' reader_groups {json.dumps(group)}; }}\n')
    check('repaired write compiles', out['response']['ok'], out)

    def finding_visible(token):
        error, found = tool(token, 'query', {'graph_id': graph})
        return any(n['id'] == record for n in found['results'][0]['result']['graph']['nodes'])
    check('compiling wrote nothing', not finding_visible(owner))
    status, ran = http('POST', '/v1/execute', agent, out['response']['artifacts']['program'])
    check('compiled write runs through /v1/execute', status == 200, ran)
    check('the owner sees the finding', finding_visible(owner))
    check('a non-member reader does not', not finding_visible(outsider))
    error, found = tool(owner, 'query', {'graph_id': graph})
    stamp = [e['provenance'] for e in found['results'][0]['result']['fact_provenance']
             if e['fact'].get('node_id') == record]
    check('the agent is the writer', stamp and stamp[0]['writer'] == AGENT
          and stamp[0]['on_behalf_of'] == [OWNER], stamp)

    # 3. A graph the agent cannot see.
    out = compile_source(agent, f'use G graph {json.dumps(HIDDEN)}; lens L from G {{ }}')
    check('hidden graph is E_UNAVAILABLE', not out['response']['ok']
          and out['response']['error']['code'] == 'E_UNAVAILABLE', out)

    # 4. A read-only agent cannot compile a write.
    out = compile_source(reader, write + f' reader_groups {json.dumps(group)}; }}\n')
    check('read-only caller gets the read-only policy', out['policy']['read_only'] is True, out)
    check('its write is E_EFFECT_DENIED', not out['response']['ok']
          and out['response']['error']['code'] == 'E_EFFECT_DENIED', out)

    # 5. The policy is the host's.
    error, out = tool(agent, 'compile', {'source': 'x', 'policy': {'untrusted': False}})
    check('a policy argument is refused', error and out['code'] == 'E_MCP_INPUT', out)
finally:
    server.terminate()
    server.wait(timeout=30)

report = {'profile': 'weave-engine-b4-agent-compile/1', 'seed': seed, 'dimensions': DIMS,
          'checks': len(checks), 'status': 'passed'}
if a.report:
    a.report.write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report))
