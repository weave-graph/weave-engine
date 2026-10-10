"""Subprocess transport to the native engine; no graph semantics in Python."""

from __future__ import annotations

import hashlib
import json
import math
import os
import shutil
import subprocess
from copy import deepcopy
from pathlib import Path
from typing import Any, Iterable, Mapping

from .data import graph_data, read_csv
from .expressions import program, query, recorded_query, validate_read_only
from .results import AnalysisResult, CommitReceipt, QueryResult, assert_same_input, canonical_json


class WeaveError(RuntimeError):
    """Base class for native, protocol and transport errors."""


class NativeError(WeaveError):
    def __init__(self, code: str, message: str, *, envelope: Mapping[str, Any]):
        self.code, self.message = code, message
        self.envelope = deepcopy(dict(envelope))
        super().__init__(f"{code}: {message}")


class ProtocolError(WeaveError):
    """The native process failed to return the documented JSON envelope."""


class OperationTimeout(WeaveError):
    """Execution timed out. A mutating operation may already have committed.

    No automatic retry is performed: inspect the durable state before retrying.
    """

    def __init__(self, operation: str, seconds: float):
        self.operation, self.seconds = operation, seconds
        self.commit_status = "unknown" if operation in {"import", "execute"} else "read_only"
        super().__init__(f"native {operation} exceeded {seconds:g}s; commit status: {self.commit_status}")


def _find_binary(binary: str | Path | None) -> Path:
    requested = str(binary) if binary is not None else os.environ.get("WEAVE_SCIENCE_BINARY")
    if requested:
        candidate = shutil.which(requested) or requested
        path = Path(candidate).expanduser().resolve()
        if not path.is_file() or not os.access(path, os.X_OK):
            raise FileNotFoundError(f"native weave-science binary is not executable: {path}")
        return path
    found = shutil.which("weave-science")
    if found:
        return Path(found).resolve()
    # Convenient for an editable installation in the source repository.
    for parent in Path(__file__).resolve().parents:
        for profile in ("release", "debug"):
            for name in ("weave-science", "weave-science.exe"):
                candidate = parent / "target" / profile / name
                if candidate.is_file() and os.access(candidate, os.X_OK):
                    return candidate.resolve()
    raise FileNotFoundError("build cargo build --release -p weave-science and set WEAVE_SCIENCE_BINARY or pass binary=...")


