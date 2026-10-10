"""Authorized result envelopes and reproducible experiment artifacts."""

from __future__ import annotations

import csv
import hashlib
import json
import os
import tempfile
from copy import deepcopy
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping, TYPE_CHECKING

from .expressions import validate_read_only

if TYPE_CHECKING:
    from .client import Engine


def canonical_json(value: Any) -> str:
    return json.dumps(value, sort_keys=True, ensure_ascii=False, allow_nan=False,
                      separators=(",", ":"))


class ReproducibilityError(ValueError):
    """Current authorized dependencies differ from an experiment's actual input."""


def assert_same_input(expected: Mapping[str, Any], actual: Mapping[str, Any]) -> None:
    if canonical_json(expected) != canonical_json(actual):
        raise ReproducibilityError("selected input changed under current policy or live dependencies; exact replay is unavailable")


def _save_json(path: str | Path, value: Any) -> Path:
    destination = Path(path)
    destination.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix=f".{destination.name}.", dir=destination.parent)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
            stream.write(json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False,
                                    allow_nan=False) + "\n")
        os.replace(temporary, destination)
    except BaseException:
        Path(temporary).unlink(missing_ok=True)
        raise
    return destination


def _pin_program(source: Mapping[str, Any], result: Mapping[str, Any]) -> dict[str, Any]:
    """Pin only revisions unambiguously witnessed by this successful result."""
    pins: dict[str, set[str]] = {}
    for graph_id, revision in result.get("snapshots", {}).items():
        pins.setdefault(graph_id, set()).add(revision)
    for reference in result.get("input_snapshots", []):
        pins.setdefault(reference["graph_id"], set()).add(reference["revision"])
    witnesses = result.get("recorded_observations", [])
    pinned = deepcopy(dict(source))
    validate_read_only(pinned)

    def pin_query(plan: dict[str, Any]) -> None:
        if plan.get("revision") is None:
            choices = pins.get(plan["graph_id"], set())
            if len(choices) != 1:
                raise ValueError(f"no unique observed revision for {plan['graph_id']!r}; pin the query explicitly")
            plan["revision"] = next(iter(choices))

    def visit(value: Any) -> None:
        if isinstance(value, list):
            for child in value:
                visit(child)
        elif isinstance(value, dict):
            if value.get("kind") in {"current_view", "accepted_history", "accepted_graph",
                                     "resolve_identity", "cluster", "geometry"}:
                raise ValueError("runtime service/view expressions require explicitly verified immutable selections for replay")
            if value.get("kind") == "recorded_query":
                plan, selected = value["query"], value["selection"]
                if selected.get("kind") == "local_time":
                    candidates = [w for w in witnesses
                                  if w["graph"]["graph_id"] == plan["graph_id"]
                                  and w["branch_id"] == plan.get("branch_id", "main")
                                  and w["recorded_at_ms"] <= selected["unix_millis"]]
                    if not candidates:
                        raise ValueError("recorded query has no exact checkpoint witness")
                    latest = max(w["recorded_at_ms"] for w in candidates)
                    candidates = [w for w in candidates if w["recorded_at_ms"] == latest]
                    if len(candidates) != 1:
                        raise ValueError("ambiguous recorded checkpoint; provide an explicit checkpoint")
                    witness = candidates[0]
                    value["selection"] = {"kind": "checkpoint", "observer": witness["observer"],
                                          "checkpoint": witness["checkpoint"]}
                return  # recorded_query forbids a simultaneous explicit revision
            if value.get("kind") == "query" or value.get("op") == "query":
                pin_query(value["query"])
            if value.get("op") == "join":
                # Legacy Command::Join carries QueryPlans, not expressions.
                pin_query(value["left"])
                pin_query(value["right"])
            for child in value.values():
                visit(child)

    visit(pinned)
    return pinned


@dataclass(frozen=True)
class CommitReceipt:
    revision: str
    event_id: str | None
    changed: bool
    raw: dict[str, Any]


