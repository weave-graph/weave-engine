"""Real installed SDK/native integration; set WEAVE_SCIENCE_BINARY to enable."""

import hashlib
import json
import os
import tempfile
import unittest
from pathlib import Path

from weave_science import Engine, Experiment, NativeError, ReproducibilityError, edge, node, query
from weave_science.results import canonical_json


@unittest.skipUnless(os.environ.get("WEAVE_SCIENCE_BINARY"), "set WEAVE_SCIENCE_BINARY for native integration")
class NativeIntegration(unittest.TestCase):
    def test_temporal_revision_cas_pinned_replay_and_reopen(self):
        with tempfile.TemporaryDirectory() as directory:
            db = Path(directory) / "science.db"
            engine = Engine(db, actor="scientist", write_graphs=["experiment"])
            records = [node("a", entity_id="A", space_id="toy", vector=[1.0, 0.0]),
                       node("b", entity_id="B", space_id="toy", vector=[0.8, 0.2]),
                       node("c", entity_id="C", space_id="toy", vector=[0.0, 1.0])]
            assertions = [edge("ab", "a", "b", valid_from=0, valid_to=10)]
            old = engine.import_graph("experiment", nodes=records, edges=assertions)
            selected = engine.query("experiment", revision=old.revision)
            stats = engine.analyze(selected, algorithm="components", mode="weak", valid_at=5)
            artifact = stats.save_experiment(Path(directory) / "experiment.json", parameters={"valid_at": 5})
            input_artifact = stats.input.save_experiment(Path(directory) / "input.json")
            assertions[0]["valid_time"]["end"] = 5
            new = engine.import_graph("experiment", nodes=records, edges=assertions, expected_head=old.revision)
            self.assertNotEqual(new.revision, old.revision)
            with self.assertRaises(NativeError):
                engine.import_graph("experiment", nodes=records, edges=assertions, expected_head=old.revision)
            reopened = Engine(db, actor="scientist")
            self.assertEqual(reopened.query("experiment", revision=old.revision).graph, selected.graph)
            self.assertEqual(Experiment.load(Path(directory) / "experiment.json").replay(reopened).analysis, stats.analysis)
            self.assertEqual(input_artifact.replay(reopened).raw, selected.raw)
            self.assertEqual(artifact.record["snapshots"], {"experiment": old.revision})
            self.assertEqual(reopened.nearest(selected, [1, 0], space_id="toy", k=2).input.snapshots,
                             selected.snapshots)
            self.assertEqual(reopened.analyze(query("experiment", revision=new.revision),
                                             algorithm="components", mode="weak", valid_at=5).input.coverage, "complete")

    def test_hidden_vectors_and_topology_are_authorized_before_analysis(self):
        with tempfile.TemporaryDirectory() as directory:
            db = Path(directory) / "restricted.db"
            owner = Engine(db, actor="owner", write_graphs=["g"])
            owner.import_graph("g", nodes=[node("visible", space_id="toy", vector=[0.0, 1.0]),
                                          node("hidden", space_id="toy", vector=[1.0, 0.0], readers=["owner"])],
                               edges=[edge("secret", "visible", "hidden", readers=["owner"])])
            outsider = Engine(db, actor="outsider")
            selected = outsider.query("g")
            self.assertEqual([n["id"] for n in selected.nodes], ["visible"])
            self.assertEqual(selected.edges, [])
            nearest = outsider.nearest(selected, [1, 0], space_id="toy", k=2)
            self.assertEqual([n["id"] for n in nearest.input.nodes], ["visible"])
            with self.assertRaises(NativeError) as failure:
                outsider.import_graph("forbidden", nodes=[node("n")])
            self.assertEqual(failure.exception.code, "E_FORBIDDEN")

    def test_pinned_root_cannot_silently_replay_changed_live_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            engine = Engine(Path(directory) / "live.db", actor="scientist", write_graphs=["g", "evidence"])
            first_metadata = engine.import_graph("evidence", nodes=[node("before")])
            root = engine.import_graph("g", {"nodes": [node("root")], "edges": [], "attachments": [
                {"id": "live-evidence", "host": {"kind": "graph"}, "key": "evidence",
                 "value": {"kind": "live_graph", "graph_id": "evidence", "branch_id": "main"},
                 "valid_time": {"start": 0, "end": None}}]})
            original = engine.query("g", revision=root.revision, include_metadata=True)
            analysis = engine.analyze(original, algorithm="degree")
            artifact_path = Path(directory) / "live.json"
            analysis.save_experiment(artifact_path)
            engine.import_graph("evidence", nodes=[node("after")], expected_head=first_metadata.revision)
            current = engine.query("g", revision=root.revision, include_metadata=True)
            self.assertNotEqual(current.graph, original.graph)
            with self.assertRaises(ReproducibilityError):
                engine.analyze(original, algorithm="degree")
            with self.assertRaises(ReproducibilityError):
                Experiment.load(artifact_path).replay(engine)

    def test_rehashed_artifact_commit_is_rejected_without_persisting_graph(self):
        with tempfile.TemporaryDirectory() as directory:
            engine = Engine(Path(directory) / "pure.db", actor="scientist", write_graphs=["g", "forged"])
            engine.import_graph("g", nodes=[node("n")])
            path = Path(directory) / "artifact.json"
            engine.query("g").save_experiment(path)
            record = json.loads(path.read_text())
            record["replay_request"] = {"operation": "execute", "program": {"version": "0.21.0", "commands": [
                {"op": "commit", "graph_id": "forged", "data": {"nodes": [node("surprise")]}}]}}
            record.pop("sha256")
            record["sha256"] = hashlib.sha256(canonical_json(record).encode()).hexdigest()
            path.write_text(json.dumps(record))
            with self.assertRaisesRegex(ValueError, "read-only"):
                Experiment.load(path).replay(engine)
            # A create-only CAS succeeds only because replay did not create it.
            self.assertTrue(engine.import_graph("forged", nodes=[node("intentional")]).changed)


if __name__ == "__main__":
    unittest.main()
