#!/usr/bin/env python3
"""Independent source-claim privacy and atomicity checks through both real CLIs."""
import argparse
import copy
import json
import sqlite3
import subprocess
import tempfile
from pathlib import Path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--engine',type=Path,default=Path(__file__).resolve().parents[1])
    parser.add_argument('--language',type=Path,default=Path(__file__).resolve().parents[2]/'weave-language')
    args=parser.parse_args()
    compiled=subprocess.run(['cargo','run','--locked','--quiet','--','plan','examples/assertions.weave'],cwd=args.language,capture_output=True,text=True,check=True)
    source=json.loads(compiled.stdout)
    # Select the source graph and reusable values without a projection that
    # intentionally fails when its named assertion becomes unavailable.
    selected=[c for c in source['commands'] if (c.get('op')=='commit' and c.get('graph_id')=='Claims') or c.get('name') in ('Evidence','Conflict')]
    assert len(selected)==3,selected
    plan=dict(source,commands=selected)
    checks=[]
    with tempfile.TemporaryDirectory(prefix='weave-assertions-') as tmp:
        root=Path(tmp); serial=0
        def run(program,db,actor='alice',writes=(),code=None):
            nonlocal serial
            serial+=1; path=root/f'plan-{serial}.json';path.write_text(json.dumps(program))
            command=['cargo','run','--locked','--quiet','-p','weave-engine','--','run','--db',str(db),'--actor',actor]
            for graph in writes: command += ['--write',graph]
            r=subprocess.run(command+[str(path)],cwd=args.engine,capture_output=True,text=True)
            if code:
                assert r.returncode != 0 and json.loads(r.stderr)['code']==code,r.stderr
                return
            assert r.returncode==0,r.stderr
            return json.loads(r.stdout)
        private=copy.deepcopy(plan)
        next(a for a in private['commands'][0]['data']['assertions'] if a['id']=='source-positive')['readers']=['bob']
        result=run(private,root/'private.db',writes=['Claims'])
        visible=result[1]['result']
        assert [e['id'] for e in visible['graph']['edges'] if e['predicate']=='affected']==['source-negative']
        assert result[-1]['result']['graph']['nodes'][0]['properties']['state']=='refuted'
        assert 'inspection-17' not in json.dumps(result[1:])
        checks.append('hidden positive source cannot affect visible support or explanation data')
        hidden=copy.deepcopy(plan)
        for relation in hidden['commands'][0]['data']['structural_edges']: relation['readers']=['bob']
        result=run(hidden,root/'structure.db',writes=['Claims'])
        assert not result[1]['result']['graph']['edges']
        assert result[-1]['result']['graph']['nodes'][0]['properties']['state']=='unknown'
        checks.append('structural restriction applies to every attached source claim')
        contextual=copy.deepcopy(plan)
        next(a for a in contextual['commands'][0]['data']['assertions'] if a['id']=='source-positive')['context']={'graph_id':'World','revision':'world-1'}
        run(contextual,root/'context.db',writes=['Claims'],code='E_CONTEXT_REQUIRED')
        checks.append('contextual claims cannot silently become unconditional support')
        stored=copy.deepcopy(plan)
        for claim in stored['commands'][0]['data']['assertions']: claim['readers']=['alice']
        db=root/'persisted.db';result=run(stored,db,writes=['Claims'])
        graph=result[1]['result']['graph']
        assert all(e['derived_from'] for e in graph['edges'])
        for item in graph['nodes']+graph['edges']: item['readers']=[]
        result=run({'version':result[1]['result']['version'],'commands':[{'op':'commit','graph_id':'Derived','data':graph}]},db,writes=['Derived'])
        query={'version':source['version'],'commands':[{'op':'query','query':{'graph_id':'Derived'}}]}
        result=run(query,db,actor='bob')
        assert not result[-1]['result']['graph']['edges']
        assert result[-1]['result']['coverage']=='partial'
        checks.append('repersisted materialized claims retain private source premises even if readers are cleared')
        invalid=copy.deepcopy(source)
        next(c for c in invalid['commands'] if c.get('graph_id')=='Claims')['data']['profile']='legacy'
        bad=root/'bad.db'
        # The precise diagnostic is intentionally not coupled to validator ordering.
        serial+=1;path=root/f'bad-{serial}.json';path.write_text(json.dumps(invalid))
        r=subprocess.run(['cargo','run','--locked','--quiet','-p','weave-engine','--','run','--db',str(bad),'--actor','alice','--write','Empty','--write','Claims',str(path)],cwd=args.engine,capture_output=True,text=True)
        assert r.returncode!=0,r.stdout
        with sqlite3.connect(bad) as connection:
            assert connection.execute('SELECT COUNT(*) FROM events').fetchone()[0]==0
            assert connection.execute('SELECT COUNT(*) FROM revisions').fetchone()[0]==0
        checks.append('explicit records in legacy profile reject with whole-program rollback')
    print(json.dumps({'status':'passed','contract':source['version'],'checks':checks},indent=2))


if __name__=='__main__': main()
