#!/usr/bin/env python3
"""Populate real 0.16/schema14 fixtures, then verify atomic15 migration and exact old identities."""
import argparse, hashlib, json, sqlite3, subprocess, tempfile
from pathlib import Path
p=argparse.ArgumentParser()
p.add_argument('--old-view',type=Path,required=True)
p.add_argument('--old-trace',type=Path,required=True)
p.add_argument('--view',type=Path,default=Path('target/debug/examples/source_view_fixture'))
p.add_argument('--trace',type=Path,default=Path('target/debug/examples/three_peer_trace'))
p.add_argument('--storage',type=Path,default=Path('target/debug/examples/storage_probe'))
p.add_argument('--report',type=Path)
a=p.parse_args()
def run(binary,*args,code=0):
 r=subprocess.run([str(binary.resolve()),*map(str,args)],capture_output=True,text=True,timeout=30)
 assert r.returncode==code,(binary,r.returncode,r.stdout,r.stderr)
 return json.loads(r.stdout) if code==0 else r.stderr
def digest(domain,value):return 'sha256:'+hashlib.sha256(json.dumps([domain,value],separators=(',',':')).encode()).hexdigest()
with tempfile.TemporaryDirectory(prefix='weave-snapshot-migration-') as tmp:
 root=Path(tmp);db=root/'store.sqlite';request=root/'request.json';templatefile=root/'template.json'
 run(a.old_view,db,'seed')
 def trace(binary,op,**body):
  request.write_text(json.dumps({'op':op,**body}));return run(binary,db,'P',request)
 trace(a.old_trace,'init')
 trace(a.old_trace,'execute',program={'version':'0.16.0','commands':[{'op':'commit','graph_id':'Evidence','data':{}}]})
 revision=trace(a.old_trace,'head',graph='Evidence',branch='main')['revision']
 export={'reference':{'graph_id':'Evidence','revision':revision},'recipient':'W','branch':'main','nonce':'migration-old-response'}
 original=trace(a.old_trace,'export_signed',**export)
 expr={'kind':'query','query':{'graph_id':'Fleet','revision':None,'branch_id':'main','predicate':None,'from':None,'to':None,'valid_at':None,'include_metadata':False,'max_depth':8}}
 protocol='0.16.0';name='Migration';rev='1';clock='fixed'
 source={'name':'weave:view-template:'+digest('weave-source-label-v1',['view_template',name])[7:],'revision':rev,'digest':digest('weave-view-template-source-v1',[protocol,name,rev,expr,clock])}
 template={'format':'weave-view-registration/1','protocol':protocol,'name':name,'revision':rev,'expression':expr,'clock':clock,'source_revisions':[source]}
 template['definition_digest']=digest('weave-view-definition-v1',[template['format'],protocol,name,rev,expr,clock,[source]])
 templatefile.write_text(json.dumps(template))
 registered=run(a.old_view,db,'register',templatefile,'migration-view','fixed')
 before_current=run(a.old_view,db,'current','migration-view',template['definition_digest'],'fixed')
 def state():
  with sqlite3.connect(db) as c:
   return {'marker':c.execute('PRAGMA user_version').fetchone()[0],**{t:c.execute('SELECT * FROM '+t+' ORDER BY rowid').fetchall() for t in ['revisions','events','admission_receipts','live_views','view_sources']}}
 before=state();assert before['marker']==14
 run(a.storage,db,'crash',code=82);assert state()==before
 run(a.storage,db,'after_commit',code=83)
 committed=state();assert committed['marker']==15 and {**committed,'marker':14}==before
 current=run(a.view,db,'current','migration-view',template['definition_digest'],'fixed')
 assert current==before_current,(current,before_current)
 after=state();assert after['marker']==15 and {**after,'marker':14}==before
 replay=trace(a.trace,'export_signed',**export)
 assert replay['response']['duplicate'] is True
 assert replay['response']['result']==original['response']['result']
 assert state()==after
 error=run(a.old_view,db,'current','migration-view',template['definition_digest'],'fixed',code=1)
 assert 'E_STORAGE_VERSION' in error and state()==after
 report={'old_marker':14,'new_marker':15,'checks':['populated old template and signed receipt','precommit rollback','postcommit death and exact restart','unchanged persisted template/cache/receipt/rows','exact historical response replay','old binary refusal'], 'template_digest':template['definition_digest']}
 if a.report:a.report.write_text(json.dumps(report,indent=2)+'\n')
 print(json.dumps(report))
