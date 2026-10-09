#!/usr/bin/env python3
"""Actual SDK -> native durable diagnostic -> retained cluster, in fresh processes.
No Cargo invocation. Fixture manifests are trusted test-host configuration.
"""
import argparse
import copy
import ctypes
from contextlib import contextmanager
import hashlib
import json
import sqlite3
import subprocess
import sys
import tempfile
import time
try:
    import resource
except ImportError:
    resource = None
from pathlib import Path


def compile_child(library_path, request_path, output_path):
    library = ctypes.CDLL(str(library_path.resolve()))
    def function(name, count):
        f = getattr(library, 'weave_compiler_' + name)
        f.argtypes = [ctypes.c_uint32] * count
        f.restype = ctypes.c_int32
        return f
    new, write, compile_, length, read, drop = [function(n, c) for n, c in
        [('input_new', 1), ('input_write', 3), ('compile', 1), ('output_len', 1), ('output_read', 2), ('drop', 1)]]
    assert function('abi_version', 0)() == 1
    request = request_path.read_bytes()
    handle = new(len(request)); output = 0
    assert handle > 0
    try:
        for offset in range(0, len(request), 2):
            chunk = request[offset:offset + 2]
            assert write(handle, int.from_bytes(chunk, 'little'), len(chunk)) == 0
        output = compile_(handle); handle = 0
        assert output > 0
        size = length(output)
        assert 0 <= size <= 16 * 1024 * 1024 + 4096
        result = bytearray(size)
        for offset in range(0, size, 2):
            word = read(output, offset)
            assert 0 <= word <= 65535
            result[offset:offset + 2] = word.to_bytes(2, 'little')[:min(2, size - offset)]
        output_path.write_bytes(result)
    finally:
        if output: assert drop(output) == 0
        if handle: drop(handle)


@contextmanager
def database(path):
    connection = sqlite3.connect(path)
    try:
        with connection:
            yield connection
    finally:
        connection.close()


