"""Native Weave temporal graph experiments, with no required Python dependencies."""

from .client import Engine, NativeError, OperationTimeout, ProtocolError, WeaveError
from .data import edge, graph_data, node, read_csv
from .expressions import (
    CONTRACT_VERSION,
    diff,
    explain,
    join,
    program,
    query,
    recorded_query,
    union,
    window,
)
from .results import AnalysisResult, CommitReceipt, Experiment, QueryResult, ReproducibilityError

__version__ = "0.1.0"

__all__ = [
    "Engine", "NativeError", "OperationTimeout", "ProtocolError", "WeaveError",
    "AnalysisResult", "CommitReceipt", "Experiment", "QueryResult", "ReproducibilityError",
    "node", "edge", "graph_data", "read_csv", "query", "recorded_query",
    "program", "join", "union", "diff", "window", "explain", "CONTRACT_VERSION",
]