class QueryResult:
    """A copy of the native authorized result, including coverage and provenance."""

    def __init__(self, raw: Mapping[str, Any], *, context: Mapping[str, Any] | None = None):
        self.raw = deepcopy(dict(raw))
        self._context = deepcopy(dict(context or {}))

    @property
    def graph(self) -> dict[str, Any]:
        return deepcopy(self.raw["graph"])

    @property
    def nodes(self) -> list[dict[str, Any]]:
        return deepcopy(self.raw["graph"].get("nodes", []))

    @property
    def edges(self) -> list[dict[str, Any]]:
        return deepcopy(self.raw["graph"].get("edges", []))

    @property
    def snapshots(self) -> dict[str, str]:
        return deepcopy(self.raw["snapshots"])

    @property
    def coverage(self) -> str:
        return self.raw["coverage"]

    @property
    def diagnostics(self) -> list[dict[str, str]]:
        return deepcopy(self.raw.get("diagnostics", []))

    @property
    def provenance(self) -> list[dict[str, str]]:
        return deepcopy(self.raw.get("provenance", []))

    @property
    def pinned_program(self) -> dict[str, Any]:
        source = self._context.get("request", {}).get("program")
        if source is None:
            raise ValueError("this result has no source Program; query through Engine first")
        return _pin_program(source, self.raw)

    def export_json(self, path: str | Path, *, graph_only: bool = False) -> Path:
        """Export full provenance envelope by default, or an importable snapshot."""
        return _save_json(path, self.graph if graph_only else self.raw)

    def export_csv(self, directory: str | Path) -> tuple[Path, Path]:
        """Table export with JSON complex cells. Keep JSON for lossless records.

        CSV needs an explicit column schema; None and empty text share a blank
        cell. Full JSON preserves nulls and scalar types without ambiguity.
        """
        destination = Path(directory)
        destination.mkdir(parents=True, exist_ok=True)
        files = (destination / "nodes.csv", destination / "edges.csv")
        for records, path in zip((self.nodes, self.edges), files):
            names = sorted({name for row in records for name in row})
            if not names:
                names = ["id"]
            with path.open("w", newline="", encoding="utf-8") as stream:
                writer = csv.DictWriter(stream, fieldnames=names)
                writer.writeheader()
                writer.writerows({key: canonical_json(value) if isinstance(value, (dict, list, bool))
                                  else value for key, value in row.items()} for row in records)
        return files

    def to_pandas(self) -> tuple[Any, Any]:
        """Return (node_frame, edge_frame); install the optional tables extra."""
        try:
            import pandas as pd
        except ImportError as exc:
            raise ImportError("pandas conversion requires pip install 'weave-science[tables]'") from exc
        return pd.DataFrame(self.nodes), pd.DataFrame(self.edges)

    def to_networkx(self, *, positive_only: bool = True) -> Any:
        """Return an authorized MultiDiGraph retaining manifestation/edge IDs.

        This is an export for independent analysis; native algorithms use fresh
        runtime authorization. Negative assertions are excluded by default.
        """
        try:
            import networkx as nx
        except ImportError as exc:
            raise ImportError("NetworkX conversion requires pip install 'weave-science[networkx]'") from exc
        graph = nx.MultiDiGraph()
        for record in self.nodes:
            graph.add_node(record["id"], **record)
        for record in self.edges:
            if not positive_only or record.get("polarity", "positive") == "positive":
                graph.add_edge(record["from"], record["to"], key=record["id"], **record)
        graph.graph.update(snapshots=self.snapshots, coverage=self.coverage,
                           diagnostics=self.diagnostics)
        return graph

    def save_experiment(self, path: str | Path, *, label: str | None = None,
                        parameters: Mapping[str, Any] | None = None) -> "Experiment":
        return Experiment.create(self.raw, self._context, label=label, parameters=parameters).save(path)

    def __repr__(self) -> str:
        return f"QueryResult(nodes={len(self.nodes)}, edges={len(self.edges)}, coverage={self.coverage!r})"


class AnalysisResult:
    """Native metrics plus the exact authorized input and algorithm semantics."""

    def __init__(self, raw: Mapping[str, Any], *, context: Mapping[str, Any]):
        self.raw = deepcopy(dict(raw))
        self._context = deepcopy(dict(context))
        input_context = deepcopy(self._context)
        input_context["request"] = {"operation": "execute", "program": deepcopy(context["request"]["program"])}
        input_context["result_index"] = context.get("result_index", context["request"].get("result_index", 0))
        self.input = QueryResult(self.raw["input"], context=input_context)

    @property
    def analysis(self) -> dict[str, Any]:
        return deepcopy(self.raw["analysis"])

    @property
    def semantics(self) -> dict[str, Any]:
        return deepcopy(self.raw.get("semantics", {}))

    def save_experiment(self, path: str | Path, *, label: str | None = None,
                        parameters: Mapping[str, Any] | None = None) -> "Experiment":
        return Experiment.create(self.raw, self._context, label=label, parameters=parameters).save(path)


