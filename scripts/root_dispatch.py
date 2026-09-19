#!/usr/bin/env python3
"""Independent process-death recovery checks; uses the opt-in test host driver."""
import argparse
import json
import os
import re
import sqlite3
import subprocess
import tempfile
from pathlib import Path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--engine', type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    engine = args.engine.resolve()
    version = re.search(r'^version = "([^"]+)"', (engine/'crates/weave-contract/Cargo.toml').read_text(), re.M).group(1)
    subprocess.run(['cargo','build','--locked','-p','weave-engine','--bin','weave-engine','--example','dispatch_probe','--features','recovery-testing'], cwd=engine,check=True)
    suffix = '.exe' if os.name == 'nt' else ''
    runtime = engine/'target/debug'/('weave-engine'+suffix)
    probe = engine/'target/debug/examples'/('dispatch_probe'+suffix)
    checks = []
    with tempfile.TemporaryDirectory(prefix='weave-dispatch-') as tmp:
        root = Path(tmp)
        serial = 0
        def call(db, request, exit_code=0, error=None):
            nonlocal serial
            serial += 1
            path = root/f'request-{serial}.json'
            path.write_text(json.dumps(request))
            r = subprocess.run([str(probe),str(db),str(path)], text=True,capture_output=True)
            assert r.returncode == exit_code, (request,r.returncode,r.stdout,r.stderr)
            if error:
                assert error in json.loads(r.stderr)['error'], r.stderr
            if r.returncode == 0:
                return json.loads(r.stdout)
        def count(db, table):
            assert table in ['events','revisions','handler_receipts','dispatch_pending','effect_intents']
            with sqlite3.connect(db) as conn:
                return conn.execute('SELECT COUNT(*) FROM '+table).fetchone()[0]
        def setup(name):
            db = root/(name+'.db')
            plan = root/(name+'.json')
            plan.write_text(json.dumps({'version':version,'commands':[{'op':'commit','graph_id':'input','data':{'nodes':[{'id':'source','entity_id':'source','space_id':'s'}]}}]}))
            subprocess.run([str(runtime),'run','--db',str(db),'--actor','alice','--write','input',str(plan)],check=True,capture_output=True,text=True)
            call(db, {'op':'install','manifest':{'id':'adapter-v1','version':'1','artifact_digest':'sha256:'+'a'*64,'config_revision':'1','principal':'alice','subscriptions':[{'graph_id':'input','branch_id':'main'}],'output_graphs':['output'],'effect_destinations':['mock://sink'],'max_attempts':3,'lease_ms':100,'max_pending_events':100,'projection_replay':False}})
            return db
        output = {'version':version,'commands':[{'op':'commit','graph_id':'output','data':{'nodes':[{'id':'derived','entity_id':'derived','space_id':'s','readers':['alice']}]}}]}
        db = setup('atomic')
        d = call(db, {'op':'poll','adapter':'adapter-v1','now_ms':0})
        complete = {'op':'complete','adapter':'adapter-v1','event':d['id'],'lease':d['lease'],'program':output}
        call(db, dict(complete,crash_before_commit=True), exit_code=78)
        assert count(db,'events') == 1 and count(db,'revisions') == 1
        assert count(db,'handler_receipts') == 0 and count(db,'dispatch_pending') == 1
        checks.append('process death before COMMIT rolls back output, receipt and checkpoint together')
        renewed = call(db, {'op':'poll','adapter':'adapter-v1','now_ms':101})
        assert renewed['id'] == d['id'] and renewed['lease'] != d['lease']
        call(db, complete, exit_code=1,error='E_LEASE')
        checks.append('restart redelivers same occurrence with fresh lease and rejects stale lease')
        complete['lease'] = renewed['lease']
        call(db, dict(complete,crash_after_commit=True), exit_code=77)
        assert count(db,'events') == 2 and count(db,'revisions') == 2
        assert count(db,'handler_receipts') == 1 and count(db,'dispatch_pending') == 0
        receipt = call(db,complete)
        assert receipt['duplicate'] is True
        assert count(db,'events') == 2 and count(db,'revisions') == 2
        assert call(db, {'op':'poll','adapter':'adapter-v1','now_ms':202}) is None
        checks.append('process death after COMMIT before acknowledgment does not duplicate output on retry')
        changed = dict(complete,program={'version':version,'commands':[]})
        call(db,changed,exit_code=1,error='E_RECEIPT_CONFLICT')
        checks.append('same occurrence cannot be replayed with different commands')
        db = setup('effects')
        d = call(db, {'op':'poll','adapter':'adapter-v1','now_ms':0})
        request = {'op':'request_effect','adapter':'adapter-v1','event':d['id'],'lease':d['lease'],'destination':'mock://sink','key':'operation-1','payload':{'action':'append','value':1}}
        call(db,dict(request,crash_after_commit=True),exit_code=77)
        intent = call(db,request)
        assert intent['state'] == 'pending' and count(db,'effect_intents') == 1
        checks.append('lost intent acknowledgment preserves stable idempotency identity')
        call(db, {'op':'begin_effect','id':intent['id'],'crash_after_commit':True},exit_code=77)
        assert call(db,{'op':'effect','id':intent['id']})['state'] == 'unknown'
        call(db,{'op':'begin_effect','id':intent['id']},exit_code=1,error='E_EFFECT_UNKNOWN')
        checks.append('death after dispatch fence preserves unknown outcome and prevents automatic retry')
        call(db,{'op':'reconcile','id':intent['id'],'outcome':'confirmed','response':{'receipt':'mock-receipt-1'}})
        assert call(db,{'op':'effect','id':intent['id']})['state'] == 'confirmed'
        call(db,{'op':'reconcile','id':intent['id'],'outcome':'failed','response':{}},exit_code=1,error='E_EFFECT')
        checks.append('explicit reconciliation persists a terminal outcome')
    print(json.dumps({'status':'passed','contract':version,'checks':checks,'limit':'Local process deaths and durable effect fences; no network or external-system exactly-once claim.'},indent=2))


if __name__ == '__main__':
    main()
