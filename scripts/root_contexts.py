#!/usr/bin/env python3
"""Independent actual-runtime context closure and rollback acceptance."""
import argparse
import copy
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile

ROOT=Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser();parser.add_argument('--language',type=Path,default=ROOT.parent/'weave-language');parser.add_argument('--no-build',action='store_true');args=parser.parse_args()
compiler=[str(args.language.resolve()/'target/debug/weave')] if args.no_build else ['cargo','run','--locked','--quiet','--']
plan=json.loads(subprocess.check_output(compiler+['plan','examples/contexts.weave'],cwd=args.language))
if not args.no_build:subprocess.run(['cargo','build','--locked','-p','weave-engine'],cwd=ROOT,check=True)
with tempfile.TemporaryDirectory(prefix='weave-context-root-') as directory:
    tmp=Path(directory)
    def run(program,name,error=None):
        path=tmp/(name+'.json');path.write_text(json.dumps(program));db=tmp/(name+'.db')
        command=[str(ROOT/'target/debug/weave-engine'),'run','--db',str(db),'--actor','reader']
        for graph in ['World','Evidence','Claims']:command+=['--write',graph]
        result=subprocess.run(command+[str(path)],text=True,capture_output=True)
        if error:
            assert result.returncode!=0 and json.loads(result.stderr)['code']==error,result.stderr
            with sqlite3.connect(db) as c: assert c.execute('SELECT COUNT(*) FROM events').fetchone()[0]==0
            return None
        assert result.returncode==0,result.stderr
        return json.loads(result.stdout)
    mixed=copy.deepcopy(plan)
    mixed['commands'] += [
        {'op':'bind','name':'MixedStatuses','value':{'kind':'union','left':{'kind':'reference','name':'DefaultStatus'},'right':{'kind':'reference','name':'EmptyStatus'}}},
        {'op':'evaluate','value':{'kind':'context','input':{'kind':'reference','name':'MixedStatuses'},'selection':{'kind':'default'}}}]
    result=run(mixed,'mixed')[-1]['result']
    assert result['selected_context']=={'kind':'default'}
    assert len(result['graph']['nodes'])==1,result
    assert result['graph']['nodes'][0]['properties']['state']=='refuted',result
    assert result['graph']['nodes'][0]['context_scope']=={'kind':'default'}

    invalid=copy.deepcopy(plan)
    invalid['commands'].append({'op':'evaluate','value':{'kind':'context','input':{'kind':'reference','name':'Scenario'},'selection':{'kind':'default'}}})
    run(invalid,'relabel','E_CONTEXT_SCOPE')

    incompatible=copy.deepcopy(plan)
    incompatible['commands'].append({'op':'evaluate','value':{'kind':'diff','before':{'kind':'reference','name':'DefaultStatus'},'after':{'kind':'reference','name':'ScenarioStatus'}}})
    run(incompatible,'diff','E_CONTEXT_INCOMPATIBLE')
print(json.dumps({'suite':'context closure','result':'passed','checks':['mixed empty-context status cannot masquerade as default status','discarded worlds cannot be relabeled and all prior commits roll back','cross-context diff fails with whole-program rollback']}))
