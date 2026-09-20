#!/usr/bin/env python3
"""Real historical compiler/handler state survives atomic runtime schema upgrade."""
import argparse, json, sqlite3, subprocess, tempfile
from pathlib import Path
from contextlib import closing
p=argparse.ArgumentParser()
for name in ['old-compiler','old-handler','handler','storage']:
 p.add_argument('--'+name,type=Path,required=True)
p.add_argument('--old-marker',type=int,default=16)
p.add_argument('--new-marker',type=int,default=18)
p.add_argument('--report',type=Path)
a=p.parse_args()
def invoke(binary,*args,code=0):
 r=subprocess.run([str(binary.resolve()),*map(str,args)],capture_output=True,text=True,timeout=30)
 assert r.returncode==code,(binary,r.returncode,r.stdout,r.stderr)
 return json.loads(r.stdout) if code==0 else r.stderr
with tempfile.TemporaryDirectory(prefix='weave-root-handler-migration-') as tmp:
 root=Path(tmp);db=root/'store.sqlite';request=root/'request.json';source=root/'source.weave'
 def host(binary,mode,**fields):
  request.write_text(json.dumps({'mode':mode,**fields}));return invoke(binary,db,request)
 def populate(name,graph,output,complete):
  source.write_text(f'''function Keep revision "1" (graph input) {{ return input; }}
handler {name} revision "1" using Keep {{
 input event graph "{graph}" branch "main" metadata depth 0;
 on "graph.committed", "graph.accepted";
 output slot "result";
 replay pinned;
}}
''')
  artifact=invoke(a.old_compiler,'handler-plan',source,'--handler',name)
  # The CLI emits the sealed template directly; never hand-author its digest.
  if complete:
   # A real old grouped derivation catches accidental re-materialization/resealing
   # after the new runtime strengthens original-record authority on fresh queries.
   batch='historical-'+name;pin={'graph_id':graph,'revision':'logical:'+batch+':'+graph}
   premise={**pin,'assertion_id':'base'}
   data={'nodes':[{'id':n,'entity_id':n,'space_id':'s'} for n in ['a','b']],
         'edges':[{'id':'base','predicate':'p','from':'a','to':'b','valid_time':{'start':0}},
                  {'id':'derived','predicate':'q','from':'a','to':'b','valid_time':{'start':0},
                   'derived_from':[premise],'derivations':[{'operator':'historical-proof','premises':[premise],
                   'parameters':{'historical':'preserve-exactly'},'input_snapshots':[pin]}]}]}
   seed={'op':'commit_batch','batch_id':batch,'commits':[{'graph_id':graph,'data':data}]}
  else:seed={'op':'commit','graph_id':graph,'data':{}}
  host(a.old_handler,'run',program={'version':'0.18.0','commands':[seed]})
  host(a.old_handler,'install',id=name,template=artifact,output=output)
  event=host(a.old_handler,'poll',id=name)
  args={'id':name,'event':event['id'],'lease':event['lease']}
  prep=host(a.old_handler,'prepare',**args)
  args['preparation']=prep['preparation_id']
  receipt=host(a.old_handler,'complete',**args) if complete else None
  return args,receipt
 done,receipt=populate('HistoricalDone','Empty','EmptyWarnings',True)
 pending,_=populate('HistoricalPending','Installation','Warnings',False)
 new_tables=({'governed_effect_bindings','governed_effect_receipts','governed_effect_context'} if a.old_marker<17 else set())
 def snapshot():
  with closing(sqlite3.connect(db)) as c:
   tables={row[0]:row[1] for row in c.execute("SELECT name,sql FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")}
   contents={t:(sql,sorted(c.execute('SELECT * FROM "'+t.replace('"','""')+'"').fetchall(),key=repr)) for t,sql in tables.items() if t not in new_tables}
   return c.execute('PRAGMA user_version').fetchone()[0],set(tables)&new_tables,contents
 before=snapshot();assert before[0]==a.old_marker and not before[1]
 assert len(before[2]['compiled_handlers'][1])==2
 assert len(before[2]['handler_preparations'][1])==2
 assert len(before[2]['handler_receipts'][1])==1
 invoke(a.storage,db,'crash',code=82);assert snapshot()==before
 invoke(a.storage,db,'after_commit',code=83)
 migrated=snapshot();assert migrated==(a.new_marker,new_tables,before[2])
 replay=host(a.handler,'complete',**done)
 assert replay['duplicate'] is True and replay['results']==receipt['results']
 assert snapshot()==migrated
 renewed=host(a.handler,'prepare',**{k:v for k,v in pending.items() if k!='preparation'})
 assert renewed['duplicate'] is True and renewed['preparation_id']==pending['preparation']
 assert snapshot()==migrated
 committed=host(a.handler,'complete',**pending);assert committed['duplicate'] is False
 again=host(a.handler,'complete',**pending);assert again['duplicate'] is True and again['results']==committed['results']
 stable=snapshot()
 request.write_text(json.dumps({'mode':'head','graph':'Warnings'}))
 error=invoke(a.old_handler,db,request,code=1)
 assert 'E_STORAGE_VERSION' in error and snapshot()==stable
 report={'profile':'populated-compiled-handler-migration','old_marker':a.old_marker,'new_marker':a.new_marker,'old_protocol':'0.18.0','checks':['actual historical compiler emitted sealed templates','completed grouped-proof and pending empty historical preparations','precommit rollback with no new tables','postcommit death preserves every historical table byte','exact historical completion replay','unchanged preparation identity and bytes','pending prepared command completes once after migration','old binary refusal'],'status':'passed'}
 if a.report:a.report.write_text(json.dumps(report,indent=2)+'\n')
 print(json.dumps(report))
