#!/usr/bin/env python3
"""Genuine store-30 upgrade to store 31, where embedding facts live (A4, ADR-0006).

A store-30 `weave-engine` binary (protocol 0.23.0) populates a store with a
snapshot revision and fact writes and records pinned query results. The
current `storage_probe` is then killed before the schema commit (the store
must still be store 30 and readable by the old binary) and after it.
Afterwards:

- the old binary refuses the upgraded store with E_STORAGE_VERSION;
- every revision, event, fact-log and fact-index row is byte-identical;
- every pinned query returns byte-identical results, apart from the
  producing engine's protocol `version` field;
- the current binary writes embedding facts on top of the migrated fact
  line and ranks them with `nearest`, and the old binary still refuses.
"""
import argparse, json, re, sqlite3, subprocess, tempfile
from contextlib import closing
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TABLES = ["revisions", "events", "heads", "fact_lines", "fact_revisions", "fact_log", "fact_index"]


def store_marker():
    source = (ROOT / "crates/weave-engine/src/lib.rs").read_text()
    return int(re.search(r"pub const STORAGE_VERSION: i64 = (\d+);", source).group(1))


def contract_version():
    source = (ROOT / "crates/weave-contract/src/lib.rs").read_text()
    return re.search(r'pub const VERSION: &str = "([^"]+)";', source).group(1)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--old-cli", type=Path, required=True)
    p.add_argument("--cli", type=Path, default=Path("target/debug/weave-engine"))
    p.add_argument("--storage", type=Path, default=Path("target/debug/examples/storage_probe"))
    p.add_argument("--report", type=Path)
    a = p.parse_args()
    with tempfile.TemporaryDirectory(prefix="weave-embedding-migration-") as tmp:
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

        def program(commands, version="0.23.0"):
            return {"version": version, "commands": commands}

        def assert_fact(fact):
            return {"change": "assert", "fact": fact}

        first = cli(a.old_cli, "alice", program([{"op": "commit", "graph_id": "fleet", "data": {
            "nodes": [{"id": "a", "entity_id": "A", "space_id": "s"},
                      {"id": "b", "entity_id": "B", "space_id": "s", "readers": ["bob"]}],
            "edges": [{"id": "e1", "predicate": "p", "from": "a", "to": "b", "valid_time": {"start": 0}}]}}]),
            ["fleet"])
        r1 = first[0]["revision"]
        second = cli(a.old_cli, "alice", program([{"op": "write", "graph_id": "fleet", "expected_head": r1, "changes": [
            assert_fact({"kind": "node", "id": "c", "entity_id": "C", "space_id": "s",
                         "valid_time": {"start": 5, "end": 50},
                         "provenance": {"method": {"kind": "import", "importer": "etl", "version": "1"}}}),
            assert_fact({"kind": "edge", "id": "e2", "predicate": "q", "from": "a", "to": "c",
                         "valid_time": {"start": 10}, "readers": ["bob"]}),
            assert_fact({"kind": "attachment", "id": "m", "host": {"kind": "node", "id": "a"}, "key": "note",
                         "value": {"kind": "literal", "value": "checked"}, "valid_time": {"start": 0}})]}]),
            ["fleet"])
        r2 = second[0]["revision"]
        third = cli(a.old_cli, "bob", program([{"op": "write", "graph_id": "fleet", "expected_head": r2, "changes": [
            {"change": "correct", "fact": {"kind": "node", "id": "c", "entity_id": "C", "space_id": "s",
                                           "properties": {"serial": "SN-1"}}}]}]), ["fleet"])
        r3 = third[0]["revision"]
        pins = [r1, r2, r3]

        def queries():
            out = {}
            for revision in pins:
                for principal in ["alice", "bob"]:
                    for extra in [{}, {"valid_at": 20}, {"predicate": "q"}]:
                        query = {"graph_id": "fleet", "revision": revision, **extra}
                        out[json.dumps([revision, principal, extra], sort_keys=True)] = (principal, query)
            return out

        def run_queries(binary):
            return {key: cli(binary, principal, program([{"op": "query", "query": query}]))
                    for key, (principal, query) in queries().items()}

        def rows():
            with closing(sqlite3.connect(db)) as c:
                marker = c.execute("PRAGMA user_version").fetchone()[0]
                tables = {t: sorted(c.execute(f"SELECT * FROM {t}").fetchall(), key=repr) for t in TABLES}
            return marker, tables

        before = run_queries(a.old_cli)
        marker, tables = rows()
        assert marker == 30, marker

        # Process kill before the schema commit: still a complete store 30.
        r = subprocess.run([str(a.storage.resolve()), str(db), "crash", "20"], capture_output=True, timeout=60)
        assert r.returncode == 82, r
        assert rows() == (marker, tables)
        assert run_queries(a.old_cli) == before

        # Process kill after the schema commit: the current store, no row changed.
        r = subprocess.run([str(a.storage.resolve()), str(db), "after_commit", "20"], capture_output=True, timeout=60)
        assert r.returncode == 83, r
        upgraded, new_tables = rows()
        assert upgraded == store_marker(), upgraded
        assert new_tables == tables

        # The store-30 binary refuses the upgraded store.
        refusal = cli(a.old_cli, "alice", program([{"op": "query", "query": {"graph_id": "fleet", "revision": r1}}]), ok=False)
        assert "E_STORAGE_VERSION" in refusal, refusal

        # Pinned results are byte-identical apart from the producing engine version.
        after = run_queries(a.cli)
        after_text = json.dumps(after, sort_keys=True)
        compared = 0
        for key, old in before.items():
            new = after[key]
            assert len(old) == len(new) == 1
            old_result, new_result = old[0]["result"], new[0]["result"]
            assert old_result.pop("version") == "0.23.0" and new_result.pop("version") == contract_version()
            assert json.dumps(old_result, sort_keys=False) == json.dumps(new_result, sort_keys=False), key
            compared += 1

        # Embedding facts go on top of the migrated fact line.
        current = contract_version()
        cli(a.cli, "alice", program([{"op": "write", "graph_id": "fleet", "expected_head": r3, "changes": [
            assert_fact({"kind": "embedding_space", "id": "notes", "revision": "1", "encoder": "test",
                         "encoder_version": "1", "preprocessing": "none", "dimensions": 2, "metric": "euclidean"}),
            assert_fact({"kind": "embedding", "id": "va", "space": {"id": "notes", "revision": "1"},
                         "subject": {"kind": "node", "node_id": "a"}, "vector": [1.0, 0.0], "valid_time": {"start": 0}}),
            assert_fact({"kind": "embedding", "id": "vb", "space": {"id": "notes", "revision": "1"},
                         "subject": {"kind": "node", "node_id": "b"}, "vector": [0.9, 0.0], "valid_time": {"start": 0},
                         "readers": ["bob"]})]}], current), ["fleet"])
        nearest = {"kind": "nearest", "graph_id": "fleet", "space": {"id": "notes", "revision": "1"},
                   "query": {"kind": "vector", "vector": [1.0, 0.0]}, "k": 5}
        seen = {}
        for principal in ["alice", "bob"]:
            result = cli(a.cli, principal, program([{"op": "evaluate", "value": nearest}], current))[0]["result"]
            assert result["exactness"] == "exact"
            seen[principal] = [n["embedding_id"] for n in result["neighbors"]]
        assert seen == {"alice": ["va"], "bob": ["va", "vb"]}, seen
        assert json.dumps(run_queries(a.cli), sort_keys=True) == after_text, "pinned reads unchanged by the embedding write"
        refusal = cli(a.old_cli, "alice", program([{"op": "query", "query": {"graph_id": "fleet"}}]), ok=False)
        assert "E_STORAGE_VERSION" in refusal, refusal

        report = {"profile": "native-embedding-store-migration/1", "result": "passed", "old_marker": 30,
                  "new_marker": store_marker(), "pinned_revisions": len(pins), "pinned_queries": compared,
                  "process_deaths": 2,
                  "checks": ["kill before schema commit leaves store 30 readable by the old binary",
                             "kill after schema commit leaves the current store with every row unchanged",
                             "old binary refuses the current store with E_STORAGE_VERSION",
                             "pinned query results byte-identical except result.version",
                             "embedding facts written and ranked on the migrated fact line, hidden vector excluded"]}
        print(json.dumps(report))
        if a.report:
            a.report.write_text(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
