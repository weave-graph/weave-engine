#!/usr/bin/env python3
"""Actual process death around projection, erasure and explicit rebuild commits."""
import argparse,hashlib,json,sqlite3,subprocess,tempfile
from contextlib import closing
from pathlib import Path
from version_profile import store_marker

def snapshot(path):
    with closing(sqlite3.connect(path)) as c:
        return {name:c.execute('SELECT * FROM "'+name.replace('"','""')+'" ORDER BY rowid').fetchall()
                for (name,) in c.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")}

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--host',type=Path,required=True);p.add_argument('--report',type=Path);p.add_argument('--evidence-dir',type=Path)
    a=p.parse_args();trace=[]
    if a.evidence_dir:a.evidence_dir.mkdir(parents=True,exist_ok=False)
    with tempfile.TemporaryDirectory(prefix='weave-retention-recovery-') as tmp:
        db=Path(tmp)/'store.sqlite';request=Path(tmp)/'completion.json'
        def invoke(mode,code=0):
            r=subprocess.run([str(a.host.resolve()),str(db),mode,str(request)],capture_output=True,timeout=30)
            assert r.returncode==code,(mode,r.returncode,r.stdout,r.stderr)
            entry={'operation':mode,'exit':code,'response_sha256':hashlib.sha256(r.stdout).hexdigest()};trace.append(entry)
            if a.evidence_dir:
                stem=f'{len(trace):02d}-{mode}';(a.evidence_dir/(stem+'.stdout')).write_bytes(r.stdout);(a.evidence_dir/(stem+'.stderr')).write_bytes(r.stderr)
            return json.loads(r.stdout) if code==0 else None
        invoke('seed');invoke('next');initial=invoke('inspect');assert initial['marker']==store_marker() and initial['events']==3 and initial['output'] is None
        before=snapshot(db);invoke('complete_crash',82);assert snapshot(db)==before
        invoke('complete_after',83);completed=invoke('inspect');assert completed['events']==4 and completed['state']['state']=={'total':3}
        before=snapshot(db);assert invoke('complete')['duplicate'];assert snapshot(db)==before
        assert invoke('raw')['error']=='E_PROJECTION_STATE';assert snapshot(db)==before
        invoke('compact_crash',82);assert snapshot(db)==before
        invoke('compact_after',83);expired=invoke('inspect');assert expired['state_error']=='E_CHECKPOINT_EXPIRED' and expired['events']==4
        with closing(sqlite3.connect(db)) as c:
            assert c.execute('SELECT count(*) FROM retention_tombstones').fetchone()[0]==1
            assert c.execute("SELECT data FROM revisions WHERE graph_id='orphan'").fetchone()[0]==''
            assert c.execute("SELECT count(*) FROM revisions WHERE graph_id='input' AND data<>''").fetchone()[0]==2
        before=snapshot(db);invoke('rebase_crash',82);assert snapshot(db)==before
        invoke('rebase_after',83);rebuilt=invoke('inspect');assert rebuilt['state']['state']=={'total':2} and rebuilt['events']==4
        before=snapshot(db);assert invoke('complete')['duplicate'];assert snapshot(db)==before
        assert invoke('inspect')['state']==rebuilt['state']
        if a.evidence_dir:(a.evidence_dir/'actual-completion-request.json').write_bytes(request.read_bytes())
    report={'profile':'native-retention-projection-recovery/1','status':'passed','store_marker':store_marker(),'processes':len(trace),'controlled_deaths':6,'trace':trace,
        'checks':['actual immutable inputs and leased occurrence','state/output/receipt/checkpoint rollback before commit','exact persisted completion after lost response','raw completion cannot bypass state binding','unreachable payload erasure and policy commit together','input and receipt roots survive erasure','explicit rebuilt state and private checkpoint commit together','historical duplicate never rewinds rebuilt state'],
        'scope':'fixed trusted native worker, original opaque state supplied by host; payload-column erasure only'}
    if a.report:a.report.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report))

if __name__=='__main__':main()
