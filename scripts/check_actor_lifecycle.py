#!/usr/bin/env python3
"""Actual versioned native tools, physical sink, rollback observation and store26 recovery."""
import argparse,hashlib,json,shutil,sqlite3,subprocess,tempfile
from contextlib import closing
from pathlib import Path
from check_compiled_lifecycle import snapshot
from retention_migration import ACTOR_DISPOSITION_TABLES,ACTOR_LIFECYCLE_TABLES
from version_profile import store_marker

def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ['old-actor','actor']:p.add_argument('--'+name,type=Path,required=True)
    p.add_argument('--report',type=Path);p.add_argument('--evidence-dir',type=Path)
    a=p.parse_args();assert store_marker()>=27;trace=[];deaths=0
    if a.evidence_dir:a.evidence_dir.mkdir(parents=True,exist_ok=False)
    with tempfile.TemporaryDirectory(prefix='weave-actor-lifecycle-') as temporary:
        root=Path(temporary);db=root/'store.sqlite';request_file=root/'request.json'
        def invoke(binary,mode,code=0,database=None,**fields):
            nonlocal deaths
            request_file.write_text(json.dumps(fields));target=database or db
            r=subprocess.run([str(binary.resolve()),str(target),mode,str(request_file)],capture_output=True,timeout=30)
            entry={'binary':binary.name,'operation':mode,'database':target.name,'exit':r.returncode,'stdout_sha256':hashlib.sha256(r.stdout).hexdigest()};trace.append(entry)
            if a.evidence_dir:
                stem=f'{len(trace):03d}-{binary.name}-{mode}';(a.evidence_dir/(stem+'.stdout')).write_bytes(r.stdout);(a.evidence_dir/(stem+'.stderr')).write_bytes(r.stderr)
                (a.evidence_dir/(stem+'.request.json')).write_bytes(request_file.read_bytes())
            assert r.returncode==code,(entry,r.stdout,r.stderr)
            if 95<=code<=107:deaths+=1
            return json.loads(r.stdout) if code==0 else r.stderr
        def old(mode,**fields):return invoke(a.old_actor,mode,**fields)
        def actor(mode,**fields):return invoke(a.actor,mode,**fields)
        def rows(path,table):
            with closing(sqlite3.connect(path))as c:return list(c.execute('SELECT * FROM '+table+' ORDER BY rowid'))
        old('seed',clock=10);initial={'inputs':old('inputs',clock=10),'state_revision':'original26','state':{'sample':'initial','input':1}}
        old('bootstrap',request=initial,clock=10);old('lifecycle',state='running',clock=10);old('input',value=2,clock=20);old_event=old('poll',clock=20);old_completion=old('prepare',event=old_event,clock=20)
        effect=old('effect',event=old_event,payload={'physical':'old26'},clock=20);old('dispatch',intent=effect['id'],clock=20);old_done=old('complete',request=old_completion,clock=20);old('lifecycle',state='paused',clock=20)
        before=snapshot(db);assert before['marker']==26 and len(before['tables']['recorded_actor_receipts']['rows'])==1
        old_tools=rows(db.with_suffix('.tools.sqlite'),'tool_runs');old_sink=rows(db.with_suffix('.sink.sqlite'),'physical_receipts')
        actor('open',crash='before-schema',code=95);assert snapshot(db)==before
        actor('open',crash='after-schema',code=96);after=snapshot(db)
        assert after['marker']==store_marker() and set(after['tables'])==set(before['tables'])|ACTOR_LIFECYCLE_TABLES|(ACTOR_DISPOSITION_TABLES if store_marker()>=28 else frozenset())
        assert {n:after['tables'][n]for n in before['tables']}==before['tables']
        assert all(after['tables'][n]['rows']==[]for n in ACTOR_LIFECYCLE_TABLES|(ACTOR_DISPOSITION_TABLES if store_marker()>=28 else frozenset()))
        error=old('open',code=1);assert b'E_STORAGE_VERSION'in error and snapshot(db)==after
        assert rows(db.with_suffix('.tools.sqlite'),'tool_runs')==old_tools and rows(db.with_suffix('.sink.sqlite'),'physical_receipts')==old_sink
        actor('seed',clock=100);initial={'inputs':actor('inputs',clock=100),'state_revision':'v1-initial','state':{'sample':'initial','input':1}}
        actor('bootstrap',request=initial,clock=100);actor('lifecycle',state='running',clock=100);actor('input',value=2,clock=200);event=actor('poll',clock=200)
        actor('prepare',event=event,clock=200,crash='after-tool-journal',code=99);completion=actor('prepare',event=event,clock=200)
        tools=db.with_suffix('.lifecycle.tools.sqlite');sink=db.with_suffix('.lifecycle.sink.sqlite')
        assert len(rows(tools,'tool_runs'))==1
        effect=actor('effect',event=event,payload={'physical':'v1'},clock=200);actor('dispatch',intent=effect['id'],clock=200);first=actor('complete',request=completion,clock=200);actor('lifecycle',state='paused',clock=200)
        v2=actor('definition',id='native-v2',version=2);upgrade={'inputs':actor('migration_inputs',clock=200),'destination':v2,'nonce':'upgrade-v2','disposition':{'kind':'upgrade'}}
        stable=snapshot(db);actor('migrate',request=upgrade,clock=200,crash='before-migration',code=104);assert snapshot(db)==stable
        actor('migrate',request=upgrade,clock=200,crash='after-migration',code=105);upgraded=snapshot(db);assert actor('migrate',request=upgrade,clock=200)['duplicate'] and snapshot(db)==upgraded
        carried=actor('state',id='native-v2',clock=200);assert carried['state']==completion['state'] and carried['artifacts']==first['artifacts']
        actor('lifecycle',id='native-v2',state='running',clock=200);actor('input',value=3,clock=300);event2=actor('poll',id='native-v2',clock=300);completion2=actor('prepare',id='native-v2',version=2,event=event2,clock=300)
        assert completion2['state']['sample'].startswith('v2:') and len(rows(tools,'tool_runs'))==2
        effect2=actor('effect',id='native-v2',event=event2,payload={'physical':'v2'},clock=300)
        actor('dispatch',intent=effect2['id'],clock=300,crash='after-physical',code=101);assert len(rows(sink,'physical_receipts'))==2
        actor('lifecycle',id='native-v2',state='paused',clock=300)
        destination=actor('definition',id='restored',version=1);rollback={'inputs':actor('migration_inputs',id='native-v2',clock=300),'destination':destination,'nonce':'rollback-v1','disposition':{'kind':'rollback','restore_from':'native-v1'}}
        error=actor('migrate',request=rollback,clock=300,code=1);assert b'E_ACTOR_PENDING'in error
        error=actor('dispatch',intent=effect2['id'],clock=300,code=1);assert b'E_EFFECT_UNKNOWN'in error or b'E_EFFECT_AUTHORITY'in error
        actor('reconcile',intent=effect2['id'],clock=300);actor('lifecycle',id='native-v2',state='running',clock=300)
        prior=snapshot(db);actor('complete',request=completion2,clock=300,crash='before-complete',code=102);assert snapshot(db)==prior
        actor('complete',request=completion2,clock=300,crash='after-complete',code=103);second=actor('complete',request=completion2,clock=300);assert second['handler']['duplicate'];second['handler']['duplicate']=False
        actor('lifecycle',id='native-v2',state='paused',clock=300);rollback['inputs']=actor('migration_inputs',id='native-v2',clock=300)
        prior=snapshot(db);actor('migrate',request=rollback,clock=300,crash='before-migration',code=104);assert snapshot(db)==prior
        actor('migrate',request=rollback,clock=300,crash='after-migration',code=105);restored_snapshot=snapshot(db)
        restored=actor('state',id='restored',clock=300);assert restored['state']==completion['state'] and restored['artifacts']==first['artifacts']
        # A second rollback precedes any intermediate observation. Its private cut must
        # retain the original known effect frontier and resolve the real ancestor.
        nested_db=root/'nested.sqlite';shutil.copy2(db,nested_db)
        nested_definition=actor('definition',id='nested',version=1,database=nested_db)
        nested_request={'inputs':actor('migration_inputs',id='restored',clock=300,database=nested_db),'destination':nested_definition,'nonce':'nested-rollback','disposition':{'kind':'rollback','restore_from':'native-v1'}}
        actor('migrate',request=nested_request,clock=300,database=nested_db);actor('lifecycle',id='nested',state='running',clock=300,database=nested_db)
        nested_event=actor('poll',id='nested',clock=300,database=nested_db)
        nested_state=actor('state',id='nested',clock=300,database=nested_db)
        nested_observation={'adapter':'nested','event':nested_event['id'],'lease':nested_event['lease'],'prior_state_digest':actor('migration_inputs',id='nested',clock=300,database=nested_db)['state_digest'],'nonce':'nested-observation'}
        nested=actor('observe',request=nested_observation,clock=300,database=nested_db);assert nested['source_adapter']=='native-v2' and nested['source_receipt']==second
        actor('compact',policy={'history_before_ms':0,'replay_through_sequence':0},clock=300,database=nested_db)
        actor('lifecycle',id='restored',state='running',clock=300);event3=actor('poll',id='restored',clock=300);assert event3['id']==event2['id']
        assert actor('mode',id='restored',event=event3,clock=300)=='observe'
        tool_rows=rows(tools,'tool_runs');physical_rows=rows(sink,'physical_receipts');output=actor('query',clock=300)
        error=actor('prepare',id='restored',version=1,event=event3,clock=300,code=1);assert b'E_ACTOR_REPLAY'in error
        error=actor('effect',id='restored',event=event3,payload={'repeat':True},clock=300,code=1);assert b'E_ACTOR_REPLAY'in error
        assert rows(tools,'tool_runs')==tool_rows and rows(sink,'physical_receipts')==physical_rows
        current=actor('state',id='restored',clock=300);observation={'adapter':'restored','event':event3['id'],'lease':event3['lease'],'prior_state_digest':actor('migration_inputs',id='restored',clock=300)['state_digest'],'nonce':'observe-actual-v2'}
        prior=snapshot(db);actor('observe',request=observation,clock=300,crash='before-observation',code=106);assert snapshot(db)==prior
        actor('observe',request=observation,clock=300,crash='after-observation',code=107);observed=snapshot(db);out=actor('observe',request=observation,clock=300);assert out['handler']['duplicate'];out['handler']['duplicate']=False
        assert out['source_receipt']==second and snapshot(db)==observed
        assert actor('query',clock=300)==output and rows(tools,'tool_runs')==tool_rows and rows(sink,'physical_receipts')==physical_rows
        assert actor('state',id='restored',clock=300)['state']==completion2['state']
        actor('input',value=4,clock=400);event4=actor('poll',id='restored',clock=400);assert actor('mode',id='restored',event=event4,clock=400)=='compute';completion4=actor('prepare',id='restored',version=1,event=event4,clock=400)
        effect4=actor('effect',id='restored',event=event4,payload={'physical':'after-observation'},clock=400);actor('dispatch',intent=effect4['id'],clock=400);actor('complete',request=completion4,clock=400)
        latest=actor('state',id='restored',clock=400);latest_snapshot=snapshot(db);observation['lease']='different-worker'
        assert actor('observe',request=observation,clock=400)['handler']['duplicate'];assert actor('migrate',request=upgrade,clock=400)['duplicate'];assert snapshot(db)==latest_snapshot and actor('state',id='restored',clock=400)==latest
        assert len(rows(tools,'tool_runs'))==3 and len(rows(sink,'physical_receipts'))==3
        assert len({r[5]for r in rows(tools,'tool_runs')})==2
        actor('compact',policy={'history_before_ms':0,'replay_through_sequence':0},clock=400)
        assert actor('observation',id='restored',event=event3['id'],clock=400)==out
        assert rows(db.with_suffix('.tools.sqlite'),'tool_runs')==old_tools and rows(db.with_suffix('.sink.sqlite'),'physical_receipts')==old_sink
        if a.evidence_dir:
            for path in root.glob('*.sqlite'):shutil.copy2(path,a.evidence_dir/path.name)
    report={'profile':'native-actor-lifecycle-observation/1','store_marker':store_marker(),'processes':len(trace),'controlled_deaths':deaths,'native_tool_runs':3,'physical_receipts':3,'original_store26_tool_runs':1,'original_store26_physical_receipts':1,'distinct_executed_artifacts':2,'trace':trace,'result':'passed','checks':['every original populated store26 schema and row survives interrupted upgrade','real distinct native v1/v2 code artifacts and nondeterministic journaled samples','unknown physical acknowledgment blocks transfer and reconciles actual destination receipt','version transfer and recorded prior-pair rollback atomic before/after process deaths','rollback fences new tool computation and broker intents','historical observation and private checkpoint atomic before/after deaths without graph writes, model calls or I/O','nested rollback reaches actual ancestor even before intermediate observation','historical observation/migration retries never rewind newer state/output/checkpoint','future occurrence runs the installed restored v1 artifact','typed migration/fence/observation journals survive collection'],'limits':['trusted native host state ABI; artifact hashing is not execution attestation','local SQLite destination idempotency and reconciliation, no arbitrary remote exactly-once','source/portable bindings, full graph-output reconstruction and broader original assurance remain required']}
    if a.report:a.report.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report))
if __name__=='__main__':main()
