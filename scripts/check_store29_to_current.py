#!/usr/bin/env python3
"""Populated store 29 to the current store, directly and through store 30 (A6).

`check_fact_store_migration.py` (29 -> current) and
`check_embedding_store_migration.py` (30 -> current) each prove one upgrade
from a store their own old binary populated. This check follows Phase A's
whole storage path from a store 29 database populated by the exact store-29
binary, along both routes a user can take:

* **direct:** the current binary upgrades store 29 to the current store in
  one schema transaction;
* **stepwise:** the exact store-30 binary upgrades it to store 30 and writes
  facts with the store-30 protocol, then the current binary upgrades it.

At every upgrade step, a process killed before the schema commit leaves the
previous store, unchanged and readable by its own binary. A process killed
after the commit leaves a complete new store. Every older binary on the path
refuses the upgraded store with E_STORAGE_VERSION. Pinned queries return
byte-identical results (apart from the producing engine's `version` field).
Every pre-upgrade revision, event and fact row is byte-identical, and SQLite
`integrity_check` passes. On the current store, the Phase A operators work
over the migrated lines:

* a fact `write` that corrects a node of a migrated snapshot revision;
* embedding facts and `nearest`;
* `as_of` at a recorded time before the upgrade;
* `expand`;
* `changes` from a pinned pre-upgrade revision;
* a reader restriction from the store-29 data, still applied.

The pinned queries are byte-identical again after those writes.
"""
import argparse, json, re, sqlite3, subprocess, tempfile, time
from contextlib import closing
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ROW_TABLES = ["revisions", "events", "heads", "fact_lines", "fact_revisions", "fact_log", "fact_index"]


def source_value(path, pattern):
    return re.search(pattern, (ROOT / path).read_text()).group(1)


CURRENT_MARKER = int(source_value("crates/weave-engine/src/lib.rs", r"pub const STORAGE_VERSION: i64 = (\d+);"))
CURRENT_VERSION = source_value("crates/weave-contract/src/lib.rs", r'pub const VERSION: &str = "([^"]+)";')


class Store:
    def __init__(self, root, name):
        self.root = root
        self.db = root / f"{name}.sqlite"

    def cli(self, binary, actor, program, writes=(), ok=True):
        plan = self.root / "plan.json"
        plan.write_text(json.dumps(program))
        args = [str(binary.resolve()), "run", "--db", str(self.db), "--actor", actor]
        for graph in writes:
            args += ["--write", graph]
        r = subprocess.run(args + [str(plan)], capture_output=True, text=True, timeout=120)
        if ok:
            assert r.returncode == 0, (binary, r.stdout, r.stderr)
            return json.loads(r.stdout)
        assert r.returncode != 0, (binary, r.stdout)
        return r.stderr

    def rows(self):
        with closing(sqlite3.connect(self.db)) as c:
            marker = c.execute("PRAGMA user_version").fetchone()[0]
            assert c.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
            names = {n for (n,) in c.execute("SELECT name FROM sqlite_master WHERE type='table'")}
            tables = {t: sorted(c.execute(f"SELECT * FROM {t}").fetchall(), key=repr)
                      for t in ROW_TABLES if t in names}
        return marker, tables


def program(version, commands):
    return {"version": version, "commands": commands}


