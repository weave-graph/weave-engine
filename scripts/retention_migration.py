"""Independent SQL oracle for the default, non-erasing store22 migration."""
import hashlib,json,sqlite3
from contextlib import closing
RETENTION_TABLES=frozenset(('retention_policy','retention_roots','retention_tombstones','retention_adapter_states','retention_projection_receipts','retention_view_epochs','retention_retired_branches','retention_stateful_adapters'))
def retention_tables(path):
    with closing(sqlite3.connect(path)) as c:
        return {n for (n,) in c.execute("SELECT name FROM sqlite_master WHERE type='table'") if n in RETENTION_TABLES}
def assert_retention_baseline(path,marker):
    assert_lifecycle_baseline(path,marker)
    assert_compiled_lifecycle_baseline(path,marker)
    assert_compiled_rebuild_baseline(path,marker)
    assert_recorded_actor_baseline(path,marker)
    assert_actor_lifecycle_baseline(path,marker)
    assert_actor_disposition_baseline(path,marker)
    assert_causal_dispatch_baseline(path,marker)
    if marker<22:
        assert not retention_tables(path);return
    assert retention_tables(path)==RETENTION_TABLES
    with closing(sqlite3.connect(path)) as c:
        generation,raw,digest,epoch=c.execute('SELECT generation,policy,digest,epoch FROM retention_policy WHERE id=1').fetchone()
        policy={'history_before_ms':0,'replay_through_sequence':0}
        assert generation==0 and json.loads(raw)==policy and len(epoch)==48 and all(x in '0123456789abcdef' for x in epoch)
        encoded=json.dumps([generation,policy,epoch],separators=(',',':')).encode()
        assert digest=='sha256:'+hashlib.sha256(encoded).hexdigest()
        assert c.execute('SELECT count(*) FROM retention_policy').fetchone()[0]==1
        assert sorted(c.execute('SELECT principal,epoch FROM retention_view_epochs'))==sorted((p,epoch) for (p,) in c.execute('SELECT principal FROM view_schedule_cursors'))
        for name in RETENTION_TABLES-{'retention_policy','retention_view_epochs'}:
            assert c.execute('SELECT count(*) FROM '+name).fetchone()[0]==0,name

LIFECYCLE_TABLES=frozenset(("delivery_cancellations","projection_rebuild_requests","projection_migrations"))
def assert_lifecycle_baseline(path,marker):
    with closing(sqlite3.connect(path)) as c:
        present={n for (n,) in c.execute("SELECT name FROM sqlite_master WHERE type=\"table\"") if n in LIFECYCLE_TABLES}
        if marker<23:
            assert not present;return
        assert present==LIFECYCLE_TABLES
        for name in LIFECYCLE_TABLES:assert c.execute("SELECT count(*) FROM "+name).fetchone()[0]==0,name

COMPILED_LIFECYCLE_TABLES=frozenset(("compiled_migrations",))
def assert_compiled_lifecycle_baseline(path,marker):
    with closing(sqlite3.connect(path)) as c:
        present={n for (n,) in c.execute("SELECT name FROM sqlite_master WHERE type='table'") if n in COMPILED_LIFECYCLE_TABLES}
        if marker<24:
            assert not present;return
        assert present==COMPILED_LIFECYCLE_TABLES
        for name in COMPILED_LIFECYCLE_TABLES:assert c.execute("SELECT count(*) FROM "+name).fetchone()[0]==0,name

COMPILED_REBUILD_TABLES=frozenset(("compiled_rebuild_receipts","compiled_replay_states"))
def assert_compiled_rebuild_baseline(path,marker):
    with closing(sqlite3.connect(path)) as c:
        present={n for (n,) in c.execute("SELECT name FROM sqlite_master WHERE type='table'") if n in COMPILED_REBUILD_TABLES}
        if marker<25:
            assert not present;return
        assert present==COMPILED_REBUILD_TABLES
        for name in COMPILED_REBUILD_TABLES:assert c.execute("SELECT count(*) FROM "+name).fetchone()[0]==0,name

