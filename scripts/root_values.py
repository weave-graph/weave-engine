#!/usr/bin/env python3
"""Exercise reusable graph values through separate compiler/runtime processes."""
import argparse
import copy
import json
import sqlite3
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
    compiled = subprocess.run(['cargo','run','--locked','--quiet','--','plan','examples/composed.weave'],cwd=LANGUAGE,capture_output=True,text=True,check=True)
    plan = json.loads(compiled.stdout)
    with tempfile.TemporaryDirectory(prefix='weave-values-') as tmp:
        root = Path(tmp)
        def execute(program, case):
            path = root/(case+'.json')
            path.write_text(json.dumps(program))
            db = root/(case+'.db')
            command = ['cargo','run','--locked','--quiet','-p','weave-engine','--','run','--db',str(db),'--actor','bob']
            for graph in ['Operations','Advisories','Guidance']:
                command += ['--write',graph]
            return subprocess.run(command+[str(path)],cwd=ENGINE,text=True,capture_output=True), db
        done,db = execute(plan,'composed')
        assert done.returncode == 0, done.stderr
        result = json.loads(done.stdout)[-1]['result']
        assert len(result['graph']['edges']) == 1
        edge = result['graph']['edges'][0]
        assert edge['predicate'] == 'needs_fix'
        assert edge['valid_time'] == {'start':170,'end':200}
        assert {p['assertion_id'] for p in edge['derived_from']} == {'uses-model','affected-model','repair'}
        assert len(result['input_snapshots']) == 3
        with sqlite3.connect(db) as conn:
            assert conn.execute('SELECT COUNT(*) FROM events').fetchone()[0] == 3
            assert conn.execute('SELECT COUNT(*) FROM revisions').fetchone()[0] == 3
        # Compiler validation is not an authority boundary: malformed raw plans must fail atomically.
        invalid = copy.deepcopy(plan)
        bind = next(c for c in plan['commands'] if c['op']=='bind')
        invalid['commands'].append(copy.deepcopy(bind))
        rejected,db = execute(invalid,'duplicate-binding')
        assert rejected.returncode != 0
        assert json.loads(rejected.stderr)['code'] == 'E_BINDING', rejected.stderr
        with sqlite3.connect(db) as conn:
            assert conn.execute('SELECT COUNT(*) FROM events').fetchone()[0] == 0
            assert conn.execute('SELECT COUNT(*) FROM revisions').fetchone()[0] == 0
        print(json.dumps({'status':'passed','contract':plan['version'],'checks':[
            'compiled join-filter-join graph composition', 'three exact leaf premises',
            'three immutable input revisions', 'only explicit commits create events',
            'malformed raw binding rejected', 'invalid program atomically rolled back'
        ]},indent=2))

if __name__ == '__main__':
    main()
