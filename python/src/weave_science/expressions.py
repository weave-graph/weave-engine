"""Builders for the existing engine contract, without a second evaluator."""

from copy import deepcopy
from typing import Any, Mapping

CONTRACT_VERSION = "0.21.0"
READ_ONLY_COMMANDS = frozenset({"query", "evaluate", "bind", "join", "recorded_range", "accepted_range"})


def validate_read_only(value: Mapping[str, Any]) -> None:
    commands = value.get("commands")
    if not isinstance(commands, list) or any(not isinstance(command, Mapping)
            or command.get("op") not in READ_ONLY_COMMANDS for command in commands):
        raise ValueError("a read-only Program is required; use execute for commits")


def query(graph_id: str, *, revision: str | None = None, branch_id: str = "main",
          predicate: str | None = None, from_id: str | None = None,
          to_id: str | None = None, valid_at: int | None = None,
          include_metadata: bool = False, max_depth: int = 8) -> dict[str, Any]:
    """An engine graph expression. Explicit revision pins recorded state.

    valid_at selects half-open edge validity. A filtered query selects edge
    endpoints; use analyze(..., valid_at=...) to retain isolated visible nodes.
    """
    plan = {"graph_id": graph_id, "branch_id": branch_id,
            "include_metadata": include_metadata, "max_depth": max_depth}
    for key, value in (("revision", revision), ("predicate", predicate),
                       ("from", from_id), ("to", to_id), ("valid_at", valid_at)):
        if value is not None:
            plan[key] = value
    return {"kind": "query", "query": plan}


def recorded_query(graph_id: str, *, recorded_at_ms: int | None = None,
                   observer: str | None = None, checkpoint: str | None = None,
                   **selection: Any) -> dict[str, Any]:
    """Read a local recorded-time cut or an exact observation checkpoint.

    The runtime resolves and authorizes the witness. Caller-supplied timestamps
    do not assign system time to a commit.
    """
    if recorded_at_ms is not None:
        if observer is not None or checkpoint is not None:
            raise ValueError("choose recorded_at_ms or observer/checkpoint")
        cut = {"kind": "local_time", "unix_millis": recorded_at_ms}
    elif observer is not None and checkpoint is not None:
        cut = {"kind": "checkpoint", "observer": observer, "checkpoint": checkpoint}
    else:
        raise ValueError("provide recorded_at_ms or both observer and checkpoint")
    return {"kind": "recorded_query", "query": query(graph_id, **selection)["query"],
            "selection": cut}


def program(value: Mapping[str, Any]) -> dict[str, Any]:
    """Make a read-only Program from a graph expression or copy a full Program."""
    if "commands" in value:
        return deepcopy(dict(value))
    return {"version": CONTRACT_VERSION,
            "commands": [{"op": "evaluate", "value": deepcopy(dict(value))}]}


def join(left: Mapping[str, Any], right: Mapping[str, Any], *,
         output_predicate: str) -> dict[str, Any]:
    """Temporal/context-aware join through matching entity and space."""
    return {"kind": "join", "left": deepcopy(dict(left)), "right": deepcopy(dict(right)),
            "output_predicate": output_predicate, "match_on": "entity_space_to_from"}


def union(left: Mapping[str, Any], right: Mapping[str, Any]) -> dict[str, Any]:
    return {"kind": "union", "left": deepcopy(dict(left)), "right": deepcopy(dict(right))}


def diff(before: Mapping[str, Any], after: Mapping[str, Any]) -> dict[str, Any]:
    return {"kind": "diff", "before": deepcopy(dict(before)), "after": deepcopy(dict(after))}


def window(value: Mapping[str, Any], start: int, end: int | None) -> dict[str, Any]:
    return {"kind": "window", "input": deepcopy(dict(value)),
            "window": {"start": start, "end": end}}


def explain(value: Mapping[str, Any]) -> dict[str, Any]:
    return {"kind": "explain", "input": deepcopy(dict(value))}
