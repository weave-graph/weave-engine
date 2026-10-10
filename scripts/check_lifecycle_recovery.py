#!/usr/bin/env python3
"""Genuine compacted store22 upgrade and compiled cancellation/state transfer process deaths."""
import argparse, hashlib, json, shutil, sqlite3, subprocess, tempfile
from contextlib import closing
from pathlib import Path
from version_profile import store_marker
from retention_migration import COMPILED_LIFECYCLE_TABLES,COMPILED_REBUILD_TABLES
LIFECYCLE_TABLES=frozenset(('delivery_cancellations','projection_rebuild_requests','projection_migrations'))

def snapshot(path):
    with closing(sqlite3.connect(path)) as c:
        tables=c.execute("SELECT name,sql FROM sqlite_master WHERE type='table' ORDER BY name").fetchall()
        return {'marker':c.execute('PRAGMA user_version').fetchone()[0], 'tables':{name:{'schema':schema,'rows':sorted(c.execute('SELECT * FROM "'+name.replace('"','""')+'"').fetchall(),key=repr)} for name,schema in tables}}

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--old-host',type=Path,required=True);p.add_argument('--host',type=Path,required=True);p.add_argument('--report',type=Path);p.add_argument('--evidence-dir',type=Path)
    a=p.parse_args();trace=[];old=a.old_host.resolve();host=a.host.resolve()
    if a.evidence_dir:a.evidence_dir.mkdir(parents=True,exist_ok=False)
    with tempfile.TemporaryDirectory(prefix='weave-lifecycle-recovery-') as temporary:
        root=Path(temporary);db=root/'store.sqlite';request=root/'completion.json';migration=root/'migration.json'
        def invoke(binary,database,mode,body=request,code=0):
            r=subprocess.run([str(binary),str(database),mode,str(body)],capture_output=True,timeout=30)
            entry={'host':'old-store22' if binary==old else 'current','operation':mode,'exit':r.returncode,'stdout_sha256':hashlib.sha256(r.stdout).hexdigest()};trace.append(entry)
            if a.evidence_dir:
                stem=f'{len(trace):02d}-{mode}';(a.evidence_dir/(stem+'.stdout')).write_bytes(r.stdout);(a.evidence_dir/(stem+'.stderr')).write_bytes(r.stderr)
            assert r.returncode==code,(entry,r.stdout,r.stderr)
            return json.loads(r.stdout) if code==0 else r.stderr
        for mode in ['seed','next','complete','compact','rebase']:invoke(old,db,mode)
        prior=invoke(old,db,'inspect');before=snapshot(db);assert before['marker']==22 and prior['state']['state']=={'total':2}
        assert not LIFECYCLE_TABLES.intersection(before['tables'])
        assert len(before['tables']['retention_tombstones']['rows'])==1
        assert len(before['tables']['retention_projection_receipts']['rows'])==1
        invoke(host,db,'open_crash',code=82);assert snapshot(db)==before
        invoke(host,db,'open_after',code=83);after=snapshot(db);assert after['marker']==store_marker()
        new_tables=LIFECYCLE_TABLES|(COMPILED_LIFECYCLE_TABLES if store_marker()>=24 else frozenset())|(COMPILED_REBUILD_TABLES if store_marker()>=25 else frozenset())
        assert set(after['tables'])==set(before['tables'])|new_tables
        assert {n:after['tables'][n] for n in before['tables']}==before['tables']
        assert all(after['tables'][n]['rows']==[] for n in new_tables)
        current=invoke(host,db,'inspect');assert {**current,'marker':22}==prior
        assert invoke(host,db,'duplicate_completion')['duplicate'];assert snapshot(db)==after
        error=invoke(old,db,'inspect',code=1);assert b'E_STORAGE_VERSION' in error;assert snapshot(db)==after
        invoke(host,db,'prepare_upgrade',migration);before_upgrade=snapshot(db)
        invoke(host,db,'migrate_crash',migration,82);assert snapshot(db)==before_upgrade
        invoke(host,db,'migrate_after',migration,83);upgraded=snapshot(db)
        assert invoke(host,db,'migrate',migration)['duplicate'];assert snapshot(db)==upgraded
        states={r[0]:json.loads(r[2]) for r in upgraded['tables']['retention_adapter_states']['rows']}
        assert states['projection2']['state']=={'sum':2,'algorithm':2}
        checkpoints={r[0]:r[3] for r in upgraded['tables']['dispatch_adapters']['rows']};assert checkpoints['projection2']==checkpoints['projection']
        invoke(host,db,'prepare_rollback',migration);before_rollback=snapshot(db)
        invoke(host,db,'migrate_crash',migration,82);assert snapshot(db)==before_rollback
        invoke(host,db,'migrate_after',migration,83);rolled=snapshot(db)
        assert invoke(host,db,'migrate',migration)['duplicate'];assert snapshot(db)==rolled
        states={r[0]:json.loads(r[2]) for r in rolled['tables']['retention_adapter_states']['rows']};assert states['projection3']['state']==prior['state']['state'] and states['projection3']['inputs']['snapshots']==prior['state']['inputs']['snapshots']
        checkpoints={r[0]:r[3] for r in rolled['tables']['dispatch_adapters']['rows']};assert checkpoints['projection3']==checkpoints['projection']
        assert rolled['tables']['heads']==after['tables']['heads'] and rolled['tables']['events']==after['tables']['events']
        cancellation_db=root/'compiled.sqlite';cancellation=root/'cancellation.json'
        seeded=invoke(host,cancellation_db,'seed_cancel',cancellation);before_cancel=snapshot(cancellation_db)
        invoke(host,cancellation_db,'cancel_crash',cancellation,82);assert snapshot(cancellation_db)==before_cancel
        invoke(host,cancellation_db,'cancel_after',cancellation,83);canceled=snapshot(cancellation_db)
        assert invoke(host,cancellation_db,'cancel',cancellation)['duplicate'];assert snapshot(cancellation_db)==canceled
        assert invoke(host,cancellation_db,'canceled_preparation',cancellation)['error']=='E_DELIVERY_CANCELED';assert snapshot(cancellation_db)==canceled
        assert canceled['tables']['heads']==before_cancel['tables']['heads'] and canceled['tables']['events']==before_cancel['tables']['events']
        assert canceled['tables']['handler_preparations']==before_cancel['tables']['handler_preparations'] and canceled['tables']['handler_receipts']['rows']==[] and canceled['tables']['dispatch_pending']['rows']==[]
        assert len(canceled['tables']['delivery_cancellations']['rows'])==1
        if a.evidence_dir:
            shutil.copytree(root,a.evidence_dir/'actual-fixtures')
            for name,value in [('store22-before',before),('store23-upgraded',after),('migration-before',before_upgrade),('migration-after',upgraded),('rollback-after',rolled),('cancel-before',before_cancel),('cancel-after',canceled)]:
                (a.evidence_dir/(name+'.json')).write_text(json.dumps(value,indent=2)+'\n')
    report={'profile':'native-lifecycle-recovery/1','status':'passed','old_marker':22,'new_marker':store_marker(),'processes':len(trace),'controlled_deaths':8,'trace':trace,'checks':['real old22 compacted payload, opaque state, immutable receipt and private checkpoint preserved byte-for-byte','all prior table schemas and rows survive initialization','precommit schema death and postcommit restart','old22 host refuses the current store without writes','exact prior completed occurrence never rewinds rebuilt state','new artifact/state/private checkpoint/retirement/receipt commit together','rollback restores recorded prior artifact/state/checkpoint in fresh namespace','genuine stale-CAS compiled preparation retained and canceled occurrence cannot execute','all four pre/postcommit process-death pairs'],'scope':'fixed trusted native pure adapters; no source migration, effectful rollback or hostile module isolation claim'}
    if a.report:a.report.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report))
if __name__=='__main__':main()