class Engine:
    """Persistent native engine scoped to an explicit principal and write grants.

    Every call opens the same SQLite database in the native process. No network,
    compiler, service or browser is needed. write_graphs is host authority and is
    supplied separately from untrusted JSON Programs.
    """

    def __init__(self, db: str | Path, *, actor: str = "researcher",
                 write_graphs: Iterable[str] = (), binary: str | Path | None = None,
                 timeout: float = 30.0, max_response_bytes: int = 64 * 1024 * 1024):
        if not isinstance(actor, str) or not actor:
            raise ValueError("actor must be a nonempty principal ID")
        if not isinstance(timeout, (int, float)) or isinstance(timeout, bool) or not math.isfinite(timeout) or timeout <= 0:
            raise ValueError("timeout must be a positive finite number of seconds")
        if not isinstance(max_response_bytes, int) or isinstance(max_response_bytes, bool) or max_response_bytes <= 0:
            raise ValueError("max_response_bytes must be a positive integer")
        if str(db) == ":memory:":
            raise ValueError("subprocess sessions need a persistent database path, not :memory:")
        self.db = Path(db).expanduser().resolve()
        self.actor = actor
        self.write_graphs = tuple(write_graphs)
        if any(not isinstance(g, str) or not g for g in self.write_graphs):
            raise ValueError("write_graphs must contain nonempty graph IDs")
        self.binary = _find_binary(binary)
        self.timeout, self.max_response_bytes = float(timeout), max_response_bytes
        self._binary_stat = self._binary_identity()
        hasher = hashlib.sha256()
        with self.binary.open("rb") as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                hasher.update(chunk)
        self.binary_sha256 = hasher.hexdigest()

    def _binary_identity(self) -> tuple[int, ...]:
        stat = self.binary.stat()
        return (stat.st_dev, stat.st_ino, stat.st_size, stat.st_mtime_ns, stat.st_ctime_ns)

    def _check_binary(self) -> None:
        if self._binary_identity() != self._binary_stat:
            raise WeaveError("native binary changed during this session; create a new Engine to record its exact identity")

    def _request(self, request: Mapping[str, Any]) -> dict[str, Any]:
        self._check_binary()
        args = [str(self.binary), "--db", str(self.db), "--actor", self.actor]
        for graph_id in self.write_graphs:
            args.extend(("--write", graph_id))
        body = canonical_json(request).encode("utf-8")
        try:
            completed = subprocess.run(args, input=body, stdout=subprocess.PIPE,
                                       stderr=subprocess.PIPE, timeout=self.timeout, check=False)
        except subprocess.TimeoutExpired as exc:
            raise OperationTimeout(str(request.get("operation", "unknown")), self.timeout) from exc
        except OSError as exc:
            raise WeaveError(f"could not execute native engine: {exc}") from exc
        self._check_binary()
        if len(completed.stdout) > self.max_response_bytes:
            raise ProtocolError(f"native response exceeds {self.max_response_bytes} bytes")
        try:
            response = json.loads(completed.stdout.decode("utf-8"), parse_constant=_invalid_number)
        except (ValueError, UnicodeDecodeError) as exc:
            detail = completed.stderr.decode("utf-8", errors="replace")[-2000:]
            raise ProtocolError(f"native engine returned invalid JSON (exit {completed.returncode}): {detail}") from exc
        if not isinstance(response, dict) or not isinstance(response.get("ok"), bool):
            raise ProtocolError("native response lacks a boolean ok field")
        if not all(isinstance(response.get(key), str) for key in ("science_version", "contract_version")):
            raise ProtocolError("native response lacks protocol versions")
        if response["science_version"] != "0.1.0" or response["contract_version"] != "0.21.0":
            raise ProtocolError("native response uses an unsupported protocol version")
        if not response["ok"]:
            error = response.get("error")
            if not isinstance(error, dict) or not all(isinstance(error.get(key), str) for key in ("code", "message")):
                raise ProtocolError("native error lacks code/message")
            raise NativeError(error["code"], error["message"], envelope=response)
        if completed.returncode != 0:
            raise ProtocolError(f"native success envelope contradicts exit status {completed.returncode}")
        if "result" not in response:
            raise ProtocolError("native success response has no result")
        return response

    def _context(self, request: Mapping[str, Any], response: Mapping[str, Any]) -> dict[str, Any]:
        return {"request": deepcopy(dict(request)), "actor": self.actor,
                "result_index": request.get("result_index", 0),
                "binary_sha256": self.binary_sha256,
                "science_version": response["science_version"],
                "contract_version": response["contract_version"]}

    def capabilities(self) -> dict[str, Any]:
        return self._request({"operation": "capabilities"})["result"]

    def execute(self, program: Mapping[str, Any]) -> dict[str, Any]:
        """Execute an arbitrary existing JSON Program with native validation."""
        return self._request({"operation": "execute", "program": deepcopy(dict(program))})["result"]

    def import_graph(self, graph_id: str, data: Mapping[str, Any] | None = None, *,
                     nodes: Any = None, edges: Any = None, branch_id: str = "main",
                     expected_head: str | None = None) -> CommitReceipt:
        """Commit a full graph snapshot, with compare-and-swap revision control.

        expected_head=None creates a new branch. Updating an existing branch
        requires its previous revision. The snapshot replaces current state;
        prior immutable revisions remain queryable.
        """
        if data is not None and (nodes is not None or edges is not None):
            raise ValueError("provide data or nodes/edges, not both")
        request = {"operation": "import", "graph_id": graph_id, "branch_id": branch_id,
                   "expected_head": expected_head,
                   "data": deepcopy(dict(data)) if data is not None else graph_data(
                       () if nodes is None else nodes, () if edges is None else edges)}
        response = self._request(request)
        receipts = response["result"].get("results", [])
        if len(receipts) != 1 or receipts[0].get("kind") not in {"committed", "unchanged"}:
            raise ProtocolError("import did not return a single commit receipt")
        receipt = receipts[0]
        return CommitReceipt(receipt["revision"], receipt.get("event_id"),
                             receipt["kind"] == "committed", deepcopy(receipt))

    def import_csv(self, graph_id: str, nodes_path: str | Path, edges_path: str | Path, *,
                   node_options: Mapping[str, Any] | None = None,
                   edge_options: Mapping[str, Any] | None = None,
                   **commit_options: Any) -> CommitReceipt:
        """Import engine-shaped CSVs; explicitly declare JSON/integer columns."""
        nodes = read_csv(nodes_path, **dict(node_options or {}))
        edges = read_csv(edges_path, **dict(edge_options or {}))
        return self.import_graph(graph_id, nodes=nodes, edges=edges, **commit_options)

    def query(self, graph_id: str, **selection: Any) -> QueryResult:
        return self.evaluate(query(graph_id, **selection))

    def recorded_query(self, graph_id: str, **selection: Any) -> QueryResult:
        return self.evaluate(recorded_query(graph_id, **selection))

    def evaluate(self, expression: Mapping[str, Any], *, result_index: int = 0) -> QueryResult:
        """Evaluate an existing graph expression or read-only JSON Program.

        Use execute for Programs that commit. Invalid selection is rejected
        before launching the native process.
        """
        self._validate_result_index(result_index)
        selected_program = program(expression)
        validate_read_only(selected_program)
        request = {"operation": "execute", "program": selected_program}
        response = self._request(request)
        context = self._context(request, response)
        context["result_index"] = result_index
        return self._query_result(response, context=context, result_index=result_index)

    @staticmethod
    def _validate_result_index(result_index: int) -> None:
        if isinstance(result_index, bool) or not isinstance(result_index, int) or result_index < 0:
            raise ValueError("result_index must be a nonnegative integer")

    @staticmethod
    def _query_result(response: Mapping[str, Any], *, context: Mapping[str, Any],
                      result_index: int) -> QueryResult:
        Engine._validate_result_index(result_index)
        try:
            selected = response["result"]["results"][result_index]
            if selected["kind"] != "queried":
                raise ProtocolError("selected Program result is not a query result")
            return QueryResult(selected["result"], context=context)
        except (KeyError, IndexError, TypeError) as exc:
            raise ProtocolError("native response has no selected query result") from exc

    def analyze(self, value: str | Mapping[str, Any] | QueryResult, *,
                algorithm: str = "degree", result_index: int = 0,
                allow_partial: bool = False, limits: Mapping[str, Any] | None = None,
                **parameters: Any) -> AnalysisResult:
        """Run native analysis of a freshly authorized, selected graph result.

        value may be a graph ID, graph expression, Program, or prior QueryResult.
        A prior result is re-read with exact pinned inputs through current policy.
        Algorithms: degree, components, shortest_paths, pagerank, nearest_vectors.
        """
        if isinstance(value, QueryResult):
            selected_program = value.pinned_program
            result_index = value._context.get("result_index", result_index)
        elif isinstance(value, str):
            selected_program = program(query(value))
        else:
            selected_program = program(value)
        self._validate_result_index(result_index)
        request = {"operation": "analyze", "program": selected_program,
                   "result_index": result_index, "analysis": {"algorithm": algorithm, **parameters},
                   "allow_partial": allow_partial}
        if limits is not None:
            request["limits"] = deepcopy(dict(limits))
        response = self._request(request)
        if isinstance(value, QueryResult):
            assert_same_input(value.raw, response["result"]["input"])
        return AnalysisResult(response["result"], context=self._context(request, response))

    def nearest(self, value: str | Mapping[str, Any] | QueryResult,
                vector: Iterable[float], *, space_id: str, property: str = "vector",
                metric: str = "cosine", k: int = 10, **options: Any) -> AnalysisResult:
        return self.analyze(value, algorithm="nearest_vectors", query=list(vector),
                            space_id=space_id, property=property, metric=metric, k=k, **options)


def _invalid_number(value: str) -> None:
    raise ValueError(f"non-finite JSON number {value}")
