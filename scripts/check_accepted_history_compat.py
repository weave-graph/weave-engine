#!/usr/bin/env python3
"""Read real prior store20 governance with the new native selector; optional atomic store21 upgrade."""
from retention_migration import RETENTION_TABLES,retention_tables,assert_retention_baseline
from version_profile import store_marker
import argparse
from contextlib import closing
import hashlib
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile


def snapshot(path):
    with closing(sqlite3.connect(path)) as connection:
        tables = connection.execute("SELECT name,sql FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").fetchall()
        return connection.execute('PRAGMA user_version').fetchone()[0], {
            name:(sql, sorted(connection.execute('SELECT * FROM "'+name.replace('"','""')+'"').fetchall(),key=repr)) for name,sql in tables
        }


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--old-host',type=Path,required=True)
    parser.add_argument('--host',type=Path,required=True)
    parser.add_argument('--report',type=Path)
    parser.add_argument('--old-marker',type=int,default=20,choices=range(20,store_marker()+1))
    parser.add_argument('--new-marker',type=int,default=20,choices=range(20,store_marker()+1))
    parser.add_argument('--storage',type=Path)
    args=parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='weave-accepted-history-compat-') as temporary:
        root=Path(temporary);db=root/'store';cut=root/'cut.json';trace=[]
        def invoke(binary,*fields):
            result=subprocess.run([str(binary.resolve()),str(db),*map(str,fields)],capture_output=True,timeout=30)
            assert result.returncode==0,(result.stdout,result.stderr)
            trace.append({'operation':fields[0],'response_sha256':hashlib.sha256(result.stdout).hexdigest()})
            return json.loads(result.stdout)
        seeded=invoke(args.old_host,'seed')
        first=seeded['accepted']['decision_id']
        original=invoke(args.old_host,'accepted','team',first)
        before=snapshot(db);assert before[0]==args.old_marker
        if args.new_marker>args.old_marker:
            assert args.storage is not None
            for mode,code in [('crash',82),('after_commit',83)]:
                result=subprocess.run([str(args.storage.resolve()),str(db),mode,str(args.old_marker)],capture_output=True,timeout=30)
                assert result.returncode==code,(result.stdout,result.stderr)
                trace.append({'operation':mode,'exit':code,'response_sha256':hashlib.sha256(result.stdout).hexdigest()})
                after=snapshot(db)
                if mode=='crash': assert after==before
                else:
                    assert after[0]==args.new_marker and {n:after[1][n] for n in before[1]}==before[1]
                    assert_retention_baseline(db,args.new_marker)
            before=snapshot(db)
        with closing(sqlite3.connect(db)) as connection:
            observer=connection.execute('SELECT source FROM engine_identity WHERE id=1').fetchone()[0]
            accepted_at=connection.execute('SELECT accepted_at_ms FROM governance_decisions WHERE id=?',(first,)).fetchone()[0]
        def history(selection):
            cut.write_text(json.dumps(selection))
            return invoke(args.host,'accepted_history',cut,'team')
        old=history({'kind':'at_time','observer':observer,'unix_millis':accepted_at})
        assert old['observation']['decision_id']==first and old['observation']['source']==seeded['source']
        assert snapshot(db)==before
        normalized=dict(old['result']);normalized['version']=original['version'];assert normalized==original
        invoke(args.host,'change')
        after_change=snapshot(db)
        pinned=history({'kind':'decision','observer':observer,'decision_id':first})
        assert pinned==old and snapshot(db)==after_change
        newer=invoke(args.host,'publish')
        after_accept=snapshot(db)
        current=history({'kind':'at_time','observer':observer,'unix_millis':accepted_at})
        assert current['observation']['decision_id']==newer['decision_id'] and current['observation']['source']!=old['observation']['source']
        assert history({'kind':'decision','observer':observer,'decision_id':first})==old
        assert snapshot(db)==after_accept
        if args.new_marker==args.old_marker:
            historical=invoke(args.old_host,'accepted','team',first)
            assert historical==old['result'] and snapshot(db)==after_accept
        else:
            result=subprocess.run([str(args.old_host.resolve()),str(db),'accepted','team',first],capture_output=True,timeout=30)
            assert result.returncode==1 and b'E_STORAGE_VERSION' in result.stderr
            assert snapshot(db)==after_accept
            trace.append({'operation':'old-reader-refusal','exit':1,'response_sha256':hashlib.sha256(result.stderr).hexdigest()})
    report={'profile':f'populated-store{args.old_marker}-accepted-history-compat/1','status':'passed','old_marker':args.old_marker,'store_marker':args.new_marker,'checks':['actual prior runtime authored governance, approvals and protected occurrence','native date selection rewrites no original table, row or marker','source correction preserves exact accepted decision','new acceptance validates original signed quorum and persistent ordering','equal-time ancestry selects latest genuine acceptance','old runtime still reads exact original accepted result after new acceptance' if args.new_marker==args.old_marker else 'atomic migration preserves original rows and old runtime refuses the new marker'],'processes':len(trace),'trace':trace,'scope':'native fixed trusted host; optional atomic retention-store upgrade retains every original row and history'}
    if args.report:args.report.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report))


if __name__=='__main__':main()
