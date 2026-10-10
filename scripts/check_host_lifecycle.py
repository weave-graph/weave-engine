#!/usr/bin/env python3
"""Actual C request2 recovery with genuine actor history and independent tool/sink journals."""
import argparse,ctypes,hashlib,json,os,shutil,sqlite3,subprocess,sys,tempfile,time
from contextlib import closing
from pathlib import Path
from check_compiled_lifecycle import snapshot

def encode(value):return json.dumps(value,separators=(',',':'),ensure_ascii=False).encode()
def child(path):
    a=json.loads(path.read_text());library=ctypes.CDLL(a['library'])
    free=library.weave_native_free;free.argtypes=[ctypes.c_void_p];free.restype=None
    def call(name,*parts):
        f=getattr(library,'weave_host_'+name);f.argtypes=[item for _ in parts for item in (ctypes.c_void_p,ctypes.c_size_t)];f.restype=ctypes.c_void_p
        buffers=[ctypes.create_string_buffer(part)for part in parts];args=[value for buffer,part in zip(buffers,parts)for value in (ctypes.cast(buffer,ctypes.c_void_p),len(part))]
        pointer=f(*args);assert pointer
        try:return ctypes.string_at(pointer)
        finally:free(pointer)
    opened=json.loads(call('open',a['database'].encode(),encode(a['authority'])));assert opened['ok'],opened
    token=opened['value']['handle'].encode()
    if a.get('crash')=='before-call':os._exit(114)
    raw=call(a.get('entry','call'),token,encode(a['request']));reply=json.loads(raw)
    if a.get('crash')=='after-call':
        assert reply['ok'],reply
        os._exit(115)
    assert json.loads(call('close',token))['ok']
    sys.stdout.buffer.write(raw)

