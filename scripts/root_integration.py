#!/usr/bin/env python3
"""Run the actual language CLI against the actual native engine CLI."""
import json
import argparse
import subprocess
import tempfile
from pathlib import Path

LANGUAGE = Path(__file__).resolve().parents[2] / 'weave-language'
ENGINE = Path(__file__).resolve().parents[1]

def run(args, cwd):
    result = subprocess.run(args, cwd=cwd, text=True, capture_output=True)
    if result.returncode:
        raise RuntimeError(f'{args[0]} failed ({result.returncode}): {result.stderr}')
    return result.stdout

def main():
    global LANGUAGE, ENGINE
    parser = argparse.ArgumentParser()
    parser.add_argument('--language', type=Path, default=LANGUAGE)
    parser.add_argument('--engine', type=Path, default=ENGINE)
    args = parser.parse_args()
    LANGUAGE, ENGINE = args.language.resolve(), args.engine.resolve()
    plan = json.loads(run(['cargo', 'run', '--locked', '--quiet', '--', 'plan', 'examples/fleet.weave'], LANGUAGE))
    with tempfile.TemporaryDirectory(prefix='weave-integration-') as temp:
        base = Path(temp)
        path = base / 'program.json'
        path.write_text(json.dumps(plan))
        command = ['cargo', 'run', '--locked', '--quiet', '-p', 'weave-engine', '--', 'run', '--db', str(base/'graph.db'), '--actor', 'integration-user']
        output = json.loads(run(command + ['--write', 'Fleet', str(path)], ENGINE))
        revision = output[0]['revision']
        result = output[-1]['result']
        assert result['coverage'] == 'partial', result
        assert [e['id'] for e in result['graph']['edges']] == ['installed-1'], result
        assert any(d['code'] == 'E_DEPENDENCY_UNAVAILABLE' for d in result['diagnostics'])
        # Query the persisted immutable revision from a separate process.
        query = {'op':'query', 'query':{
            'graph_id':'Fleet', 'revision':revision, 'branch_id':'main',
            'predicate':'installed', 'valid_at':150,
            'include_metadata':True, 'max_depth':4
        }}
        path.write_text(json.dumps({'version':plan['version'], 'commands':[query]}))
        reopened = json.loads(run(command + [str(path)], ENGINE))[-1]['result']
        assert result == reopened, 'pinned query changed across engine process restart'
        # Temporal intervals are half-open: the upper boundary excludes the edge.
        query['query']['valid_at'] = 200
        path.write_text(json.dumps({'version':plan['version'], 'commands':[query]}))
        at_end = json.loads(run(command + [str(path)], ENGINE))[-1]['result']
        assert not at_end['graph']['edges'], 'upper interval boundary must be excluded'
        # Host write authority is not implied by compiling a valid graph program.
        path.write_text(json.dumps(plan))
        denied = subprocess.run(command + [str(path)], cwd=ENGINE, text=True, capture_output=True)
        assert denied.returncode != 0
        assert json.loads(denied.stderr)['code'] == 'E_FORBIDDEN', denied.stderr
        # Compare-and-swap rejects recreating the existing branch.
        conflict = subprocess.run(command + ['--write','Fleet',str(path)], cwd=ENGINE, text=True, capture_output=True)
        assert conflict.returncode != 0
        assert json.loads(conflict.stderr)['code'] == 'E_CONFLICT', conflict.stderr
        print(json.dumps({'status':'passed', 'contract':plan['version'], 'checks':[
            'language plan executed by engine', 'missing metadata reports partial coverage',
            'pinned result survives process restart', 'half-open valid time',
            'host authority enforced', 'existing head requires compare-and-swap'
        ]}, indent=2))

if __name__ == '__main__':
    main()
