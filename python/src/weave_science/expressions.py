"""Builders for the existing engine contract, without a second evaluator."""

import math
from copy import deepcopy
from typing import Any, Iterable, Mapping

CONTRACT_VERSION = "1.0.0"
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


def as_of(graph_id: str, recorded_at_ms: int, **selection: Any) -> dict[str, Any]:
    """Any query of the branch as this replica knew it at a recorded time.

    The revision is selected through the recorded-time index and authorized
    record by record, so a reader who cannot see every record gets the visible
    subset, exactly as a read pinned to that revision. Unlike recorded_query it
    returns no checkpoint witness. ``selection`` takes the query() options
    except ``revision``.
    """
    if "revision" in selection:
        raise ValueError("as_of selects the revision; do not pass revision")
    _time(recorded_at_ms, "recorded_at_ms")
    return {"kind": "as_of", "query": query(graph_id, **selection)["query"],
            "unix_millis": recorded_at_ms}


def expand(graph_id: str, seeds: Iterable[str] = (), *, hops: int, direction: str = "out",
           predicates: Iterable[str] = (), valid_at: int | None = None,
           revision: str | None = None, as_of: int | None = None,
           branch_id: str = "main", limit: int = 10_000,
           seeds_from: Mapping[str, Any] | None = None) -> dict[str, Any]:
    """The exact k-hop neighborhood of the visible seed nodes.

    Every node within ``hops`` hops along visible edges whose predicate is in
    ``predicates`` (empty: any) and, with ``valid_at``, that are valid then,
    plus every such edge that leaves a node fewer than ``hops`` hops away.
    ``direction`` is ``out``, ``in`` or ``both``. Read a pinned ``revision``,
    the branch ``as_of`` a recorded time, or the head. More than ``limit``
    nodes plus edges fails with E_BUDGET; nothing is truncated. ``seeds_from``
    (contract 0.24) adds the nodes of another graph expression as seeds, for
    example the hits of ``nearest_embeddings``.
    """
    seeds = list(seeds)
    if (not seeds and seeds_from is None) or not all(isinstance(seed, str) and seed for seed in seeds):
        raise ValueError("expand needs at least one seed node ID")
    if isinstance(hops, bool) or not isinstance(hops, int) or not 1 <= hops <= 16:
        raise ValueError("hops must be an integer from 1 to 16")
    if direction not in {"out", "in", "both"}:
        raise ValueError("direction must be out, in or both")
    if revision is not None and as_of is not None:
        raise ValueError("choose revision or as_of")
    plan: dict[str, Any] = {"kind": "expand", "graph_id": graph_id, "branch_id": branch_id,
                            "seeds": seeds, "hops": hops, "direction": direction, "limit": limit}
    predicates = list(predicates)
    if predicates:
        plan["predicates"] = predicates
    for key, value in (("valid_at", valid_at), ("revision", revision), ("as_of", as_of)):
        if value is not None:
            plan[key] = value
    if seeds_from is not None:
        plan["seeds_from"] = deepcopy(dict(seeds_from))
    return plan


def _bound(time: int | None, cursor: str | None, *, default_head: bool) -> dict[str, Any]:
    if time is not None and cursor is not None:
        raise ValueError("choose a recorded time or a cursor for each end")
    if time is not None:
        _time(time, "recorded time")
        return {"kind": "as_of", "unix_millis": time}
    if cursor is not None:
        return {"kind": "cursor", "revision": cursor}
    if default_head:
        return {"kind": "head"}
    raise ValueError("changes needs since_ms or since_cursor")


def changes(graph_id: str, *, since_ms: int | None = None, since_cursor: str | None = None,
            until_ms: int | None = None, until_cursor: str | None = None,
            entities: Iterable[str] = (), predicates: Iterable[str] = (),
            branch_id: str = "main", limit: int = 10_000) -> dict[str, Any]:
    """Facts asserted, retracted or corrected between two states of a branch.

    The ``from`` state is the branch as known at ``since_ms`` or the exact
    revision ``since_cursor`` (a previous change set's ``cursor``); the ``to``
    state is ``until_ms``, ``until_cursor`` or the head. The result is the
    fact-by-fact difference of the two authorized states.
    """
    plan: dict[str, Any] = {"kind": "changes", "graph_id": graph_id, "branch_id": branch_id,
                            "from": _bound(since_ms, since_cursor, default_head=False),
                            "to": _bound(until_ms, until_cursor, default_head=True),
                            "limit": limit}
    entities, predicates = list(entities), list(predicates)
    if entities:
        plan["entities"] = entities
    if predicates:
        plan["predicates"] = predicates
    return plan


