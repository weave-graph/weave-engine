#!/usr/bin/env python3
"""Independent native experiment acceptance; no third-party Python packages needed.

This controller deliberately compares the native analytics with different
reference methods: all-pairs dynamic programming for reachability/distances and
a direct linear solve for PageRank. It never calculates the engine's results.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import math
import os
import platform
import random
import subprocess
import tempfile
import time
from pathlib import Path

CONTRACT_VERSION = "0.21.0"
SCIENCE_VERSION = "0.1.0"
REPOSITORY = Path(__file__).resolve().parents[1]


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()


def digest_file(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def host_identity():
    value = {"system": platform.system(), "release": platform.release(), "machine": platform.machine(),
             "python": platform.python_version(), "logical_cpus": os.cpu_count(),
             "cpu_model": platform.processor() or None, "physical_memory_bytes": None}
    if platform.system() == "Darwin":
        for key, query in (("cpu_model", "machdep.cpu.brand_string"), ("physical_memory_bytes", "hw.memsize")):
            completed = subprocess.run(["sysctl", "-n", query], capture_output=True, text=True, check=False)
            if completed.returncode == 0:
                raw = completed.stdout.strip()
                value[key] = int(raw) if key == "physical_memory_bytes" else raw
    elif platform.system() == "Linux":
        cpuinfo = Path("/proc/cpuinfo")
        if cpuinfo.is_file():
            for line in cpuinfo.read_text().splitlines():
                if line.startswith("model name"):
                    value["cpu_model"] = line.split(":", 1)[1].strip()
                    break
        try:
            value["physical_memory_bytes"] = os.sysconf("SC_PAGE_SIZE") * os.sysconf("SC_PHYS_PAGES")
        except (ValueError, OSError, AttributeError):
            pass
    return value


def source_identity(revision=None):
    def git(*arguments):
        result = subprocess.run(["git", "-C", str(REPOSITORY), *arguments],
                                capture_output=True, check=False)
        return result.stdout if result.returncode == 0 else b""
    changes = git("diff", "HEAD", "--", ".")
    tree = hashlib.sha256()
    files = 0
    for relative in sorted(set(git("ls-files", "--cached", "--others", "--exclude-standard", "-z").split(b"\0"))):
        if not relative:
            continue
        path = REPOSITORY / os.fsdecode(relative)
        if path.is_file():
            tree.update(relative + b"\0" + digest_file(path).encode() + b"\n")
            files += 1
    return {"revision": revision or git("rev-parse", "HEAD").decode().strip() or None,
            "working_tree_dirty": bool(git("status", "--porcelain")),
            "tracked_diff_sha256": hashlib.sha256(changes).hexdigest(),
            "working_tree_files_sha256": tree.hexdigest(), "working_tree_file_count": files,
            "binary_source_mapping": "revision supplied by caller" if revision else
                                     "current checkout; binary digest is authoritative"}


class Native:
    def __init__(self, binary, database, actor="researcher", writable=(), timeout=60):
        self.binary = Path(binary).resolve()
        self.database = Path(database)
        self.actor = actor
        self.writable = tuple(writable)
        self.timeout = timeout
        self.calls = []

    def request(self, value, expect_error=None):
        command = [str(self.binary), "--db", str(self.database), "--actor", self.actor]
        for graph in self.writable:
            command.extend(["--write", graph])
        started = time.perf_counter()
        completed = subprocess.run(command, input=canonical(value), capture_output=True,
                                   timeout=self.timeout, check=False)
        elapsed = time.perf_counter() - started
        try:
            envelope = json.loads(completed.stdout)
        except (ValueError, UnicodeError) as error:
            raise AssertionError(f"native stdout is not a JSON envelope: {completed.stdout[:500]!r}; "
                                 f"stderr={completed.stderr[:500]!r}") from error
        self.calls.append({"operation": value["operation"], "request_sha256": hashlib.sha256(canonical(value)).hexdigest(),
                           "elapsed_seconds": elapsed, "exit_code": completed.returncode,
                           "stdout_bytes": len(completed.stdout), "response_sha256": hashlib.sha256(canonical(envelope)).hexdigest()})
        assert envelope.get("science_version") == SCIENCE_VERSION, envelope
        assert envelope.get("contract_version") == CONTRACT_VERSION, envelope
        if expect_error is not None:
            assert not envelope.get("ok") and completed.returncode != 0, envelope
            codes = {expect_error} if isinstance(expect_error, str) else set(expect_error)
            assert envelope["error"]["code"] in codes, envelope
            return envelope["error"]
        assert completed.returncode == 0 and envelope.get("ok") is True, envelope
        return envelope["result"]

    def import_graph(self, graph_id, data, expected_head=None):
        result = self.request({"operation": "import", "graph_id": graph_id, "data": data,
                               "expected_head": expected_head})
        receipts = result["results"]
        assert len(receipts) == 1 and receipts[0]["kind"] in ("committed", "unchanged"), receipts
        return receipts[0]["revision"]

    def execute(self, commands):
        return self.request({"operation": "execute", "program": program(commands)})["results"]

    def query(self, query):
        result = self.execute([{"op": "query", "query": query}])
        assert len(result) == 1 and result[0]["kind"] == "queried", result
        return result[0]["result"]

    def analyze(self, input_query, algorithm, commands=None, **options):
        return self.request({"operation": "analyze", "program": program(commands or [{"op": "query", "query": input_query}]),
                             "analysis": {"algorithm": algorithm, **options}})


def program(commands):
    return {"version": CONTRACT_VERSION, "commands": commands}


def node(identifier, *, vector=None, readers=(), metadata=()):
    properties = {"integer_exactness": 9007199254740993}
    if vector is not None:
        properties["vector"] = vector
    return {"id": identifier, "entity_id": f"entity:{identifier}", "space_id": "experiment-space",
            "properties": properties, "readers": list(readers), "metadata": list(metadata)}


def edge(identifier, start, end, *, interval=(0, None), polarity="positive", readers=(), metadata=()):
    return {"id": identifier, "predicate": "link", "from": start, "to": end,
            "valid_time": {"start": interval[0], "end": interval[1]}, "polarity": polarity,
            "readers": list(readers), "metadata": list(metadata), "properties": {}}


def positive_pairs(graph, valid_at=None):
    return [(value["from"], value["to"]) for value in graph["edges"]
            if value.get("polarity", "positive") == "positive"
            and (valid_at is None or (value["valid_time"]["start"] <= valid_at
                 and (value["valid_time"]["end"] is None or valid_at < value["valid_time"]["end"])))]


def matrix_distances(identifiers, pairs, directed=True):
    """Floyd-Warshall, independent of the runtime BFS and Kosaraju algorithms."""
    identifiers = sorted(identifiers)
    offsets = {identifier: i for i, identifier in enumerate(identifiers)}
    result = [[math.inf] * len(identifiers) for _ in identifiers]
    for i in range(len(identifiers)):
        result[i][i] = 0
    for start, end in pairs:
        result[offsets[start]][offsets[end]] = min(result[offsets[start]][offsets[end]], 1)
        if not directed:
            result[offsets[end]][offsets[start]] = min(result[offsets[end]][offsets[start]], 1)
    for k in range(len(identifiers)):
        for i in range(len(identifiers)):
            for j in range(len(identifiers)):
                result[i][j] = min(result[i][j], result[i][k] + result[k][j])
    return identifiers, result


def component_oracle(identifiers, pairs, mode):
    ids, distances = matrix_distances(identifiers, pairs, directed=mode == "strong")
    remaining = set(ids)
    result = []
    for index, identifier in enumerate(ids):
        if identifier in remaining:
            group = [other for j, other in enumerate(ids)
                     if distances[index][j] < math.inf and distances[j][index] < math.inf]
            remaining.difference_update(group)
            result.append(group)
    return sorted(result)


def linear_solve(matrix, vector):
    """Gaussian elimination with partial pivoting, for small reference graphs."""
    rows = [row[:] + [value] for row, value in zip(matrix, vector)]
    size = len(rows)
    for column in range(size):
        pivot = max(range(column, size), key=lambda row: abs(rows[row][column]))
        rows[column], rows[pivot] = rows[pivot], rows[column]
        assert abs(rows[column][column]) > 1e-14, "singular PageRank reference system"
        scale = rows[column][column]
        rows[column] = [value / scale for value in rows[column]]
        for row in range(size):
            if row == column:
                continue
            factor = rows[row][column]
            rows[row] = [left - factor * right for left, right in zip(rows[row], rows[column])]
    return [row[-1] for row in rows]


def pagerank_oracle(identifiers, pairs, damping=0.85):
    ids = sorted(identifiers)
    size = len(ids)
    if not size:
        return {}
    index = {identifier: i for i, identifier in enumerate(ids)}
    counts = [[0] * size for _ in ids]
    for start, end in pairs:
        counts[index[start]][index[end]] += 1
    matrix = [[float(i == j) for j in range(size)] for i in range(size)]
    for start in range(size):
        total = sum(counts[start])
        for end in range(size):
            transition = counts[start][end] / total if total else 1 / size
            matrix[end][start] -= damping * transition
    return dict(zip(ids, linear_solve(matrix, [(1 - damping) / size] * size)))


def assert_close(actual, expected, tolerance=1e-9):
    assert math.isfinite(actual) and math.isfinite(expected), (actual, expected)
    assert abs(actual - expected) <= tolerance, (actual, expected, tolerance)


def graph_identifiers(value):
    return {node["id"] for node in value["graph"]["nodes"]}, {edge["id"] for edge in value["graph"]["edges"]}


def verify_topology(runtime, query, graph, *, valid_at=None, use_networkx=False):
    ids = sorted(value["id"] for value in graph["nodes"])
    pairs = positive_pairs(graph, valid_at)
    timing = []
    common = {} if valid_at is None else {"valid_at": valid_at}
    degree = runtime.analyze(query, "degree", **common)
    expected = {identifier: {"in_degree": sum(end == identifier for _, end in pairs),
                             "out_degree": sum(start == identifier for start, _ in pairs)} for identifier in ids}
    for value in expected.values():
        value["total_degree"] = value["in_degree"] + value["out_degree"]
    assert degree["analysis"]["nodes"] == expected, degree
    assert degree["analysis"]["node_count"] == len(ids)
    assert degree["analysis"]["edge_count"] == len(pairs)
    timing.append(degree["analysis"])
    for mode in ("weak", "strong"):
        components = runtime.analyze(query, "components", mode=mode, **common)
        assert sorted(components["analysis"]["components"]) == component_oracle(ids, pairs, mode), components
        timing.append(components["analysis"])
    for directed in (True, False):
        ordered, distances = matrix_distances(ids, pairs, directed)
        for source in ids[:3]:
            shortest = runtime.analyze(query, "shortest_paths", source=source, directed=directed, **common)
            expected_distance = {identifier: None if value == math.inf else value
                                 for identifier, value in zip(ordered, distances[ordered.index(source)])}
            assert shortest["analysis"]["distances"] == expected_distance, shortest
            for target, parent in shortest["analysis"]["predecessors"].items():
                if parent is not None:
                    assert (parent, target) in pairs or (not directed and (target, parent) in pairs)
                    assert expected_distance[parent] + 1 == expected_distance[target]
            timing.append(shortest["analysis"])
    rank = runtime.analyze(query, "pagerank", damping=0.85, tolerance=1e-12, max_iterations=1000, **common)
    assert rank["analysis"]["converged"] is True, rank
    reference = pagerank_oracle(ids, pairs)
    assert set(rank["analysis"]["scores"]) == set(ids)
    for identifier in ids:
        assert_close(rank["analysis"]["scores"][identifier], reference[identifier])
    assert_close(sum(rank["analysis"]["scores"].values()), 1 if ids else 0)
    if use_networkx and ids:
        import networkx as nx
        graph = nx.MultiDiGraph()
        graph.add_nodes_from(ids)
        graph.add_edges_from(pairs)
        independent = nx.pagerank(graph, alpha=0.85, tol=1e-13, max_iter=1000)
        for identifier in ids:
            assert_close(rank["analysis"]["scores"][identifier], independent[identifier])
    timing.append(rank["analysis"])
    return {"nodes": len(ids), "positive_edges": len(pairs), "analysis_sha256": hashlib.sha256(canonical(timing)).hexdigest()}


def run_acceptance(runtime, seed, random_cases, use_networkx=False, case_results=None):
    report = [] if case_results is None else case_results
    capabilities = runtime.request({"operation": "capabilities"})
    evidence = {"nodes": [node("evidence-public"), node("evidence-HIDDEN-x77", readers=("owner",))], "edges": []}
    evidence_revision = runtime.import_graph("evidence", evidence)
    reference = {"graph_id": "evidence", "revision": evidence_revision}
    private_id = "private-HIDDEN-node-x77"
    full = {"nodes": [node("A", vector=[1.0, 0.0], metadata=(reference,)),
                       node("B", vector=[0.0, 1.0]), node("C", vector=[1.0, 1.0]),
                       node("D", vector=[-1.0, 0.0]), node("I", vector=[1.0, 0.0]),
                       node(private_id, vector=[0.999, 0.001], readers=("owner",))],
            "edges": [edge("ab1", "A", "B", metadata=(reference,)), edge("ab2", "A", "B"),
                      edge("bc", "B", "C"), edge("ca", "C", "A"), edge("cc", "C", "C"),
                      edge("dd", "D", "D"), edge("negative-ad", "A", "D", polarity="negative"),
                      edge("HIDDEN-edge-x77", "A", private_id, readers=("owner",)),
                      edge("HIDDEN-path-x77", private_id, "D", readers=("owner",))]}
    revision = runtime.import_graph("science", full)
    query = {"graph_id": "science", "revision": revision, "include_metadata": True}
    selected = runtime.query(query)
    assert graph_identifiers(selected) == ({"A", "B", "C", "D", "I"},
                                         {"ab1", "ab2", "bc", "ca", "cc", "dd", "negative-ad"})
    assert "HIDDEN" not in json.dumps(selected), "denied topology or metadata leaked"
    assert selected["coverage"] == "complete", selected
    assert selected["graph"]["nodes"][0]["properties"]["integer_exactness"] == 9007199254740993
    resolved = {item["reference"]["graph_id"]: item for item in selected["metadata_graphs"]}
    assert resolved["evidence"]["reference"] == reference
    assert {value["id"] for value in resolved["evidence"]["graph"]["nodes"]} == {"evidence-public"}
    assert next(value for value in selected["graph"]["nodes"] if value["id"] == "A")["metadata"] == [reference]
    assert next(value for value in selected["graph"]["edges"] if value["id"] == "ab1")["metadata"] == [reference]
    assert selected["provenance"] and selected["input_snapshots"]
    topology = verify_topology(runtime, query, selected["graph"], use_networkx=use_networkx)
    # Every native call opens a new process and database connection.
    assert runtime.query(query) == selected, "reopen changed pinned snapshot"
    for algorithm, options in (("degree", {}), ("components", {"mode": "strong"}),
                               ("pagerank", {"max_iterations": 1000, "tolerance": 1e-12})):
        analyzed = runtime.analyze(query, algorithm, **options)
        assert analyzed["input"] == selected, "analytics discarded metadata/provenance/snapshot envelope"
    report.append({"case": "authorized-multigraph-metadata-reopen", "status": "passed", **topology,
                   "snapshots": selected["snapshots"], "input_snapshots": selected["input_snapshots"],
                   "query_result_sha256": hashlib.sha256(canonical(selected)).hexdigest()})

    neighbors = []
    for metric in ("euclidean", "cosine"):
        answer = runtime.analyze(query, "nearest_vectors", property="vector", space_id="experiment-space",
                                 query=[1.0, 0.0], metric=metric, k=10)
        expected = []
        for value in selected["graph"]["nodes"]:
            vector = value["properties"]["vector"]
            distance = math.dist(vector, [1.0, 0.0]) if metric == "euclidean" else 1 - vector[0] / math.hypot(*vector)
            expected.append((distance, value["id"]))
        expected.sort()
        assert [value["id"] for value in answer["analysis"]["neighbors"]] == [identifier for _, identifier in expected], answer
        candidates = {value["id"]: value for value in selected["graph"]["nodes"]}
        for actual, (distance, _) in zip(answer["analysis"]["neighbors"], expected):
            assert_close(actual["distance"], distance, 1e-12)
            assert actual["entity_id"] == candidates[actual["id"]]["entity_id"]
            assert actual["space_id"] == candidates[actual["id"]]["space_id"]
        assert answer["analysis"]["candidate_count"] == 5 and answer["analysis"]["dimensions"] == 2
        assert "HIDDEN" not in json.dumps(answer)
        neighbors.append(answer["analysis"])
    report.append({"case": "exact-vector-distance-ties-and-visibility", "status": "passed",
                   "analyses": neighbors})

    # Mutating hidden data cannot change visible analytics. Snapshot digests are
    # deliberately excluded: the declared underlying revision really changed.
    alternate = copy.deepcopy(full)
    alternate["nodes"][-1]["properties"]["vector"] = [-10000, 2]
    alternate["edges"].extend([edge(f"HIDDEN-extra-{i}", private_id, "C", readers=("owner",)) for i in range(8)])
    alternate_revision = runtime.import_graph("hidden-control", alternate)
    alternate_query = {"graph_id": "hidden-control", "revision": alternate_revision, "include_metadata": True}
    for algorithm, options in (("degree", {}), ("components", {"mode": "strong"}),
                               ("pagerank", {"max_iterations": 1000, "tolerance": 1e-12}),
                               ("nearest_vectors", {"property": "vector", "space_id": "experiment-space",
                                                    "query": [1.0, 0.0], "metric": "euclidean", "k": 5})):
        assert runtime.analyze(query, algorithm, **options)["analysis"] == runtime.analyze(alternate_query, algorithm, **options)["analysis"]
    report.append({"case": "hidden-input-pair-noninterference", "status": "passed"})

    temporal = {"nodes": [node(value) for value in ("A", "B", "C", "I")],
                "edges": [edge("early-ab", "A", "B", interval=(0, 10)),
                          edge("early-bc", "B", "C", interval=(0, 10)),
                          edge("later-ac", "A", "C", interval=(10, None))]}
    early = runtime.import_graph("temporal", temporal)
    recorded_commands = [{"op": "evaluate", "value": {"kind": "recorded_query", "query": {"graph_id": "temporal"},
                            "selection": {"kind": "local_time", "unix_millis": int(time.time() * 1000)}}}]
    recorded = runtime.execute(recorded_commands)[0]["result"]
    assert len(recorded["recorded_observations"]) == 1
    observation = recorded["recorded_observations"][0]
    assert observation["graph"] == {"graph_id": "temporal", "revision": early}
    for at, expected in ((-1, set()), (0, {"early-ab", "early-bc"}),
                         (9, {"early-ab", "early-bc"}), (10, {"later-ac"})):
        result = runtime.query({"graph_id": "temporal", "revision": early, "valid_at": at})
        assert {value["id"] for value in result["graph"]["edges"]} == expected, result
    for at in (0, 9, 10):
        verify_topology(runtime, {"graph_id": "temporal", "revision": early}, temporal, valid_at=at)
    corrected = copy.deepcopy(temporal)
    corrected["edges"] = [corrected["edges"][0], corrected["edges"][2], edge("late-correction-ac", "A", "C", interval=(5, 10))]
    latest = runtime.import_graph("temporal", corrected, expected_head=early)
    assert latest != early
    before = runtime.analyze({"graph_id": "temporal", "revision": early}, "shortest_paths", source="A", target="C", valid_at=5)
    after = runtime.analyze({"graph_id": "temporal"}, "shortest_paths", source="A", target="C", valid_at=5)
    assert before["analysis"]["distances"]["C"] == 2 and after["analysis"]["distances"]["C"] == 1
    assert before["analysis"]["path"] == ["A", "B", "C"] and after["analysis"]["path"] == ["A", "C"]
    checkpoint_command = [{"op": "evaluate", "value": {"kind": "recorded_query", "query": {"graph_id": "temporal"},
                           "selection": {"kind": "checkpoint", "observer": observation["observer"], "checkpoint": observation["checkpoint"]}}}]
    assert runtime.execute(checkpoint_command)[0]["result"] == recorded, "late correction mutated the recorded observation"
    error = runtime.request({"operation": "import", "graph_id": "temporal", "data": temporal, "expected_head": early}, expect_error="E_CONFLICT")
    assert runtime.query({"graph_id": "temporal"})["snapshots"]["temporal"] == latest
    report.append({"case": "valid-time-half-open-late-correction-recorded-reopen", "status": "passed",
                   "revisions": [early, latest], "observation": observation, "stale_write_error": error["code"]})

    left = {"nodes": [node("A"), node("B")], "edges": [edge("left-ab", "A", "B", interval=(0, 10))]}
    right = {"nodes": [node("B"), node("C")], "edges": [edge("right-bc", "B", "C", interval=(5, 15))]}
    left_revision = runtime.import_graph("left", left)
    right_revision = runtime.import_graph("right", right)
    join = {"op": "join", "left": {"graph_id": "left", "revision": left_revision},
            "right": {"graph_id": "right", "revision": right_revision}, "output_predicate": "derived",
            "match_on": "entity_space_to_from"}
    joined = runtime.execute([join])[0]["result"]
    assert len(joined["graph"]["edges"]) == 1, joined
    assertion = joined["graph"]["edges"][0]
    entities = {value["id"]: value["entity_id"] for value in joined["graph"]["nodes"]}
    assert (entities[assertion["from"]], entities[assertion["to"]], assertion["valid_time"]) == ("entity:A", "entity:C", {"start": 5, "end": 10}), assertion
    assert {value["assertion_id"] for value in joined["provenance"]} == {"left-ab", "right-bc"}
    assert {value["graph_id"] for value in joined["input_snapshots"]} == {"left", "right"}
    assert runtime.analyze(None, "shortest_paths", commands=[join], source=assertion["from"], target=assertion["to"])["analysis"]["distances"][assertion["to"]] == 1
    disjoint = copy.deepcopy(right)
    disjoint["edges"][0]["valid_time"] = {"start": 10, "end": 15}
    disjoint_revision = runtime.import_graph("disjoint", disjoint)
    disjoint_join = {**join, "right": {"graph_id": "disjoint", "revision": disjoint_revision}}
    assert runtime.execute([disjoint_join])[0]["result"]["graph"]["edges"] == []
    report.append({"case": "reusable-temporal-join-independent-premises", "status": "passed",
                   "input_snapshots": joined["input_snapshots"], "provenance": joined["provenance"]})

    random_generator = random.Random(seed)
    for index in range(random_cases):
        count = 2 + index % 7
        identifiers = [f"r{i}" for i in range(count)]
        generated = {"nodes": [node(value) for value in identifiers],
                     "edges": [edge(f"e{i}", random_generator.choice(identifiers), random_generator.choice(identifiers),
                                    polarity="negative" if i % 7 == 0 else "positive") for i in range(count * 3)]}
        identifier = f"random-{index}"
        runtime.import_graph(identifier, generated)
        oracle = verify_topology(runtime, {"graph_id": identifier}, generated, use_networkx=use_networkx)
        report.append({"case": "seeded-small-graph-oracle", "index": index, "status": "passed", **oracle})
    empty = {"nodes": [], "edges": []}
    runtime.import_graph("empty", empty)
    for algorithm, options in (("degree", {}), ("components", {"mode": "strong"}), ("pagerank", {})):
        answer = runtime.analyze({"graph_id": "empty"}, algorithm, **options)["analysis"]
        assert not answer.get("nodes", answer.get("components", answer.get("scores"))), answer
    report.append({"case": "empty-graph-defined-results", "status": "passed"})
    runtime.request({"operation": "analyze", "program": program([{"op": "query", "query": query}]),
                     "analysis": {"algorithm": "degree"}, "limits": {"max_nodes": 1}}, expect_error="E_SCIENCE_BUDGET")
    runtime.request({"operation": "analyze", "program": program([{"op": "query", "query": query}]),
                     "analysis": {"algorithm": "shortest_paths", "source": "not-a-node"}}, expect_error="E_SCIENCE_SOURCE")
    runtime.request({"operation": "analyze", "program": program([{"op": "commit", "graph_id": "forbidden-analysis-write", "data": empty}]),
                     "analysis": {"algorithm": "degree"}}, expect_error="E_SCIENCE_READ_ONLY")
    report.append({"case": "budget-invalid-source-and-pure-analysis-rejection", "status": "passed"})
    for options in ({"query": [1.0]}, {"query": [0.0, 0.0], "metric": "cosine"}):
        analysis = {"algorithm": "nearest_vectors", "property": "vector", "space_id": "experiment-space",
                    "query": [1.0, 0.0], "metric": "euclidean", "k": 5, **options}
        runtime.request({"operation": "analyze", "program": program([{"op": "query", "query": query}]),
                         "analysis": analysis}, expect_error="E_SCIENCE_VECTOR")
    report.append({"case": "vector-dimensions-and-zero-cosine-rejected", "status": "passed"})
    missing_reference = {"graph_id": "unavailable-evidence", "revision": "unavailable-revision"}
    runtime.import_graph("partial", {"nodes": [node("observed", metadata=(missing_reference,))], "edges": []})
    partial_query = {"graph_id": "partial", "include_metadata": True}
    partial = runtime.query(partial_query)
    assert partial["coverage"] == "partial" and partial["diagnostics"], partial
    request = {"operation": "analyze", "program": program([{"op": "query", "query": partial_query}]), "analysis": {"algorithm": "degree"}}
    runtime.request(request, expect_error="E_SCIENCE_PARTIAL")
    opted = runtime.request({**request, "allow_partial": True})
    assert opted["input"] == partial and opted["analysis"]["node_count"] == 1
    runtime.request({"operation": "execute", "program": program([{"op": "query", "query": {"graph_id": "unavailable-graph"}}])}, expect_error="E_UNAVAILABLE")
    runtime.request({"operation": "execute", "program": program([{"op": "query", "query": {"graph_id": "science", "revision": "unavailable-revision"}}])}, expect_error="E_UNAVAILABLE")
    report.append({"case": "partial-consent-retains-diagnostics-and-missing-snapshot-fails", "status": "passed",
                   "coverage": partial["coverage"], "diagnostics": partial["diagnostics"]})
    restricted = Native(runtime.binary, runtime.database, actor=runtime.actor, timeout=runtime.timeout)
    restricted.request({"operation": "import", "graph_id": "science", "expected_head": revision, "data": empty}, expect_error="E_FORBIDDEN")
    runtime.calls.extend(restricted.calls)
    assert runtime.query(query) == selected, "denied write changed pinned results"
    report.append({"case": "writes-require-independent-host-grant", "status": "passed"})
    return capabilities, report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", type=Path, required=True, help="built weave-science executable")
    parser.add_argument("--report", "--output", dest="report", type=Path, required=True)
    parser.add_argument("--revision", help="verified source revision used to build this executable")
    parser.add_argument("--seed", type=int, default=1337)
    parser.add_argument("--random-cases", type=int, default=6)
    parser.add_argument("--timeout", type=float, default=60)
    parser.add_argument("--networkx", action="store_true", help="also compare PageRank with installed NetworkX")
    args = parser.parse_args()
    if args.random_cases < 0 or args.timeout <= 0:
        parser.error("random-cases must be nonnegative and timeout must be positive")
    if not args.engine.is_file():
        parser.error("engine executable does not exist")
    report = {"format": "weave-science-acceptance/1", "status": "running", "seed": args.seed,
              "science_version": SCIENCE_VERSION, "contract_version": CONTRACT_VERSION,
              "binary_sha256": digest_file(args.engine), "source": source_identity(args.revision),
              "host": host_identity(),
              "cases": [], "oracles": ["Floyd-Warshall all-pairs dynamic programming", "Gaussian elimination PageRank linear system",
                          "exhaustive vector distances", "explicit immutable and half-open temporal fixture"]}
    if args.networkx:
        import networkx
        report["networkx_version"] = networkx.__version__
    started = time.perf_counter()
    with tempfile.TemporaryDirectory(prefix="weave-science-acceptance-") as temporary:
        writable = ["evidence", "science", "hidden-control", "temporal", "left", "right", "disjoint", "empty", "partial"]
        writable += [f"random-{index}" for index in range(args.random_cases)]
        runtime = Native(args.engine, Path(temporary) / "experiments.db", writable=writable, timeout=args.timeout)
        try:
            report["capabilities"], report["cases"] = run_acceptance(runtime, args.seed, args.random_cases, args.networkx, report["cases"])
            report["status"] = "passed"
        except Exception as error:
            report["status"] = "failed"
            report["failure"] = {"type": type(error).__name__, "message": str(error)}
            raise
        finally:
            report["calls"] = runtime.calls
            report["elapsed_seconds"] = time.perf_counter() - started
            report["limitations"] = ["native single-replica experiment acceptance, no distributed or app-platform claim",
                                     "unweighted positive directed multigraph analytics; model properties are not implicit weights",
                                     "pair noninterference compares released analytics, not underlying revision digests",
                                     "seeded finite cases supplement, rather than prove, general correctness"]
            args.report.parent.mkdir(parents=True, exist_ok=True)
            args.report.write_text(json.dumps(report, indent=2, allow_nan=False) + "\n")
    print(json.dumps({"status": report["status"], "cases": len(report["cases"]), "native_calls": len(report["calls"]),
                      "elapsed_seconds": report["elapsed_seconds"], "report": str(args.report)}))


if __name__ == "__main__":
    main()
