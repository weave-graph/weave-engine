#!/usr/bin/env python3
"""Run the source-backed scenario in an existing booted iOS simulator.
Does not build, install an app, create a simulator or change its power state.
"""
import argparse
import json
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile

p = argparse.ArgumentParser()
p.add_argument('--simulator', required=True)
p.add_argument('--compiler-sdk', type=Path, required=True)
p.add_argument('--host', type=Path, required=True, help='aarch64-apple-ios-sim native_scenario executable')
p.add_argument('--peer-host', type=Path, required=True, help='aarch64-apple-ios-sim three_peer_trace executable')
p.add_argument('--fixtures', type=Path, required=True)
p.add_argument('--report', type=Path, required=True)
p.add_argument('--evidence-dir', type=Path)
a = p.parse_args()
state = json.loads(subprocess.check_output(['xcrun', 'simctl', 'list', 'devices', '--json']))
matches = [(runtime, device) for runtime, devices in state['devices'].items() for device in devices if device['udid'] == a.simulator]
assert len(matches) == 1 and matches[0][1]['state'] == 'Booted', 'select an existing booted simulator'
with tempfile.TemporaryDirectory(prefix='weave-ios-controller-') as temp:
    root = Path(temp)
    for name, binary in [('native_scenario', a.host), ('three_peer_trace', a.peer_host)]:
        wrapper = root / name
        argv = ['xcrun', 'simctl', 'spawn', a.simulator, str(binary.resolve())]
        wrapper.write_text('#!/bin/sh\nexec ' + shlex.join(argv) + ' "$@"\n')
        wrapper.chmod(0o755)
    args = [sys.executable, str(Path(__file__).with_name('check_native_scenario.py')),
            '--compiler-sdk', str(a.compiler_sdk.resolve()), '--host', str(root / 'native_scenario'),
            '--peer-host', str(root / 'three_peer_trace'), '--fixtures', str(a.fixtures.resolve()),
            '--report', str(a.report.resolve())]
    if a.evidence_dir:
        args += ['--evidence-dir', str(a.evidence_dir.resolve())]
    subprocess.run(args, check=True)
report = json.loads(a.report.read_bytes())
report['execution_host'] = {'profile': 'ios-simulator-rust-process', 'runtime': matches[0][0],
                            'device': matches[0][1]['name'], 'target': 'aarch64-apple-ios-sim',
                            'compiler_host': sys.platform}
report['limits'] += ['simctl-spawned Rust processes and host-owned temporary SQLite files; no application sandbox/container',
                     'compiler SDK runs on the host; identical source artifacts are executed by simulator-target binaries',
                     'process-exit faults are not OS power loss, device energy, mobile app lifecycle or physical-device acceptance',
                     'controller child RSS does not measure simulator engine RSS']
a.report.write_bytes(json.dumps(report, indent=2).encode() + b'\n')
print(json.dumps({'execution_host': report['execution_host'], 'status': report['status']}))
