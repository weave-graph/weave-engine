#!/usr/bin/env python3
"""Genuine store-32 upgrade to store 33, where optional review lives (B5, #12).

A store-32 `weave-engine` binary (B2 part 3, protocol 1.2.0) enables the
principal registry, grants a team Write on a graph, writes facts with literal
and group readers, and records pinned query results. The current
`storage_probe` is then killed before the schema commit (the store must still
be store 32 and readable by the old binary) and after it. Afterwards:

- the old binary refuses the upgraded store with E_STORAGE_VERSION, so it can
  never let a writer publish directly to a reviewed graph;
- every revision, event, head, fact-log and fact-index row is byte-identical;
- every pinned query returns byte-identical results, apart from the
  producing engine's protocol `version` field;
- the current binary makes the graph reviewed: a direct write fails with
  E_REVIEW_REQUIRED, a proposal is invisible to a reader until a reviewer
  accepts it, and the accepted fact carries review provenance.
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
    with tempfile.TemporaryDirectory(prefix="weave-review-migration-") as tmp:
        root = Path(tmp)
        db = root / "store.sqlite"

        def invoke(args, ok=True):
            r = subprocess.run(args, capture_output=True, text=True, timeout=60)
            if ok:
                assert r.returncode == 0, (args, r.stdout, r.stderr)
                return json.loads(r.stdout)
            assert r.returncode != 0, (args, r.stdout)
            return r.stderr

        def cli(binary, actor, program, ok=True):
            plan = root / "plan.json"
            plan.write_text(json.dumps(program))
            return invoke([str(binary.resolve()), "run", "--db", str(db), "--actor", actor, str(plan)], ok)

        def registry(binary, actor, *command):
            args = [str(binary.resolve()), "registry", command[0], "--db", str(db), "--actor", actor]
            if len(command) > 1:
                changes = root / "changes.json"
                changes.write_text(json.dumps(command[1]))
                args.append(str(changes))
            return invoke(args)

        def review(actor, request, ok=True):
            path = root / "review.json"
            path.write_text(json.dumps(request))
            return invoke([str(a.cli.resolve()), "review", "--db", str(db), "--actor", actor, str(path)], ok)

        def program(commands, version="1.2.0"):
            return {"version": version, "commands": commands}

        def assert_fact(fact):
            return {"change": "assert", "fact": fact}

        registry(a.old_cli, "root", "enable")
        registry(a.old_cli, "root", "apply", [
            {"change": "add_principal", "id": "alice", "kind": "user"},
            {"change": "add_principal", "id": "bob", "kind": "user"},
            {"change": "add_principal", "id": "dave", "kind": "user"},
            {"change": "add_principal", "id": "team", "kind": "group"},
            {"change": "add_member", "group": "team", "member": "alice"},
            {"change": "add_member", "group": "team", "member": "bob"},
            {"change": "grant", "graph_id": "kb", "subject": "team", "permission": "write"},
            {"change": "grant", "graph_id": "kb", "subject": "dave", "permission": "read"}])
        first = cli(a.old_cli, "alice", program([{"op": "write", "graph_id": "kb", "changes": [
            assert_fact({"kind": "node", "id": "a", "entity_id": "A", "space_id": "s"}),
            assert_fact({"kind": "node", "id": "b", "entity_id": "B", "space_id": "s", "reader_groups": ["team"]})]}]))
        r1 = first[0]["revision"]
        second = cli(a.old_cli, "bob", program([{"op": "write", "graph_id": "kb", "expected_head": r1, "changes": [
            assert_fact({"kind": "edge", "id": "e", "predicate": "p", "from": "a", "to": "b",
                         "valid_time": {"start": 0}, "readers": ["bob"],
                         "provenance": {"method": {"kind": "human"}}})]}]))
        r2 = second[0]["revision"]
        pins = [r1, r2]

        def queries():
            out = {}
            for revision in pins:
                for principal in ["alice", "bob", "dave"]:
                    query = {"graph_id": "kb", "revision": revision}
                    out[json.dumps([revision, principal], sort_keys=True)] = (principal, query)
            return out

        def run_queries(binary):
            return {key: cli(binary, principal, program([{"op": "query", "query": query}]))
                    for key, (principal, query) in queries().items()}

        def rows():
            with closing(sqlite3.connect(db)) as c:
                marker = c.execute("PRAGMA user_version").fetchone()[0]
                tables = {t: sorted(c.execute(f"SELECT * FROM {t}").fetchall(), key=repr) for t in TABLES}
                review_tables = sorted(r[0] for r in c.execute(
                    "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'review_%'"))
            return marker, tables, review_tables

        before = run_queries(a.old_cli)
        marker, tables, review_tables = rows()
        assert marker == 32, marker
        assert review_tables == [], review_tables

        # Process kill before the schema commit: still a complete store 32.
        r = subprocess.run([str(a.storage.resolve()), str(db), "crash", "20"], capture_output=True, timeout=60)
        assert r.returncode == 82, r
        assert rows() == (marker, tables, review_tables)
        assert run_queries(a.old_cli) == before

        # Process kill after the schema commit: the current store, no row changed.
        r = subprocess.run([str(a.storage.resolve()), str(db), "after_commit", "20"], capture_output=True, timeout=60)
        assert r.returncode == 83, r
        upgraded, new_tables, new_review = rows()
        assert upgraded == store_marker() == 33, upgraded
        assert new_tables == tables
        assert new_review == ["review_decisions", "review_policies", "review_proposals",
                              "review_versions", "review_votes"], new_review

        # The store-32 binary refuses the upgraded store.
        refusal = cli(a.old_cli, "alice", program([{"op": "query", "query": {"graph_id": "kb", "revision": r1}}]), ok=False)
        assert "E_STORAGE_VERSION" in refusal, refusal

        # Pinned results are byte-identical apart from the producing engine version.
        after = run_queries(a.cli)
        compared = 0
        for key, old in before.items():
            new = after[key]
            assert len(old) == len(new) == 1
            old_result, new_result = old[0]["result"], new[0]["result"]
            assert old_result.pop("version") == "1.2.0" and new_result.pop("version") == contract_version()
            assert json.dumps(old_result, sort_keys=False) == json.dumps(new_result, sort_keys=False), key
            compared += 1

        # Review on the migrated store.
        current = contract_version()
        applied = review("root", {"action": "set_policy", "graph_id": "kb", "expected_revision": 0,
                                  "mode": {"kind": "reviewed", "reviewers": ["team"], "threshold": 1}})
        assert applied["outcome"] == "applied" and applied["policy"]["revision"] == 1, applied
        refused = cli(a.cli, "alice", program([{"op": "write", "graph_id": "kb", "changes": [
            assert_fact({"kind": "node", "id": "c", "entity_id": "C", "space_id": "s"})]}], current), ok=False)
        assert "E_REVIEW_REQUIRED" in refused, refused
        receipt = review("alice", {"action": "propose", "graph_id": "kb", "branch_id": "main", "changes": [
            assert_fact({"kind": "node", "id": "c", "entity_id": "C", "space_id": "s",
                         "provenance": {"method": {"kind": "human"}}})]})

        def visible(principal):
            result = cli(a.cli, principal, program([{"op": "query", "query": {"graph_id": "kb"}}], current))
            return result[0]["result"]

        nodes = lambda result: sorted(n["id"] for n in result["graph"]["nodes"])
        assert nodes(visible("dave")) == ["a"], nodes(visible("dave"))
        assert review("dave", {"action": "list", "graph_id": "kb"}) == []
        accepted = review("bob", {"action": "vote", "proposal_id": receipt["proposal_id"],
                                  "digest": receipt["digest"], "verdict": "approve"})
        assert accepted["status"] == "accepted", accepted
        published = visible("dave")
        assert nodes(published) == ["a", "c"], nodes(published)
        entry = next(e for e in published["fact_provenance"] if e["fact"].get("node_id") == "c")
        assert entry["provenance"]["writer"] == "alice", entry
        assert entry["provenance"]["review"]["reviewers"] == ["bob"], entry
        assert entry["provenance"]["review"]["proposal_id"] == receipt["proposal_id"], entry
        refusal = cli(a.old_cli, "alice", program([{"op": "query", "query": {"graph_id": "kb"}}]), ok=False)
        assert "E_STORAGE_VERSION" in refusal, refusal

        report = {"profile": "native-review-store-migration/1", "result": "passed", "old_marker": 32,
                  "new_marker": store_marker(), "pinned_revisions": len(pins), "pinned_queries": compared,
                  "process_deaths": 2,
                  "checks": ["kill before schema commit leaves store 32 readable by the old binary",
                             "kill after schema commit leaves store 33 with every data row unchanged and the review tables created",
                             "old binary refuses store 33 with E_STORAGE_VERSION",
                             "pinned query results byte-identical except result.version",
                             "a reviewed graph refuses direct writes on the migrated store",
                             "a proposal is invisible to a reader until accepted; the accepted fact carries review provenance"]}
        print(json.dumps(report))
        if a.report:
            a.report.write_text(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
