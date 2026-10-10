import hashlib
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from weave_science import (
    Engine, Experiment, NativeError, OperationTimeout, ProtocolError, QueryResult,
    edge, graph_data, join, node, program, query, read_csv, recorded_query,
)
from weave_science.results import canonical_json


def envelope(result=None, **fields):
    return {"ok": True, "science_version": "0.1.0", "contract_version": "0.21.0",
            "result": result or {}, **fields}


def native_result(graph_id="g", revision="r1"):
    return {"version": "0.21.0", "graph": {"nodes": [], "edges": []},
            "snapshots": {graph_id: revision},
            "input_snapshots": [{"graph_id": graph_id, "revision": revision}],
            "coverage": "complete", "diagnostics": [], "provenance": [],
            "metadata_graphs": []}


class ClientTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.binary = Path(self.tmp.name) / "native with spaces"
        self.binary.write_text("#!/bin/sh\nexit 0\n")
        self.binary.chmod(0o755)
        self.engine = Engine(Path(self.tmp.name) / "data with spaces.db", binary=self.binary,
                             actor="principal with spaces", write_graphs=["graph with spaces"])

    def respond(self, value, returncode=0):
        return patch("weave_science.client.subprocess.run", return_value=subprocess.CompletedProcess(
            [], returncode, json.dumps(value).encode(), b""))

    def test_transport_uses_argument_vector_and_finite_timeout(self):
        with self.respond(envelope()) as run:
            self.engine.capabilities()
        args, kwargs = run.call_args
        self.assertEqual(args[0], [str(self.binary.resolve()), "--db", str(self.engine.db), "--actor",
                                  "principal with spaces", "--write", "graph with spaces"])
        self.assertNotIn("shell", kwargs)
        self.assertEqual(kwargs["timeout"], 30.0)
        self.assertEqual(json.loads(kwargs["input"]), {"operation": "capabilities"})

    def test_import_uses_cas_and_preserves_data(self):
        snapshot = graph_data([node("n", entity_id="entity", space_id="physical")])
        receipt = {"kind": "committed", "revision": "r2", "event_id": "ev"}
        with self.respond(envelope({"results": [receipt]})) as run:
            actual = self.engine.import_graph("g", snapshot, expected_head="r1")
        request = json.loads(run.call_args.kwargs["input"])
        self.assertEqual(request["expected_head"], "r1")
        self.assertEqual(request["data"], snapshot)
        self.assertEqual(actual.revision, "r2")
        self.assertTrue(actual.changed)

    def test_native_denial_preserves_code_and_envelope(self):
        response = envelope(ok=False, error={"code": "E_DENIED", "message": "write denied"})
        with self.respond(response, returncode=1):
            with self.assertRaises(NativeError) as failure:
                self.engine.import_graph("g")
        self.assertEqual(failure.exception.code, "E_DENIED")
        self.assertEqual(failure.exception.envelope, response)

    def test_timeout_is_not_retried_and_mutation_is_uncertain(self):
        with patch("weave_science.client.subprocess.run", side_effect=subprocess.TimeoutExpired([], 0.1)) as run:
            with self.assertRaises(OperationTimeout) as failure:
                self.engine.import_graph("g")
        self.assertEqual(run.call_count, 1)
        self.assertEqual(failure.exception.commit_status, "unknown")

    def test_invalid_and_future_protocols_fail_explicitly(self):
        for response in ([], {"ok": True}, envelope(science_version="1.0.0"),
                         envelope(contract_version="1.0.0"), envelope(contract_version="0.22.0")):
            with self.subTest(response=response), self.respond(response):
                with self.assertRaises(ProtocolError):
                    self.engine.capabilities()

    def test_native_success_must_match_exit_status(self):
        with self.respond(envelope(), returncode=9):
            with self.assertRaises(ProtocolError):
                self.engine.capabilities()

    def test_query_diagnostics_and_snapshots_are_retained(self):
        result = native_result()
        result["coverage"] = "partial"
        result["diagnostics"] = [{"code": "E_MISSING", "message": "dependency unavailable"}]
        with self.respond(envelope({"results": [{"kind": "queried", "result": result}]})):
            actual = self.engine.query("g")
        self.assertEqual(actual.coverage, "partial")
        self.assertEqual(actual.diagnostics, result["diagnostics"])
        self.assertEqual(actual.pinned_program["commands"][0]["value"]["query"]["revision"], "r1")

    def test_analysis_of_old_result_reauthorizes_exact_revision(self):
        result = QueryResult(native_result(), context={"request": {"operation": "execute", "program": program(query("g"))}})
        with self.respond(envelope({"input": native_result(), "analysis": {"algorithm": "degree"}, "semantics": {}})) as run:
            self.engine.analyze(result, valid_at=7)
        request = json.loads(run.call_args.kwargs["input"])
        self.assertEqual(request["program"]["commands"][0]["value"]["query"]["revision"], "r1")
        self.assertEqual(request["analysis"]["valid_at"], 7)
        self.assertFalse(request["allow_partial"])

    def test_evaluate_rejects_bad_index_and_mutations_before_execution(self):
        with self.respond(envelope()) as run:
            for index in (-1, True, 0.5):
                with self.assertRaises(ValueError):
                    self.engine.evaluate(query("g"), result_index=index)
            with self.assertRaises(ValueError):
                self.engine.evaluate({"version": "0.21.0", "commands": [{"op": "commit", "graph_id": "g"}]})
        run.assert_not_called()

    def test_replaced_binary_cannot_receive_stale_identity(self):
        self.binary.write_text("#!/bin/sh\nexit 1\n")
        from weave_science import WeaveError
        with self.respond(envelope()) as run:
            with self.assertRaisesRegex(WeaveError, "binary changed"):
                self.engine.capabilities()
        run.assert_not_called()

    def test_analysis_input_keeps_selected_program_result_index(self):
        source = {"version": "0.21.0", "commands": [{"op": "bind", "name": "x", "value": query("g")},
                                                   {"op": "evaluate", "value": {"kind": "reference", "name": "x"}}]}
        with self.respond(envelope({"input": native_result(), "analysis": {"algorithm": "degree"}, "semantics": {}})):
            result = self.engine.analyze(source, result_index=1)
        with self.respond(envelope({"input": native_result(), "analysis": {"algorithm": "degree"}, "semantics": {}})) as run:
            self.engine.analyze(result.input)
        self.assertEqual(json.loads(run.call_args.kwargs["input"])["result_index"], 1)