class Experiment:
    """Integrity-checked JSON record, including the exact request and snapshots.

    SHA-256 detects edits, not authorship. The record does not grant access to
    stored graphs, and replays go through the native engine's current policy.
    """

    FORMAT = "weave-science-experiment/1"

    def __init__(self, record: Mapping[str, Any]):
        self.record = deepcopy(dict(record))
        if self.record.get("format") != self.FORMAT:
            raise ValueError("unsupported experiment artifact format")
        digest = self.record.pop("sha256", None)
        actual = hashlib.sha256(canonical_json(self.record).encode("utf-8")).hexdigest()
        self.record["sha256"] = digest
        if digest != actual:
            raise ValueError("experiment artifact integrity check failed")

    @classmethod
    def create(cls, result: Mapping[str, Any], context: Mapping[str, Any], *,
               label: str | None, parameters: Mapping[str, Any] | None) -> "Experiment":
        if not context or "request" not in context:
            raise ValueError("save results returned by Engine to preserve their exact request")
        request = deepcopy(context["request"])
        native_input = result["input"] if request["operation"] == "analyze" else result
        replay = deepcopy(request)
        reason = None
        try:
            replay["program"] = _pin_program(request["program"], native_input)
        except ValueError as exc:
            replay = None
            reason = str(exc)
        record = {"format": cls.FORMAT, "sdk_version": "0.1.0",
                  "science_version": context["science_version"],
                  "contract_version": context["contract_version"],
                  "binary_sha256": context.get("binary_sha256"), "actor": context["actor"],
                  "result_index": context.get("result_index", request.get("result_index", 0)),
                  "request": request, "replay_request": replay, "replay_unavailable": reason,
                  "snapshots": deepcopy(native_input["snapshots"]),
                  "input_snapshots": deepcopy(native_input.get("input_snapshots", [])),
                  "label": label, "parameters": deepcopy(dict(parameters or {})),
                  "result": deepcopy(dict(result))}
        record["sha256"] = hashlib.sha256(canonical_json(record).encode("utf-8")).hexdigest()
        return cls(record)

    @classmethod
    def load(cls, path: str | Path) -> "Experiment":
        return cls(json.loads(Path(path).read_text(encoding="utf-8")))

    def save(self, path: str | Path) -> "Experiment":
        _save_json(path, self.record)
        return self

    def replay(self, engine: "Engine", *, require_same_binary: bool = True) -> QueryResult | AnalysisResult:
        # Hashes detect edits, not trust: revalidate and enforce a pure operation
        # even for deliberately rehashed, externally supplied artifacts.
        Experiment(self.record)
        if engine.actor != self.record["actor"]:
            raise ValueError("replay requires the recorded actor; run a separate experiment for another actor")
        recorded_binary = self.record.get("binary_sha256")
        if require_same_binary and recorded_binary and engine.binary_sha256 != recorded_binary:
            raise ValueError("native binary differs; set require_same_binary=False for a version comparison")
        request = self.record.get("replay_request")
        if request is None:
            raise ValueError(f"exact replay unavailable: {self.record.get('replay_unavailable')}")
        if not isinstance(request, dict) or request.get("operation") not in {"execute", "analyze"}:
            raise ValueError("experiment replay requires an execute/analyze read operation")
        selected_program = request.get("program")
        if not isinstance(selected_program, dict):
            raise ValueError("experiment replay requires a read-only Program")
        validate_read_only(selected_program)
        expected_input = self.record["result"]["input"] if request["operation"] == "analyze" else self.record["result"]
        if _pin_program(selected_program, expected_input) != selected_program:
            raise ValueError("experiment replay request contains unpinned inputs")
        response = engine._request(request)
        if response["science_version"] != self.record["science_version"] or response["contract_version"] != self.record["contract_version"]:
            raise ValueError("replay protocol versions differ from the recorded experiment")
        context = engine._context(request, response)
        if request["operation"] == "analyze":
            actual = AnalysisResult(response["result"], context=context)
            assert_same_input(expected_input, actual.input.raw)
            return actual
        context["result_index"] = self.record.get("result_index", 0)
        actual = engine._query_result(response, context=context, result_index=context["result_index"])
        assert_same_input(expected_input, actual.raw)
        return actual
