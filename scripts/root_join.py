#!/usr/bin/env python3
"""Independent checks for language-to-runtime temporal graph joins."""
import argparse
import copy
import json
import subprocess
import tempfile
from pathlib import Path

LANGUAGE = Path(__file__).resolve().parents[2] / 'weave-language'
ENGINE = Path(__file__).resolve().parents[1]

def main():
    global LANGUAGE, ENGINE
    parser = argparse.ArgumentParser()
    parser.add_argument('--language', type=Path, default=LANGUAGE)
    parser.add_argument('--engine', type=Path, default=ENGINE)
    args = parser.parse_args()
    LANGUAGE, ENGINE = args.language.resolve(), args.engine.resolve()
    compiled = subprocess.run(['cargo','run','--locked','--quiet','--','plan','examples/join.weave'], cwd=LANGUAGE, capture_output=True, text=True, check=True)
    original = json.loads(compiled.stdout)
    with tempfile.TemporaryDirectory(prefix='weave-join-') as tmp:
        root = Path(tmp)
        def execute(plan, case):
            path = root/(case+'.json')
            path.write_text(json.dumps(plan))
            done = subprocess.run(['cargo','run','--locked','--quiet','-p','weave-engine','--','run','--db',str(root/(case+'.db')),'--actor','bob','--write','Operations','--write','Advisories',str(path)], cwd=ENGINE,capture_output=True,text=True,check=True)
            return json.loads(done.stdout)[-1]['result']
        result = execute(original, 'baseline')
        assert len(result['graph']['edges']) == 1
        edge = result['graph']['edges'][0]
        assert edge['valid_time'] == {'start':150,'end':200}
        assert {p['graph_id'] for p in edge['derived_from']} == {'Operations','Advisories'}
        assert len(result['input_snapshots']) == 2
        assert edge['readers'] == ['bob']
        def changed():
            plan = copy.deepcopy(original)
            right = next(c['data'] for c in plan['commands'] if c['op']=='commit' and c['graph_id']=='Advisories')
            return plan, right
        plan,right = changed()
        right['edges'][0]['valid_time']['start'] = 200
        assert not execute(plan,'disjoint')['graph']['edges']
        plan,right = changed()
        right['edges'][0]['readers'] = ['alice']
        assert not execute(plan,'private')['graph']['edges']
        plan,right = changed()
        right['edges'][0]['polarity'] = 'negative'
        assert not execute(plan,'negative')['graph']['edges']
        plan,right = changed()
        right['nodes'][0]['entity_id'] = 'different-model'
        assert not execute(plan,'different-identity')['graph']['edges']
        plan,right = changed()
        right['nodes'][1]['id'] = 'device'
        right['edges'][0]['to'] = 'device'
        collision = execute(plan,'colliding-local-ids')
        assert len(collision['graph']['nodes']) == 2
        assert len({n['id'] for n in collision['graph']['nodes']}) == 2
        print(json.dumps({'status':'passed','contract':original['version'],'checks':[
            'actual compiled join executes', 'valid-time intersection', 'both premise revisions retained',
            'derived result restricted to principal', 'disjoint intervals excluded',
            'private and negative premises excluded', 'identity keys required', 'local IDs cannot collide'
        ]},indent=2))

if __name__ == '__main__':
    main()
