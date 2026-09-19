#!/usr/bin/env python3
"""Real Swift/native-host restart acceptance; optional dedicated iOS simulator run."""
import argparse
import json
import os
from pathlib import Path
import plistlib
import subprocess
import tempfile

ROOT=Path(__file__).resolve().parents[1]
p=argparse.ArgumentParser()
p.add_argument('--ios',action='store_true')
p.add_argument('--simctl',default='xcrun simctl',help='simctl executable or xcrun simctl')
p.add_argument('--runtime',help='installed simulator runtime identifier; required for --ios')
p.add_argument('--device-type',help='installed device type identifier; required for --ios')
p.add_argument('--report',type=Path)
a=p.parse_args()
if a.ios and not (a.runtime and a.device_type):p.error('--ios requires installed --runtime and --device-type')

def run(command,timeout=180,env=None):
    r=subprocess.run(command,cwd=ROOT,text=True,capture_output=True,timeout=timeout,env=env)
    if r.returncode:raise RuntimeError(f'{command[0]} failed ({r.returncode}): {r.stderr[-6000:]} {r.stdout[-1000:]}')
    return r.stdout

simctl=['xcrun','simctl'] if a.simctl=='xcrun simctl' else [a.simctl]
report={'host':'ios-simulator' if a.ios else 'macos-native','stages':[]}
with tempfile.TemporaryDirectory(prefix='weave-native-') as directory:
    work=Path(directory)
    if a.ios:
        target='aarch64-apple-ios-sim'
        env=dict(os.environ,IPHONEOS_DEPLOYMENT_TARGET='17.0')
        run(['cargo','build','--release','--locked','-p','weave-native','--target',target],env=env)
        sdk=run(['xcrun','--sdk','iphonesimulator','--show-sdk-path']).strip()
        app=work/'WeaveProbe.app';app.mkdir()
        executable=app/'WeaveProbe'
        run(['swiftc','-target','arm64-apple-ios17.0-simulator','-sdk',sdk,'-import-objc-header',str(ROOT/'crates/weave-native/include/weave_native.h'),str(ROOT/'hosts/swift/Weave.swift'),str(ROOT/'hosts/swift/Probe.swift'),str(ROOT/f'target/{target}/release/libweave_native.a'),'-framework','UIKit','-framework','Foundation','-framework','Security','-o',str(executable)])
        (app/'Info.plist').write_bytes(plistlib.dumps({'CFBundleIdentifier':'org.weave-graph.acceptance','CFBundleName':'Weave Acceptance','CFBundleExecutable':'WeaveProbe','CFBundlePackageType':'APPL','CFBundleVersion':'1','CFBundleShortVersionString':'0.1','MinimumOSVersion':'17.0','LSRequiresIPhoneOS':True,'CFBundleSupportedPlatforms':['iPhoneSimulator'],'UILaunchScreen':{},'UISupportedInterfaceOrientations':['UIInterfaceOrientationPortrait']}))
        run(['codesign','--force','--sign','-',str(app)])
        simulator=run(simctl+['create','Weave Acceptance',a.device_type,a.runtime]).strip()
        try:
            run(simctl+['boot',simulator],timeout=30)
            run(simctl+['bootstatus',simulator,'-b'],timeout=180)
            run(simctl+['install',simulator,str(app)],timeout=60)
            report['runtime']=a.runtime;report['device_type']=a.device_type
            for stage in ['seed','read','rollback','read']:
                output=run(simctl+['launch','--console',simulator,'org.weave-graph.acceptance',stage],timeout=60)
                rows=[json.loads(line) for line in output.splitlines() if line.startswith('{')]
                if len(rows)!=1:raise RuntimeError('simulator probe returned no unique result')
                report['stages'].append(rows[0])
        finally:
            subprocess.run(simctl+['shutdown',simulator],capture_output=True,timeout=30)
            run(simctl+['delete',simulator],timeout=30)
    else:
        run(['cargo','build','--release','--locked','-p','weave-native'])
        executable=work/'weave-swift-probe'
        run(['swiftc','-import-objc-header',str(ROOT/'crates/weave-native/include/weave_native.h'),str(ROOT/'hosts/swift/Weave.swift'),str(ROOT/'hosts/swift/Probe.swift'),str(ROOT/'target/release/libweave_native.a'),'-framework','Foundation','-framework','Security','-o',str(executable)])
        for stage in ['seed','read','rollback','read']:
            report['stages'].append(json.loads(run([str(executable),str(work/'database'),stage],timeout=30)))
assert report['stages'][0]['seeded']
for row in report['stages'][1:]:
    assert row['restarted_persistence'] and row['pinned_history']
    assert (row['alice_nodes'],row['bob_nodes'],row['exact_decimal'])==(2,1,'9007199254740993')
assert report['stages'][2]['rollback']
report['result']='passed'
report['limits']=['no network used; networking not disabled','simulator is not a physical device','local persistence/privacy/rollback profile, not full sync/governance/mobile app acceptance']
encoded=json.dumps(report,indent=2)+'\n'
if a.report:a.report.write_text(encoded)
print(encoded)
