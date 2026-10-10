#!/usr/bin/env python3
"""Genuine native24 history and actual source-compiled snapshot reconstruction deaths."""
import argparse
from contextlib import closing
import hashlib
import json
from pathlib import Path
import shutil
import sqlite3
import subprocess
import tempfile
from check_compiled_lifecycle import snapshot
from version_profile import store_marker
from retention_migration import ACTOR_LIFECYCLE_TABLES,RECORDED_ACTOR_TABLES


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ['compiler','old-handler','handler','storage']:p.add_argument('--'+name,type=Path,required=True)
    p.add_argument('--report',type=Path);p.add_argument('--evidence-dir',type=Path)
    a=p.parse_args();trace=[];deaths=0;assert store_marker()>=25
    if a.evidence_dir:a.evidence_dir.mkdir(parents=True,exist_ok=False)
    with tempfile.TemporaryDirectory(prefix='weave-compiled-rebuild-') as temporary:
        root=Path(temporary);db=root/'store.sqlite';request=root/'request.json'
        def invoke(binary,*args,code=0):
            nonlocal deaths
            r=subprocess.run([str(binary.resolve()),*map(str,args)],capture_output=True,timeout=30)
            entry={'binary':binary.name,'args':[Path(x).name if isinstance(x,Path) else str(x) for x in args],
                   'exit':r.returncode,'stdout_sha256':hashlib.sha256(r.stdout).hexdigest()};trace.append(entry)
            if a.evidence_dir:
                stem=f'{len(trace):03d}-{binary.name}';(a.evidence_dir/(stem+'.stdout')).write_bytes(r.stdout);(a.evidence_dir/(stem+'.stderr')).write_bytes(r.stderr)
            assert r.returncode==code,(entry,r.stdout,r.stderr)
            if code in [82,83,92,94]:deaths+=1
            return json.loads(r.stdout) if code==0 else r.stderr
        def host(binary,mode,code=0,**fields):
            request.write_text(json.dumps({'mode':mode,**fields}));return invoke(binary,db,request,code=code)
        def write(binary,graph,value,now,readers=['collector']):
            head=host(binary,'head',graph=graph,now=now)['head']
            host(binary,'run',now=now,program={'version':'0.21.0','commands':[{'op':'commit','graph_id':graph,'expected_head':head,
                'data':{'nodes':[{'id':'n','entity_id':'e','space_id':'s','readers':readers,'properties':{'value':value}}]}}]})
        def prepare(binary,adapter,now):
            event=host(binary,'poll',id=adapter,now=now);assert event
            args={'id':adapter,'event':event['id'],'lease':event['lease'],'now':now}
            prepared=host(binary,'prepare',**args);return {**args,'preparation':prepared['preparation_id']}
        artifacts=[]
        for revision in ['1','2']:
            source=root/f'handler-{revision}.weave';source.write_text(f'''function Keep revision "{revision}" (graph input) {{ return input; }}
handler Reconstruct revision "{revision}" using Keep {{
 input event graph "Installation" branch "main" metadata depth 0;
 on "graph.committed", "graph.accepted";
 output slot "result";
 replay pinned;
}}
''')
            artifact=invoke(a.compiler,'handler-plan',source,'--handler','Reconstruct');assert artifact['protocol']=='0.21.0'
            artifacts.append(artifact);(root/f'artifact-{revision}.json').write_text(json.dumps(artifact,indent=2)+'\n')
        def transfer(binary,source,destination,artifact,kind,nonce,now):
            inputs=host(binary,'migration_inputs',id=source,now=now)
            with closing(sqlite3.connect(db)) as c:registered=json.loads(c.execute('SELECT registration FROM compiled_handlers WHERE adapter=?',(source,)).fetchone()[0])
            manifest=registered['manifest'].copy();manifest.update(id=destination,version=artifact['revision'],config_revision=artifact['revision'],artifact_digest=artifact['definition_digest'])
            body={'inputs':inputs,'destination':manifest,'template':artifact,'output':registered['output'],'nonce':nonce,'disposition':kind}
            host(binary,'migrate',request=body,now=now);return body
        write(a.old_handler,'Other',10,5);write(a.old_handler,'Other',11,6)
        write(a.old_handler,'Installation',1,10)
        host(a.old_handler,'install',id='source1',template=artifacts[0],output='Quality',now=10)
        old_completion=prepare(a.old_handler,'source1',10);old_receipt=host(a.old_handler,'complete',**old_completion)
        write(a.old_handler,'Installation',2,20);host(a.old_handler,'state',id='source1',state='paused',now=20)
        transfer(a.old_handler,'source1','source2',artifacts[1],{'kind':'upgrade'},'old-upgrade',20)
        transfer(a.old_handler,'source2','source3',artifacts[0],{'kind':'rollback','restore_from':'source1'},'old-rollback',20)
        before=snapshot(db);assert before['marker']==24 and len(before['tables']['compiled_migrations']['rows'])==2
        new_tables={'compiled_rebuild_receipts','compiled_replay_states'} | (RECORDED_ACTOR_TABLES if store_marker()>=26 else frozenset())|(ACTOR_LIFECYCLE_TABLES if store_marker()>=27 else frozenset());assert not new_tables.intersection(before['tables'])
        invoke(a.storage,db,'crash',30,code=82);assert snapshot(db)==before
        invoke(a.storage,db,'after_commit',30,code=83);after=snapshot(db)
        assert after['marker']==store_marker() and set(after['tables'])==set(before['tables'])|new_tables
        assert {n:after['tables'][n] for n in before['tables']}==before['tables']
        assert all(after['tables'][n]['rows']==[] for n in new_tables)
        error=host(a.old_handler,'head',code=1,graph='Quality');assert b'E_STORAGE_VERSION' in error and snapshot(db)==after
        host(a.handler,'compact',policy={'history_before_ms':30,'replay_through_sequence':3},now=30)
        compacted=snapshot(db);assert len(compacted['tables']['retention_tombstones']['rows'])==1
        host(a.handler,'state',id='source3',state='running',now=30)
        error=host(a.handler,'poll',id='source3',code=1,now=30);assert b'E_CHECKPOINT_EXPIRED' in error
        host(a.handler,'state',id='source3',state='paused',now=30)
        def rebuild(nonce,now):
            body={'inputs':host(a.handler,'rebuild_inputs',id='source3',now=now),'nonce':nonce}
            prior=snapshot(db)
            host(a.handler,'rebuild',code=94,request=body,kill_before_commit=True,now=now);assert snapshot(db)==prior
            host(a.handler,'rebuild',code=92,request=body,kill_after_commit=True,now=now);committed=snapshot(db)
            receipt=host(a.handler,'rebuild',request=body,now=now);assert receipt['duplicate'] and snapshot(db)==committed
            return body,receipt,prior,committed
        first,first_receipt,first_before,first_after=rebuild('first',30)
        def value(reference=None):
            q={'graph_id':'Quality'}
            if reference:q['revision']=reference['revision']
            return host(a.handler,'query',query=q,now=60)['graph']['nodes'][0]['properties']['value']
        assert value()==2
        host(a.handler,'state',id='source3',state='running',now=30);assert host(a.handler,'poll',id='source3',now=30) is None
        public_before=host(a.handler,'rebuild_inputs',id='source3',now=35)
        write(a.handler,'Other',999,35,readers=['outsider']);assert host(a.handler,'poll',id='source3',now=35) is None
        assert host(a.handler,'rebuild_inputs',id='source3',now=35)==public_before
        write(a.handler,'Installation',3,40);pending=prepare(a.handler,'source3',40);write(a.handler,'Quality',99,40)
        error=host(a.handler,'complete',code=1,**pending);assert b'E_CONFLICT' in error
        cancel={'adapter':'source3','event':pending['event'],'expected_lease':pending['lease'],'nonce':'stale','reason':'stale_output'}
        cancel_before=snapshot(db)
        host(a.handler,'cancel',code=94,request=cancel,kill_before_commit=True,now=40);assert snapshot(db)==cancel_before
        host(a.handler,'cancel',code=92,request=cancel,kill_after_commit=True,now=40);cancel_after=snapshot(db)
        assert host(a.handler,'cancel',request=cancel,now=40)['rebuild_required'] and snapshot(db)==cancel_after
        error=host(a.handler,'poll',id='source3',code=1,now=40);assert b'E_CHECKPOINT_EXPIRED' in error
        host(a.handler,'state',id='source3',state='paused',now=40)
        second,_,second_before,second_after=rebuild('again',40);assert value()==3
        assert value(first_receipt['output'])==2
        stable=snapshot(db);assert host(a.handler,'rebuild',request=first,now=50)['duplicate'] and snapshot(db)==stable
        transfer(a.handler,'source3','source4',artifacts[1],{'kind':'upgrade'},'new-upgrade',50)
        host(a.handler,'state',id='source4',state='running',now=50);write(a.handler,'Installation',4,50)
        current=prepare(a.handler,'source4',50);assert not host(a.handler,'complete',**current)['duplicate'];assert value()==4
        final=snapshot(db)
        for table in ['compiled_handlers','compiled_migrations','handler_preparations','handler_receipts']:
            for row in before['tables'][table]['rows']:assert row in final['tables'][table]['rows'],table
        assert len(final['tables']['compiled_rebuild_receipts']['rows'])==2 and len(final['tables']['compiled_replay_states']['rows'])==2
        checkpoints={r[0]:r[3] for r in final['tables']['dispatch_adapters']['rows']}
        assert all(r[4]==checkpoints[r[0]] for r in final['tables']['compiled_replay_states']['rows'])
        if a.evidence_dir:
            shutil.copytree(root,a.evidence_dir/'actual-fixtures')
            for name,state in [('native24-before',before),('native25-after',after),('compacted',compacted),('first-before',first_before),('first-after',first_after),('cancel-before',cancel_before),('cancel-after',cancel_after),('second-before',second_before),('second-after',second_after),('final',final)]:
                (a.evidence_dir/(name+'.json')).write_text(json.dumps(state,indent=2)+'\n')
    assert deaths==8
    report={'profile':'source-compiled-reconstruction/1','status':'passed','old_marker':24,'new_marker':store_marker(),
            'processes':len(trace),'controlled_deaths':deaths,'trace':trace,
            'checks':['two genuine source compiler artifacts and real old24 completed/prepared/upgrade/rollback history',
                      'atomic schema upgrade preserves all prior schemas/rows and old reader refuses new store',
                      'actual orphan payload erasure and explicit compiled replay expiry',
                      'kernel recipe computes current output and paired private checkpoint in one reconstruction transaction',
                      'private scans cannot change public reconstruction inputs','actual stale CAS preparation cancellation fences until another reconstruction',
                      'historical reconstruction retry never overwrites newer output or rewinds checkpoint',
                      'compatible artifact upgrade preserves actual replay-ready state and subsequent real completion',
                      'schema, both reconstructions and state-bound cancellation pre/postcommit process-death pairs'],
            'scope':'trusted native pure stateless compiled reconstruction; opaque/effectful actors, external journals and source/portable lifecycle commands remain mandatory'}
    if a.report:a.report.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report))


if __name__=='__main__':main()
