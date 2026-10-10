#!/usr/bin/env python3
"""Genuine old28 actor/effect journals and actual two-adapter causal process deaths."""
import argparse,hashlib,json,shutil,sqlite3,subprocess,tempfile
from contextlib import closing
from pathlib import Path
from check_compiled_lifecycle import snapshot
from version_profile import store_marker

def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ['old-disposition-controller','old-lifecycle-controller','old26-actor','old27-actor','old28-actor','host']:p.add_argument('--'+name,type=Path,required=True)
    p.add_argument('--report',type=Path);p.add_argument('--evidence-dir',type=Path);a=p.parse_args();assert store_marker()==29;trace=[];deaths=0
    if a.evidence_dir:a.evidence_dir.mkdir(parents=True,exist_ok=False)
    with tempfile.TemporaryDirectory(prefix='weave-causal-dispatch-')as temporary:
        root=Path(temporary);generated=root/'genuine-store28';old_report=root/'genuine-store28.json'
        command=['python3',str(a.old_disposition_controller.resolve()),'--old-controller',str(a.old_lifecycle_controller.resolve()),'--old26-actor',str(a.old26_actor.resolve()),'--old-actor',str(a.old27_actor.resolve()),'--actor',str(a.old28_actor.resolve()),'--report',str(old_report),'--evidence-dir',str(generated)]
        r=subprocess.run(command,capture_output=True,timeout=180);assert r.returncode==0,(r.stdout,r.stderr)
        history=json.loads(old_report.read_text());assert(history['processes'],history['controlled_deaths'],history['genuine_store27_processes'],history['genuine_store27_deaths'])==(44,7,88,12)
        if a.evidence_dir:
            shutil.copytree(generated,a.evidence_dir/'genuine-store28');shutil.copy2(old_report,a.evidence_dir/old_report.name)
            (a.evidence_dir/'genuine-store28-driver.stdout').write_bytes(r.stdout);(a.evidence_dir/'genuine-store28-driver.stderr').write_bytes(r.stderr)
        for path in generated.glob('*.sqlite'):shutil.copy2(path,root/path.name)
        db=root/'store.sqlite';request_file=root/'request.json'
        def rows(path,table):
            with closing(sqlite3.connect(path))as c:return list(c.execute('SELECT * FROM '+table+' ORDER BY rowid'))
        sidecars={path.name:hashlib.sha256(path.read_bytes()).hexdigest()for path in root.glob('*.sqlite')if path!=db}
        def invoke(binary,mode,code=0,**fields):
            nonlocal deaths
            request_file.write_text(json.dumps(fields));r=subprocess.run([str(binary.resolve()),str(db),mode,str(request_file)],capture_output=True,timeout=30)
            entry={'binary':binary.name,'operation':mode,'exit':r.returncode,'stdout_sha256':hashlib.sha256(r.stdout).hexdigest()};trace.append(entry)
            if a.evidence_dir:
                stem=f'{len(trace):03d}-{binary.name}-{mode}';(a.evidence_dir/(stem+'.stdout')).write_bytes(r.stdout);(a.evidence_dir/(stem+'.stderr')).write_bytes(r.stderr);(a.evidence_dir/(stem+'.request.json')).write_bytes(request_file.read_bytes())
            assert r.returncode==code,(entry,r.stdout,r.stderr)
            if code in [95,96,110,111,112,113]:deaths+=1
            return json.loads(r.stdout)if code==0 else r.stderr
        def host(mode,**fields):return invoke(a.host,mode,**fields)
        before=snapshot(db);assert before['marker']==28 and before['tables']['recorded_actor_cancellations']['rows']
        host('open',crash='before-schema',code=95);assert snapshot(db)==before
        host('open',crash='after-schema',code=96);after=snapshot(db);assert after['marker']==29 and set(after['tables'])==set(before['tables'])|{'event_causation','dispatch_causal_policies','dispatch_circuits'}
        assert {n:after['tables'][n]for n in before['tables']}==before['tables']
        with closing(sqlite3.connect(db))as c:
            assert c.execute('SELECT count(*) FROM event_causation').fetchone()[0]==c.execute('SELECT count(*) FROM events').fetchone()[0]
            assert c.execute('SELECT count(*) FROM dispatch_causal_policies').fetchone()[0]==c.execute('SELECT count(*) FROM dispatch_adapters').fetchone()[0]
            assert c.execute('SELECT count(*) FROM dispatch_circuits').fetchone()[0]==0
            for (body,)in c.execute('SELECT body FROM event_causation'):
                record=json.loads(body);assert record['origin']=='legacy_boundary'and record['depth']==0 and record['parent']is None and record['root']==record['event']
        error=invoke(a.old28_actor,'open',code=1);assert b'E_STORAGE_VERSION'in error and snapshot(db)==after
        host('input',graph='loop-a',value=0);host('install',id='loop-A',depth=4);host('install',id='loop-B',depth=4)
        lag=host('lag',id='loop-A');assert lag['visible_backlog_lower_bound']==1 and not lag['backlog_truncated'] and not lag['circuit_open']
        host('input',graph='loop-private',value=999,private=True);assert host('lag',id='loop-A')==lag
        event=host('poll',id='loop-A');program=host('recipe',graph='loop-b',value=1);stable=snapshot(db)
        host('complete',id='loop-A',event=event,program=program,crash='before-completion',code=110);assert snapshot(db)==stable
        host('complete',id='loop-A',event=event,program=program,crash='after-completion',code=111);committed=snapshot(db);first=(event,program)
        assert host('complete',id='loop-A',event=event,program=program)['duplicate']and snapshot(db)==committed
        for value in [2,3,4]:
            adapter,graph=('loop-B','loop-a')if value%2==0 else('loop-A','loop-b')
            event=host('poll',id=adapter);program=host('recipe',graph=graph,value=value);assert not host('complete',id=adapter,event=event,program=program)['duplicate']
        with closing(sqlite3.connect(db))as c:
            body=c.execute('SELECT body FROM event_causation ORDER BY rowid DESC LIMIT 1').fetchone()[0];lineage=json.loads(body);assert lineage['depth']==4 and lineage['parent']and lineage['adapter']=='loop-B'
            frontier=c.execute("SELECT checkpoint FROM dispatch_adapters WHERE id='loop-A'").fetchone()[0];blocked=c.execute('SELECT sequence FROM events WHERE event_id=?',(lineage['event'],)).fetchone()[0]
        stable=snapshot(db);host('poll',id='loop-A',crash='before-circuit',code=112);assert snapshot(db)==stable
        host('poll',id='loop-A',crash='after-circuit',code=113);status=host('lag',id='loop-A');assert status['circuit_open']and status['lifecycle']=='paused'and status['pending']is None and status['visible_backlog_lower_bound']==1
        assert not({'checkpoint','sequence','root','parent','event','lease'}&set(status))
        with closing(sqlite3.connect(db))as c:
            assert frontier<=c.execute("SELECT checkpoint FROM dispatch_adapters WHERE id='loop-A'").fetchone()[0]<blocked
        host('lifecycle',id='loop-A',state='running');stable=snapshot(db)
        assert host('complete',id='loop-A',event=first[0],program=first[1])['duplicate']and snapshot(db)==stable
        error=host('poll',id='loop-A',code=1);assert b'E_CIRCUIT_OPEN'in error
        host('policy',id='loop-A',depth=5);host('lifecycle',id='loop-A',state='running');event=host('poll',id='loop-A');program=host('recipe',graph='loop-b',value=5);stable=snapshot(db)
        host('complete',id='loop-A',event=event,program=program,crash='before-completion',code=110);assert snapshot(db)==stable
        host('complete',id='loop-A',event=event,program=program,crash='after-completion',code=111);committed=snapshot(db)
        assert host('complete',id='loop-A',event=event,program=program)['duplicate']and snapshot(db)==committed
        stable=snapshot(db);host('poll',id='loop-B',crash='before-circuit',code=112);assert snapshot(db)==stable
        host('poll',id='loop-B',crash='after-circuit',code=113);assert host('lag',id='loop-B')['circuit_open']
        host('policy',id='loop-B',depth=6);host('lifecycle',id='loop-B',state='running');event=host('poll',id='loop-B');program=host('recipe',graph='loop-a',value=4)
        count=len(rows(db,'events'));receipt=host('complete',id='loop-B',event=event,program=program);assert receipt['results'][0]['kind']=='unchanged'and len(rows(db,'events'))==count
        stable=snapshot(db);assert host('complete',id='loop-B',event=event,program=program)['duplicate']and snapshot(db)==stable
        host('compact');final=snapshot(db)
        for table in ['recorded_actor_definitions','recorded_actor_states','recorded_actor_receipts','recorded_actor_migrations','recorded_actor_observations','recorded_actor_cancellations','effect_intents']:
            assert final['tables'][table]['rows']==before['tables'][table]['rows']
        for name,digest in sidecars.items():assert hashlib.sha256((root/name).read_bytes()).hexdigest()==digest
        if a.evidence_dir:
            for path in root.glob('*.sqlite'):shutil.copy2(path,a.evidence_dir/path.name)
    report={'profile':'native-causal-dispatch/1','store_marker':29,'result':'passed','processes':len(trace),'controlled_deaths':deaths,'genuine_store28_processes':44,'genuine_store28_deaths':7,'genuine_store27_processes':88,'genuine_store27_deaths':12,'checks':['genuine old28 actor/effect/version/observation/cancellation journals preserved across interrupted modern upgrade','actual kernel-bound two-adapter parent/root/depth lineage commits with output/receipt/checkpoint','pre/post completion deaths and stale retries neither lose nor duplicate child events','private/unsubscribed occurrences do not change owner lag counts or expose global offsets','pre/post circuit deaths preserve suspension without acknowledging blocked source','explicit drained owner policy change permits another bounded step and the other adapter suspends','actual no-op and duplicate completion emit no child occurrence','typed collection preserves actual old actor/effect journals and every independent tool/sink sidecar'],'limits':['trusted native local graph-handler causal profile; old history is a boundary without invented former ancestry','separate governance/mount/remote taxonomies, source/portable commands, CPU/RSS isolation and all original assurance remain required','owner-scoped lag is bounded and reports visible lower bounds; no public global offset']}
    if a.report:a.report.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report))
if __name__=='__main__':main()
