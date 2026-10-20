"""Record/CSV helpers; engine validation remains authoritative."""

from __future__ import annotations

import csv
import json
import math
from copy import deepcopy
from pathlib import Path
from typing import Any, Iterable, Mapping


def _integer(value: Any, name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{name} must be an integer in the dataset's time unit")
    if not -(2**63) <= value < 2**63:
        raise ValueError(f"{name} must fit signed 64-bit engine time")
    return value


def node(id: str, *, entity_id: str | None = None, space_id: str = "science",
         properties: Mapping[str, Any] | None = None,
         vector: Iterable[float] | None = None, vector_property: str = "vector",
         **fields: Any) -> dict[str, Any]:
    """Make a manifestation. Provide entity_id to share identity across spaces.

    A vector is a node property in its explicitly declared space. Encoder/model
    identity can be included in properties; no similarity-to-identity lowering
    is performed by this helper.
    """
    props = deepcopy(dict(properties or {}))
    if vector is not None:
        values = list(vector)
        if not values or any(isinstance(v, bool) or not isinstance(v, (int, float))
                             or not math.isfinite(v) for v in values):
            raise ValueError("vector must contain finite numeric coordinates")
        if vector_property in props:
            raise ValueError(f"property {vector_property!r} was provided twice")
        props[vector_property] = values
    return {"id": id, "entity_id": id if entity_id is None else entity_id,
            "space_id": space_id, "properties": props, **deepcopy(fields)}


def edge(id: str, from_id: str, to_id: str, *, predicate: str = "related",
         valid_from: int = 0, valid_to: int | None = None,
         properties: Mapping[str, Any] | None = None,
         **fields: Any) -> dict[str, Any]:
    """Make an assertion with half-open valid time [valid_from, valid_to)."""
    start = _integer(valid_from, "valid_from")
    end = None if valid_to is None else _integer(valid_to, "valid_to")
    if end is not None and end <= start:
        raise ValueError("valid_to must be greater than valid_from")
    return {"id": id, "from": from_id, "to": to_id, "predicate": predicate,
            "valid_time": {"start": start, "end": end},
            "properties": deepcopy(dict(properties or {})), **deepcopy(fields)}


def embedding_space(id: str, *, revision: str, encoder: str, encoder_version: str,
                    preprocessing: str, dimensions: int,
                    metric: str = "cosine_distance") -> dict[str, Any]:
    """An embedding space fact (contract 0.24) for ``Engine.import_facts``.

    Two vectors are comparable only when their spaces are equal in every
    field; a new encoder version is a new space ``revision`` (no ``@``).
    Weave does not generate embeddings: the encoder is the caller's.
    """
    if metric not in {"cosine_distance", "euclidean"}:
        raise ValueError("metric must be cosine_distance or euclidean")
    if isinstance(dimensions, bool) or not isinstance(dimensions, int) or not 1 <= dimensions <= 4096:
        raise ValueError("dimensions must be an integer from 1 to 4096")
    if "@" in revision:
        raise ValueError("a space revision cannot contain '@'")
    return {"kind": "embedding_space", "id": id, "revision": revision, "encoder": encoder,
            "encoder_version": encoder_version, "preprocessing": preprocessing,
            "dimensions": dimensions, "metric": metric}


def embedding(id: str, space: Mapping[str, Any], subject: str, vector: Iterable[float], *,
              subject_kind: str = "node", valid_from: int = 0, valid_to: int | None = None,
              readers: Iterable[str] = (), evidence: Iterable[Mapping[str, Any]] = (),
              source: str | None = None) -> dict[str, Any]:
    """An embedding fact of ``space`` (an ``embedding_space`` fact) on a subject.

    The vector is rounded to binary32 by the engine. Its provenance claims
    ``method: model`` with the space's encoder and version; ``evidence`` may
    cite the embedded text (for example an attachment as an ``assertion``).
    Give restricted text's ``readers`` so the vector is restricted with it.
    """
    values = list(vector)
    if len(values) != space["dimensions"] or any(
            isinstance(v, bool) or not isinstance(v, (int, float)) or not math.isfinite(v)
            for v in values):
        raise ValueError("vector must have the space's dimensions and finite components")
    if subject_kind not in {"node", "entity", "edge", "assertion"}:
        raise ValueError("subject_kind must be node, entity, edge or assertion")
    start = _integer(valid_from, "valid_from")
    end = None if valid_to is None else _integer(valid_to, "valid_to")
    if end is not None and end <= start:
        raise ValueError("valid_to must be greater than valid_from")
    claim: dict[str, Any] = {"method": {"kind": "model", "model": space["encoder"],
                                        "version": space["encoder_version"]}}
    evidence = [deepcopy(dict(e)) for e in evidence]
    if evidence:
        claim["evidence"] = evidence
    if source is not None:
        claim["source"] = source
    return {"kind": "embedding", "id": id,
            "space": {"id": space["id"], "revision": space["revision"]},
            "subject": {"kind": subject_kind, f"{subject_kind}_id": subject},
            "vector": values, "valid_time": {"start": start, "end": end},
            "readers": list(readers), "provenance": claim}


def asserts(facts: Iterable[Mapping[str, Any]]) -> list[dict[str, Any]]:
    """``assert`` changes for ``Engine.import_facts``."""
    return [{"change": "assert", "fact": deepcopy(dict(fact))} for fact in facts]


def _records(values: Any) -> list[dict[str, Any]]:

    # pandas is optional and imported only by result conversion helpers.
    if hasattr(values, "to_dict") and not isinstance(values, Mapping):
        values = values.to_dict(orient="records")
    return [deepcopy(dict(item)) for item in values]


def graph_data(nodes: Iterable[Mapping[str, Any]],
               edges: Iterable[Mapping[str, Any]] = (), **fields: Any) -> dict[str, Any]:
    """Materialize engine-shaped records or pandas frames into a full snapshot.

    Advanced contract fields (schema, attachments, assertions, influence) remain
    intact. This function does not assign revisions or bypass native validation.
    """
    return {"nodes": _records(nodes), "edges": _records(edges), **deepcopy(fields)}


def read_csv(path: str | Path, *, json_columns: Iterable[str] = (),
             integer_columns: Iterable[str] = (),
             float_columns: Iterable[str] = (), encoding: str = "utf-8-sig") -> list[dict[str, Any]]:
    """Read CSV with explicit types. Empty typed cells become None.

    CSV has no type schema; unspecified fields remain strings. JSON columns are
    useful for vectors, metadata refs, properties and valid_time.
    """
    json_fields, int_fields, float_fields = map(set, (json_columns, integer_columns, float_columns))
    if (json_fields & int_fields) or (json_fields & float_fields) or (int_fields & float_fields):
        raise ValueError("a CSV column can have only one conversion type")
    with Path(path).open(newline="", encoding=encoding) as stream:
        reader = csv.DictReader(stream)
        headers = reader.fieldnames
        if not headers or len(headers) != len(set(headers)):
            raise ValueError("CSV requires unique column names")
        unknown = (json_fields | int_fields | float_fields) - set(headers)
        if unknown:
            raise ValueError(f"unknown CSV columns: {sorted(unknown)}")
        result = []
        for line, row in enumerate(reader, 2):
            if None in row or any(v is None for v in row.values()):
                raise ValueError(f"CSV line {line} has a different column count")
            try:
                for name in json_fields | int_fields | float_fields:
                    value = row[name]
                    if value == "":
                        row[name] = None
                    elif name in json_fields:
                        row[name] = json.loads(value, parse_constant=_invalid_json_number)
                    elif name in int_fields:
                        row[name] = _integer(int(value), name)
                    else:
                        row[name] = float(value)
                        if not math.isfinite(row[name]):
                            raise ValueError("non-finite floating point value")
            except (ValueError, TypeError, json.JSONDecodeError) as exc:
                raise ValueError(f"invalid CSV value at line {line}: {exc}") from exc
            result.append(row)
        return result


def _invalid_json_number(value: str) -> None:
    raise ValueError(f"non-finite JSON number {value}")
