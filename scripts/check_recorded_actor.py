#!/usr/bin/env python3
"""Actual native actor/tool/sink journals, process deaths and populated store25 upgrade."""
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
from retention_migration import RECORDED_ACTOR_TABLES
from version_profile import store_marker


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ['compiler','old-handler','handler','actor']:p.add_argument('--'+name,type=Path,required=True)
    p.add_argument('--report',type=Path);p.add_argument('--evidence-dir',type=Path)
    a=p.parse_args();trace=[];deaths=0
    assert store_marker()==26
    if a.evidence_dir:a.evidence_dir.mkdir(parents=True,exist_ok=False)
    with tempfile.TemporaryDirectory(prefix='weave-recorded-actor-') as temporary:
        root=Path(temporary);db=root/'store.sqlite';request=root/'request.json'
        def invoke(binary,*args,code=0):
            nonlocal deaths
            r=subprocess.run([str(binary.resolve()),*map(str,args)],capture_output=True,timeout=30)
            entry={'binary':binary.name,'args':[Path(x).name if isinstance(x,Path) else str(x) for x in args],'exit':r.returncode,'stdout_sha256':hashlib.sha256(r.stdout).hexdigest()};trace.append(entry)
            if a.evidence_dir:
                stem=f'{len(trace):03d}-{binary.name}';(a.evidence_dir/(stem+'.stdout')).write_bytes(r.stdout);(a.evidence_dir/(stem+'.stderr')).write_bytes(r.stderr)
            assert r.returncode==code,(entry,r.stdout,r.stderr)
            if 95<=code<=103:deaths+=1
            return json.loads(r.stdout) if code==0 else r.stderr
        def compiled(binary,mode,code=0,**fields):
            request.write_text(json.dumps({'mode':mode,**fields}));return invoke(binary,db,request,code=code)
        def actor(mode,code=0,**fields):
            request.write_text(json.dumps(fields));return invoke(a.actor,db,mode,request,code=code)
        def write_old(graph,value,now):
            head=compiled(a.old_handler,'head',graph=graph,now=now)['head']
            compiled(a.old_handler,'run',now=now,program={'version':'0.21.0','commands':[{'op':'commit','graph_id':graph,'expected_head':head,'data':{'nodes':[{'id':'n','entity_id':'e','space_id':'s','readers':['collector'],'properties':{'value':value}}]}}]})
        source=root/'actual-handler.weave';source.write_text('''function Keep revision "1" (graph input) { return input; }
handler KeepActual revision "1" using Keep {
 input event graph "Installation" branch "main" metadata depth 0;
 on "graph.committed", "graph.accepted";
 output slot "result";
 replay pinned;
}
''')
        artifact=invoke(a.compiler,'handler-plan',source,'--handler','KeepActual');assert artifact['protocol']=='0.21.0'
        (root/'actual-artifact.json').write_text(json.dumps(artifact,indent=2)+'\n')
        write_old('Installation',1,10);compiled(a.old_handler,'install',id='old25',template=artifact,output='Quality',now=10)
        event=compiled(a.old_handler,'poll',id='old25',now=10)
        prepared=compiled(a.old_handler,'prepare',id='old25',event=event['id'],lease=event['lease'],now=10)
        old_completion={'id':'old25','event':event['id'],'lease':event['lease'],'preparation':prepared['preparation_id'],'now':10}
        old_receipt=compiled(a.old_handler,'complete',**old_completion)
        write_old('Installation',2,20);compiled(a.old_handler,'state',id='old25',state='paused',now=20)
        reconstruction={'inputs':compiled(a.old_handler,'rebuild_inputs',id='old25',now=20),'nonce':'old-reconstruction'}
        old_rebuild=compiled(a.old_handler,'rebuild',request=reconstruction,now=20)
        before=snapshot(db);assert before['marker']==25
        assert len(before['tables']['compiled_rebuild_receipts']['rows'])==1
        assert len(before['tables']['compiled_replay_states']['rows'])==1
        actor('open',crash='before-schema',code=95);assert snapshot(db)==before
        actor('open',crash='after-schema',code=96);after=snapshot(db)
        assert after['marker']==26 and set(after['tables'])==set(before['tables'])|RECORDED_ACTOR_TABLES
        assert {n:after['tables'][n] for n in before['tables']}==before['tables']
        assert all(after['tables'][n]['rows']==[] for n in RECORDED_ACTOR_TABLES)
        refused=compiled(a.old_handler,'head',graph='Quality',code=1);assert b'E_STORAGE_VERSION' in refused and snapshot(db)==after
        replay=compiled(a.handler,'rebuild',request=reconstruction,now=20);assert replay['duplicate'] and {**replay,'duplicate':False}==old_rebuild and snapshot(db)==after
        compiled(a.handler,'state',id='old25',state='running',now=20)
        assert compiled(a.handler,'complete',**old_completion)=={**old_receipt,'duplicate':True}
        actor('seed',clock=100)
        def boot(clock=100,state=None):
            return {'inputs':actor('inputs',clock=clock),'state_revision':f'initial-{clock}','state':state or {'total':1}}
        initial=boot();prior=snapshot(db)
        actor('bootstrap',request=initial,clock=100,crash='before-bootstrap',code=97);assert snapshot(db)==prior
        actor('bootstrap',request=initial,clock=100,crash='after-bootstrap',code=98);installed=snapshot(db)
        assert len(installed['tables']['recorded_actor_states']['rows'])==1
        stale=actor('bootstrap',request=initial,clock=100,code=1);assert b'E_CONFLICT' in stale and snapshot(db)==installed
        actor('lifecycle',state='running',clock=100)
        actor('input',value=2,clock=200);delivery=actor('poll',clock=200);assert delivery
        actor('prepare',event=delivery,clock=200,crash='after-tool-journal',code=99)
        tools=db.with_suffix('.tools.sqlite')
        sink=db.with_suffix('.sink.sqlite')
        def tool_rows():
            with closing(sqlite3.connect(tools)) as c:return c.execute('SELECT * FROM tool_runs ORDER BY event').fetchall()
        assert len(tool_rows())==1
        completion=actor('prepare',event=delivery,clock=200);assert len(tool_rows())==1
        unchanged_tools=tool_rows();assert completion['tool_results'][0]['value']['sample']==unchanged_tools[0][3]
        effect=actor('effect',event=delivery,payload={'request':'before-io'},clock=200)
        refused=actor('complete',request=completion,clock=200,code=1);assert b'E_ACTOR_EFFECT_PENDING' in refused
        actor('dispatch',intent=effect['id'],clock=200,crash='after-unknown',code=100)
        stable=snapshot(db);refused=actor('dispatch',intent=effect['id'],clock=200,code=1);assert b'E_EFFECT_UNKNOWN' in refused and snapshot(db)==stable
        assert not sink.exists()
        actor('fail_absent',intent=effect['id'],clock=200)
        prior=snapshot(db);actor('complete',request=completion,clock=200,crash='before-complete',code=102);assert snapshot(db)==prior
        actor('complete',request=completion,clock=200,crash='after-complete',code=103);committed=snapshot(db)
        first=actor('complete',request=completion,clock=200);assert first['handler']['duplicate'] and first['effects'][0]['state']=='failed' and snapshot(db)==committed
        assert tool_rows()==unchanged_tools
        state=actor('state',clock=200);assert state['state']==completion['state'];assert state['artifacts']==first['artifacts']
        public_inputs=actor('inputs',clock=200);state_before=actor('state',clock=200)
        with closing(sqlite3.connect(db)) as c:cp=c.execute("SELECT checkpoint FROM dispatch_adapters WHERE id='recorded-actor'").fetchone()[0]
        actor('other',value='private-unsubscribed',clock=210);assert actor('poll',clock=210) is None
        assert actor('inputs',clock=210)==public_inputs and actor('state',clock=210)==state_before
        with closing(sqlite3.connect(db)) as c:
            cp2=c.execute("SELECT checkpoint FROM dispatch_adapters WHERE id='recorded-actor'").fetchone()[0]
            assert cp2>cp and c.execute("SELECT checkpoint FROM recorded_actor_states WHERE adapter='recorded-actor'").fetchone()[0]==cp2
        actor('input',value=3,clock=300);delivery2=actor('poll',clock=300);completion2=actor('prepare',event=delivery2,clock=300)
        assert len(tool_rows())==2
        effect2=actor('effect',event=delivery2,payload={'request':'physical-once'},clock=300)
        actor('dispatch',intent=effect2['id'],clock=300,crash='after-physical',code=101)
        with closing(sqlite3.connect(sink)) as c:physical_before=c.execute('SELECT * FROM physical_receipts').fetchall();assert len(physical_before)==1
        stable=snapshot(db);refused=actor('complete',request=completion2,clock=300,code=1);assert b'E_ACTOR_EFFECT_PENDING' in refused and snapshot(db)==stable
        refused=actor('dispatch',intent=effect2['id'],clock=300,code=1);assert b'E_EFFECT_UNKNOWN' in refused
        actor('reconcile',intent=effect2['id'],clock=300)
        second=actor('complete',request=completion2,clock=300);assert not second['handler']['duplicate'] and second['effects'][0]['state']=='confirmed'
        expected_state=actor('state',clock=300);expected_output=actor('query',clock=300);expected_db=snapshot(db)
        first_retry=actor('complete',request=completion,clock=300);second_retry=actor('complete',request=completion2,clock=300)
        assert first_retry==first and second_retry=={**second,'handler':{**second['handler'],'duplicate':True}}
        assert actor('state',clock=300)==expected_state and actor('query',clock=300)==expected_output and snapshot(db)==expected_db
        assert len(tool_rows())==2
        with closing(sqlite3.connect(sink)) as c:assert c.execute('SELECT * FROM physical_receipts').fetchall()==physical_before
        actor('lifecycle',state='paused',clock=400)
        actor('compact',policy={'history_before_ms':250,'replay_through_sequence':3},clock=400)
        actor('lifecycle',state='running',clock=400);refused=actor('poll',clock=400,code=1);assert b'E_CHECKPOINT_EXPIRED' in refused
        actor('lifecycle',state='paused',clock=400);reconstructed=boot(400,expected_state['state']);actor('bootstrap',request=reconstructed,clock=400)
        actor('lifecycle',state='running',clock=400);assert actor('poll',clock=400) is None
        actor('input',value=4,clock=500);delivery3=actor('poll',clock=500);completion3=actor('prepare',event=delivery3,clock=500)
        third=actor('complete',request=completion3,clock=500);assert not third['handler']['duplicate'] and len(tool_rows())==3
        final_state=actor('state',clock=500);assert final_state['state']['input']==4
        if a.evidence_dir:
            for f in root.iterdir():
                if f.is_file():shutil.copyfile(f,a.evidence_dir/f.name)
            for name,value in [('old-before',before),('old-after',after),('first-committed',committed),('later-completed',expected_db),('final',snapshot(db))]:
                (a.evidence_dir/(name+'.json')).write_text(json.dumps(value,indent=2)+'\n')
        report={'profile':'native-recorded-actor-journals/1','store_marker':26,'processes':len(trace),'controlled_deaths':deaths,'tool_runs':3,'physical_receipts':1,'checks':['actual source-compiled old25 completion and snapshot reconstruction state preserved byte for byte','schema/bootstrap/actor-completion pre/post commit deaths','actual native nondeterministic tool outputs in an independent durable host journal','unknown before I/O has verified reference sink absence and never retries automatically','physical sink commit with lost acknowledgment reconciles from actual destination evidence','recorded terminal effect and tool artifacts commit with outputs/state/private checkpoint','exact historical retries never rerun tools, repeat physical actions or rewind later state','private/nonmatching event scans preserve public state and input identities','retention epoch requires explicit reconstruction before new delivery','later new computation remains possible after reconstruction'],'trace':trace,'result':'passed','limits':['trusted native actor/host journal with fixed owner and local SQLite idempotent sink','actual fixture source bytes are stored and hashed; hashing is not execution attestation','no arbitrary remote exactly-once, model quality, CPU/RSS isolation or portable actor claim','actor artifact upgrade/rollback, source bindings and broader original assurance remain required']}
        encoded=json.dumps(report,indent=2)+'\n'
        if a.report:a.report.write_text(encoded)
        print(encoded)


if __name__=='__main__':main()
