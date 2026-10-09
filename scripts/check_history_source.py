#!/usr/bin/env python3
"""Real SDK artifacts select actual SQL-observed receipt and acceptance histories."""
from version_profile import store_marker
import argparse
from contextlib import closing
import hashlib
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time
import tomllib


def sha(raw): return hashlib.sha256(raw).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler-sdk', type=Path, required=True)
    parser.add_argument('--host', type=Path, required=True)
    parser.add_argument('--evidence-dir', type=Path)
    parser.add_argument('--report', type=Path)
    args = parser.parse_args()
    protocol = tomllib.loads((Path(__file__).parents[1]/'crates/weave-contract/Cargo.toml').read_text())['package']['version']
    started = time.monotonic(); compilers = []; runtimes = []
    with tempfile.TemporaryDirectory(prefix='weave-source-history-') as temporary:
        root = Path(temporary); db = root/'store.db'; plan = root/'plan.json'
        evidence = args.evidence_dir or root/'evidence'; evidence.mkdir(parents=True, exist_ok=True)
        def compile_source(name, source, ok=True):
            request = evidence/(name+'-request.json'); response = evidence/(name+'-response.json')
            request.write_text(json.dumps({'format':'weave-compiler-request/1','entry_id':'history/'+name,'source':source,'modules':[]}))
            subprocess.run([sys.executable,str(Path(__file__).with_name('check_native_scenario.py')),'--compile-child',str(args.compiler_sdk.resolve()),str(request),str(response)],check=True,timeout=30)
            raw=response.read_bytes(); value=json.loads(raw); assert value['ok']==ok,value
            compilers.append({'source':name,'request_sha256':sha(request.read_bytes()),'response_sha256':sha(raw),'response_bytes':len(raw)})
            if ok:
                assert value['artifacts']['program']['version']==protocol
                assert value['artifacts']['values']['Exact']['value']==9007199254740993
                return value['artifacts']['program']
            assert value['error']['code']=='E_FUNCTION_EFFECT',value
        def invoke(name, mode, program=None, error=None):
            fields=[]
            if program is not None: plan.write_text(json.dumps(program)); fields=[str(plan)]
            result=subprocess.run([str(args.host.resolve()),str(db),mode,*fields],capture_output=True,timeout=30)
            assert result.returncode==(1 if error else 0),(name,result.stdout,result.stderr)
            raw=result.stderr if error else result.stdout; value=json.loads(raw)
            if error: assert value['code']==error,value
            (evidence/(name+'-runtime.json')).write_bytes(raw)
            runtimes.append({'operation':name,'exit':result.returncode,'response_sha256':sha(raw),'response_bytes':len(raw)})
            return value
        def source(name,text):return compile_source(name,text+'\nvalue Exact 9007199254740993;')
        def observed(decision):
            with closing(sqlite3.connect(db)) as c:
                observer=c.execute('SELECT source FROM engine_identity WHERE id=1').fetchone()[0]
                acceptance=c.execute('SELECT accepted_at_ms FROM governance_decisions WHERE id=?',(decision,)).fetchone()[0]
                recording=c.execute("SELECT id,revision,recorded_at_ms FROM head_observations WHERE graph_id='Fleet' ORDER BY rowid DESC LIMIT 1").fetchone()
                return observer,acceptance,recording
        seeded=invoke('seed','live_seed'); first=seeded['accepted']['decision_id']
        observer,t1,r1=observed(first); assert t1>=r1[2]
        old_plan=source('old',f'accepted_history Old view "team" accepted_at {t1} at 7;')
        old=invoke('old','live_run',old_plan)[0]['result']; assert old['graph']['nodes'][0]['properties']['turn']==0
        witness=old['accepted_observations'][0]; assert witness['observer']==observer and witness['decision_id']==first and witness['accepted_at_ms']==t1 and witness['source']==seeded['source']
        for pin in [witness['source'],witness['occurrence']]:assert pin in old['input_snapshots']
        invoke('change','live_change'); assert invoke('old-after-correction','live_run',old_plan)[0]['result']==old
        published=invoke('publish','live_publish'); second=published['decision_id']; _,t2,r2=observed(second)
        assert t2>t1 and r2[2]>r1[2] and t2>=r2[2]
        latest=invoke('latest','live_run',source('latest',f'accepted_history New view "team" accepted_at {t2} at 7;'))[0]['result']
        assert latest['graph']['nodes'][0]['properties']['turn']==1 and latest['accepted_observations'][0]['decision_id']==second
        exact=source('exact',f'accepted_history Old view "team" decision {json.dumps(first)} observer {json.dumps(observer)} at 7;')
        assert invoke('exact','live_run',exact)[0]['result']==old
        assert invoke('fixed-after-acceptance','live_run',old_plan)[0]['result']==old
        empty=invoke('empty','live_run',source('empty',f'accepted_history Old view "team" accepted_at {t1} at 10;'))[0]['result']
        assert not empty['graph']['nodes'] and not empty['graph']['edges'] and empty['accepted_observations']==old['accepted_observations']
        ranges=source('ranges',f'''recorded_range R graph "Fleet" branch "main" observer {json.dumps(observer)} between {r1[2]} and {r2[2]} limit 10 at 10;
accepted_range A view "team" observer {json.dumps(observer)} between {t1} and {t2} limit 10 at 10;''')
        output=invoke('ranges','live_run',ranges)
        for item in output:
            assert item['kind']=='history_ranged'; range_=item['range']; assert len(range_['changes'])==1
            assert range_['changes'][0]==range_['start_state']; assert not range_['start_state']['graph']['nodes']
        assert output[0]['range']['start_state']['recorded_observations'][0]['checkpoint']==r1[0]
        assert output[1]['range']['start_state']['accepted_observations'][0]['decision_id']==first
        foreign=source('foreign',f'accepted_history Old view "team" decision {json.dumps(first)} observer "foreign";')
        invoke('foreign','live_run',foreign,'E_GOV_HISTORY_UNAVAILABLE')
        invoke('outsider','live_outsider',old_plan,'E_GOV_HISTORY_UNAVAILABLE')
        before=source('before',f'accepted_history Old view "team" accepted_at {t1-1};')
        invoke('before','live_run',before,'E_GOV_HISTORY_UNAVAILABLE')
        compile_source('hidden', 'function Hidden revision "1" (graph input) { accepted_history H view "team" accepted_at 10; return input; }',False)
        for name,new in [('old-expression',old_plan['commands'][0]),('old-range',ranges['commands'][0])]:
            forged={'version':'0.20.0','commands':[{'op':'commit','graph_id':'Marker','data':{}},new]}
            invoke(name,'live_run',forged,'E_VERSION')
            with closing(sqlite3.connect(db)) as c:assert not c.execute("SELECT 1 FROM heads WHERE graph_id='Marker'").fetchall()
        with closing(sqlite3.connect(db)) as c: marker=c.execute('PRAGMA user_version').fetchone()[0]; assert marker==store_marker()
    report={'profile':'actual-sdk-accepted-history-and-ranges/1','status':'passed','protocol':protocol,'store_marker':marker,'compiler_processes':len(compilers),'runtime_processes':len(runtimes),'seconds':round(time.monotonic()-started,3),'observed':{'observer':observer,'first_recorded':r1[2],'first_accepted':t1,'second_recorded':r2[2],'second_accepted':t2},'checks':['actual full SDK bytes and exact i64','actual SystemClock recording and genuine signed acceptance','source correction and later acceptance preserve fixed old selection','explicit decision replay matches local acceptance-time query','empty valid-time output preserves both protected and source pins','authorized start state and all half-open changes on both axes','foreign observer, outsider and preacceptance fail closed','pure hidden reads denied by actual SDK','old protocol cannot publish earlier writes'],'compiler_trace':compilers,'runtime_trace':runtimes,'scope':'trusted native SystemClock fixture with host-owned SQLite; no universal cut or full portable/lifecycle/retention claim'}
    if args.report:args.report.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report))


if __name__=='__main__':main()
