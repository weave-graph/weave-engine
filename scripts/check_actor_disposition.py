#!/usr/bin/env python3
"""Real store27 actor history, unknown physical outcomes and owner-disposition deaths."""
import argparse,hashlib,json,shutil,sqlite3,subprocess,tempfile
from contextlib import closing
from pathlib import Path
from check_compiled_lifecycle import snapshot
from version_profile import store_marker
from retention_migration import CAUSAL_DISPATCH_TABLES

def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ['old-controller','old26-actor','old-actor','actor']:p.add_argument('--'+name,type=Path,required=True)
    p.add_argument('--report',type=Path);p.add_argument('--evidence-dir',type=Path)
    a=p.parse_args();assert store_marker()>=28;trace=[];deaths=0
    if a.evidence_dir:a.evidence_dir.mkdir(parents=True,exist_ok=False)
    with tempfile.TemporaryDirectory(prefix='weave-actor-disposition-')as temporary:
        root=Path(temporary);generated=root/'genuine-store27';old_report=root/'genuine-store27.json';request_file=root/'request.json'
        r=subprocess.run(['python3',str(a.old_controller.resolve()),'--old-actor',str(a.old26_actor.resolve()),'--actor',str(a.old_actor.resolve()),'--report',str(old_report),'--evidence-dir',str(generated)],capture_output=True,timeout=180)
        assert r.returncode==0,(r.stdout,r.stderr);history=json.loads(old_report.read_text());assert(history['processes'],history['controlled_deaths'])==(88,12)
        if a.evidence_dir:
            shutil.copytree(generated,a.evidence_dir/'genuine-store27');shutil.copy2(old_report,a.evidence_dir/old_report.name)
            (a.evidence_dir/'genuine-store27-driver.stdout').write_bytes(r.stdout);(a.evidence_dir/'genuine-store27-driver.stderr').write_bytes(r.stderr)
        db=root/'store.sqlite'
        for path in generated.glob('*.sqlite'):shutil.copy2(path,root/path.name)
        def invoke(binary,mode,code=0,**fields):
            nonlocal deaths
            request_file.write_text(json.dumps(fields));r=subprocess.run([str(binary.resolve()),str(db),mode,str(request_file)],capture_output=True,timeout=30)
            entry={'binary':binary.name,'operation':mode,'exit':r.returncode,'stdout_sha256':hashlib.sha256(r.stdout).hexdigest()};trace.append(entry)
            if a.evidence_dir:
                stem=f'{len(trace):03d}-{binary.name}-{mode}';(a.evidence_dir/(stem+'.stdout')).write_bytes(r.stdout);(a.evidence_dir/(stem+'.stderr')).write_bytes(r.stderr);(a.evidence_dir/(stem+'.request.json')).write_bytes(request_file.read_bytes())
            assert r.returncode==code,(entry,r.stdout,r.stderr)
            if 95<=code<=109:deaths+=1
            return json.loads(r.stdout)if code==0 else r.stderr
        def actor(mode,**fields):return invoke(a.actor,mode,id='restored',**fields)
        def rows(path,table):
            with closing(sqlite3.connect(path))as c:return list(c.execute('SELECT * FROM '+table+' ORDER BY rowid'))
        before=snapshot(db);assert before['marker']==27
        for table in ['recorded_actor_migrations','recorded_actor_observations','recorded_actor_receipts']:assert before['tables'][table]['rows']
        tools=db.with_suffix('.lifecycle.tools.sqlite');sink=db.with_suffix('.lifecycle.sink.sqlite');original26_tools=rows(db.with_suffix('.tools.sqlite'),'tool_runs');original26_sink=rows(db.with_suffix('.sink.sqlite'),'physical_receipts');old_tools=rows(tools,'tool_runs');old_physical=rows(sink,'physical_receipts');assert len(old_tools)==3 and len(old_physical)==3
        actor('open',crash='before-schema',code=95);assert snapshot(db)==before
        actor('open',crash='after-schema',code=96);after=snapshot(db);assert after['marker']==store_marker() and set(after['tables'])==set(before['tables'])|{'recorded_actor_cancellations'}|(CAUSAL_DISPATCH_TABLES if store_marker()>=29 else frozenset())
        assert {n:after['tables'][n]for n in before['tables']}==before['tables'] and after['tables']['recorded_actor_cancellations']['rows']==[]
        error=invoke(a.old_actor,'open',code=1);assert b'E_STORAGE_VERSION'in error and snapshot(db)==after
        with closing(sqlite3.connect(db))as c:event3=c.execute("SELECT event_id FROM recorded_actor_observations WHERE adapter='restored'").fetchone()[0]
        original_observation=actor('observation',event=event3,clock=400)
        actor('input',value=5,clock=500);event=actor('poll',clock=500);completion=actor('prepare',event=event,clock=500)
        effect=actor('effect',event=event,payload={'physical':'lost-ack-before-disposition'},clock=500)
        actor('dispatch',intent=effect['id'],clock=500,crash='after-physical',code=101);assert len(rows(sink,'physical_receipts'))==4
        def cancellation(event,nonce):return {'adapter':'restored','event':event['id'],'expected_lease':event['lease'],'nonce':nonce,'reason':'owner_stop'}
        cancel=cancellation(event,'unknown-disposition');stable=snapshot(db);error=actor('cancel',request=cancel,clock=500,code=1);assert b'E_EFFECT_UNKNOWN'in error and snapshot(db)==stable
        actor('reconcile',intent=effect['id'],clock=500);terminal=rows(db,'effect_intents');stable=snapshot(db)
        actor('cancel',request=cancel,clock=500,crash='before-cancellation',code=108);assert snapshot(db)==stable and rows(db,'effect_intents')==terminal
        actor('cancel',request=cancel,clock=500,crash='after-cancellation',code=109);disposed=snapshot(db);receipt=actor('cancel',request=cancel,clock=500);assert receipt['duplicate']and receipt['rebuild_required']and snapshot(db)==disposed and rows(db,'effect_intents')==terminal
        error=actor('complete',request=completion,clock=500,code=1);assert b'E_DELIVERY_CANCELED'in error
        error=actor('state',clock=500,code=1);assert b'E_CHECKPOINT_EXPIRED'in error
        actor('lifecycle',state='running',clock=500);error=actor('poll',clock=500,code=1);assert b'E_CHECKPOINT_EXPIRED'in error
        actor('lifecycle',state='paused',clock=500)
        def initialize(value,clock):
            request={'inputs':actor('inputs',clock=clock),'state_revision':f'owner-initialization-{value}','state':{'sample':'explicit-owner-state','input':value}}
            actor('bootstrap',request=request,clock=clock);actor('lifecycle',state='running',clock=clock)
        initialize(5,500);actor('input',value=6,clock=600);event2=actor('poll',clock=600);completion2=actor('prepare',event=event2,clock=600);effect2=actor('effect',event=event2,payload={'physical':'never-dispatched'},clock=600)
        cancel2=cancellation(event2,'pending-disposition');stable=snapshot(db);physical=rows(sink,'physical_receipts')
        actor('cancel',request=cancel2,clock=600,crash='before-cancellation',code=108);assert snapshot(db)==stable and rows(sink,'physical_receipts')==physical
        actor('cancel',request=cancel2,clock=600,crash='after-cancellation',code=109);receipt2=actor('cancel',request=cancel2,clock=600)
        with closing(sqlite3.connect(db))as c:
            state,response=c.execute('SELECT state,response FROM effect_intents WHERE id=?',(effect2['id'],)).fetchone();assert state=='failed' and json.loads(response)=={'owner_disposition':receipt2['receipt_id'],'outcome':'not_dispatched'}
            assert c.execute("SELECT count(*) FROM handler_receipts WHERE adapter='restored'AND event_id IN (?,?)",(event['id'],event2['id'])).fetchone()[0]==0
        error=actor('dispatch',intent=effect2['id'],clock=600,code=1);assert b'E_EFFECT_AUTHORITY'in error
        assert rows(sink,'physical_receipts')==physical
        initialize(6,600);error=actor('dispatch',intent=effect2['id'],clock=600,code=1);assert b'E_EFFECT_UNKNOWN'in error and rows(sink,'physical_receipts')==physical
        actor('input',value=7,clock=700);event4=actor('poll',clock=700);completion4=actor('prepare',event=event4,clock=700);actor('complete',request=completion4,clock=700)
        current=actor('state',clock=700);stable=snapshot(db);assert actor('cancel',request=cancel,clock=700)['duplicate'];assert actor('cancel',request=cancel2,clock=700)['duplicate'];assert snapshot(db)==stable and actor('state',clock=700)==current
        assert actor('observation',event=event3,clock=700)==original_observation
        actor('compact',policy={'history_before_ms':0,'replay_through_sequence':0},clock=700)
        assert rows(tools,'tool_runs')[:3]==old_tools and rows(sink,'physical_receipts')[:3]==old_physical
        assert rows(db.with_suffix('.tools.sqlite'),'tool_runs')==original26_tools and rows(db.with_suffix('.sink.sqlite'),'physical_receipts')==original26_sink
        assert len(rows(tools,'tool_runs'))==6 and len(rows(sink,'physical_receipts'))==4
        if a.evidence_dir:
            for path in root.glob('*.sqlite'):shutil.copy2(path,a.evidence_dir/path.name)
    report={'profile':'native-actor-disposition/1','store_marker':store_marker(),'result':'passed','processes':len(trace),'controlled_deaths':deaths,'genuine_store27_processes':88,'genuine_store27_deaths':12,'old_tool_runs':3,'total_tool_runs':6,'old_physical_receipts':3,'total_physical_receipts':4,'trace':trace,'checks':['actual old27 version/state/effect/observation journals preserved with every original schema/row across interrupted upgrade','actual new physical action loses acknowledgment and blocks cleanup until destination receipt reconciliation','before/after cancellation deaths preserve terminal receipts and pair state/checkpoint/paused/rebuild/audit','genuine pending intent becomes immutable failed/not-dispatched without a physical action','completion/new delivery/dispatch are fenced after cancellation','explicit owner initialization precedes future native computation','both old cancellation retries preserve newer state/output/checkpoint','original observation, tool journal, physical receipts and typed roots survive collection'],'limits':['trusted native owner/host and local SQLite destination-specific evidence','cleanup is not graph-output reconstruction or new model truth','source/portable commands, expiry, causal/resource controls and every original assurance remain required']}
    if a.report:a.report.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report))
if __name__=='__main__':main()
