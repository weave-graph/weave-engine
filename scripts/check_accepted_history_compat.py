#!/usr/bin/env python3
"""Read real prior store20 governance with the new native selector; no schema change."""
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
        before=snapshot(db);assert before[0]==20
        with closing(sqlite3.connect(db)) as connection:
            observer=connection.execute('SELECT source FROM engine_identity WHERE id=1').fetchone()[0]
            accepted_at=connection.execute('SELECT accepted_at_ms FROM governance_decisions WHERE id=?',(first,)).fetchone()[0]
        def history(selection):
            cut.write_text(json.dumps(selection))
            return invoke(args.host,'accepted_history',cut,'team')
        old=history({'kind':'at_time','observer':observer,'unix_millis':accepted_at})
        assert old['observation']['decision_id']==first and old['observation']['source']==seeded['source']
        assert snapshot(db)==before
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
        historical=invoke(args.old_host,'accepted','team',first)
        assert historical==old['result'] and snapshot(db)==after_accept
    report={'profile':'populated-store20-accepted-history-compat/1','status':'passed','store_marker':20,'checks':['actual prior runtime authored governance, approvals and protected occurrence','native date selection rewrites no original table, row or marker','source correction preserves exact accepted decision','new acceptance validates original signed quorum and persistent ordering','equal-time ancestry selects latest genuine acceptance','old runtime still reads exact original accepted result after new acceptance'],'processes':len(trace),'trace':trace,'scope':'native fixed trusted host; no new schema or older-runtime refusal required'}
    if args.report:args.report.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report))


if __name__=='__main__':main()