def _time(value: Any, name: str) -> None:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise ValueError(f"{name} must be a nonnegative integer of Unix milliseconds")


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


def _space(space: Any) -> dict[str, str]:
    if isinstance(space, Mapping):
        identifier, revision = space.get("id"), space.get("revision")
    elif isinstance(space, (tuple, list)) and len(space) == 2:
        identifier, revision = space
    else:
        raise ValueError("space must be (id, revision) or {'id': ..., 'revision': ...}")
    if not isinstance(identifier, str) or not identifier or not isinstance(revision, str) or not revision:
        raise ValueError("space needs a nonempty id and revision")
    return {"id": identifier, "revision": revision}


def _selection(plan: dict[str, Any], revision: str | None, as_of: int | None,
               valid_at: int | None) -> None:
    if revision is not None and as_of is not None:
        raise ValueError("choose revision or as_of")
    if as_of is not None:
        _time(as_of, "as_of")
    for key, value in (("revision", revision), ("as_of", as_of), ("valid_at", valid_at)):
        if value is not None:
            plan[key] = value


def nearest_embeddings(graph_id: str, space: Any, *, vector: Iterable[float] | None = None,
                       embedding_id: str | None = None, k: int = 10,
                       valid_at: int | None = None, candidates: Mapping[str, Any] | None = None,
                       revision: str | None = None, as_of: int | None = None,
                       branch_id: str = "main") -> dict[str, Any]:
    """Exact k nearest embedding facts of one space (contract 0.24 ``nearest``).

    The query is a caller-supplied ``vector`` in the space, or the visible
    embedding fact ``embedding_id`` (its own subject is excluded). Only
    embeddings valid at ``valid_at`` and, with ``candidates``, whose subject
    is a record of that graph expression (for example an ``expand``) are
    ranked, after the engine authorizes them: hidden vectors never change a
    visible rank or score. ``k`` is 1 to 1,000.
    """
    if (vector is None) == (embedding_id is None):
        raise ValueError("give exactly one of vector or embedding_id")
    if isinstance(k, bool) or not isinstance(k, int) or not 1 <= k <= 1000:
        raise ValueError("k must be an integer from 1 to 1000")
    if vector is not None:
        values = list(vector)
        if not values or any(isinstance(v, bool) or not isinstance(v, (int, float))
                             or not math.isfinite(v) for v in values):
            raise ValueError("vector must contain finite numeric components")
        query_value: dict[str, Any] = {"kind": "vector", "vector": values}
    else:
        query_value = {"kind": "embedding", "embedding_id": embedding_id}
    plan: dict[str, Any] = {"kind": "nearest", "graph_id": graph_id, "branch_id": branch_id,
                            "space": _space(space), "query": query_value, "k": k}
    _selection(plan, revision, as_of, valid_at)
    if candidates is not None:
        plan["candidates"] = deepcopy(dict(candidates))
    return plan


def similar_candidates(graph_id: str, space: Any, *, k: int = 1, max_score: float | None = None,
                       valid_at: int | None = None, sources: Mapping[str, Any] | None = None,
                       targets: Mapping[str, Any] | None = None, exclude_linked: bool = False,
                       limit: int = 1000, revision: str | None = None, as_of: int | None = None,
                       branch_id: str = "main") -> dict[str, Any]:
    """Candidate ``weave:similar_to`` relations between visible embeddings.

    For every source embedding (all, or the subjects of ``sources``), its
    ``k`` nearest targets of another subject at most ``max_score`` apart.
    ``exclude_linked`` keeps only pairs not already joined by a visible edge.
    Candidates are result values with provenance; they are never written.
    """
    if isinstance(k, bool) or not isinstance(k, int) or not 1 <= k <= 1000:
        raise ValueError("k must be an integer from 1 to 1000")
    if isinstance(limit, bool) or not isinstance(limit, int) or not 1 <= limit <= 100_000:
        raise ValueError("limit must be an integer from 1 to 100000")
    plan: dict[str, Any] = {"kind": "similar_candidates", "graph_id": graph_id,
                            "branch_id": branch_id, "space": _space(space), "k": k, "limit": limit}
    if max_score is not None:
        if isinstance(max_score, bool) or not isinstance(max_score, (int, float)) \
                or not math.isfinite(max_score) or max_score < 0:
            raise ValueError("max_score must be a finite nonnegative distance")
        plan["max_score"] = max_score
    if exclude_linked:
        plan["exclude_linked"] = True
    for key, value in (("sources", sources), ("targets", targets)):
        if value is not None:
            plan[key] = deepcopy(dict(value))
    _selection(plan, revision, as_of, valid_at)
    return plan
