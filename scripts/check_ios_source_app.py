#!/usr/bin/env python3
"""Build a Swift test app against an existing native static library and run it in
an existing booted simulator. No Cargo, simulator creation or power-state change.
Only the uniquely named app installed by this invocation is uninstalled.
"""
import argparse
import hashlib
import json
from pathlib import Path
import plistlib
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]


def encoded(value):
    return json.dumps(value, ensure_ascii=False, separators=(',', ':')).encode()


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def run(argv, timeout=60):
    result = subprocess.run([str(a) for a in argv], capture_output=True, timeout=timeout)
    assert result.returncode == 0, (argv[0], result.returncode, result.stdout[-2000:], result.stderr[-5000:])
    return result.stdout


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--simulator', required=True, help='existing booted simulator UUID')
    parser.add_argument('--library', type=Path, required=True, help='aarch64-apple-ios-sim libweave_native.a')
    parser.add_argument('--compiler-sdk', type=Path, required=True, help='native host compiler SDK')
    parser.add_argument('--fixtures', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--evidence-dir', type=Path, required=True)
    args = parser.parse_args()
    simctl = ['xcrun', 'simctl']
    state = json.loads(run(simctl + ['list', 'devices', '--json']))
    matches = [(runtime, device) for runtime, devices in state['devices'].items()
               for device in devices if device['udid'] == args.simulator]
    assert len(matches) == 1 and matches[0][1]['state'] == 'Booted', 'use an existing booted simulator'
    args.evidence_dir.mkdir(parents=True, exist_ok=True)
    trace, compiles, variants = [], [], []
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix='weave-ios-source-app-') as temp:
        work = Path(temp)
        sdk = run(['xcrun', '--sdk', 'iphonesimulator', '--show-sdk-path']).decode().strip()
        executable = work / 'SourceProbe'
        run(['xcrun', 'swiftc', '-target', 'arm64-apple-ios17.0-simulator', '-sdk', sdk,
             '-import-objc-header', ROOT / 'crates/weave-native/include/weave_host.h',
             ROOT / 'hosts/swift/WeaveHost.swift', ROOT / 'hosts/swift/SourceProbe.swift',
             args.library.resolve(), '-framework', 'UIKit', '-framework', 'Foundation',
             '-framework', 'Security', '-o', executable], timeout=120)
        executable_digest = sha(executable.read_bytes())
        for variant in ['base', 'renamed']:
            directory = args.fixtures.resolve() / variant
            config = json.loads((directory / 'manifest.json').read_bytes())
            graphs = config['graphs']
            evidence = args.evidence_dir / variant
            evidence.mkdir(exist_ok=True)
            app = work / (variant + '.app')
            app.mkdir()
            shutil.copy2(executable, app / 'SourceProbe')
            identity = 'org.weave-graph.source-acceptance.' + uuid.uuid4().hex
            (app / 'Info.plist').write_bytes(plistlib.dumps({
                'CFBundleIdentifier': identity, 'CFBundleName': 'Weave Source Acceptance',
                'CFBundleExecutable': 'SourceProbe', 'CFBundlePackageType': 'APPL',
                'CFBundleVersion': '1', 'CFBundleShortVersionString': '0.1',
                'MinimumOSVersion': '17.0', 'LSRequiresIPhoneOS': True,
                'CFBundleSupportedPlatforms': ['iPhoneSimulator'], 'UILaunchScreen': {},
                'UISupportedInterfaceOrientations': ['UIInterfaceOrientationPortrait']}))
            artifacts = {}

            def compile_source(kind, pins=None):
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
                        modules.append({'id': module['id'], 'revision': module['revision'], 'source': raw.decode()})
                request = evidence / (kind + '-sdk-request.json')
                response = evidence / (kind + '-sdk-response.json')
                request.write_bytes(encoded({'format': 'weave-compiler-request/1',
                                             'entry_id': variant + '/ios/' + kind, 'source': source, 'modules': modules}))
                run([sys.executable, ROOT / 'scripts/check_native_scenario.py', '--compile-child',
                     args.compiler_sdk.resolve(), request, response])
                raw = response.read_bytes()
                result = json.loads(raw)
                assert result['ok'], result
                artifacts[kind] = result['artifacts']
                compiles.append({'variant': variant, 'kind': kind, 'request_sha256': sha(request.read_bytes()),
                                 'response_sha256': sha(raw), 'response_bytes': len(raw)})
                return raw

            compile_source('seed')
            handler = compile_source('handler')
            seed_batch = artifacts['seed']['program']['commands'][0]
            old_pins = {c['graph_id']: 'logical:' + seed_batch['batch_id'] + ':' + c['graph_id'] for c in seed_batch['commits']}
            compile_source('offline', {'EVIDENCE_REVISION_JSON': old_pins[graphs['evidence']],
                                       'INSTALLATION_REVISION_JSON': old_pins[graphs['installation']]})
            template = artifacts['handler']['handler_templates'][config['handlers']['selected']]
            manifest = {'id': config['adapters']['diagnostic'], 'version': '1', 'artifact_digest': template['definition_digest'],
                        'config_revision': '1', 'principal': config['principal'],
                        'subscriptions': [{'graph_id': template['input']['graph_id'], 'branch_id': 'main'}],
                        'output_graphs': [graphs['warnings']], 'effect_destinations': [], 'max_attempts': 20,
                        'lease_ms': 3600000, 'max_pending_events': 100, 'projection_replay': True}
            install = {'name': config['handlers']['selected'], 'manifest': manifest,
                       'output': {'slot': config['output_slot'], 'graph_id': graphs['warnings'], 'branch_id': 'main'}}
            (app / 'fixture.json').write_bytes(encoded(config))
            (app / 'authority.json').write_bytes(encoded({'principal': config['principal'], 'writable_graphs': list(graphs.values())}))
            (app / 'reviewer.json').write_bytes(encoded({'principal': config['reviewer'], 'writable_graphs': []}))
            (app / 'install.json').write_bytes(encoded(install))
            (app / 'handler-sdk.json').write_bytes(handler)

            def request(program):
                return encoded({'format': 'weave-host-request/1', 'operation': {'kind': 'execute', 'program': program}})

            # Python retains exact integer values when encoding these compiler
            # Programs; Swift copies UTF-8 request/response bytes without reencoding.
            for kind in ['seed', 'offline']:
                raw = request(artifacts[kind]['program'])
                (app / (kind + '-request.json')).write_bytes(raw)
                (evidence / (kind + '-host-request.json')).write_bytes(raw)
            for name, graph, revision in [
                    ('old-installation', graphs['installation'], old_pins[graphs['installation']]),
                    ('current-installation', graphs['installation'], None),
                    ('warnings', graphs['warnings'], None), ('clusters', graphs['clusters'], None)]:
                query = {'graph_id': graph, 'include_metadata': True}
                if revision:
                    query['revision'] = revision
                raw = request({'version': '0.19.0', 'commands': [{'op': 'evaluate', 'value': {'kind': 'query', 'query': query}}]})
                (app / (name + '-request.json')).write_bytes(raw)
            (app / 'rollback-request.json').write_bytes(request({'version': '0.19.0', 'commands': [
                {'op': 'commit', 'graph_id': graphs['clusters'], 'data': {'nodes': [], 'edges': []}},
                {'op': 'commit', 'graph_id': graphs['installation'], 'expected_head': 'stale', 'data': {'nodes': [], 'edges': []}}]}))
            run(['codesign', '--force', '--sign', '-', app])
            installed = False
            data = None
            try:
                run(simctl + ['install', args.simulator, app])
                installed = True
                container = Path(run(simctl + ['get_app_container', args.simulator, identity, 'data']).decode().strip())
                data = container / 'Library/Application Support/WeaveSourceProbe'
                for stage in ['seed', 'offline', 'offline-duplicate', 'poll', 'prepare-lost', 'prepare',
                              'complete-lost', 'complete', 'read', 'reviewer', 'foreign', 'rollback']:
                    before = time.monotonic()
                    result = subprocess.run(simctl + ['launch', '--console', args.simulator, identity, stage], capture_output=True, timeout=60)
                    (evidence / (stage + '-stdout.txt')).write_bytes(result.stdout)
                    (evidence / (stage + '-stderr.txt')).write_bytes(result.stderr)
                    lost = stage.endswith('-lost')
                    if lost:
                        assert result.returncode in [0, 92], (stage, result.returncode, result.stderr)
                        assert not (data / (stage + '-report.json')).exists(), 'lost response must not release an app report'
                    else:
                        assert result.returncode == 0, (stage, result.returncode, result.stdout, result.stderr)
                        assert (data / (stage + '-report.json')).exists(), (stage, result.stdout, result.stderr)
                        report = json.loads((data / (stage + '-report.json')).read_bytes())
                        assert report['stage'] == stage and report['status'] == 'passed', report
                    trace.append({'variant': variant, 'stage': stage, 'exit': result.returncode,
                                  'seconds': round(time.monotonic() - before, 6), 'stdout_sha256': sha(result.stdout),
                                  'stderr_sha256': sha(result.stderr), 'lost_response': lost})
                def outcome(name):
                    value = json.loads((data / (name + '.json')).read_bytes())
                    assert value['ok']
                    return value['value']
                prepared = outcome('prepare-preparation')
                receipt = outcome('complete-completion')
                assert prepared['duplicate'] and receipt['duplicate']
                inventory = outcome('seed-inventory')
                assert inventory['selected'] == json.loads(handler)
                assert set(inventory['inventory']['handler_templates']) == set(config['handlers'].values())
                assert artifacts['handler']['values']['ExactCounter']['value'] == 9007199254740993
                old = outcome('read-old-installation')[0]['result']
                current = outcome('read-current-installation')[0]['result']
                warning = outcome('read-warnings')[0]['result']
                denied = outcome('reviewer-warnings')[0]['result']
                assert next(n for n in old['graph']['nodes'] if n['id'] == 'device')['properties']['serial'] == 9007199254740993
                assert current['graph']['attachments'][0]['value']['reference']['revision'] != old['graph']['attachments'][0]['value']['reference']['revision']
                assert len(warning['graph']['edges']) == 1 and warning['graph']['edges'][0]['predicate'] == config['warning_predicate']
                assert {p['assertion_id'] for p in warning['graph']['edges'][0]['derived_from']} >= {'negative-measurement', config['selection']['attachment_id']}
                assert not denied['graph']['nodes'] and not denied['graph']['edges']
                assert json.loads((data / 'foreign-denial.json').read_bytes())['error']['code'] == 'E_HOST_AUTH'
                assert json.loads((data / 'rollback-denial.json').read_bytes())['error']['code'] == 'E_CONFLICT'
                connection = sqlite3.connect(data / 'engine.sqlite')
                try:
                    store_marker = connection.execute('PRAGMA user_version').fetchone()[0]
                    events = connection.execute('SELECT count(*) FROM events').fetchone()[0]
                    assert events == 6
                    assert connection.execute('SELECT count(*) FROM dispatch_pending').fetchone()[0] == 0
                    assert connection.execute('SELECT count(*) FROM effect_intents').fetchone()[0] == 0
                    assert connection.execute('SELECT count(*) FROM heads WHERE graph_id=?', (graphs['clusters'],)).fetchone()[0] == 0
                finally:
                    connection.close()
                for path in data.glob('*.json'):
                    shutil.copy2(path, evidence / path.name)
                variants.append({'store_marker': store_marker, 'variant': variant, 'definition_digest': template['definition_digest'],
                                 'events': events, 'preparation_id': prepared['preparation_id'],
                                 'receipt_sha256': sha((data / 'complete-completion.json').read_bytes()),
                                 'database_bytes': (data / 'engine.sqlite').stat().st_size})
            finally:
                if data and data.exists():
                    for path in data.glob('*.json'):
                        shutil.copy2(path, evidence / path.name)
                if installed:
                    run(simctl + ['uninstall', args.simulator, identity])
    assert len({v['store_marker'] for v in variants}) == 1
    report = {'profile': 'ios-source-app/1', 'status': 'passed', 'runtime': matches[0][0],
              'device': matches[0][1]['name'], 'target': 'aarch64-apple-ios-sim', 'compiler_host': sys.platform,
              'protocol': '0.19.0', 'store_marker': variants[0]['store_marker'], 'application_processes': len(trace),
              'compiler_processes': len(compiles), 'lost_responses': sum(t['lost_response'] for t in trace),
              'seconds': round(time.monotonic() - started, 3), 'native_library_sha256': sha(args.library.read_bytes()),
              'application_executable_sha256': executable_digest, 'variants': variants, 'trace': trace, 'compiler_trace': compiles,
              'checks': ['actual source-backed offline edits in application-owned SQLite', 'atomic logical metadata rebind and original pinned history',
                         'exact integer and complete SDK artifacts across Swift/C', 'durable diagnostic prepare/complete after lost app responses',
                         'private reader and foreign adapter denial', 'atomic rollback with no ghost event'],
              'limits': ['simulator application-container profile; no physical device or OS power-loss/energy evidence',
                         'compiler SDK runs on macOS; no source compilation inside the app',
                         'cluster/signed-peer/governance/effect continuation remains the separate simulator Rust-process profile',
                         'no network used; network connectivity is not disabled', 'no app upgrade, eviction or quota-exhaustion matrix']}
    args.report.write_bytes(json.dumps(report, indent=2).encode() + b'\n')
    print(json.dumps({k: v for k, v in report.items() if k not in ['trace', 'compiler_trace', 'variants']}, indent=2))


if __name__ == '__main__':
    main()
