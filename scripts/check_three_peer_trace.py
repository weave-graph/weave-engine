#!/usr/bin/env python3
"""Existing-API native P/W/T trace. Export and recipe execution are TRUSTED HOST operations.
No build, network service, new crypto format or production credentials are created.
"""
import argparse
import copy
import hashlib
import json
import subprocess
import tempfile
import sqlite3
import time
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument('--probe', type=Path, default=Path('target/debug/examples/three_peer_trace'))
p.add_argument('--report', type=Path)
p.add_argument('--signed-export', action='store_true')
a = p.parse_args()
trace = []
checks = []
transferred = 0
with tempfile.TemporaryDirectory(prefix='weave-three-peers-') as tmp:
    root = Path(tmp)
    dbs = {peer: root / f'{peer}.sqlite' for peer in 'PWT'}
    def call(peer, op, expect=0, **data):
        request = {'op':op, **data}
        path = root / 'request.json'
        path.write_text(json.dumps(request))
        started = time.monotonic()
        r = subprocess.run([str(a.probe.resolve()),str(dbs[peer]),peer,str(path)],capture_output=True,text=True,timeout=30)
        elapsed = time.monotonic()-started
        assert r.returncode == expect, (peer,op,r.returncode,r.stdout[-1000:],r.stderr[-3000:])
        text = r.stdout if expect == 0 else r.stderr
        value = json.loads(text) if text.strip() else None
        trace.append({'peer':peer,'operation':op,'exit':r.returncode,'seconds':round(elapsed,6),'stdout_bytes':len(r.stdout.encode()),'response_sha256':hashlib.sha256(text.encode()).hexdigest()})
        return value
    info = {peer: call(peer,'init') for peer in 'PWT'}
    owner = info['P']['owner']; version=info['P']['protocol']
    def plan(*commands): return {'version':version,'commands':list(commands)}
    def head(peer,g,b='main'): return call(peer,'head',graph=g,branch=b)['revision']
    def ref(g,r): return {'graph_id':g,'revision':r}
    def events(peer): return call(peer,'counts')['events']
    def query(peer,g,r,**extra): return call(peer,'query',query={'graph_id':g,'revision':r,**extra})
    def value(peer,expression):
        return call(peer,'execute',program=plan({'op':'evaluate','value':expression}))[0]['result']
    def commit(g,data,b='main',expected=None):return {'op':'commit','graph_id':g,'branch_id':b,'expected_head':expected,'data':data}
    def scope(data):
        data=copy.deepcopy(data)
        for key in ['nodes','edges','structural_edges','assertions','attachments']:
            for obj in data.get(key,[]): obj['readers']=[owner]
        return data
    def export(peer,reference,recipient,branch='main'):
        global transferred
        if a.signed_export:
            nonce=f"export-{reference['graph_id']}-{reference['revision']}-{recipient}"
            kwargs={'reference':reference,'recipient':recipient,'branch':branch,'nonce':nonce}
            if transferred == 0:
                def receipts():
                    with sqlite3.connect(dbs[peer]) as conn:
                        return conn.execute('SELECT COUNT(*) FROM admission_receipts').fetchone()[0]
                before=receipts()
                call(peer,'export_signed',**kwargs,kill_before_commit=True,expect=95)
                assert receipts()==before
                call(peer,'export_signed',**kwargs,kill_after_commit=True,expect=96)
                assert receipts()==before+1
            exported=call(peer,'export_signed',**kwargs)
            retry=call(peer,'export_signed',**kwargs)
            assert retry['response']['duplicate']
            assert retry['response']['result']==exported['response']['result']
            before=events(recipient)
            tampered=copy.deepcopy(exported)
            tampered['response']['result']['binding']['served_at_ms']+=1
            assert call(recipient,'verify_export',server=peer,bundle=tampered,expect=1)['code'] in ['E_SIGNATURE','E_EXPORT_BINDING']
            verified=call(recipient,'verify_export',server=peer,bundle=exported)
            assert verified['verified'] and events(recipient)==before
            capsule=verified['capsule']
        else:
            exported=call(peer,'export',reference=reference)
            assert exported['trusted_export'] is True
            capsule=exported['capsule']
        assert 'Annotations' not in {r['graph_id'] for r in capsule['revisions']}
        transferred += len(json.dumps(capsule,separators=(',',':')).encode())
        return capsule
    def proposed(peer,capsule,nonce):return call(peer,'propose',capsule=capsule,nonce=nonce)
    def integrate(peer,proposal,branch,nonce,expected=None,expect=0):
        return call(peer,'integrate',expect=expect,proof=proposal['proof'],request={'proposal_id':proposal['admitted']['result']['id'],'branch_id':branch,'expected_head':expected,'nonce':nonce})
    def adapter(peer,id,graph,branch='main',outputs=None):return call(peer,'install_adapter',id=id,subscriptions=[{'graph_id':graph,'branch_id':branch}],outputs=outputs or [])
    def complete(peer,id,envelope,program,**kwargs):return call(peer,'complete',id=id,event=envelope['id'],lease=envelope['lease'],program=program,**kwargs)

    evidence={'nodes':[{'id':'baseline','entity_id':'measurement-baseline','space_id':'operations'},{'id':'gateway','entity_id':'gateway','space_id':'operations'}],'edges':[{'id':'expected-quality','from':'baseline','to':'gateway','predicate':'quality_ok','valid_time':{'start':0,'end':100}}]}
    installation={'nodes':[{'id':'device','entity_id':'device','space_id':'operations'},{'id':'physical','entity_id':'device','space_id':'physical'},{'id':'gateway','entity_id':'gateway','space_id':'operations'}],'edges':[{'id':'connection','from':'device','to':'gateway','predicate':'connected','valid_time':{'start':0,'end':100}},{'id':'counterpart','from':'device','to':'physical','predicate':'counterpart','valid_time':{'start':0,'end':100}}],'attachments':[{'id':'evidence-binding','host':{'kind':'edge','id':'connection'},'key':'evidence','value':{'kind':'graph','reference':ref('Evidence','logical:seed:Evidence')},'valid_time':{'start':0,'end':100},'required':True}]}
    call('W','execute',program=plan({'op':'commit_batch','batch_id':'seed','commits':[{'graph_id':'Evidence','data':evidence},{'graph_id':'Installation','data':installation}]},commit('Annotations',{'nodes':[{'id':'private-note','entity_id':'secret','space_id':'private','readers':['independent-reviewer'],'properties':{'text':'not in operational working set'}}]})))
    original=ref('Installation',head('W','Installation')); old_evidence=ref('Evidence',head('W','Evidence'))
    initial=export('W',original,'P')
    before=events('P'); prop=proposed('P',initial,'working-set')
    assert events('P')==before and head('P','Installation') is None
    assert proposed('P',initial,'working-set')['admitted']['duplicate']
    integrate('P',prop,'main','working-set-accept')
    call('P','accept_revision',reference=old_evidence,branch='main',expected=None)
    assert query('P','Installation',original['revision'])['graph']==query('W','Installation',original['revision'])['graph']
    for g,r in [('Installation',original['revision']),('Evidence',old_evidence['revision'])]: call('P','fork',reference=ref(g,r),branch='phone')
    checks.append('signed isolated receipt, explicit root/dependency acceptance, private annotation exclusion and working history')

    # No W/T child starts during this segment: P works with retained snapshots offline.
    offline_start=len(trace)
    adapter('P','diagnostic','Installation','phone',['Warning'])
    prior_event=call('P','poll',id='diagnostic')
    if prior_event is not None:
        assert prior_event['graph']==original and prior_event['event_type']=='graph.accepted'
        complete('P','diagnostic',prior_event,plan())
    new_evidence=copy.deepcopy(evidence)
    new_evidence['nodes'].append({'id':'measurement','entity_id':'measurement-offline','space_id':'operations'})
    new_evidence['edges'].append({'id':'bad-quality','from':'measurement','to':'gateway','predicate':'quality_ok','polarity':'negative','valid_time':{'start':5,'end':20}})
    rebound=copy.deepcopy(installation); rebound['attachments'][0]['value']['reference']=ref('Evidence','logical:offline:Evidence')
    before=events('P')
    call('P','execute',program=plan({'op':'commit_batch','batch_id':'offline','commits':[{'graph_id':'Evidence','branch_id':'phone','expected_head':old_evidence['revision'],'data':new_evidence},{'graph_id':'Installation','branch_id':'phone','expected_head':original['revision'],'data':rebound}]}))
    assert events('P')==before+2
    changed=ref('Installation',head('P','Installation','phone'))
    pinned_query={'kind':'query','query':{'graph_id':'Installation','revision':changed['revision'],'include_metadata':True,'valid_at':7}}
    metadata={'kind':'metadata','input':pinned_query,'host':{'kind':'edge','id':'connection'},'key':'evidence'}
    atom={'predicate':'quality_ok','from':{'kind':'variable','name':'x'},'to':{'kind':'variable','name':'y'},'polarity':'negative'}
    warning_atom={**atom,'predicate':'warning','polarity':'positive'}
    reason={'kind':'reason','input':metadata,'rules':{'id':'diagnostic-rule','revision':'1','rules':[{'id':'bad-quality-warning','head':warning_atom,'body':[atom]}]}}
    result=value('P',{'kind':'filter','input':reason,'predicate':'warning','valid_at':7})
    assert len(result['graph']['edges'])==1
    premises=result['graph']['edges'][0]['derived_from']
    assert {r['assertion_id'] for r in premises}>={'bad-quality','evidence-binding'}
    envelope=call('P','poll',id='diagnostic');assert envelope['graph']==changed and envelope['event_type']=='graph.committed'
    warning_data=scope(result['graph']); warning_plan=plan(commit('Warning',warning_data))
    bad=copy.deepcopy(warning_plan)
    for n in bad['commands'][0]['data']['nodes']:n['readers']=[]
    assert complete('P','diagnostic',envelope,bad,expect=1)['code']=='E_EGRESS'
    before=events('P'); complete('P','diagnostic',envelope,warning_plan,kill_before_commit=True,expect=94)
    assert head('P','Warning') is None and events('P')==before
    complete('P','diagnostic',envelope,warning_plan,kill_after_commit=True,expect=92)
    assert events('P')==before+1
    assert complete('P','diagnostic',envelope,warning_plan)['duplicate'] and events('P')==before+1
    warning=ref('Warning',head('P','Warning'))
    warning_read=query('P','Warning',warning['revision'])
    assert warning_read['coverage']=='complete' and len(warning_read['graph']['edges'])==1, warning_read
    outsider=call('P','query',query={'graph_id':'Warning','revision':warning['revision']},outsider=True)
    assert not outsider['graph']['nodes'] and not outsider['graph']['edges']
    adapter('P','cluster','Warning',outputs=['ClustersPhone'])
    cluster_event=call('P','poll',id='cluster')
    assert cluster_event is not None
    cluster=call('P','cluster',selection={'source':warning,'context':{'kind':'default'},'valid_at':7,'predicate':'warning','levels':2})
    complete('P','cluster',cluster_event,plan(commit('ClustersPhone',scope(cluster['graph']))))
    phone_cluster=ref('ClustersPhone',head('P','ClustersPhone'))
    assert all(item['peer']=='P' for item in trace[offline_start:])
    assert query('P','Evidence',old_evidence['revision'])['graph']['edges'][0]['id']=='expected-quality'
    checks.append('offline atomic rebind, explicit negative-evidence rule, attachment provenance, durable diagnostic receipt and local cluster')
    checks.append('before-COMMIT rollback, after-COMMIT response loss, duplicate suppression and independent-reviewer denial')

    # W changes its own branch while disconnected; reconnection must retain both branches.
    competing=copy.deepcopy(installation);competing['nodes'][0]['properties']={'workstation_note':'concurrent'}
    call('W','execute',program=plan(commit('Installation',competing,expected=original['revision'])))
    work_head=head('W','Installation')
    offline_capsule=export('P',changed,'W','phone')
    p_to_w=proposed('W',offline_capsule,'offline-to-work')
    before=events('W');error=integrate('W',p_to_w,'main','conflict',expected=original['revision'],expect=1)
    assert error['code']=='E_CONFLICT' and events('W')==before and head('W','Installation')==work_head
    accepted_import=integrate('W',p_to_w,'phone-import','keep-offline')
    assert accepted_import['reference']==changed and accepted_import['event_id'] is not None
    assert integrate('W',p_to_w,'phone-import','keep-offline')['duplicate']
    assert head('W','Installation')==work_head
    call('W','accept_revision',reference=ref('Evidence','logical:offline:Evidence'),branch='phone-import',expected=None)
    # Drop a transfer before invoking receiver, then repeat the identical proposal after restart.
    p_cluster_capsule=export('P',phone_cluster,'W'); before=events('W')
    assert events('W')==before
    cluster_proposal=proposed('W',p_cluster_capsule,'cluster-to-work')
    assert proposed('W',p_cluster_capsule,'cluster-to-work')['admitted']['duplicate']
    integrate('W',cluster_proposal,'phone-import','cluster-work-accept')
    call('W','accept_revision',reference=warning,branch='phone-import',expected=None)
    larger=call('W','cluster',selection={'source':changed,'context':{'kind':'default'},'valid_at':7,'predicate':'connected','levels':2})
    call('W','execute',program=plan(commit('ClustersWork',scope(larger['graph']))))
    assert head('W','ClustersPhone','phone-import')==phone_cluster['revision'] and head('W','ClustersWork') is not None
    assert query('W','Evidence',old_evidence['revision'])['coverage']=='complete'
    checks.append(('signed-export' if a.signed_export else 'trusted-export') + ' reconnect, signed proposal duplicates, local CAS conflict, explicit branch integration and coexisting organizations')

    team_capsule=export('W',phone_cluster,'T','phone-import')
    team_prop=proposed('T',team_capsule,'work-to-team')
    integrate('T',team_prop,'main','team-import')
    receipt=call('T','govern',source=phone_cluster,view='team-review',id='accept-offline-evidence',branch='main')
    chosen={'view_id':'team-review','decision_id':receipt['decision_id']}
    accepted=call('T','accepted',selection=chosen)
    assert accepted['graph'].get('influence',{}).get('assertions')
    explained=value('T',{'kind':'explain','input':{'kind':'accepted_graph','selection':chosen}})
    assert explained['graph']['nodes']
    assert call('T','accepted',selection=chosen,outsider=True,expect=1)['code']=='E_GOV_UNAVAILABLE'
    checks.append('team-local genuine owner decision, exact accepted occurrence/explanation, current expiry and multi-user denial')

    decision=next(r for r in accepted['graph']['influence']['assertions'] if r['graph_id'].startswith('weave:governance:decision:'))
    adapter('T','maintenance',decision['graph_id'])
    maintenance=call('T','poll',id='maintenance');assert maintenance is not None
    intent=call('T','effect_request',id='maintenance',event=maintenance['id'],lease=maintenance['lease'],key='maintenance-once',payload={'decision':receipt['decision_id'],'action':'inspect connection'})
    destination=root/'fake-destination.jsonl'
    call('T','effect_send_and_lose_response',intent=intent['id'],destination_file=str(destination),expect=93)
    assert call('T','effect_status',intent=intent['id'])['state']=='unknown'
    assert call('T','effect_begin',intent=intent['id'],expect=1)['code']=='E_EFFECT_UNKNOWN'
    observed=[json.loads(line) for line in destination.read_text().splitlines()]
    assert len(observed)==1 and observed[0]['intent']==intent['id']
    call('T','effect_reconcile',intent=intent['id'],evidence={'destination_receipt':observed[0]})
    assert call('T','effect_status',intent=intent['id'])['state']=='confirmed'
    assert call('T','accepted',selection=chosen,fixture_clock_ms=10000,expect=1)['code']=='E_GOV_UNAVAILABLE'
    checks.append('one fake external action, durable unknown after response-loss kill, no automatic redispatch and explicit reconciliation')
    event_counts={peer:events(peer) for peer in 'PWT'}
if a.signed_export:
    checks.append('signed whole-closure export, exact response retry, pre/postcommit export death, paired verification and tamper denial without receiver mutation')
report={'profile':'native-three-peer-signed-export/1' if a.signed_export else 'native-three-peer-existing-apis/1','status':'passed','protocol':version,'peers':3,'process_invocations':len(trace),'transferred_capsule_bytes':transferred,'events':event_counts,'checks':checks,'trace':trace,'limits':[('native signed export/paired response verification; host-installed test keys, no network transport service' if a.signed_export else 'trusted export and trusted host recipes; no authenticated export endpoint or new peer crypto protocol'),'same owner across peers; independent reviewer denied; no declassification','graph.committed/graph.accepted events; no typed MetaGraphRebound or compiled reactor','whole-capsule transfer; no selective proofs, implicit multi-root acceptance or semantic merge','separate effect request and handler completion; fake local destination only','small native fixture; no browser/mobile persistence or production performance claim']}
text=json.dumps(report,indent=2)+'\n'
if a.report:a.report.write_text(text)
print(json.dumps({k:v for k,v in report.items() if k!='trace'},indent=2))
