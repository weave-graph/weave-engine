#!/usr/bin/env python3
"""Actual native producer/engine and independent fake-sink process recovery. No network."""
import argparse
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile

p=argparse.ArgumentParser()
p.add_argument('--probe',type=Path,default=Path(__file__).resolve().parents[1]/'target/debug/examples/governed_effect_probe')
p.add_argument('--report',type=Path)
p.add_argument('--compiler',type=Path,help='compile the real source identity handler instead of sealing a native fixture')
a=p.parse_args()
processes=0
checks=[]
with tempfile.TemporaryDirectory(prefix='weave-governed-effects-') as temporary:
 root=Path(temporary)
 template=None
 if a.compiler:
  source=root/'reference.weave'
  source.write_text('function Identity revision "1" (graph input) { return input; }\nhandler ReferenceRequest revision "1" using Identity { input event graph "input" branch "main" metadata depth 0; on "graph.accepted", "graph.committed"; output slot "request"; replay pinned; }\n')
  compiled=subprocess.run([str(a.compiler.resolve()),'handler-plan',str(source),'--handler','ReferenceRequest'],capture_output=True,text=True)
  assert compiled.returncode==0,(compiled.stdout,compiled.stderr)
  template=json.loads(compiled.stdout)
 def call(db,mode,expected=0,**values):
  global processes
  processes+=1
  body={'mode':mode,**values}
  request=root/f'{processes}.json'
  request.write_text(json.dumps(body))
  result=subprocess.run([str(a.probe.resolve()),str(db),str(request)],capture_output=True,text=True)
  assert result.returncode==expected,(body,result.returncode,result.stdout,result.stderr)
  return json.loads(result.stdout) if expected==0 else result.stderr
 def row(db,query):
  with sqlite3.connect(db) as c:return c.execute(query).fetchone()[0]
 def setup(name):
  db=root/f'{name}.db';seed=call(db,'seed',**({'template':template} if template is not None else {}));d=seed['delivery']
  assert seed['preparation']['preparation_id']
  return db,{'event':d['event']['id'],'lease':d['lease']}
 def get_intent(receipt):
  assert receipt['disposition']['kind']=='intent',receipt
  return receipt['disposition']['intent_id']
 def status(db,intent):return call(db,'status',intent=intent)
 evidence={'receipt_id':'audited-no-send','response_digest':'sha256:'+'0'*64}

 db,delivery=setup('enqueue')
 call(db,'enqueue',86,crash='before',**delivery)
 assert row(db,'SELECT count(*) FROM effect_intents')==0
 assert row(db,'SELECT count(*) FROM governed_effect_receipts')==0
 assert row(db,'SELECT count(*) FROM governance_delivery_receipts')==0
 assert row(db,'SELECT count(*) FROM governance_delivery_pending')==1
 call(db,'enqueue',87,crash='after',**delivery)
 receipt=call(db,'enqueue',**delivery);assert receipt['duplicate'];intent=get_intent(receipt)
 assert row(db,'SELECT count(*) FROM effect_intents')==1
 assert row(db,'SELECT count(*) FROM governance_delivery_receipts')==1
 checks.append('enqueue precommit rollback and postcommit exact receipt replay')
 call(db,'begin',86,intent=intent,crash='before')
 assert status(db,intent)['state']=='pending'
 call(db,'begin',87,intent=intent,crash='after')
 uncertain=status(db,intent);assert uncertain['state']=='unknown' and uncertain['attempt_id']
 assert 'E_EFFECT_UNKNOWN' in call(db,'begin',1,intent=intent)
 call(db,'reconcile',intent=intent,attempt=uncertain['attempt_id'],outcome='failed',evidence=evidence)
 assert status(db,intent)['state']=='failed'
 checks.append('unknown fence survives lost dispatch response without a second ticket')

 db,delivery=setup('idempotent')
 intent=get_intent(call(db,'enqueue',**delivery));ticket=call(db,'begin',intent=intent)
 payload=json.loads(bytes(ticket['payload']))
 assert payload['format']=='weave-governed-graph-effect/1'
 assert payload['graph']['influence']['snapshots']
 assert payload['graph']['attachments'], 'compiled attribution retained'
 sink=root/'idempotent-sink.db'
 sink_args={'payload':ticket['payload'],'idempotency_key':ticket['idempotency_key'],'idempotent':True}
 call(sink,'sink',86,crash='before',**sink_args)
 assert row(sink,'SELECT count(*) FROM actions')==0
 call(sink,'sink',87,crash='after',**sink_args)
 assert row(sink,'SELECT count(*) FROM actions')==1
 assert status(db,intent)['state']=='unknown'
 assert 'E_EFFECT_UNKNOWN' in call(db,'begin',1,intent=intent)
 # A trusted broker can query/replay an idempotent sink's SAME immutable ticket.
 # This is deliberately not an automatic engine redispatch or a new attempt.
 received=call(sink,'sink',**sink_args)
 assert row(sink,'SELECT count(*) FROM actions')==1
 tampered={**sink_args,'payload':[1,2,3]};assert 'conflict' in call(sink,'sink',1,**tampered)
 reconciliation={'intent':intent,'attempt':ticket['attempt_id'],'outcome':'confirmed','evidence':received}
 call(db,'reconcile',86,crash='before',**reconciliation)
 assert status(db,intent)['state']=='unknown'
 call(db,'reconcile',87,crash='after',**reconciliation)
 call(db,'reconcile',**reconciliation)
 assert status(db,intent)['state']=='confirmed'
 assert 'E_EFFECT_CONFLICT' in call(db,'reconcile',1,**{**reconciliation,'outcome':'failed'})
 call(db,'revoke');call(db,'reconcile',**reconciliation)
 call(db,'status',1,intent=intent)
 checks.append('compiled request to accepted occurrence to immutable intent and one idempotent sink action')
 checks.append('sink and reconciliation pre/postcommit death with exact terminal evidence replay')
 checks.append('revocation denies disclosure while exact owner reconciliation remains available')

 db,delivery=setup('nonidempotent')
 intent=get_intent(call(db,'enqueue',**delivery));ticket=call(db,'begin',intent=intent)
 sink=root/'nonidempotent-sink.db'
 call(sink,'sink',87,crash='after',payload=ticket['payload'],idempotency_key=ticket['idempotency_key'],idempotent=False)
 assert row(sink,'SELECT count(*) FROM actions')==1
 assert row(sink,'SELECT count(*) FROM receipts')==0
 assert status(db,intent)['state']=='unknown'
 assert 'E_EFFECT_UNKNOWN' in call(db,'begin',1,intent=intent)
 call(db,'cancel',1,intent=intent)
 assert row(sink,'SELECT count(*) FROM actions')==1
 checks.append('non-idempotent lost response remains unknown and is never canceled or redispatched')
 report={'profile':'native governed canonical graph reference sink','protocol':'0.18.0','store':17,'native_processes':processes,'compiler_processes':1 if a.compiler else 0,'producer_origin':'actual source handler-plan' if a.compiler else 'native sealed identity fixture','checks':checks,'scope':'trusted native grants, fixed test identities, separate local SQLite sink; no remote Execute capability or production destination'}
 if a.report:
  a.report.parent.mkdir(parents=True,exist_ok=True);a.report.write_text(json.dumps(report,indent=2)+'\n')
 print(json.dumps(report,indent=2))
