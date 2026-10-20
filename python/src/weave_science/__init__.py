"""Native Weave temporal graph experiments, with no required Python dependencies."""

from .client import Engine, NativeError, OperationTimeout, ProtocolError, WeaveError
from .data import asserts, edge, embedding, embedding_space, graph_data, node, read_csv
from .expressions import (
    CONTRACT_VERSION,
    as_of,
    changes,
    diff,
    expand,
    explain,
    join,
    nearest_embeddings,
    program,
    query,
    recorded_query,
    similar_candidates,
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
    "as_of", "expand", "changes",
    "embedding_space", "embedding", "asserts", "nearest_embeddings", "similar_candidates",
]

