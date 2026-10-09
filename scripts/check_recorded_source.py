#!/usr/bin/env python3
"""Fresh real SDK processes and durable CLI reads select independent SQL-observed history.
No client-installed recording clock and no fabricated sealed artifacts.
"""
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


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler-sdk', type=Path, required=True)
    parser.add_argument('--engine', type=Path, required=True)
    parser.add_argument('--view-host', type=Path, required=True)
    parser.add_argument('--evidence-dir', type=Path)
    parser.add_argument('--report', type=Path)
    args = parser.parse_args()
    protocol = tomllib.loads((Path(__file__).parents[1]/"crates/weave-contract/Cargo.toml").read_text())["package"]["version"]
    store_marker = int(__import__("re").search(r"pub const STORAGE_VERSION: i64 = (\d+);",(Path(__file__).parents[1]/"crates/weave-engine/src/lib.rs").read_text()).group(1))
    compiler_trace, runtime_trace = [], []
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix='weave-recorded-source-') as temporary:
        root = Path(temporary)
        evidence = args.evidence_dir or root / 'evidence'
        evidence.mkdir(parents=True, exist_ok=True)
        db = root / 'store.db'

        def compile_source(name, source):
            request_path = evidence / (name + '-request.json')
            response_path = evidence / (name + '-response.json')
            request_path.write_text(json.dumps({'format':'weave-compiler-request/1', 'entry_id':'recorded/'+name, 'source':source, 'modules':[]}), encoding='utf-8')
            subprocess.run([sys.executable, str(Path(__file__).with_name('check_native_scenario.py')), '--compile-child', str(args.compiler_sdk.resolve()), str(request_path), str(response_path)], check=True, timeout=30)
            raw = response_path.read_bytes()
            response = json.loads(raw)
            assert response['ok'], response
            assert response['artifacts']['program']['version'] == protocol
            compiler_trace.append({'source':name,'request_sha256':sha(request_path.read_bytes()),'response_sha256':sha(raw),'response_bytes':len(raw)})
            return response['artifacts']

        def run(name, program, error=None):
            plan = root / 'plan.json'
            plan.write_text(json.dumps(program, ensure_ascii=False), encoding='utf-8')
            result = subprocess.run([str(args.engine.resolve()), 'run', '--db', str(db), '--actor', 'owner', '--write', 'Facts', '--write', 'Other', str(plan)], capture_output=True, timeout=30)
            assert result.returncode == (1 if error else 0), (name, result.returncode, result.stdout, result.stderr)
            raw = result.stderr if error else result.stdout
            value = json.loads(raw)
            if error:
                assert value['code'] == error, value
            (evidence / (name + '-runtime.json')).write_bytes(raw)
            runtime_trace.append({'operation':name,'exit':result.returncode,'response_sha256':sha(raw),'response_bytes':len(raw)})
            return value

        def source(value, expected=None):
            replacement = '' if expected is None else ' replace revision ' + json.dumps(expected)
            return f'''graph Facts{replacement} {{
 node "a" entity "A" space "s" property "exact" 9007199254740993;
 node "b" entity "B" space "s";
 edge "e" from "a" to "b" relation "p" valid 5 until 8 property "value" {value};
}}'''

        seed = compile_source('seed', source(1))
        run('seed', seed['program'])
        with closing(sqlite3.connect(db)) as connection:
            observer = connection.execute('SELECT source FROM engine_identity WHERE id=1').fetchone()[0]
            first = connection.execute("SELECT id,revision,recorded_at_ms FROM head_observations WHERE graph_id='Facts' ORDER BY rowid DESC LIMIT 1").fetchone()
        correction = compile_source('correction', source(2, first[1]))
        run('correction', correction['program'])
        with closing(sqlite3.connect(db)) as connection:
            second = connection.execute("SELECT id,revision,recorded_at_ms FROM head_observations WHERE graph_id='Facts' ORDER BY rowid DESC LIMIT 1").fetchone()
        assert second[2] > first[2], (first, second)

        def query(name, criterion, valid=7):
            artifacts = compile_source(name, f'recorded_handle H graph "Facts" branch "main" {criterion};\npin Historical from H at {valid};')
            result = run(name, artifacts['program'])[-1]['result']
            if valid == 7:
                assert next(n for n in result['graph']['nodes'] if n['entity_id']=='A')['properties']['exact'] == 9007199254740993
            assert len(result['recorded_observations']) == 1
            witness = result['recorded_observations'][0]
            assert witness['observer'] == observer and witness['branch_id'] == 'main'
            assert witness['graph'] in result['input_snapshots']
            return result

        old = query('old', 'known_at '+str(first[2]))
        latest = query('latest', 'known_at '+str(second[2]))
        assert old['graph']['edges'][0]['properties']['value'] == 1
        assert latest['graph']['edges'][0]['properties']['value'] == 2
        assert old['recorded_observations'][0]['checkpoint'] == first[0]
        assert latest['recorded_observations'][0]['checkpoint'] == second[0]
        criterion = 'checkpoint '+json.dumps(first[0])+' observer '+json.dumps(observer)
        exact = query('checkpoint', criterion)
        assert exact == old
        expired = query('outside-valid', criterion, 8)
        assert expired['graph']['edges'] == [] and expired['recorded_observations'] == old['recorded_observations']
        foreign = compile_source('foreign', f'recorded_handle H graph "Facts" branch "main" checkpoint {json.dumps(first[0])} observer "urn:weave:replica:foreign"; pin Bad from H;')
        run('foreign', foreign['program'], 'E_HISTORY_UNAVAILABLE')
        template = compile_source('view', f'recorded_handle H graph "Facts" branch "main" known_at {first[2]}; view_template Historical revision "1" from H clock tick {{ at 7; }}')['view_templates']['Historical']
        assert template['expression']['selection'] == {'kind':'local_time','unix_millis':first[2]}
        assert template['expression']['query']['valid_at'] == 7 and template['clock'] == 'tick'
        template_path = evidence / 'view-template.json'
        template_path.write_text(json.dumps(template), encoding='utf-8')
        def view(name, mode, *fields):
            result = subprocess.run([str(args.view_host.resolve()), str(db), mode, *map(str,fields)],capture_output=True,timeout=30)
            assert result.returncode == 0, (result.stdout,result.stderr)
            value=json.loads(result.stdout)
            (evidence/(name+'-runtime.json')).write_bytes(result.stdout)
            runtime_trace.append({'operation':name,'exit':0,'response_sha256':sha(result.stdout),'response_bytes':len(result.stdout)})
            return value['result']
        registered=view('register-view','recorded_register',template_path,'recorded-source-view',7)
        assert registered['graph']['edges'][0]['properties']['value'] == 1
        assert registered['recorded_observations'] == old['recorded_observations']
        advanced=view('tick-view','recorded_refresh','recorded-source-view',8)
        assert advanced['graph']['edges'] == [] and advanced['recorded_observations'] == old['recorded_observations']
        restored=view('reopen-view','recorded_refresh','recorded-source-view',8)
        assert restored['graph'] == advanced['graph'] and restored['recorded_observations'] == registered['recorded_observations']
        historical=view('historical-view','recorded_register',template_path,'separate-historical-view',7)
        assert historical['graph'] == registered['graph'] and historical['recorded_observations'] == registered['recorded_observations']
        forged_old = {'version':'0.19.0','commands':[{'op':'commit','graph_id':'Other','data':{}}, {'op':'evaluate','value':template['expression']}]}
        run('old-protocol', forged_old, 'E_VERSION')
        with closing(sqlite3.connect(db)) as connection:
            assert connection.execute("SELECT COUNT(*) FROM heads WHERE graph_id='Other'").fetchone()[0] == 0
            assert connection.execute('PRAGMA user_version').fetchone()[0] == store_marker
    report = {'profile':'actual-sdk-recorded-source/1','status':'passed','protocol':protocol,'store_marker':store_marker,'compiler_processes':len(compiler_trace),'runtime_processes':len(runtime_trace),'seconds':round(time.monotonic()-started,3),'checks':['real full SDK responses retain exact integers','durable late correction preserves old recorded knowledge','independent SQL oracle identifies selected checkpoint and local observer','explicit checkpoint replays after correction','valid time stays separate and empty result preserves witness','foreign observer fails closed','actual sealed source view installs, ticks and reopens with the same recorded checkpoint','old protocol fails before earlier write'],'compiler_trace':compiler_trace,'runtime_trace':runtime_trace,'scope':'native durable SystemClock CLI, small fixture; no global recorded cut or source range/accepted-view history claim'}
    if args.report:
        args.report.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report))


if __name__ == '__main__':
    main()
