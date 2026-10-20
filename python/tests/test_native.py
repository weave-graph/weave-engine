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

    def test_incremental_fact_batches_after_snapshot_import(self):
        with tempfile.TemporaryDirectory() as directory:
            db = Path(directory) / "facts.db"
            engine = Engine(db, actor="importer", write_graphs=["fleet"])
            base = engine.import_graph("fleet", nodes=[node("a", entity_id="A", space_id="s"),
                                                       node("b", entity_id="B", space_id="s")],
                                       edges=[edge("ab", "a", "b", valid_from=0)])
            unit = {"kind": "node", "id": "u", "entity_id": "U", "space_id": "s",
                    "valid_time": {"start": 10, "end": 50},
                    "provenance": {"source": "erp", "method": {"kind": "import", "importer": "csv", "version": "1"}}}
            first = engine.import_facts("fleet", [{"change": "assert", "fact": unit}], expected_head=base.revision)
            self.assertTrue(first.changed and first.revision.startswith("fact:sha256:"))
            second = engine.import_facts("fleet", [{"change": "retract", "target": {"kind": "edge", "id": "ab"}}])
            again = engine.import_facts("fleet", [{"change": "assert", "fact": unit}])
            self.assertFalse(again.changed)
            self.assertEqual(again.revision, second.revision)
            with self.assertRaises(NativeError) as stale:
                engine.import_facts("fleet", [{"change": "retract", "target": {"kind": "node", "id": "u"}}],
                                    expected_head=first.revision)
            self.assertEqual(stale.exception.code, "E_CONFLICT")
            reopened = Engine(db, actor="analyst")
            self.assertEqual(sorted(n["id"] for n in reopened.query("fleet").nodes), ["a", "b", "u"])
            self.assertEqual([e["id"] for e in reopened.query("fleet", revision=first.revision).edges], ["ab"])
            self.assertEqual([n["id"] for n in reopened.query("fleet", valid_at=60).nodes], [])
            program = {"version": "0.22.0", "commands": [{"op": "query", "query": {"graph_id": "fleet"}}]}
            provenance = reopened.execute(program)["results"][0]["result"]["fact_provenance"]
            unit_entry = [p for p in provenance if p["fact"].get("node_id") == "u"][0]["provenance"]
            self.assertEqual((unit_entry["writer"], unit_entry["source"], unit_entry["method"]["kind"]),
                             ("importer", "erp", "import"))

    def test_expand_as_of_changes_and_explain_for_a_restricted_reader(self):
        import time
        with tempfile.TemporaryDirectory() as directory:
            db = Path(directory) / "fleet.db"
            loader = Engine(db, actor="loader", write_graphs=["fleet"])
            unit = lambda i, readers=(): {"kind": "node", "id": i, "entity_id": i.upper(),
                                          "space_id": "fleet", "readers": list(readers)}
            at = lambda i, u: {"kind": "edge", "id": i, "predicate": "located_at", "from": u,
                               "to": "site", "valid_time": {"start": 0}}
            first = loader.import_facts("fleet", [{"change": "assert", "fact": f} for f in [
                unit("site"), unit("u1"), unit("u2", ["admin"]), at("a1", "u1"), at("a2", "u2")]])
            time.sleep(0.01)
            known = int(time.time() * 1000)
            time.sleep(0.01)
            loader.import_facts("fleet", [{"change": "assert", "fact": unit("u3")},
                                          {"change": "assert", "fact": at("a3", "u3")},
                                          {"change": "retract", "target": {"kind": "edge", "id": "a1"}}])
            analyst = Engine(db, actor="analyst")
            # k-hop: the restricted unit stays out.
            near = analyst.expand("fleet", ["site"], hops=1, direction="in")
            self.assertEqual(sorted(n["id"] for n in near.nodes), ["site", "u3"])
            self.assertEqual(sorted(e["id"] for e in near.edges), ["a3"])
            self.assertEqual(near.coverage, "complete")
            self.assertEqual(len(near.fact_provenance), len(near.nodes) + len(near.edges))
            # As known then: the visible subset, not E_HISTORY_UNAVAILABLE.
            then = analyst.as_of("fleet", known, predicate="located_at")
            self.assertEqual(sorted(e["id"] for e in then.edges), ["a1"])
            with self.assertRaises(NativeError) as denied:
                analyst.recorded_query("fleet", recorded_at_ms=known, predicate="located_at")
            self.assertEqual(denied.exception.code, "E_HISTORY_UNAVAILABLE")
            self.assertEqual(sorted(e["id"] for e in analyst.expand(
                "fleet", ["site"], hops=1, direction="in", as_of=known).edges), ["a1"])
            # What changed, and the pull feed continuing from its cursor.
            changed = analyst.changes("fleet", since_cursor=first.revision)
            self.assertEqual(sorted((c["change"], c["kind"], c["id"]) for c in changed.changes),
                             [("asserted", "edge", "a3"), ("asserted", "node", "u3"), ("retracted", "edge", "a1")])
            self.assertEqual(len(changed.fact_provenance), 3)
            self.assertEqual(analyst.changes("fleet", since_cursor=changed.cursor).changes, [])
            self.assertEqual([c["id"] for c in analyst.changes("fleet", since_ms=known, entities=["U3"]).changes], ["u3", "a3"])
            admin = Engine(db, actor="admin").changes("fleet", since_ms=0, until_ms=known)
            self.assertIn(("asserted", "edge", "a2"), [(c["change"], c["kind"], c["id"]) for c in admin.changes])
            # Explanations and exact replay of pinned results.
            self.assertTrue(analyst.explain({"kind": "expand", "graph_id": "fleet", "seeds": ["site"],
                                             "hops": 1, "direction": "in"}).nodes)
            for result in (then, near, changed):
                replayed = analyst.evaluate(result.pinned_program)
                self.assertEqual(replayed.raw["graph"], result.raw["graph"])
                self.assertEqual(replayed.changes, result.changes)

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