RECORDED_ACTOR_TABLES=frozenset(("recorded_actor_definitions","recorded_actor_states","recorded_actor_receipts"))
def assert_recorded_actor_baseline(path,marker):
    with closing(sqlite3.connect(path)) as c:
        present={n for (n,) in c.execute("SELECT name FROM sqlite_master WHERE type='table'") if n in RECORDED_ACTOR_TABLES}
        if marker<26:
            assert not present;return
        assert present==RECORDED_ACTOR_TABLES
        for name in RECORDED_ACTOR_TABLES:assert c.execute("SELECT count(*) FROM "+name).fetchone()[0]==0,name

ACTOR_LIFECYCLE_TABLES=frozenset(("recorded_actor_migrations","recorded_actor_replay_fences","recorded_actor_observations"))
def assert_actor_lifecycle_baseline(path,marker):
    with closing(sqlite3.connect(path)) as c:
        present={n for (n,) in c.execute("SELECT name FROM sqlite_master WHERE type='table'") if n in ACTOR_LIFECYCLE_TABLES}
        if marker<27:
            assert not present;return
        assert present==ACTOR_LIFECYCLE_TABLES
        for name in ACTOR_LIFECYCLE_TABLES:assert c.execute("SELECT count(*) FROM "+name).fetchone()[0]==0,name

ACTOR_DISPOSITION_TABLES=frozenset(("recorded_actor_cancellations",))
def assert_actor_disposition_baseline(path,marker):
    with closing(sqlite3.connect(path)) as c:
        present={n for(n,)in c.execute("SELECT name FROM sqlite_master WHERE type='table'")if n in ACTOR_DISPOSITION_TABLES}
        if marker<28:
            assert not present;return
        assert present==ACTOR_DISPOSITION_TABLES
        for name in ACTOR_DISPOSITION_TABLES:assert c.execute("SELECT count(*) FROM "+name).fetchone()[0]==0,name

CAUSAL_DISPATCH_TABLES=frozenset(("event_causation","dispatch_causal_policies","dispatch_circuits"))
def assert_causal_dispatch_baseline(path,marker):
    with closing(sqlite3.connect(path))as c:
        present={n for(n,)in c.execute("SELECT name FROM sqlite_master WHERE type='table'")if n in CAUSAL_DISPATCH_TABLES}
        if marker<29:
            assert not present;return
        assert present==CAUSAL_DISPATCH_TABLES
        events=dict(c.execute('SELECT event_id,sequence FROM events'))
        records=list(c.execute('SELECT event_id,body,digest FROM event_causation'));assert {event for event,_,_ in records}==set(events)
        for event,body,digest in records:
            record=json.loads(body);assert record['event']==event and record['sequence']==events[event]
            encoded=json.dumps(record,ensure_ascii=False,separators=(',',':')).encode();assert digest=='sha256:'+hashlib.sha256(encoded).hexdigest()
            assert record['origin']in ['local_root','legacy_boundary','handler']
            if record['origin']=='handler':assert record['parent']in events and events[record['parent']]<events[event] and 1<=record['depth']<=64
            else:assert record['depth']==0 and record['parent']is None and record['root']==event and record['adapter']is None and record['registration_digest']is None
        policies=list(c.execute('SELECT adapter,body,digest FROM dispatch_causal_policies'));assert {adapter for adapter,_,_ in policies}=={id for(id,)in c.execute('SELECT id FROM dispatch_adapters')}
        for adapter,body,digest in policies:
            policy=json.loads(body);assert policy=={'max_depth':16}
            assert digest=='sha256:'+hashlib.sha256(json.dumps([adapter,policy],ensure_ascii=False,separators=(',',':')).encode()).hexdigest()
        assert c.execute('SELECT count(*) FROM dispatch_circuits').fetchone()[0]==0