def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ['library','actor','old-disposition-controller','old-lifecycle-controller','old26-actor','old27-actor','old28-actor']:p.add_argument('--'+name,type=Path,required=True)
    p.add_argument('--report',type=Path);p.add_argument('--evidence-dir',type=Path);a=p.parse_args();trace=[];deaths=0
    if a.evidence_dir:a.evidence_dir.mkdir(parents=True,exist_ok=False)
    with tempfile.TemporaryDirectory(prefix='weave-host-lifecycle-')as temporary:
        root=Path(temporary);generated=root/'genuine-store28';old_report=root/'genuine-store28.json'
        cmd=['python3',str(a.old_disposition_controller.resolve()),'--old-controller',str(a.old_lifecycle_controller.resolve()),'--old26-actor',str(a.old26_actor.resolve()),'--old-actor',str(a.old27_actor.resolve()),'--actor',str(a.old28_actor.resolve()),'--report',str(old_report),'--evidence-dir',str(generated)]
        result=subprocess.run(cmd,capture_output=True,timeout=180);assert result.returncode==0,(result.stdout,result.stderr)
        old=json.loads(old_report.read_text());assert(old['processes'],old['controlled_deaths'],old['genuine_store27_processes'],old['genuine_store27_deaths'])==(44,7,88,12)
        if a.evidence_dir:
            shutil.copytree(generated,a.evidence_dir/'genuine-store28');shutil.copy2(old_report,a.evidence_dir/old_report.name)
            (a.evidence_dir/'genuine-store28-driver.stdout').write_bytes(result.stdout);(a.evidence_dir/'genuine-store28-driver.stderr').write_bytes(result.stderr)
        for path in generated.glob('*.sqlite'):shutil.copy2(path,root/path.name)
        db=root/'store.sqlite';request_file=root/'request.json';invocation=root/'invocation.json'
        def rows(path,table):
            with closing(sqlite3.connect(path))as c:return list(c.execute('SELECT * FROM '+table+' ORDER BY rowid'))
        before=snapshot(db);original_sidecars={path.name:hashlib.sha256(path.read_bytes()).hexdigest()for path in root.glob('*.sqlite')if path!=db and '.lifecycle.'not in path.name}
        tool=root/'store.lifecycle.tools.sqlite';sink=root/'store.lifecycle.sink.sqlite';old_tools=rows(tool,'tool_runs');old_receipts=rows(sink,'physical_receipts');assert len(old_tools)==6 and len(old_receipts)==4
        def execute(cmd,request,label,code):
            nonlocal deaths
            result=subprocess.run(cmd,capture_output=True,timeout=40);trace.append({'operation':label,'exit':result.returncode,'stdout_sha256':hashlib.sha256(result.stdout).hexdigest()})
            if a.evidence_dir:
                stem=f'{len(trace):03d}-{label}';(a.evidence_dir/(stem+'.request.json')).write_bytes(encode(request));(a.evidence_dir/(stem+'.stdout')).write_bytes(result.stdout);(a.evidence_dir/(stem+'.stderr')).write_bytes(result.stderr)
            assert result.returncode==code,(label,result.returncode,result.stdout,result.stderr)
            if code in [99,101,114,115]:deaths+=1
            return json.loads(result.stdout)if code==0 else result.stderr
        def ccall(operation=None,request=None,entry='call',crash=None,principal='owner',graphs=None,ok=True,code=0,format='weave-host-request/2'):
            payload=request if request is not None else {'format':format,'operation':operation}
            invocation.write_bytes(encode({'library':str(a.library.resolve()),'database':str(db),'authority':{'principal':principal,'writable_graphs':graphs if graphs is not None else ['actor-input','actor-output','actor-other']},'request':payload,'entry':entry,'crash':crash}))
            label='c-'+(operation['kind']if operation else entry)+(('-'+crash)if crash else '')
            reply=execute([sys.executable,str(Path(__file__).resolve()),'--child',str(invocation)],json.loads(invocation.read_text()),label,code)
            if code:return reply
            assert reply['ok']==ok,reply
            assert reply['poisoned']is False
            if entry=='call'and ok:assert reply['requires_fence']is True
            return reply['value']if ok else reply['error']
        def actor(mode,id='restored',version=1,code=0,**fields):
            request={'id':id,'version':version,'clock':int(time.time()*1000),**fields};request_file.write_bytes(encode(request))
            return execute([str(a.actor.resolve()),str(db),mode,str(request_file)],request,'native-'+mode,code)
        capabilities=ccall({'kind':'capabilities'});assert capabilities['store_marker']==29 and len(capabilities['operations'])==31
        after=snapshot(db);assert after['marker']==29 and set(after['tables'])==set(before['tables'])|{'event_causation','dispatch_causal_policies','dispatch_circuits'}
        assert {n:after['tables'][n]for n in before['tables']}==before['tables']
        state=ccall({'kind':'actor_state','adapter':'restored'})
        assert ccall({'kind':'actor_state','adapter':'restored'},principal='foreign',ok=False)['code']=='E_HOST_AUTH'
        assert ccall({'kind':'lag','adapter':'restored'},graphs=['actor-input'],ok=False)['code']=='E_HOST_AUTH'
        assert ccall({'kind':'lifecycle','adapter':'restored','state':'paused'},format='weave-host-request/1',ok=False)['code']=='E_HOST_VERSION'
        fresh=actor('definition',id='installed-by-host');ccall(request=fresh,entry='install_actor')
        assert ccall({'kind':'install_recorded_actor','definition':fresh},ok=False)['code']=='E_HOST_INPUT'
        actor('input',value=8);event=ccall({'kind':'poll','adapter':'restored'})
        assert ccall({'kind':'actor_delivery_mode','adapter':'restored','event':event['id'],'lease':event['lease']})=='compute'
        ccall({'kind':'actor_run_inputs','adapter':'restored','event':event['id'],'lease':event['lease']})
        actor('prepare',event=event,crash='after-tool-journal',code=99);assert len(rows(tool,'tool_runs'))==7
        request=actor('prepare',event=event);assert len(rows(tool,'tool_runs'))==7
        effect=actor('effect',event=event,payload={'action':8});actor('dispatch',intent=effect['id'],crash='after-physical',code=101);assert len(rows(sink,'physical_receipts'))==5
        cancel={'adapter':'restored','event':event['id'],'expected_lease':event['lease'],'nonce':'unknown-dispose','reason':'owner_stop'}
        assert ccall({'kind':'actor_cancel','request':cancel},ok=False)['code']=='E_EFFECT_UNKNOWN'
        assert ccall({'kind':'reconcile_effect','intent':effect['id'],'state':'confirmed','response':{'invented':True}},ok=False)['code']=='E_HOST_INPUT'
        assert ccall({'kind':'lag','adapter':'restored'})['visible_unknown_effects']==1
        assert ccall({'kind':'actor_complete','request':request},ok=False)['code']=='E_ACTOR_EFFECT_PENDING'
        assert ccall({'kind':'actor_state','adapter':'restored'})==state
        actor('reconcile',intent=effect['id'])
        ccall({'kind':'actor_complete','request':request},crash='before-call',code=114)
        assert ccall({'kind':'actor_state','adapter':'restored'})==state
        ccall({'kind':'actor_complete','request':request},crash='after-call',code=115)
        receipt=ccall({'kind':'actor_complete','request':request});assert receipt['handler']['duplicate']
        assert len(rows(tool,'tool_runs'))==7 and len(rows(sink,'physical_receipts'))==5
        ccall({'kind':'lifecycle','adapter':'restored','state':'paused'})
        inputs=ccall({'kind':'actor_migration_inputs','adapter':'restored'});destination=actor('definition',id='host-v2',version=2)
        transfer={'inputs':inputs,'destination':destination,'nonce':'host-upgrade','disposition':{'kind':'upgrade'}}
        ccall({'kind':'actor_migrate','request':transfer},crash='after-call',code=115);assert ccall({'kind':'actor_migrate','request':transfer})['duplicate']
        ccall({'kind':'lifecycle','adapter':'host-v2','state':'running'});actor('input',value=9);second=ccall({'kind':'poll','adapter':'host-v2'})
        request2=actor('prepare',id='host-v2',version=2,event=second);ccall({'kind':'actor_complete','request':request2});assert len(rows(tool,'tool_runs'))==8
        ccall({'kind':'lifecycle','adapter':'host-v2','state':'paused'});inputs=ccall({'kind':'actor_migration_inputs','adapter':'host-v2'});restored=actor('definition',id='host-restored')
        transfer={'inputs':inputs,'destination':restored,'nonce':'host-rollback','disposition':{'kind':'rollback','restore_from':'restored'}}
        rollback=ccall({'kind':'actor_migrate','request':transfer});ccall({'kind':'lifecycle','adapter':'host-restored','state':'running'});history=ccall({'kind':'poll','adapter':'host-restored'})
        assert ccall({'kind':'actor_delivery_mode','adapter':'host-restored','event':history['id'],'lease':history['lease']})=='observe'
        assert ccall({'kind':'actor_run_inputs','adapter':'host-restored','event':history['id'],'lease':history['lease']},ok=False)['code']=='E_ACTOR_REPLAY'
        observation={'adapter':'host-restored','event':history['id'],'lease':history['lease'],'prior_state_digest':rollback['state_digest'],'nonce':'host-observe'}
        ccall({'kind':'actor_observe','request':observation},crash='after-call',code=115);ccall({'kind':'actor_observe','request':observation})
        ccall({'kind':'actor_observation','adapter':'host-restored','event':history['id']});assert len(rows(tool,'tool_runs'))==8 and len(rows(sink,'physical_receipts'))==5
        actor('input',value=10);pending=ccall({'kind':'poll','adapter':'host-restored'});actor('prepare',id='host-restored',event=pending);undispatched=actor('effect',id='host-restored',event=pending,payload={'action':10})
        cancel={'adapter':'host-restored','event':pending['id'],'expected_lease':pending['lease'],'nonce':'host-dispose','reason':'owner_stop'}
        ccall({'kind':'actor_cancel','request':cancel},crash='after-call',code=115);assert ccall({'kind':'actor_cancel','request':cancel})['duplicate']
        with closing(sqlite3.connect(db))as c:assert c.execute('SELECT state FROM effect_intents WHERE id=?',(undispatched['id'],)).fetchone()[0]=='failed'
        assert len(rows(sink,'physical_receipts'))==5
        inputs=ccall({'kind':'actor_inputs','adapter':'host-restored'});initialize={'inputs':inputs,'state_revision':'host-initialize','state':{'owner_chosen':10}}
        ccall({'kind':'actor_bootstrap','request':initialize},crash='after-call',code=115)
        # Bootstrap has current-state CAS; uncertain initialization is inspected, not blindly replayed.
        current=ccall({'kind':'actor_state','adapter':'host-restored'});assert current['state']=={'owner_chosen':10}
        ccall({'kind':'lifecycle','adapter':'host-restored','state':'running'});actor('input',value=11);next_event=ccall({'kind':'poll','adapter':'host-restored'});next_request=actor('prepare',id='host-restored',event=next_event);ccall({'kind':'actor_complete','request':next_request})
        current=ccall({'kind':'actor_state','adapter':'host-restored'});assert ccall({'kind':'actor_cancel','request':cancel})['duplicate'];assert ccall({'kind':'actor_state','adapter':'host-restored'})==current
        actor('compact',policy={'history_before_ms':0,'replay_through_sequence':0});final=snapshot(db)
        for table in ['recorded_actor_definitions','recorded_actor_receipts','recorded_actor_migrations','recorded_actor_observations','recorded_actor_cancellations','effect_intents']:
            assert all(row in final['tables'][table]['rows']for row in before['tables'][table]['rows']),table
        assert all(row in rows(tool,'tool_runs')for row in old_tools)and all(row in rows(sink,'physical_receipts')for row in old_receipts)
        assert len(rows(tool,'tool_runs'))==10 and len(rows(sink,'physical_receipts'))==5
        for name,digest in original_sidecars.items():assert hashlib.sha256((root/name).read_bytes()).hexdigest()==digest
        if a.evidence_dir:
            for path in root.glob('*.sqlite'):shutil.copy2(path,a.evidence_dir/path.name)
    report={'profile':'native-host-lifecycle/2','store_marker':29,'result':'passed','processes':len(trace),'controlled_deaths':deaths,'genuine_store28_processes':44,'genuine_store28_deaths':7,'old_native_tool_runs':6,'native_tool_runs':10,'old_physical_receipts':4,'physical_receipts':5,'native_library_sha256':hashlib.sha256(a.library.read_bytes()).hexdigest(),'trace':trace,'checks':['strict request2 capability negotiation and fixed current owner/output authority','genuine old28 actor/effect/cancellation/version/observation histories preserved across C-owned migration','actual native journal lost response and physical lost acknowledgment preserve Unknown until trusted reconciliation','actual C actor completion/version transfer/rollback/default observation/cancellation/init survive response loss and process restart','actual original artifacts/tool outcomes and immutable receipts remain historical; ten native tool runs and five physical receipts','Pending cancellation never dispatches a physical action and old retries do not rewind later work','initial actor installation remains separate trusted C configuration; operational reconciliation/installation rejected','typed collection preserves original journals and independent store26 sidecars'],'limits':['trusted local native embedding and native tool execution; no model truth or CPU/RSS sandbox claim','C deaths bracket invocation/response production; exact kernel precommit deaths remain covered by prior core controllers','bootstrap uncertainty is inspected against current state CAS, not automatically retried','complete source operation compilation and actual browser/mobile application integration remain required','all original formal/cryptographic/quality/network/platform/public-release requirements remain active']}
    if a.report:a.report.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report))

if __name__=='__main__':
    if len(sys.argv)==3 and sys.argv[1]=='--child':child(Path(sys.argv[2]))
    else:main()