def populate_store29(store, cli29):
    """The store-29 population of check_fact_store_migration.py, plus pins."""
    v = "0.21.0"
    nodes = [{"id": "a", "entity_id": "A", "space_id": "s"},
             {"id": "b", "entity_id": "B", "space_id": "s", "readers": ["bob"]},
             {"id": "c", "entity_id": "C", "space_id": "s"}]
    edges = [{"id": "e1", "predicate": "p", "from": "a", "to": "c", "valid_time": {"start": 0, "end": 50}},
             {"id": "e2", "predicate": "q", "from": "a", "to": "b", "valid_time": {"start": 10}}]
    attachments = [{"id": "m", "host": {"kind": "node", "id": "a"}, "key": "firmware",
                    "value": {"kind": "literal", "value": "1.0"}, "valid_time": {"start": 0}}]
    r1 = store.cli(cli29, "alice", program(v, [{"op": "commit", "graph_id": "fleet", "data": {
        "nodes": nodes, "edges": edges, "attachments": attachments}}]), ["fleet"])[0]["revision"]
    nodes[0]["properties"] = {"model": "x"}
    r2 = store.cli(cli29, "bob", program(v, [{"op": "commit", "graph_id": "fleet", "expected_head": r1,
                                              "data": {"nodes": nodes, "edges": edges}}]), ["fleet"])[0]["revision"]
    explicit = {"profile": "explicit", "nodes": nodes[:1] + nodes[2:],
                "structural_edges": [{"id": "s", "predicate": "p", "from": "a", "to": "c"}],
                "assertions": [{"id": "x", "edge_id": "s", "source": "sensor", "valid_time": {"start": 0}},
                               {"id": "y", "edge_id": "s", "source": "audit", "valid_time": {"start": 0},
                                "polarity": "negative", "readers": ["bob"]}]}
    batch = store.cli(cli29, "carol", program(v, [{"op": "commit_batch", "batch_id": "b1", "commits": [
        {"graph_id": "claims", "data": explicit}, {"graph_id": "sites", "data": {"nodes": nodes[2:]}}]}]),
        ["claims", "sites"])
    return [("fleet", r1), ("fleet", r2)] + [(c["graph_id"], c["revision"]) for c in batch[0]["commits"]]


def pinned_queries(store, binary, version, pins):
    """Run every pinned query as a `version` program. The version stays that of
    the binary that wrote the pins, so the results keep its semantics."""
    out = {}
    for graph, revision in pins:
        for principal in ["alice", "bob"]:
            for extra in [{}, {"valid_at": 20}, {"predicate": "p"}]:
                key = json.dumps([graph, revision, principal, extra], sort_keys=True)
                result = store.cli(binary, principal, program(version, [{"op": "query", "query": {
                    "graph_id": graph, "revision": revision, **extra}}]))
                assert len(result) == 1
                body = dict(result[0]["result"])
                body.pop("version")  # the producing engine's protocol version
                out[key] = json.dumps(body, sort_keys=False)
    return out


def upgrade_step(store, old_marker, old_cli, old_version, storage, pins, before):
    """Kill before and after the schema commit of one upgrade step."""
    marker, tables = store.rows()
    assert marker == old_marker, marker
    r = subprocess.run([str(storage.resolve()), str(store.db), "crash", "20"], capture_output=True, timeout=120)
    assert r.returncode == 82, r
    assert store.rows() == (marker, tables), "kill before commit changed the store"
    assert pinned_queries(store, old_cli, old_version, pins) == before, "old binary reads the unchanged store"
    r = subprocess.run([str(storage.resolve()), str(store.db), "after_commit", "20"], capture_output=True, timeout=120)
    assert r.returncode == 83, r
    upgraded, new_tables = store.rows()
    for table, content in tables.items():
        assert new_tables[table] == content, f"{table} changed during the upgrade"
    return upgraded


def refused_by(store, binaries):
    for name, (binary, version) in binaries.items():
        error = store.cli(binary, "alice", program(version, [{"op": "query", "query": {"graph_id": "fleet"}}]), ok=False)
        assert "E_STORAGE_VERSION" in error, (name, error)