def sha(raw): return hashlib.sha256(raw).hexdigest()
def encoded(value): return json.dumps(value, ensure_ascii=False, separators=(',', ':')).encode()

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--compiler-sdk', type=Path, required=True)
    parser.add_argument('--host', type=Path, required=True)
    parser.add_argument('--fixtures', type=Path, required=True)
    parser.add_argument('--upgrade-host', type=Path, help='switch from a real store19/SDK0.19 host to the current host after retaining an empty cluster receipt')
    parser.add_argument('--peer-host', type=Path, help='continue actual source state through signed P/W/T exchange, governance and effect fencing')
    parser.add_argument('--accepted-history', action='store_true', help='verify native accepted-time selection over the actual transferred source')
    parser.add_argument('--report', type=Path)
    parser.add_argument('--evidence-dir', type=Path)
    args = parser.parse_args()
    if args.accepted_history and not args.peer_host:
        parser.error('--accepted-history requires --peer-host')
    trace, variants, compiler_trace = [], [], []
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix='weave-native-scenario-') as temp:
        root = Path(temp)
        for config_path in sorted(args.fixtures.glob('*/manifest.json')):
            config = json.loads(config_path.read_bytes())
            directory = config_path.parent
            variant = config['variant']; graphs = config['graphs']; adapters = config['adapters']
            work = root / variant; work.mkdir()
            evidence = args.evidence_dir / variant if args.evidence_dir else work / 'evidence'
            evidence.mkdir(parents=True, exist_ok=True)
            active_host = args.host
            db = work / 'phone.db'; journal = db.with_suffix('.host.db')
            runtime_config = work / 'config.json'; runtime_config.write_bytes(encoded(config))
            peer_dbs = {'P':db, 'W':work/'work.db', 'T':work/'team.db'}
            peer_trace = []
            peer_clocks = {'P':20, 'W':20, 'T':20}
            def peer_call(peer, op, expect=0, **data):
                path = work / 'peer-request.json'; path.write_bytes(encoded({'op':op, 'fixture_clock_ms':peer_clocks[peer], **data}))
                r = subprocess.run([str(args.peer_host.resolve()),str(peer_dbs[peer]),peer,str(path),str(runtime_config)],capture_output=True,timeout=30)
                assert r.returncode==expect,(peer,op,r.returncode,r.stdout[-1500:],r.stderr[-1500:])
                peer_trace.append({'peer':peer,'operation':op,'exit':r.returncode,'response_sha256':sha(r.stdout+r.stderr)})
                return json.loads(r.stdout if expect==0 else r.stderr) if (r.stdout if expect==0 else r.stderr).strip() else None
            if args.peer_host:
                for peer in 'PWT': identity = peer_call(peer,'init')
                config['principal'] = identity['owner']
                runtime_config.write_bytes(encoded(config))
            now = 100
            def invoke(mode, expect=0, error=None, **data):
                nonlocal now
                request = {'mode': mode, 'now': now, **data}
                path = work / 'request.json'; path.write_bytes(encoded(request))
                before = time.monotonic()
                result = subprocess.run([str(active_host.resolve()), str(db), str(runtime_config), str(path)], capture_output=True, timeout=30)
                assert result.returncode == expect, (variant, mode, result.returncode, result.stdout[-2000:], result.stderr[-2000:])
                value = json.loads(result.stdout) if result.stdout.strip() else None
                trace.append({'variant': variant, 'operation': mode, 'exit': result.returncode, 'seconds': round(time.monotonic()-before, 6), 'response_sha256': sha(result.stdout), 'response_bytes': len(result.stdout)})
                if error:
                    assert value['ok'] is False and value['error']['code'] in error, value
                    return value
                if expect == 0 and mode != 'inspect':
                    assert value['ok'] is True, (variant, mode, value)
                    return value['value']
                return value
            def program(*commands): return {'version': '0.19.0', 'commands': list(commands)}
            def execute(p, **extra): return invoke('call', operation={'kind': 'execute', 'program': p}, **extra)
            def inspect(): return invoke('inspect')
            def query(graph, revision, **extra):
                return execute(program({'op': 'evaluate', 'value': {'kind': 'query', 'query': {'graph_id': graph, 'revision': revision, **extra}}}))[0]['result']
            def poll(key):
                nonlocal now
                now += 1100
                return invoke('call', operation={'kind':'poll', 'adapter': adapters[key]})
            compilations = {}
            def compile_source(kind, pins=None, altered=False):
                source = (directory / config['files'][kind]).read_text()
                for token in config.get('substitutions', {}).get(kind, []):
                    placeholder = config['tokens'][token]
                    assert source.count(placeholder) == 1
                    source = source.replace(placeholder, json.dumps(pins[token], ensure_ascii=False))
                modules = []
                if kind == 'handler':
                    for module in config['modules']:
                        raw = (directory / module['file']).read_bytes()
                        assert sha(raw) == module['sha256']
                        modules.append({'id':module['id'], 'revision':module['revision'], 'source':raw.decode() + ('// altered\n' if altered else '')})
                suffix = kind + ('-altered' if altered else '') + '-' + str(len(compiler_trace))
                request_path = evidence / (suffix + '-request.json'); response_path = evidence / (suffix + '-response.json')
                request_path.write_bytes(encoded({'format':'weave-compiler-request/1', 'entry_id':variant+'/'+kind, 'source':source, 'modules':modules}))
                subprocess.run([sys.executable, str(Path(__file__).resolve()), '--compile-child', str(args.compiler_sdk.resolve()), str(request_path), str(response_path)], check=True, timeout=30)
                raw = response_path.read_bytes(); response = json.loads(raw)
                compiler_trace.append({'variant':variant, 'kind':kind, 'request_sha256':sha(request_path.read_bytes()), 'response_sha256':sha(raw), 'response_bytes':len(raw)})
                if altered:
                    assert not response['ok'] and response['error']['code'] == 'E_MODULE_DIGEST', response
                else:
                    assert response['ok'], response
                    compilations[kind] = response
                runtime_artifact = work / (suffix + '-response.json')
                runtime_artifact.write_bytes(raw)
                return runtime_artifact
            seed = compile_source('seed'); handler = compile_source('handler')
            compile_source('handler', altered=True)
            # Artifact parsing must preserve all scalar/template members and exact numbers.
            artifact = compilations['handler']['artifacts']
            assert set(artifact['handler_templates']) == set(config['handlers'].values())
            assert artifact['values']['ExactCounter']['value'] == 9007199254740993
            definition = artifact['handler_templates'][config['handlers']['selected']]['definition_digest']
            invoke('execute_artifact', artifact=str(seed)); initial = inspect()
            old_evidence = initial['heads']['evidence']; old_installation = initial['heads']['installation']
            assert initial['events'] == 2
            old = query(graphs['installation'], old_installation, include_metadata=True)
            assert next(n for n in old['graph']['nodes'] if n['id']=='device')['properties']['serial'] == 9007199254740993
            assert old['graph']['attachments'][0]['value']['reference']['revision'] == old_evidence
            invoke('install', artifact=str(handler))
            first = poll('diagnostic')
            preparation = invoke('prepare', adapter=adapters['diagnostic'], event=first['id'], lease=first['lease'])
            invoke('complete', adapter=adapters['diagnostic'], event=first['id'], lease=first['lease'], preparation=preparation['preparation_id'])
            empty_warning = inspect()['heads']['warnings']
            assert query(graphs['warnings'], empty_warning)['graph']['edges'] == []
            invoke('install_cluster')
            # Empty consumed snapshots stay protected and retain scoped navigation coverage.
            empty_event = poll('cluster')
            empty_recipe = compile_source('cluster', {'WARNING_REVISION_JSON':empty_event['graph']['revision']})
            empty_record = invoke('cluster_prepare', adapter=adapters['cluster'], event=empty_event['id'], lease=empty_event['lease'], artifact=str(empty_recipe))['record_id']
            empty_receipt = invoke('cluster_complete', record=empty_record, lease=empty_event['lease'])
            if args.upgrade_host:
                with database(db) as c:
                    assert c.execute('PRAGMA user_version').fetchone()[0] == 19
                    history_before = c.execute('SELECT * FROM head_observations ORDER BY rowid').fetchall()
                with database(journal) as c:
                    journal_before = c.execute('SELECT * FROM retained ORDER BY id').fetchall()
                active_host = args.upgrade_host
                historical = invoke('cluster_complete', record=empty_record, lease='expired-lease')
                assert historical == {**empty_receipt, 'duplicate':True}
                with database(db) as c:
                    assert c.execute('PRAGMA user_version').fetchone()[0] == 20
                    assert c.execute('SELECT * FROM head_observations ORDER BY rowid').fetchall() == history_before
                with database(journal) as c:
                    assert c.execute('SELECT * FROM retained ORDER BY id').fetchall() == journal_before
            offline = compile_source('offline', {'EVIDENCE_REVISION_JSON':old_evidence, 'INSTALLATION_REVISION_JSON':old_installation})
            before = inspect(); invoke('execute_artifact', artifact=str(offline)); changed = inspect()
            assert changed['events'] == before['events'] + 2
            assert changed['heads']['evidence'] != old_evidence and changed['heads']['installation'] != old_installation
            assert query(graphs['installation'],old_installation,include_metadata=True)['graph'] == old['graph']
            current = query(graphs['installation'],changed['heads']['installation'],include_metadata=True)
            assert current['graph']['attachments'][0]['value']['reference']['revision'] == changed['heads']['evidence']
            # Re-executing this exact logical batch reuses its immutable receipt.
            invoke('execute_artifact', artifact=str(offline))
            assert inspect() == changed
            event = poll('diagnostic'); event_args = {'adapter':adapters['diagnostic'], 'event':event['id'], 'lease':event['lease']}
            before = inspect()
            invoke('prepare', expect=94, kill_before_commit=True, **event_args)
            assert inspect() == before
            invoke('prepare', expect=92, kill_after_commit=True, **event_args)
            prepared = invoke('prepare', **event_args)
            assert prepared['duplicate']
            invoke('raw', program=program(), error=['E_HANDLER_BOUND'], **event_args)
            invoke('complete', expect=94, kill_before_commit=True, preparation=prepared['preparation_id'], **event_args)
            assert inspect() == before
            invoke('complete', expect=92, kill_after_commit=True, preparation=prepared['preparation_id'], **event_args)
            once = inspect()
            receipt = invoke('complete', preparation=prepared['preparation_id'], **event_args)
            assert receipt['duplicate'] and once['events'] == before['events']+1 and inspect()==once
            invoke('raw', program=program(), error=['E_HANDLER_BOUND'], **event_args)
            warnings = query(graphs['warnings'],once['heads']['warnings'])
            assert len(warnings['graph']['edges']) == 1
            warning = warnings['graph']['edges'][0]
            assert warning['predicate'] == config['warning_predicate']
            assert {p['assertion_id'] for p in warning['derived_from']} >= {'negative-measurement', config['selection']['attachment_id']}
            denied = execute(program({'op':'evaluate','value':{'kind':'query','query':{'graph_id':graphs['warnings'],'revision':once['heads']['warnings']}}}), reviewer=True)[0]['result']
            assert denied['graph']['edges'] == [] and denied['graph']['nodes'] == [], denied
            cluster_event = poll('cluster'); cluster_args = {'adapter':adapters['cluster'], 'event':cluster_event['id'], 'lease':cluster_event['lease']}
            cluster = compile_source('cluster', {'WARNING_REVISION_JSON':cluster_event['graph']['revision']})
            before = inspect()
            invoke('cluster_prepare', expect=94, kill_before_commit=True, artifact=str(cluster), **cluster_args)
            with database(journal) as c: assert c.execute('SELECT count(*) FROM retained').fetchone()[0] == 1
            invoke('cluster_prepare', expect=92, kill_after_commit=True, artifact=str(cluster), **cluster_args)
            prepared_cluster = invoke('cluster_prepare', artifact=str(cluster), **cluster_args)
            assert prepared_cluster['duplicate']; record_id = prepared_cluster['record_id']
            with database(journal) as c:
                body,bundle = c.execute('SELECT body,bundle FROM retained WHERE id=?',(record_id,)).fetchone()
            retained = json.loads(body)['retained']
            assert bundle == cluster.read_bytes()
            assert retained['recipe'] == compilations['cluster']['artifacts']['program']
            assert retained['result']['coverage'] == 'partial'
            assert retained['event'] == cluster_event['graph'] and retained['completion']['commands'][0]['expected_head'] == before['heads']['clusters']
            assert {p['graph_id'] for p in retained['closure']} >= {graphs['installation'],graphs['evidence'],graphs['warnings']}
            invoke('cluster_complete', record=record_id, lease=cluster_event['lease'], reviewer=True, error=['E_HOST_AUTH'])
            invoke('cluster_complete', record=record_id, lease=cluster_event['lease'], narrowed=True, error=['E_HOST_AUTH'])
            invoke('cluster_complete', expect=94, kill_before_commit=True, record=record_id, lease=cluster_event['lease'])
            assert inspect() == before
            invoke('cluster_complete', expect=92, kill_after_commit=True, record=record_id, lease=cluster_event['lease'])
            once = inspect()
            duplicate = invoke('cluster_complete', record=record_id, lease='expired-lease')
            assert duplicate['duplicate'] and once['events']==before['events']+1 and inspect()==once
            invoke('cluster_complete', record=record_id, lease='expired-lease', narrowed=True, error=['E_HOST_AUTH'])
            cluster_value = query(graphs['clusters'], once['heads']['clusters'])
            assert any(n['space_id']=='weave:navigation' for n in cluster_value['graph']['nodes'])
            if args.peer_host:
                ref = {'graph_id':graphs['clusters'],'revision':once['heads']['clusters']}
                annotation = program({'op':'commit','graph_id':graphs['annotations'],'data':{'nodes':[{'id':'secret-note','entity_id':'reviewer-only','space_id':'private','readers':[config['reviewer']]}]}})
                execute(annotation)
                # Working peer already has a concurrent organization; integration preserves it.
                peer_call('W','execute',program=program({'op':'commit','graph_id':graphs['clusters'],'data':{'nodes':[{'id':'work-only','entity_id':'work-only','space_id':'work'}]}}))
                work_head = peer_call('W','head',graph=graphs['clusters'],branch='main')['revision']
                total_bytes = 0
                for sender,receiver,branch in [('P','W','main'),('W','T','phone-import')]:
                    export_args = {'reference':ref,'recipient':receiver,'branch':branch,'nonce':variant+'-'+sender+'-'+receiver}
                    if sender=='P':
                        peer_call(sender,'export_signed',expect=95,kill_before_commit=True,**export_args)
                        peer_call(sender,'export_signed',expect=96,kill_after_commit=True,**export_args)
                    exported = peer_call(sender,'export_signed',**export_args)
                    retried = peer_call(sender,'export_signed',**export_args)
                    assert retried['response']['duplicate'] and retried['response']['result']==exported['response']['result']
                    tampered = copy.deepcopy(exported); tampered['response']['result']['binding']['served_at_ms']+=1
                    assert peer_call(receiver,'verify_export',expect=1,server=sender,bundle=tampered)['code'] in ['E_SIGNATURE','E_EXPORT_BINDING']
                    capsule = peer_call(receiver,'verify_export',server=sender,bundle=exported)['capsule']
                    assert graphs['annotations'] not in {r['graph_id'] for r in capsule['revisions']}
                    assert {graphs['evidence'],graphs['installation'],graphs['warnings'],graphs['clusters']} <= {r['graph_id'] for r in capsule['revisions']}
                    total_bytes += len(encoded(capsule))
                    proposal = peer_call(receiver,'propose',capsule=capsule,nonce=variant+'-proposal-'+receiver)
                    assert peer_call(receiver,'propose',capsule=capsule,nonce=variant+'-proposal-'+receiver)['admitted']['duplicate']
                    target_branch = 'phone-import' if receiver=='W' else 'main'
                    integration = {'proposal_id':proposal['admitted']['result']['id'],'branch_id':target_branch,'expected_head':None,'nonce':variant+'-integrate-'+receiver}
                    peer_call(receiver,'integrate',proof=proposal['proof'],request=integration)
                    peer_call(receiver,'integrate',proof=proposal['proof'],request=integration)
                    # Receiving bytes does not authorize re-export. Explicitly retain the
                    # needed dependency heads under this host's separate local authority.
                    for graph in sorted({r['graph_id'] for r in capsule['revisions']} - {ref['graph_id']}):
                        rows = [r for r in capsule['revisions'] if r['graph_id']==graph]
                        parents = {r.get('parent') for r in rows}
                        tips = [r for r in rows if r['revision'] not in parents]
                        assert len(tips)==1, (graph,tips)
                        peer_call(receiver,'accept_revision',reference={'graph_id':graph,'revision':tips[0]['revision']},branch=target_branch,expected=None)
                assert peer_call('W','head',graph=graphs['clusters'],branch='main')['revision']==work_head

                if args.accepted_history:
                    with database(peer_dbs['T']) as c:
                        observer_T = c.execute('SELECT source FROM engine_identity WHERE id=1').fetchone()[0]
                        received_at = c.execute('SELECT recorded_at_ms FROM head_observations WHERE graph_id=? ORDER BY rowid DESC LIMIT 1',(ref['graph_id'],)).fetchone()[0]
                    peer_call('T','accepted_history',view=variant+'-team',cut={'kind':'at_time','observer':observer_T,'unix_millis':20},expect=1)
                    peer_clocks['T'] = 30
                decision = peer_call('T','govern',source=ref,view=variant+'-team',id=variant+'-accept',branch='main')
                chosen = {'view_id':variant+'-team','decision_id':decision['decision_id']}
                accepted = peer_call('T','accepted',selection=chosen)
                assert accepted['graph']['nodes']
                if args.accepted_history:
                    with database(peer_dbs['T']) as c:
                        accepted_at = c.execute('SELECT accepted_at_ms FROM governance_decisions WHERE id=?',(decision['decision_id'],)).fetchone()[0]
                    assert received_at == 20 and accepted_at == 30
                    peer_call('T','accepted_history',view=variant+'-team',cut={'kind':'at_time','observer':observer_T,'unix_millis':25},expect=1)
                    history = peer_call('T','accepted_history',view=variant+'-team',cut={'kind':'at_time','observer':observer_T,'unix_millis':30})
                    exact = peer_call('T','accepted_history',view=variant+'-team',cut={'kind':'decision','observer':observer_T,'decision_id':decision['decision_id']})
                    assert history == exact and history['result'] == accepted
                    assert history['observation']['accepted_at_ms'] == accepted_at and history['observation']['source'] == ref
                    peer_call('T','accepted_history',view=variant+'-team',cut={'kind':'decision','observer':observer_T,'decision_id':decision['decision_id']},outsider=True,expect=1)
                    with database(peer_dbs['P']) as c:
                        observer_P = c.execute('SELECT source FROM engine_identity WHERE id=1').fetchone()[0]
                    peer_call('T','accepted_history',view=variant+'-team',cut={'kind':'decision','observer':observer_P,'decision_id':decision['decision_id']},expect=1)

                assert peer_call('T','accepted',selection=chosen,outsider=True,expect=1)['code']=='E_GOV_UNAVAILABLE'
                explanation = peer_call('T','execute',program=program({'op':'evaluate','value':{'kind':'explain','input':{'kind':'accepted_graph','selection':chosen}}}))[0]['result']
                assert explanation['graph']['nodes'] and {graphs['evidence'],graphs['installation'],graphs['warnings']} <= {p['graph_id'] for p in explanation['input_snapshots']}
                gate = next(p for p in accepted['graph']['influence']['assertions'] if p['graph_id'].startswith('weave:governance:decision:'))
                peer_call('T','install_adapter',id='maintenance',subscriptions=[{'graph_id':gate['graph_id'],'branch_id':'main'}],outputs=[])
                delivery = peer_call('T','poll',id='maintenance')
                intent = peer_call('T','effect_request',id='maintenance',event=delivery['id'],lease=delivery['lease'],key=variant+'-once',payload={'decision':decision['decision_id'],'source':ref})
                destination = work/'sink.jsonl'
                peer_call('T','effect_send_and_lose_response',expect=93,intent=intent['id'],destination_file=str(destination))
                assert peer_call('T','effect_status',intent=intent['id'])['state']=='unknown'
                assert peer_call('T','effect_begin',expect=1,intent=intent['id'])['code']=='E_EFFECT_UNKNOWN'
                observed = [json.loads(line) for line in destination.read_text().splitlines()]
                assert len(observed)==1
                peer_call('T','effect_reconcile',intent=intent['id'],evidence={'receipt':observed[0]})
                assert peer_call('T','effect_status',intent=intent['id'])['state']=='confirmed'
                assert peer_call('T','accepted',selection=chosen,fixture_clock_ms=10000,expect=1)['code']=='E_GOV_UNAVAILABLE'
                # Update only for the host-authored private annotation event above.
                once = inspect()
                variants_peer = {'processes':len(peer_trace),'transferred_bytes':total_bytes,'decision_id':decision['decision_id'],'effect_id':intent['id'],'sink_actions':len(observed),'trace':peer_trace}
            else:
                variants_peer = None
            # Independent journal mutation cannot drop exact semantic closure.
            altered = json.loads(body); altered['retained']['closure']=[]
            with database(journal) as c:
                c.execute('UPDATE retained SET body=? WHERE id=?',(encoded(altered),record_id))
            invoke('cluster_complete', record=record_id, lease='expired-lease', error=['E_HOST_JOURNAL_INTEGRITY'])
            with database(journal) as c: c.execute('UPDATE retained SET body=? WHERE id=?',(body,record_id))
            # Rehashing a structurally altered journal is still rejected by the
            # kernel's reconstructed semantic closure before a historical receipt.
            def journal_hash(record):
                return 'sha256:' + sha(encoded(['weave-source-v1',['weave-native-cluster-record-v1',record]]))
            assert journal_hash(json.loads(body)) == record_id
            altered_id = journal_hash(altered)
            with database(journal) as c:
                c.execute('UPDATE retained SET id=?,body=? WHERE id=?',(altered_id,encoded(altered),record_id))
            invoke('cluster_complete', record=altered_id, lease='expired-lease', error=['E_RETAINED_CLUSTER'])
            with database(journal) as c:
                c.execute('UPDATE retained SET id=?,body=? WHERE id=?',(record_id,body,altered_id))
            # Remove one actual exact premise independently of the native process.
            # Even an existing completion receipt cannot stand in for unavailable input.
            with database(db) as c:
                missing = c.execute('SELECT * FROM revisions WHERE revision=?',(changed['heads']['evidence'],)).fetchone()
                c.execute('DELETE FROM revisions WHERE revision=?',(changed['heads']['evidence'],))
            invoke('cluster_complete', record=record_id, lease='expired-lease', error=['E_RETAINED_CLUSTER','E_UNAVAILABLE','E_INTEGRITY','E_DEPENDENCY_UNAVAILABLE'])
            with database(db) as c: c.execute('INSERT INTO revisions VALUES(?,?,?,?,?,?)',missing)

            # A receipt without its journal must never cause regenerated cluster output.
            with database(journal) as c: c.execute('DELETE FROM retained WHERE id=?',(record_id,))
            invoke('cluster_prepare', artifact=str(cluster), error=['E_HOST_JOURNAL_MISSING'], **cluster_args)
            assert inspect()==once
            with database(db) as c: store_marker = c.execute('PRAGMA user_version').fetchone()[0]
            variants.append({'store_marker':store_marker,'variant':variant,'definition_digest':definition,'seed_pins':{'evidence':old_evidence,'installation':old_installation},'offline_pins':changed['heads'],'warning_pin':once['heads']['warnings'],'cluster_pin':once['heads']['clusters'],'cluster_record_id':record_id,'retained_bytes':len(body)+len(bundle),'journal_body_sha256':sha(body),'receipt_sha256':sha(encoded(duplicate['results'])),'events':once['events'],'peer_continuation':variants_peer})
    assert len(variants)==2 and variants[0]['definition_digest'] != variants[1]['definition_digest']
    assert variants[0]['store_marker'] == variants[1]['store_marker']
    report = {'profile':'native-compiled-scenario-bc/1' if args.peer_host else 'native-compiled-scenario-b/1','status':'passed','protocol':cluster_value['version'],'source_protocol':compilations['seed']['artifacts']['program']['version'],'upgraded_from_store19':bool(args.upgrade_host),'native_accepted_history':bool(args.accepted_history),'store_marker':variants[0]['store_marker'],'variants':variants,'runtime_processes':len(trace)+sum(v['peer_continuation']['processes'] if v['peer_continuation'] else 0 for v in variants),'facade_processes':len(trace),'peer_processes':sum(v['peer_continuation']['processes'] if v['peer_continuation'] else 0 for v in variants),'compiler_processes':len(compiler_trace),'controlled_deaths':sum(t['exit'] in [92,94] for t in trace)+sum(sum(t['exit'] in [93,95,96] for t in v['peer_continuation']['trace']) if v['peer_continuation'] else 0 for v in variants),'seconds':round(time.monotonic()-started,3),'maximum_child_rss_native_units':resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss if resource else None,'child_rss_units':'bytes' if sys.platform=='darwin' else 'KiB' if resource else None,'child_rss_scope':'controller child processes including compiler; excludes simulator grandchildren','checks':['real complete SDK artifacts with exact integers and changed pinned modules','atomic offline source rebind and immutable old history','compiled negative-evidence handler, raw-bypass rejection and private reader denial','handler and host-journal preparation/completion deaths before and after commit','empty and nonempty retained clusters preserve scoped partial coverage and whole-input gates','immutable exact CAS/body/pins and historical receipts','narrowed/foreign authority, rehashed trimmed closure, missing exact premise and missing journal fail closed'],'limits':['trusted native process and host journal; no untrusted adapter isolation','native signed whole-capsule P/W/T continuation; no network service or selective disclosure' if args.peer_host else 'single offline store; signed transfer/governance/effect continuation remains separate','cluster navigation stays scoped Partial; no global coverage or incremental claim','small deterministic fixture; browser/mobile scenario and resource-scale gates remain open'],'peer_checks':['signed whole-closure export with pre/postcommit death and exact reply reuse','tamper denial, isolated proposal and explicit dependency retention','concurrent workstation organization unchanged','genuine team acceptance and source-backed historical explanation','private annotation omitted, reviewer denied and current policy expiry','one unknown effect fence and explicit destination reconciliation'] + (['replica receipt precedes genuine governed acceptance; actual SQL-observed date/decision selection with outsider and foreign observer denial'] if args.accepted_history else []) if args.peer_host else [],'compiler_trace':compiler_trace,'trace':trace}
    if args.report: args.report.write_bytes(json.dumps(report,indent=2).encode()+b'\n')
    print(json.dumps({k:v for k,v in report.items() if k not in ['trace','compiler_trace','variants']},indent=2))

if __name__=='__main__':
    if len(sys.argv)>1 and sys.argv[1]=='--compile-child':
        compile_child(Path(sys.argv[2]),Path(sys.argv[3]),Path(sys.argv[4]))
    else: main()
