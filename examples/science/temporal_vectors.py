#!/usr/bin/env python3
"""Reproducible temporal graph correction and exact vector retrieval experiment."""

import argparse
import json
from pathlib import Path

from weave_science import Engine, Experiment, edge, node, query


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("experiment-output"))
    parser.add_argument("--binary", type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    database = args.output / "temporal-vectors.sqlite"
    if database.exists():
        parser.error(f"{database} already exists; choose a new --output directory")
    engine = Engine(database, actor="scientist", write_graphs=["documents"], binary=args.binary)
    records = [
        node("a", entity_id="document-A", space_id="toy-embedding-v1", vector=[1.0, 0.0], properties={"label": "alpha"}),
        node("b", entity_id="document-B", space_id="toy-embedding-v1", vector=[0.9, 0.1], properties={"label": "beta"}),
        node("c", entity_id="document-C", space_id="toy-embedding-v1", vector=[0.0, 1.0], properties={"label": "gamma"}),
        node("d", entity_id="document-D", space_id="toy-embedding-v1", vector=[-1.0, 0.0], properties={"label": "isolate"}),
    ]
    assertions = [edge("ab", "a", "b", predicate="cites", valid_from=0, valid_to=10),
                  edge("bc", "b", "c", predicate="cites", valid_from=0, valid_to=20),
                  edge("ca", "c", "a", predicate="cites", valid_from=10)]
    original = engine.import_graph("documents", nodes=records, edges=assertions)
    selected = engine.query("documents", revision=original.revision)
    selected.export_json(args.output / "original-graph.json")
    selected.export_csv(args.output / "tables")
    before = engine.analyze(selected, algorithm="components", mode="weak", valid_at=8)
    before.save_experiment(args.output / "before.json", label="connectivity before correction",
                           parameters={"valid_at": 8, "dataset": "toy-embedding-v1", "seed": 0})
    vectors = engine.nearest(selected, [1.0, 0.0], space_id="toy-embedding-v1", metric="cosine", k=3)
    vectors.save_experiment(args.output / "vectors.json", label="exact cosine neighbors")

    # A late correction changes valid time in a new immutable recorded revision.
    assertions[0]["valid_time"]["end"] = 6
    corrected = engine.import_graph("documents", nodes=records, edges=assertions,
                                    expected_head=original.revision)
    after = engine.analyze(query("documents", revision=corrected.revision),
                           algorithm="components", mode="weak", valid_at=8)
    after.save_experiment(args.output / "after.json", label="connectivity after correction",
                          parameters={"valid_at": 8})
    rank = engine.analyze(selected, algorithm="pagerank", valid_at=8,
                          damping=0.85, tolerance=1e-10, max_iterations=200)
    rank.save_experiment(args.output / "pagerank.json")

    # Reopen with no write grants and replay the old snapshot after the correction.
    reopened = Engine(database, actor="scientist", binary=args.binary)
    replayed = Experiment.load(args.output / "before.json").replay(reopened)
    assert replayed.analysis == before.analysis
    assert reopened.query("documents", revision=original.revision).graph == selected.graph
    report = {"old_revision": original.revision, "new_revision": corrected.revision,
              "components_before": before.analysis, "components_after": after.analysis,
              "exact_neighbors": vectors.analysis, "pagerank": rank.analysis,
              "old_snapshot_replayed_after_correction": True,
              "capabilities": reopened.capabilities()}
    (args.output / "report.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps(report, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
