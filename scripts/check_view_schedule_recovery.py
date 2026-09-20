#!/usr/bin/env python3
"""Real process death for owner queue+cursor and view+manifest+ack atomicity; no build."""
import argparse
import json
import subprocess
import tempfile
from pathlib import Path
p=argparse.ArgumentParser()
p.add_argument('--probe',type=Path,default=Path('target/debug/examples/view_schedule_probe'))
a=p.parse_args()
with tempfile.TemporaryDirectory(prefix='weave-schedule-') as directory:
    db=Path(directory)/'db.sqlite'
    def run(mode,status=0):
        r=subprocess.run([str(a.probe.resolve()),str(db),mode],capture_output=True,text=True)
        assert r.returncode==status,(mode,r.returncode,r.stderr)
        return json.loads(r.stdout) if r.stdout else None
    initial=run('prepare')
    assert (initial['generation'],initial['pending'],initial['cursor'])==(1,0,1)
    changed=run('change')
    run('scan_before',90)
    failed=run('inspect')
    assert (failed['pending'],failed['cursor'],failed['generation'])==(0,1,1)
    run('scan_after',91)
    queued=run('inspect')
    assert (queued['pending'],queued['cursor'])==(1,2)
    assert run('scan')==queued
    run('tick')
    run('drain_before',92)
    failed=run('inspect')
    assert (failed['pending'],failed['generation'],failed['tick'],failed['requested'])==(1,1,0,11)
    assert failed['manifest']==initial['manifest']
    run('drain_after',93)
    done=run('inspect')
    assert (done['pending'],done['generation'],done['tick'],done['events'])==(0,2,11,2)
    assert done['manifest']['generation']==2 and done['manifest']['tick']==11
    assert done['manifest']['input_snapshots'][0]['revision']==changed['head']
    assert run('drain')=={'worked':False,'generation':None}
    assert run('inspect')==done
print('view scheduling process death: 6 queue/cursor/publication/retry checks passed')
