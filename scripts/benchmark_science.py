#!/usr/bin/env python3
"""Reproducible native process timings for bounded graph experiments.

Reported latency includes database open, authorized engine read, analytics,
complete provenance-bearing JSON serialization and file transfer. It is not a
kernel-only latency. Fixture generation, Python decoding and builds are excluded.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
import random
import statistics
import subprocess
import tempfile
import time
from pathlib import Path

from check_science import (CONTRACT_VERSION, SCIENCE_VERSION, canonical, digest_file,
                           edge, host_identity, node, program, source_identity)


def fixture(count, seed, dimensions):
    generator = random.Random(seed)
    identifiers = [f"n{index:06d}" for index in range(count)]
    nodes = [node(identifier, vector=[generator.uniform(-1, 1) for _ in range(dimensions)],
                  readers=("owner",) if index % 10 == 0 else ())
             for index, identifier in enumerate(identifiers)]
    edges = []
    for index, identifier in enumerate(identifiers):
        edges.append(edge(f"ring-{index}", identifier, identifiers[(index + 1) % count]))
        edges.append(edge(f"chord-{index}", identifier, identifiers[generator.randrange(count)],
                          interval=(0, 100) if index % 2 else (50, 150)))
        if index % 20 == 0:
            edges.append(edge(f"loop-{index}", identifier, identifier))
    return {"nodes": nodes, "edges": edges}


def measured_request(binary, database, request, actor, writable, directory, timeout):
    command = [str(binary), "--db", str(database), "--actor", actor]
    for graph in writable:
        command.extend(["--write", graph])
    incoming, outgoing, errors = directory / "request.json", directory / "stdout.json", directory / "stderr.json"
    incoming.write_bytes(canonical(request))
    with incoming.open("rb") as source, outgoing.open("wb") as output, errors.open("wb") as stderr:
        started = time.perf_counter()
        process = subprocess.Popen(command, stdin=source, stdout=output, stderr=stderr)
        usage = None
        try:
            if hasattr(os, "wait4"):
                # A nonblocking wait keeps the process-level timeout enforceable.
                while True:
                    pid, status, collected = os.wait4(process.pid, os.WNOHANG)
                    if pid:
                        process.returncode = os.waitstatus_to_exitcode(status)
                        usage = collected
                        break
                    if time.perf_counter() - started > timeout:
                        process.kill()
                        _, status, usage = os.wait4(process.pid, 0)
                        process.returncode = os.waitstatus_to_exitcode(status)
                        raise TimeoutError(f"native request exceeded {timeout} seconds")
                    time.sleep(0.005)
            else:
                process.wait(timeout=timeout)
        except subprocess.TimeoutExpired as error:
            process.kill()
            process.wait()
            raise TimeoutError(f"native request exceeded {timeout} seconds") from error
        elapsed = time.perf_counter() - started
    envelope = json.loads(outgoing.read_bytes())
    assert envelope.get("science_version") == SCIENCE_VERSION and envelope.get("contract_version") == CONTRACT_VERSION, envelope
    rss = None
    if usage is not None:
        rss = usage.ru_maxrss if platform.system() == "Darwin" else usage.ru_maxrss * 1024
    measurement = {"elapsed_seconds": elapsed, "exit_code": process.returncode,
                   "peak_child_rss_bytes": rss, "request_bytes": incoming.stat().st_size,
                   "response_bytes": outgoing.stat().st_size,
                   "request_sha256": digest_file(incoming), "response_sha256": digest_file(outgoing)}
    if not envelope.get("ok"):
        measurement["outcome"] = "rejected"
        measurement["diagnostic"] = envelope.get("error")
        return measurement, None
    assert process.returncode == 0, envelope
    measurement["outcome"] = "passed"
    return measurement, envelope["result"]


def summarize(samples):
    timings = sorted(sample["elapsed_seconds"] for sample in samples)
    memory = [sample["peak_child_rss_bytes"] for sample in samples if sample["peak_child_rss_bytes"] is not None]
    result = {"sample_count": len(samples), "median_seconds": statistics.median(timings),
              "max_seconds": max(timings), "max_peak_child_rss_bytes": max(memory) if memory else None,
              "outcomes": sorted({sample["outcome"] for sample in samples})}
    if len(timings) >= 5:
        result["p95_seconds_nearest_rank"] = timings[math.ceil(len(timings) * .95) - 1]
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--report", "--output", dest="report", type=Path, required=True)
    parser.add_argument("--revision", help="verified source revision used to build the executable")
    parser.add_argument("--sizes", nargs="+", type=int, default=[1000, 10000], help="total input node counts")
    parser.add_argument("--samples", type=int, default=3)
    parser.add_argument("--dimensions", type=int, default=8)
    parser.add_argument("--seed", type=int, default=1337)
    parser.add_argument("--timeout", type=float, default=120)
    args = parser.parse_args()
    if not args.engine.is_file():
        parser.error("engine executable does not exist")
    if args.samples < 1 or any(size < 2 or size > 100000 for size in args.sizes):
        parser.error("samples must be positive and node sizes must be between 2 and 100000")
    if not 1 <= args.dimensions <= 16384 or args.timeout <= 0:
        parser.error("dimensions must be 1..16384 and timeout must be positive")
    binary = args.engine.resolve()
    report = {"format": "weave-science-benchmark/1", "status": "running", "seed": args.seed,
              "science_version": SCIENCE_VERSION, "contract_version": CONTRACT_VERSION,
              "binary_sha256": digest_file(binary), "source": source_identity(args.revision),
              "host": host_identity(),
              "measurement": "fresh native process and SQLite reopen; engine read, analytics, full JSON serialization and file transfer; excludes build, generation, and Python decoding",
              "memory_measurement": "wait4 per-child high-water RSS" if hasattr(os, "wait4") else "unavailable on this host",
              "limitations": ["synthetic deterministic single-replica workload, not a production SLO",
                              "latencies include full provenance-bearing query echo, not isolated algorithm timings",
                              "native child RSS excludes the Python harness and operating-system page cache",
                              "small samples report median and maximum; p95 appears only for at least five samples",
                              "input/output/work-budget rejection is not demonstrated dataset capacity"], "cases": []}
    started = time.perf_counter()
    try:
        with tempfile.TemporaryDirectory(prefix="weave-science-benchmark-") as temporary:
            directory = Path(temporary)
            for count in args.sizes:
                graph = fixture(count, args.seed + count, args.dimensions)
                visible_nodes = {value["id"] for value in graph["nodes"] if not value["readers"]}
                visible_edges = [value for value in graph["edges"] if value["from"] in visible_nodes and value["to"] in visible_nodes]
                metadata = {"nodes": [node(f"evidence-{index}") for index in range(16)], "edges": []}
                case = {"input_nodes": count, "input_edges": len(graph["edges"]),
                        "authorized_nodes": len(visible_nodes), "authorized_edges": len(visible_edges),
                        "edge_density": len(graph["edges"]) / count, "vector_dimensions": args.dimensions,
                        "visibility_partitions": 2, "metadata_depth": 1, "metadata_shared_graph_nodes": 16,
                        "samples": {}, "summaries": {}}
                database = directory / f"{count}.db"
                sample, result = measured_request(binary, database, {"operation": "import", "graph_id": "evidence", "data": metadata},
                                                  "researcher", ("evidence",), directory, args.timeout)
                if result is None:
                    case["metadata_import"] = sample
                    case["status"] = "rejected"
                    report["cases"].append(case)
                    continue
                evidence_ref = {"graph_id": "evidence", "revision": result["results"][0]["revision"]}
                for index in range(1, count, 50):
                    graph["nodes"][index]["metadata"] = [evidence_ref]
                for index in range(0, len(graph["edges"]), 50):
                    graph["edges"][index]["metadata"] = [evidence_ref]
                case["node_metadata_references"] = sum(bool(value["metadata"]) for value in graph["nodes"])
                case["edge_metadata_references"] = sum(bool(value["metadata"]) for value in graph["edges"])
                case["fixture_sha256"] = hashlib.sha256(canonical(graph)).hexdigest()
                imported, result = measured_request(binary, database, {"operation": "import", "graph_id": "benchmark", "data": graph},
                                                    "researcher", ("benchmark",), directory, args.timeout)
                case["samples"]["import"] = [imported]
                if result is None:
                    case["status"] = "rejected"
                    report["cases"].append(case)
                    continue
                revision = result["results"][0]["revision"]
                case["snapshot"] = {"graph_id": "benchmark", "revision": revision}
                selected = program([{"op": "query", "query": {"graph_id": "benchmark", "revision": revision,
                                                                 "include_metadata": True}}])
                operations = {"query": {"operation": "execute", "program": selected},
                              "degree": {"operation": "analyze", "program": selected, "analysis": {"algorithm": "degree", "valid_at": 75}},
                              "weak_components": {"operation": "analyze", "program": selected, "analysis": {"algorithm": "components", "mode": "weak", "valid_at": 75}},
                              "shortest_paths": {"operation": "analyze", "program": selected, "analysis": {"algorithm": "shortest_paths", "source": "n000001", "valid_at": 75}},
                              "pagerank": {"operation": "analyze", "program": selected, "analysis": {"algorithm": "pagerank", "max_iterations": 1000, "valid_at": 75}},
                              "nearest_vectors": {"operation": "analyze", "program": selected, "analysis": {"algorithm": "nearest_vectors", "property": "vector",
                                                                                                          "space_id": "experiment-space", "query": [1.0] + [0.0] * (args.dimensions - 1), "metric": "euclidean", "k": 10}}}
                for name, request in operations.items():
                    case["samples"][name] = []
                    for _ in range(args.samples):
                        sample, result = measured_request(binary, database, request, "researcher", (), directory, args.timeout)
                        case["samples"][name].append(sample)
                        if result is None:
                            continue
                        selected_result = result["results"][0]["result"] if name == "query" else result["input"]
                        assert selected_result["coverage"] == "complete"
                        assert len(selected_result["graph"]["nodes"]) == len(visible_nodes)
                        assert len(selected_result["graph"]["edges"]) == len(visible_edges)
                        assert selected_result["metadata_graphs"]
                        if name == "degree":
                            analysis = result["analysis"]
                            assert analysis["node_count"] == len(visible_nodes) and analysis["edge_count"] == len(visible_edges)
                            assert sum(value["out_degree"] for value in analysis["nodes"].values()) == len(visible_edges)
                            assert sum(value["in_degree"] for value in analysis["nodes"].values()) == len(visible_edges)
                        elif name == "weak_components":
                            assert sorted(value for group in result["analysis"]["components"] for value in group) == sorted(visible_nodes)
                        elif name == "shortest_paths":
                            assert result["analysis"]["distances"]["n000001"] == 0
                        elif name == "pagerank":
                            assert result["analysis"]["converged"] and abs(sum(result["analysis"]["scores"].values()) - 1) < 1e-8
                        elif name == "nearest_vectors":
                            expected = sorted((math.dist(value["properties"]["vector"], request["analysis"]["query"]), value["id"])
                                              for value in graph["nodes"] if value["id"] in visible_nodes)[:10]
                            assert [value["id"] for value in result["analysis"]["neighbors"]] == [identifier for _, identifier in expected]
                            assert all(abs(actual["distance"] - expected_distance) < 1e-10
                                       for actual, (expected_distance, _) in zip(result["analysis"]["neighbors"], expected))
                            assert result["analysis"]["candidate_count"] == len(visible_nodes)
                case["summaries"] = {name: summarize(samples) for name, samples in case["samples"].items()}
                case["status"] = "passed" if all(sample["outcome"] == "passed" for samples in case["samples"].values() for sample in samples) else "rejected"
                report["cases"].append(case)
                print(json.dumps({"nodes": count, "status": case["status"], "summaries": case["summaries"]}), flush=True)
        report["status"] = "passed" if all(case["status"] == "passed" for case in report["cases"]) else "rejected"
    except Exception as error:
        report["status"] = "failed"
        report["failure"] = {"type": type(error).__name__, "message": str(error)}
        raise
    finally:
        report["elapsed_seconds"] = time.perf_counter() - started
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2, allow_nan=False) + "\n")


if __name__ == "__main__":
    main()
