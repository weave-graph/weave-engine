#!/usr/bin/env python3
"""Genuine store-29 upgrade to the store-30 fact store (A2, ADR-0004 section 10).

A store-29 `weave-engine` binary populates a store with snapshot revisions
and records pinned query results. The current `storage_probe` is then killed
before the schema commit (the store must still be store 29 and readable by
the old binary) and after it. Afterwards:

- the old binary refuses the upgraded store with E_STORAGE_VERSION;
- every pre-migration revision row is byte-identical;
- every pinned query returns byte-identical results, apart from the
  producing engine's protocol `version` field;
- the fact-store tables exist and are empty.
"""
import argparse, json, re, sqlite3, subprocess, tempfile
from contextlib import closing
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def store_marker():
    source = (ROOT / "crates/weave-engine/src/lib.rs").read_text()
    return int(re.search(r"pub const STORAGE_VERSION: i64 = (\d+);", source).group(1))


def contract_version():
    source = (ROOT / "crates/weave-contract/src/lib.rs").read_text()
    return re.search(r'pub const VERSION: &str = "([^"]+)";', source).group(1)


FACT_TABLES = {"fact_lines", "fact_revisions", "fact_log", "fact_index", "fact_checkpoints"}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--old-cli", type=Path, required=True)
    p.add_argument("--cli", type=Path, default=Path("target/debug/weave-engine"))
    p.add_argument("--storage", type=Path, default=Path("target/debug/examples/storage_probe"))
    p.add_argument("--report", type=Path)
    a = p.parse_args()
    with tempfile.TemporaryDirectory(prefix="weave-fact-migration-") as tmp:
        root = Path(tmp)
        db = root / "store.sqlite"

        def cli(binary, actor, program, writes=(), ok=True):
            plan = root / "plan.json"
            plan.write_text(json.dumps(program))
            args = [str(binary.resolve()), "run", "--db", str(db), "--actor", actor]
            for graph in writes:
                args += ["--write", graph]
            r = subprocess.run(args + [str(plan)], capture_output=True, text=True, timeout=60)
            if ok:
                assert r.returncode == 0, (binary, r.stdout, r.stderr)
                return json.loads(r.stdout)
            assert r.returncode != 0, (binary, r.stdout)
            return r.stderr

        def program(commands):
            return {"version": "0.21.0", "commands": commands}

        nodes = [{"id": "a", "entity_id": "A", "space_id": "s"},
                 {"id": "b", "entity_id": "B", "space_id": "s", "readers": ["bob"]},
                 {"id": "c", "entity_id": "C", "space_id": "s"}]
        edges = [{"id": "e1", "predicate": "p", "from": "a", "to": "c", "valid_time": {"start": 0, "end": 50}},
                 {"id": "e2", "predicate": "q", "from": "a", "to": "b", "valid_time": {"start": 10}}]
        attachments = [{"id": "m", "host": {"kind": "node", "id": "a"}, "key": "firmware",
                        "value": {"kind": "literal", "value": "1.0"}, "valid_time": {"start": 0}}]
        first = cli(a.old_cli, "alice", program([{"op": "commit", "graph_id": "fleet", "data": {
            "nodes": nodes, "edges": edges, "attachments": attachments}}]), ["fleet"])
        r1 = first[0]["revision"]
        nodes[0]["properties"] = {"model": "x"}
        second = cli(a.old_cli, "bob", program([{"op": "commit", "graph_id": "fleet", "expected_head": r1,
                                                 "data": {"nodes": nodes, "edges": edges}}]), ["fleet"])
        r2 = second[0]["revision"]
        explicit = {"profile": "explicit", "nodes": nodes[:1] + nodes[2:],
                    "structural_edges": [{"id": "s", "predicate": "p", "from": "a", "to": "c"}],
                    "assertions": [{"id": "x", "edge_id": "s", "source": "sensor", "valid_time": {"start": 0}},
                                   {"id": "y", "edge_id": "s", "source": "audit", "valid_time": {"start": 0},
                                    "polarity": "negative", "readers": ["bob"]}]}
        batch = cli(a.old_cli, "carol", program([{"op": "commit_batch", "batch_id": "b1", "commits": [
            {"graph_id": "claims", "data": explicit}, {"graph_id": "sites", "data": {"nodes": nodes[2:]}}]}]),
            ["claims", "sites"])
        pins = [("fleet", r1), ("fleet", r2)] + [(c["graph_id"], c["revision"]) for c in batch[0]["commits"]]

        def queries():
            out = {}
            for graph, revision in pins:
                for principal in ["alice", "bob"]:
                    for extra in [{}, {"valid_at": 20}, {"predicate": "p"}]:
                        query = {"graph_id": graph, "revision": revision, **extra}
                        key = json.dumps([graph, revision, principal, extra], sort_keys=True)
                        out[key] = (principal, query)
            return out

        def run_queries(binary):
            results = {}
            for key, (principal, query) in queries().items():
                results[key] = cli(binary, principal, program([{"op": "query", "query": query}]))
            return results

        def rows():
            with closing(sqlite3.connect(db)) as c:
                marker = c.execute("PRAGMA user_version").fetchone()[0]
                tables = {n for (n,) in c.execute("SELECT name FROM sqlite_master WHERE type='table'")}
                revisions = sorted(c.execute("SELECT * FROM revisions").fetchall())
                events = sorted(c.execute("SELECT * FROM events").fetchall())
            return marker, tables, revisions, events

        before = run_queries(a.old_cli)
        marker, tables, revisions, events = rows()
        assert marker == 29 and not tables & FACT_TABLES, marker

        # Process kill before the schema commit: still a complete store 29.
        r = subprocess.run([str(a.storage.resolve()), str(db), "crash", "20"], capture_output=True, timeout=60)
        assert r.returncode == 82, r
        assert rows() == (marker, tables, revisions, events)
        assert run_queries(a.old_cli) == before

        # Process kill after the schema commit: a complete current store.
        r = subprocess.run([str(a.storage.resolve()), str(db), "after_commit", "20"], capture_output=True, timeout=60)
        assert r.returncode == 83, r
        upgraded, new_tables, new_revisions, new_events = rows()
        assert upgraded == store_marker(), upgraded
        assert new_tables == tables | FACT_TABLES
        assert new_revisions == revisions and new_events == events
        with closing(sqlite3.connect(db)) as c:
            for table in FACT_TABLES:
                assert c.execute(f"SELECT count(*) FROM {table}").fetchone()[0] == 0, table

        # The store-29 binary refuses the upgraded store.
        refusal = cli(a.old_cli, "alice", program([{"op": "query", "query": {"graph_id": "fleet", "revision": r1}}]), ok=False)
        assert "E_STORAGE_VERSION" in refusal, refusal

        # Pinned results are byte-identical apart from the producing engine version.
        after = run_queries(a.cli)
        compared = 0
        for key, old in before.items():
            new = after[key]
            assert len(old) == len(new) == 1
            old_result, new_result = old[0]["result"], new[0]["result"]
            assert old_result.pop("version") == "0.21.0" and new_result.pop("version") == contract_version()
            assert json.dumps(old_result, sort_keys=False) == json.dumps(new_result, sort_keys=False), key
            compared += 1
        report = {"profile": "native-fact-store-migration/1", "result": "passed", "old_marker": 29, "new_marker": store_marker(),
                  "pinned_revisions": len(pins), "pinned_queries": compared, "process_deaths": 2,
                  "checks": ["kill before schema commit leaves store 29 readable by the old binary",
                             "kill after schema commit leaves a complete current store with empty fact tables",
                             "old binary refuses the current store with E_STORAGE_VERSION",
                             "revision and event rows unchanged",
                             "pinned query results byte-identical except result.version"]}
        print(json.dumps(report))
        if a.report:
            a.report.write_text(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