def phase_a_workload(store, cli, expected_head, fleet_head, known_before):
    """Phase A operators on the migrated store; returns check names.

    `expected_head` is the fleet head before the write; `fleet_head` is the
    last pinned store-29 snapshot revision of the fleet graph."""
    v = CURRENT_VERSION

    def change(kind, fact):
        return {"change": kind, "fact": fact}

    written = store.cli(cli, "alice", program(v, [{"op": "write", "graph_id": "fleet", "expected_head": expected_head, "changes": [
        change("correct", {"kind": "node", "id": "c", "entity_id": "C", "space_id": "s", "properties": {"serial": "SN-29"}}),
        change("assert", {"kind": "embedding_space", "id": "notes", "revision": "1", "encoder": "test",
                          "encoder_version": "1", "preprocessing": "none", "dimensions": 2, "metric": "euclidean"}),
        change("assert", {"kind": "embedding", "id": "va", "space": {"id": "notes", "revision": "1"},
                          "subject": {"kind": "node", "node_id": "a"}, "vector": [1.0, 0.0], "valid_time": {"start": 0}}),
        change("assert", {"kind": "embedding", "id": "vb", "space": {"id": "notes", "revision": "1"},
                          "subject": {"kind": "node", "node_id": "b"}, "vector": [0.9, 0.0], "valid_time": {"start": 0},
                          "readers": ["bob"]})]}]), ["fleet"])
    new_head = written[0]["revision"]
    assert new_head.startswith("fact:sha256:"), new_head

    def evaluate(principal, value):
        return store.cli(cli, principal, program(v, [{"op": "evaluate", "value": value}]))[0]["result"]

    near = {"kind": "nearest", "graph_id": "fleet", "space": {"id": "notes", "revision": "1"},
            "query": {"kind": "vector", "vector": [1.0, 0.0]}, "k": 5}
    seen = {p: [n["embedding_id"] for n in evaluate(p, near)["neighbors"]] for p in ["alice", "bob"]}
    assert seen == {"alice": ["va"], "bob": ["va", "vb"]}, seen

    # As known before the upgrade: the migrated snapshot state, not the new write.
    as_of = evaluate("alice", {"kind": "as_of", "unix_millis": known_before,
                               "query": {"graph_id": "fleet", "branch_id": "main"}})
    c_before = [n for n in as_of["graph"]["nodes"] if n["id"] == "c"]
    assert c_before and c_before[0].get("properties", {}).get("serial") is None, as_of["graph"]["nodes"]
    assert "b" not in {n["id"] for n in as_of["graph"]["nodes"]}, "restricted node hidden from alice"

    expand = {"kind": "expand", "graph_id": "fleet", "seeds": ["a"], "hops": 2, "direction": "both"}
    regions = {p: sorted(n["id"] for n in evaluate(p, expand)["graph"]["nodes"]) for p in ["alice", "bob"]}
    assert "b" not in regions["alice"] and "b" in regions["bob"], regions
    assert "c" in regions["alice"], regions

    changes = evaluate("alice", {"kind": "changes", "graph_id": "fleet",
                                 "from": {"kind": "cursor", "revision": fleet_head}, "to": {"kind": "head"}})
    entries = {(e["kind"], e["id"], e["change"]) for e in changes["changes"]["entries"]}
    assert ("node", "c", "corrected") in entries, entries
    assert not any(i == "b" or i == "vb" for _, i, _ in entries), "hidden facts stay out of alice's change set"
    assert changes["fact_provenance"], "the change set explains its facts"
    return ["fact write corrects a node of a migrated snapshot line",
            "embedding facts and nearest on the migrated line, restricted vector hidden",
            "as_of before the upgrade returns the migrated state",
            "expand over migrated edges keeps the store-29 reader restriction",
            "changes from a pinned store-29 revision to the new head"]


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--store29-cli", type=Path, required=True)
    p.add_argument("--store30-cli", type=Path, required=True)
    p.add_argument("--store30-storage", type=Path, required=True,
                   help="storage_probe of the store-30 source, built with recovery-testing")
    p.add_argument("--cli", type=Path, default=Path("target/debug/weave-engine"))
    p.add_argument("--storage", type=Path, default=Path("target/debug/examples/storage_probe"))
    p.add_argument("--report", type=Path)
    a = p.parse_args()
    report = {"profile": "native-store29-to-current/1", "result": "passed",
              "from_marker": 29, "to_marker": CURRENT_MARKER, "routes": {}}
    with tempfile.TemporaryDirectory(prefix="weave-store29-path-") as tmp:
        root = Path(tmp)

        # ---- Direct: 29 -> current in one schema transaction.
        store = Store(root, "direct")
        pins = populate_store29(store, a.store29_cli)
        before = pinned_queries(store, a.store29_cli, "0.21.0", pins)
        known_before = int(time.time() * 1000)
        time.sleep(0.01)
        marker = upgrade_step(store, 29, a.store29_cli, "0.21.0", a.storage, pins, before)
        assert marker == CURRENT_MARKER, marker
        refused_by(store, {"store29": (a.store29_cli, "0.21.0"), "store30": (a.store30_cli, "0.23.0")})
        after = pinned_queries(store, a.cli, "0.21.0", pins)
        assert after == before, "pinned results changed across 29 -> current"
        fleet_head = [r for g, r in pins if g == "fleet"][-1]
        checks = phase_a_workload(store, a.cli, fleet_head, fleet_head, known_before)
        assert pinned_queries(store, a.cli, "0.21.0", pins) == before, "pinned results changed by new writes"
        refused_by(store, {"store29": (a.store29_cli, "0.21.0"), "store30": (a.store30_cli, "0.23.0")})
        report["routes"]["direct"] = {"steps": ["29 -> %d" % CURRENT_MARKER], "process_deaths": 2,
                                      "pinned_queries": len(before), "phase_a_checks": checks}

        # ---- Stepwise: 29 -> 30 by the store-30 binary, then 30 -> current.
        store = Store(root, "stepwise")
        pins = populate_store29(store, a.store29_cli)
        before = pinned_queries(store, a.store29_cli, "0.21.0", pins)
        marker = upgrade_step(store, 29, a.store29_cli, "0.21.0", a.store30_storage, pins, before)
        assert marker == 30, marker
        refused_by(store, {"store29": (a.store29_cli, "0.21.0")})
        assert pinned_queries(store, a.store30_cli, "0.21.0", pins) == before, "pinned results changed across 29 -> 30"
        # Store-30 fact writes on the migrated snapshot line, before the next upgrade.
        fleet_head = [r for g, r in pins if g == "fleet"][-1]
        wrote = store.cli(a.store30_cli, "alice", program("0.23.0", [{"op": "write", "graph_id": "fleet",
            "expected_head": fleet_head, "changes": [{"change": "assert", "fact": {
                "kind": "node", "id": "d", "entity_id": "D", "space_id": "s", "valid_time": {"start": 5},
                "provenance": {"method": {"kind": "import", "importer": "etl", "version": "30"}}}}]}]), ["fleet"])
        pins_30 = pins + [("fleet", wrote[0]["revision"])]
        before_30 = pinned_queries(store, a.store30_cli, "0.23.0", pins_30)
        known_before = int(time.time() * 1000)
        time.sleep(0.01)
        marker = upgrade_step(store, 30, a.store30_cli, "0.23.0", a.storage, pins_30, before_30)
        assert marker == CURRENT_MARKER, marker
        refused_by(store, {"store29": (a.store29_cli, "0.21.0"), "store30": (a.store30_cli, "0.23.0")})
        assert pinned_queries(store, a.cli, "0.23.0", pins_30) == before_30, "pinned results changed across 30 -> current"
        assert pinned_queries(store, a.cli, "0.21.0", pins) == before, "store-29 pins changed across 29 -> 30 -> current"
        checks = phase_a_workload(store, a.cli, pins_30[-1][1], fleet_head, known_before)
        assert pinned_queries(store, a.cli, "0.23.0", pins_30) == before_30, "pinned results changed by new writes"
        assert pinned_queries(store, a.cli, "0.21.0", pins) == before, "store-29 pins changed by new writes"
        report["routes"]["stepwise"] = {"steps": ["29 -> 30", "30 -> %d" % CURRENT_MARKER], "process_deaths": 4,
                                        "pinned_queries": len(before_30), "phase_a_checks": checks}
    report["checks"] = [
        "kill before each schema commit leaves the previous store unchanged and readable by its binary",
        "kill after each schema commit leaves a complete new store; every earlier row unchanged; integrity_check ok",
        "every older binary on the path refuses the upgraded store with E_STORAGE_VERSION",
        "pinned query results byte-identical except result.version, before and after Phase A writes",
        "Phase A operators (write, nearest, as_of, expand, changes) over migrated lines with store-29 reader restrictions",
    ]
    print(json.dumps(report))
    if a.report:
        a.report.write_text(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
