#!/usr/bin/env python3
"""Actual source artifacts, populated store23, and atomic compiled version transfers."""
import argparse
from contextlib import closing
import hashlib
import json
from pathlib import Path
import shutil
import sqlite3
import subprocess
import tempfile
from version_profile import store_marker
from retention_migration import CAUSAL_DISPATCH_TABLES,ACTOR_DISPOSITION_TABLES,ACTOR_LIFECYCLE_TABLES,COMPILED_REBUILD_TABLES,RECORDED_ACTOR_TABLES


def snapshot(path):
    with closing(sqlite3.connect(path)) as c:
        tables=c.execute("SELECT name,sql FROM sqlite_master WHERE type='table' ORDER BY name").fetchall()
        return {'marker':c.execute('PRAGMA user_version').fetchone()[0],
                'tables':{name:{'schema':sql,'rows':sorted(c.execute('SELECT * FROM "'+name.replace('"','""')+'"').fetchall(),key=repr)} for name,sql in tables}}


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ['compiler','old-handler','handler','old-retention','old-lifecycle','lifecycle','storage']:
        p.add_argument('--'+name,type=Path,required=True)
    p.add_argument('--report',type=Path);p.add_argument('--evidence-dir',type=Path)
    a=p.parse_args();trace=[];deaths=0;new_marker=store_marker()
    assert new_marker>=24
    if a.evidence_dir:a.evidence_dir.mkdir(parents=True,exist_ok=False)
    with tempfile.TemporaryDirectory(prefix='weave-compiled-lifecycle-') as temporary:
        root=Path(temporary);db=root/'compiled.sqlite';request=root/'request.json'
        def invoke(binary,*args,code=0):
            nonlocal deaths
            r=subprocess.run([str(binary.resolve()),*map(str,args)],capture_output=True,timeout=30)
            entry={'binary':binary.name,'args':[Path(x).name if isinstance(x,Path) else str(x) for x in args],
                   'exit':r.returncode,'stdout_sha256':hashlib.sha256(r.stdout).hexdigest()};trace.append(entry)
            if a.evidence_dir:
                stem=f'{len(trace):03d}-{binary.name}'
                (a.evidence_dir/(stem+'.stdout')).write_bytes(r.stdout)
                (a.evidence_dir/(stem+'.stderr')).write_bytes(r.stderr)
            assert r.returncode==code,(entry,r.stdout,r.stderr)
            if code in [82,83,92,94]:deaths+=1
            return json.loads(r.stdout) if code==0 else r.stderr
        def host(binary,mode,code=0,**fields):
            request.write_text(json.dumps({'mode':mode,**fields}))
            return invoke(binary,db,request,code=code)
        def schema_upgrade(database):
            before=snapshot(database);assert before['marker']==23 and 'compiled_migrations' not in before['tables']
            invoke(a.storage,database,'crash',60,code=82);assert snapshot(database)==before
            invoke(a.storage,database,'after_commit',60,code=83);after=snapshot(database)
            assert after['marker']==new_marker
            added={'compiled_migrations'}|(COMPILED_REBUILD_TABLES if new_marker>=25 else frozenset())|(RECORDED_ACTOR_TABLES if new_marker>=26 else frozenset())|(ACTOR_LIFECYCLE_TABLES if new_marker>=27 else frozenset())|(ACTOR_DISPOSITION_TABLES if new_marker>=28 else frozenset())|(CAUSAL_DISPATCH_TABLES if new_marker>=29 else frozenset())
            assert set(after['tables'])==set(before['tables'])|added
            assert {n:after['tables'][n] for n in before['tables']}==before['tables']
            assert all(after['tables'][name]['rows']==[] for name in added-CAUSAL_DISPATCH_TABLES)
            return before,after
        artifacts=[]
        for revision in ['1','2']:
            source=root/f'handler-{revision}.weave'
            source.write_text(f'''function Keep revision "{revision}" (graph input) {{ return input; }}
handler KeepSource revision "{revision}" using Keep {{
 input event graph "Installation" branch "main" metadata depth 0;
 on "graph.committed", "graph.accepted";
 output slot "result";
 replay pinned;
}}
''')
            artifact=invoke(a.compiler,'handler-plan',source,'--handler','KeepSource')
            assert artifact['protocol']=='0.21.0' and artifact['revision']==revision
            artifacts.append(artifact);(root/f'artifact-{revision}.json').write_text(json.dumps(artifact,indent=2)+'\n')
        assert artifacts[0]['definition_digest']!=artifacts[1]['definition_digest']
        def write(binary,value,now):
            head=host(binary,'head',graph='Installation',now=now)['head']
            host(binary,'run',now=now,program={'version':'0.21.0','commands':[{'op':'commit','graph_id':'Installation','expected_head':head,
                'data':{'nodes':[{'id':'n','entity_id':'e','space_id':'s','readers':['collector'],'properties':{'value':value}}]}}]})
        def prepare(binary,adapter,now):
            event=host(binary,'poll',id=adapter,now=now)
            assert event is not None
            args={'id':adapter,'event':event['id'],'lease':event['lease'],'now':now}
            prepared=host(binary,'prepare',**args)
            return {**args,'preparation':prepared['preparation_id']}
        write(a.old_handler,1,10)
        host(a.old_handler,'install',id='source1',template=artifacts[0],output='Quality',now=10)
        old_completion=prepare(a.old_handler,'source1',10)
        old_receipt=host(a.old_handler,'complete',**old_completion)
        old_head=host(a.old_handler,'head',graph='Quality')['head']
        write(a.old_handler,2,20)
        host(a.old_handler,'state',id='source1',state='paused',now=20)
        source_before,source_after=schema_upgrade(db)
        host(a.handler,'state',id='source1',state='running',now=20)
        running=snapshot(db)
        assert host(a.handler,'complete',**old_completion)=={**old_receipt,'duplicate':True}
        assert snapshot(db)==running
        host(a.handler,'state',id='source1',state='paused',now=20)
        assert snapshot(db)==source_after
        old_error=host(a.old_handler,'head',code=1,graph='Quality');assert b'E_STORAGE_VERSION' in old_error
        assert snapshot(db)==source_after
        def transfer(source,destination,artifact,kind,nonce):
            inputs=host(a.handler,'migration_inputs',id=source,now=30)
            with closing(sqlite3.connect(db)) as c:
                original=json.loads(c.execute('SELECT registration FROM compiled_handlers WHERE adapter=?',(source,)).fetchone()[0])
            manifest=original['manifest'].copy()
            manifest.update(id=destination,version=artifact['revision'],config_revision=artifact['revision'],artifact_digest=artifact['definition_digest'])
            return {'inputs':inputs,'destination':manifest,'template':artifact,'output':original['output'],'nonce':nonce,'disposition':kind}
        upgrade=transfer('source1','source2',artifacts[1],{'kind':'upgrade'},'source-upgrade')
        before_upgrade=snapshot(db)
        denied=host(a.handler,'migrate',code=1,request=upgrade,outsider=True,now=30)
        assert b'E_HOST_AUTH' in denied and snapshot(db)==before_upgrade
        host(a.handler,'migrate',code=94,request=upgrade,kill_before_commit=True,now=30);assert snapshot(db)==before_upgrade
        host(a.handler,'migrate',code=92,request=upgrade,kill_after_commit=True,now=30)
        after_upgrade=snapshot(db)
        assert after_upgrade['tables']['events']==before_upgrade['tables']['events']
        assert after_upgrade['tables']['heads']==before_upgrade['tables']['heads']
        assert after_upgrade['tables']['handler_preparations']==before_upgrade['tables']['handler_preparations']
        assert after_upgrade['tables']['handler_receipts']==before_upgrade['tables']['handler_receipts']
        assert host(a.handler,'migrate',request=upgrade,now=30)['duplicate'] and snapshot(db)==after_upgrade
        host(a.handler,'state',id='source2',state='running',now=40)
        new_completion=prepare(a.handler,'source2',40);before_complete=snapshot(db)
        host(a.handler,'complete',code=94,**new_completion,kill_before_commit=True);assert snapshot(db)==before_complete
        host(a.handler,'complete',code=92,**new_completion,kill_after_commit=True);after_complete=snapshot(db)
        assert host(a.handler,'complete',**new_completion)['duplicate'] and snapshot(db)==after_complete
        upgraded_head=host(a.handler,'head',graph='Quality')['head'];assert upgraded_head!=old_head
        assert host(a.handler,'migrate',request=upgrade,now=40)['duplicate'] and snapshot(db)==after_complete
        host(a.handler,'state',id='source2',state='paused',now=50)
        rollback=transfer('source2','source3',artifacts[0],{'kind':'rollback','restore_from':'source1'},'source-rollback')
        before_rollback=snapshot(db)
        host(a.handler,'migrate',code=94,request=rollback,kill_before_commit=True,now=50);assert snapshot(db)==before_rollback
        host(a.handler,'migrate',code=92,request=rollback,kill_after_commit=True,now=50);after_rollback=snapshot(db)
        assert after_rollback['tables']['events']==before_rollback['tables']['events']
        assert after_rollback['tables']['heads']==before_rollback['tables']['heads']
        assert host(a.handler,'migrate',request=rollback,now=50)['duplicate'] and snapshot(db)==after_rollback
        host(a.handler,'state',id='source3',state='running',now=60)
        final_completion=prepare(a.handler,'source3',60)
        assert not host(a.handler,'complete',**final_completion)['duplicate']
        final=snapshot(db)
        assert host(a.handler,'migrate',request=rollback,now=60)['duplicate'] and snapshot(db)==final
        final_head=host(a.handler,'head',graph='Quality')['head'];assert final_head not in [old_head,upgraded_head]
        # All prior compiled output revisions and preparation/receipt bytes remain historical.
        for table in ['revisions','handler_preparations','handler_receipts']:
            for row in source_before['tables'][table]['rows']:assert row in final['tables'][table]['rows'],table
        assert [r for r in final['tables']['dispatch_adapters']['rows'] if r[0] in ['source1','source2']] and len(final['tables']['compiled_migrations']['rows'])==2
        # Preserve genuinely non-default native23 GC/state/migration and cancellation records, too.
        native=root/'native23.sqlite';native_request=root/'native-completion.json';native_migration=root/'native-migration.json'
        for mode in ['seed','next','complete','compact','rebase']:invoke(a.old_retention,native,mode,native_request)
        invoke(a.old_lifecycle,native,'prepare_upgrade',native_migration)
        invoke(a.old_lifecycle,native,'migrate',native_migration)
        invoke(a.old_lifecycle,native,'prepare_rollback',native_migration)
        invoke(a.old_lifecycle,native,'migrate',native_migration)
        native_before,native_after=schema_upgrade(native)
        assert len(native_before['tables']['projection_migrations']['rows'])==2
        assert len(native_before['tables']['retention_tombstones']['rows'])==1
        assert invoke(a.lifecycle,native,'migrate',native_migration)['duplicate'] and snapshot(native)==native_after
        old_error=invoke(a.old_lifecycle,native,'inspect',native_request,code=1);assert b'E_STORAGE_VERSION' in old_error
        assert snapshot(native)==native_after
        canceled=root/'canceled23.sqlite';cancellation=root/'cancellation.json'
        invoke(a.old_lifecycle,canceled,'seed_cancel',cancellation)
        invoke(a.old_lifecycle,canceled,'cancel',cancellation)
        cancel_before,cancel_after=schema_upgrade(canceled)
        assert len(cancel_before['tables']['delivery_cancellations']['rows'])==1
        assert invoke(a.lifecycle,canceled,'cancel',cancellation)['duplicate'] and snapshot(canceled)==cancel_after
        assert invoke(a.lifecycle,canceled,'canceled_preparation',cancellation)['error']=='E_DELIVERY_CANCELED'
        assert snapshot(canceled)==cancel_after
        if a.evidence_dir:
            shutil.copytree(root,a.evidence_dir/'actual-fixtures')
            for name,value in [('source23-before',source_before),('source24-after',source_after),('upgrade-before',before_upgrade),('upgrade-after',after_upgrade),('completion-after',after_complete),('rollback-before',before_rollback),('rollback-after',after_rollback),('final',final),('native23-before',native_before),('native24-after',native_after),('cancel23-before',cancel_before),('cancel24-after',cancel_after)]:
                (a.evidence_dir/(name+'.json')).write_text(json.dumps(value,indent=2)+'\n')
    assert deaths==12
    report={'profile':'source-compiled-lifecycle/1','status':'passed','old_marker':23,'new_marker':new_marker,
            'processes':len(trace),'controlled_deaths':deaths,'trace':trace,
            'checks':['two actual source compiler artifacts','genuine old23 completed handler history and paused checkpoint',
                      'all prior schemas and rows unchanged across three atomic upgrades','actual non-default GC/native state/migration/cancellation records',
                      'old23 readers refuse current stores without writes','owner authority and fresh immutable namespaces',
                      'pre/postcommit upgrade, completion and rollback deaths','actual new-version completion and recorded-artifact replay after rollback',
                      'historical outputs/preparations/receipts preserved','historical transfer duplicates never rewind current checkpoint'],
            'scope':'trusted native pure stateless compiled adapters, compatible 0.21 event/input/output scope; remaining actor, reconstruction, portable and effect profiles required'}
    if a.report:a.report.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report))


if __name__=='__main__':main()
