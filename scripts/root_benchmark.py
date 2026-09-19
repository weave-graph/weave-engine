#!/usr/bin/env python3
"""Measure the native CLI on a synthetic ring; report budget rejection honestly.

Build the release CLI first. This POSIX harness measures each child with wait4;
input generation and JSON verification are outside the child measurement.
"""
import argparse
import json
import math
import os
import platform
import statistics
import subprocess
import tempfile
import time
from pathlib import Path


def measure(command, stdout, stderr):
    with stdout.open('wb') as out, stderr.open('wb') as err:
        started = time.perf_counter()
        process = subprocess.Popen(command, stdout=out, stderr=err)
        _, status, usage = os.wait4(process.pid, 0)
        process.returncode = os.waitstatus_to_exitcode(status)
        elapsed = time.perf_counter() - started
    rss = usage.ru_maxrss if platform.system() == 'Darwin' else usage.ru_maxrss * 1024
    return {'exit_code': process.returncode, 'elapsed_seconds': elapsed,
            'peak_rss_bytes': rss, 'stdout_bytes': stdout.stat().st_size}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--revision', required=True, help='Exact verified source commit used to build the binary')
    parser.add_argument('--sizes', type=int, nargs='+', default=[10000, 100000, 1000000])
    parser.add_argument('--samples', type=int, default=5)
    args = parser.parse_args()
    if not hasattr(os, 'wait4'):
        parser.error('POSIX wait4 is required for per-process memory evidence')
    if args.samples < 1 or any(size < 4 or size % 2 for size in args.sizes):
        parser.error('positive sample count and even object counts >= 4 are required')
    binary = args.engine.resolve()
    report = {
        'build_revision': args.revision,
        'build_profile': 'cargo build --release --locked -p weave-engine',
        'profile': 'protocol 0.3 public ring with no metadata, adapters, or peers',
        'measurement': 'fresh process, JSON input, SQLite commit+query, JSON output; no build/generation time',
        'host': {'system': platform.system(), 'release': platform.release(),
                 'machine': platform.machine(), 'logical_cpus': os.cpu_count()},
        'limitations': ['synthetic single-principal workload; no production SLO',
                        'tail statistic is nearest-rank p95 of a small declared sample',
                        'budget rejection is not demonstrated capacity',
                        'RSS is the engine child only; excludes harness'],
        'cases': []}
    with tempfile.TemporaryDirectory(prefix='weave-benchmark-') as temp:
        root = Path(temp)
        for size in args.sizes:
            n = size // 2
            graph = {'nodes': [{'id': f'n{i}', 'entity_id': f'e{i}', 'space_id': 's'} for i in range(n)],
                     'edges': [{'id': f'r{i}', 'predicate': 'next', 'from': f'n{i}',
                                'to': f'n{(i+1)%n}', 'valid_time': {'start': 0, 'end': 10}}
                               for i in range(n)]}
            program = {'version': '0.3.0', 'commands': [
                {'op': 'commit', 'graph_id': 'bench', 'data': graph},
                {'op': 'query', 'query': {'graph_id': 'bench', 'valid_at': 5}}]}
            plan = root/'plan.json'
            plan.write_text(json.dumps(program, separators=(',', ':')))
            case = {'objects': size, 'nodes': n, 'edges': n, 'input_bytes': plan.stat().st_size,
                    'metadata_depth': 0, 'visibility_partitions': 1, 'samples': []}
            del program, graph
            for index in range(args.samples):
                out, err = root/'stdout.json', root/'stderr.json'
                db = root/f'{size}-{index}.db'
                sample = measure([str(binary), 'run', '--db', str(db), '--actor', 'benchmark',
                                  '--write', 'bench', str(plan)], out, err)
                if sample['exit_code']:
                    diagnostic = json.loads(err.read_text())
                    if diagnostic['code'] not in ('E_BUDGET', 'E_INPUT'):
                        raise RuntimeError(diagnostic)
                    if diagnostic['code'] == 'E_INPUT' and diagnostic['message'] != 'plan exceeds 16 MiB':
                        raise RuntimeError(diagnostic)
                    sample['diagnostic'] = diagnostic
                    sample['outcome'] = 'rejected_by_limit'
                else:
                    result = json.loads(out.read_text())[-1]['result']
                    assert result['coverage'] == 'complete', result['coverage']
                    assert len(result['graph']['nodes']) == n
                    assert len(result['graph']['edges']) == n
                    sample['outcome'] = 'passed'
                case['samples'].append(sample)
            times = sorted(s['elapsed_seconds'] for s in case['samples'])
            case['median_seconds'] = statistics.median(times)
            case['outcomes'] = sorted({s['outcome'] for s in case['samples']})
            case['p95_seconds'] = times[math.ceil(0.95*len(times))-1]
            case['max_peak_rss_bytes'] = max(s['peak_rss_bytes'] for s in case['samples'])
            report['cases'].append(case)
            print(json.dumps({k: v for k, v in case.items() if k != 'samples'}), flush=True)
    args.output.write_text(json.dumps(report, indent=2)+'\n')


if __name__ == '__main__':
    main()
