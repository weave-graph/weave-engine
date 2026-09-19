#!/usr/bin/env python3
"""Build and EXECUTE the same fixed semantic vectors as native Rust and WebAssembly."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
ROOT=Path(__file__).resolve().parents[1]
p=argparse.ArgumentParser();p.add_argument('--report',type=Path);a=p.parse_args()
def run(command):
    result=subprocess.run(command,cwd=ROOT,capture_output=True,timeout=180)
    if result.returncode:
        raise RuntimeError(result.stderr.decode(errors='replace')[-6000:])
    return result.stdout
run(['cargo','build','--locked','-p','weave-conformance','--bin','weave-conformance'])
run(['cargo','build','--locked','--release','-p','weave-conformance','--lib','--target','wasm32-unknown-unknown'])
import os
native=run([str(ROOT/'target/debug'/('weave-conformance.exe' if os.name=='nt' else 'weave-conformance'))]).rstrip(b'\n\r')
wasm_path=ROOT/'target/wasm32-unknown-unknown/release/weave_conformance.wasm'
wasm=run(['node','scripts/check_portable.mjs',str(wasm_path)])
assert native==wasm,'Native/WASM semantic output differs'
value=json.loads(native)
report={'profile':value['profile'],'contract':value['contract'],'result':'passed','cases':list(value['cases']),'output_bytes':len(native),'output_sha256':hashlib.sha256(native).hexdigest(),'wasm_sha256':hashlib.sha256(wasm_path.read_bytes()).hexdigest(),'node':run(['node','--version']).decode().strip(),'rust':run(['rustc','--version']).decode().strip(),'limits':value['limits']}
text=json.dumps(report,indent=2)+'\n'
if a.report:a.report.write_text(text)
print(text)