class DataAndArtifactTests(unittest.TestCase):
    def test_helpers_do_not_mutate_caller_rows(self):
        properties = {"label": "sample", "nested": [1, 2]}
        record = node("m1", entity_id="e", properties=properties, vector=[1, 2])
        snapshot = graph_data([record], [edge("a", "m1", "m1", valid_from=-5, valid_to=9)])
        snapshot["nodes"][0]["properties"]["nested"].append(3)
        self.assertEqual(properties, {"label": "sample", "nested": [1, 2]})
        self.assertEqual(record["properties"]["nested"], [1, 2])

    def test_invalid_vector_and_interval_fail_early(self):
        for coordinates in ([float("nan")], [True], [], ["1"]):
            with self.assertRaises(ValueError):
                node("n", vector=coordinates)
        for start, end in ((5, 5), (True, 7), (0, 2**63)):
            with self.assertRaises(ValueError):
                edge("e", "a", "b", valid_from=start, valid_to=end)

    def test_csv_explicit_types_and_bad_rows(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "nodes.csv"
            path.write_text('id,vector,timestamp\na,"[1,2]",5\nb,,\n')
            values = read_csv(path, json_columns=["vector"], integer_columns=["timestamp"])
            self.assertEqual(values[0], {"id": "a", "vector": [1, 2], "timestamp": 5})
            self.assertEqual(values[1], {"id": "b", "vector": None, "timestamp": None})
            path.write_text("id,id\na,b\n")
            with self.assertRaises(ValueError):
                read_csv(path)
            path.write_text("id,vector\na,NaN\n")
            with self.assertRaises(ValueError):
                read_csv(path, json_columns=["vector"])

    def test_nested_joins_pin_actual_selected_snapshots(self):
        value = native_result("left", "r-left")
        value["input_snapshots"].append({"graph_id": "right", "revision": "r-right"})
        expression = join(query("left"), query("right"), output_predicate="through")
        result = QueryResult(value, context={"request": {"program": program(expression)}})
        pinned = result.pinned_program["commands"][0]["value"]
        self.assertEqual(pinned["left"]["query"]["revision"], "r-left")
        self.assertEqual(pinned["right"]["query"]["revision"], "r-right")
        self.assertNotIn("revision", expression["left"]["query"])

    def test_legacy_join_command_pins_both_raw_query_plans(self):
        value = native_result("left", "r-left")
        value["input_snapshots"].append({"graph_id": "right", "revision": "r-right"})
        source = {"version": "0.21.0", "commands": [{"op": "join", "left": {"graph_id": "left"},
                    "right": {"graph_id": "right"}, "output_predicate": "through", "match_on": "entity_space_to_from"}]}
        pinned = QueryResult(value, context={"request": {"program": source}}).pinned_program
        self.assertEqual(pinned["commands"][0]["left"]["revision"], "r-left")
        self.assertEqual(pinned["commands"][0]["right"]["revision"], "r-right")

    def test_ambiguous_and_live_pins_are_not_claimed_reproducible(self):
        value = native_result()
        value["input_snapshots"].append({"graph_id": "g", "revision": "r2"})
        result = QueryResult(value, context={"request": {"program": program(query("g"))}})
        with self.assertRaises(ValueError):
            _ = result.pinned_program
        result = QueryResult(native_result(), context={"request": {"program": program({"kind": "current_view", "selection": {}})}})
        with self.assertRaises(ValueError):
            _ = result.pinned_program

    def test_recorded_time_replay_uses_runtime_checkpoint_witness(self):
        value = native_result()
        value["recorded_observations"] = [{"observer": "replica", "checkpoint": "cp1",
            "graph": {"graph_id": "g", "revision": "r1"}, "branch_id": "main",
            "recorded_at_ms": 99, "kind": "committed"}]
        result = QueryResult(value, context={"request": {"program": program(recorded_query("g", recorded_at_ms=100))}})
        pinned = result.pinned_program["commands"][0]["value"]
        self.assertEqual(pinned["selection"], {"kind": "checkpoint", "observer": "replica", "checkpoint": "cp1"})
        self.assertNotIn("revision", pinned["query"])

    def test_artifact_exact_pins_versions_and_tamper_detection(self):
        context = {"actor": "researcher", "binary_sha256": "b" * 64,
                   "science_version": "0.1.0", "contract_version": "0.21.0",
                   "request": {"operation": "execute", "program": program(query("g"))}}
        result = QueryResult(native_result(), context=context)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "artifact.json"
            saved = result.save_experiment(path, label="trial", parameters={"seed": 7})
            loaded = Experiment.load(path)
            self.assertEqual(saved.record, loaded.record)
            self.assertEqual(loaded.record["parameters"], {"seed": 7})
            pinned = loaded.record["replay_request"]["program"]["commands"][0]["value"]["query"]
            self.assertEqual(pinned["revision"], "r1")
            changed = json.loads(path.read_text())
            changed["snapshots"]["g"] = "fake"
            path.write_text(json.dumps(changed))
            with self.assertRaisesRegex(ValueError, "integrity"):
                Experiment.load(path)

    def test_mutating_program_is_never_replayed_from_artifact(self):
        source = {"version": "0.21.0", "commands": [{"op": "commit", "graph_id": "g"}]}
        result = QueryResult(native_result(), context={"actor": "researcher", "science_version": "0.1.0",
            "contract_version": "0.21.0", "request": {"operation": "execute", "program": source}})
        with tempfile.TemporaryDirectory() as directory:
            saved = result.save_experiment(Path(directory) / "record.json")
            self.assertIsNone(saved.record["replay_request"])
            self.assertIn("read-only", saved.record["replay_unavailable"])

    def test_rehashed_external_artifact_cannot_run_mutating_replay(self):
        context = {"actor": "researcher", "science_version": "0.1.0", "contract_version": "0.21.0",
                   "request": {"operation": "execute", "program": program(query("g"))}}
        artifact = Experiment.create(native_result(), context, label=None, parameters=None)
        for malicious in ({"operation": "import", "graph_id": "new"},
                          {"operation": "execute", "program": {"version": "0.21.0", "commands": [{"op": "commit"}]}},
                          {"operation": "execute", "program": {"version": "0.21.0", "commands": [{"op": "future_mutation"}]}}):
            record = artifact.record.copy()
            record["replay_request"] = malicious
            record.pop("sha256")
            record["sha256"] = hashlib.sha256(canonical_json(record).encode()).hexdigest()
            loaded = Experiment(record)
            class FakeEngine:
                actor = "researcher"
                def _request(self, request):
                    raise AssertionError("a mutating replay must never launch the engine")
            with self.subTest(request=malicious), self.assertRaises(ValueError):
                loaded.replay(FakeEngine())


if __name__ == "__main__":
    unittest.main()
