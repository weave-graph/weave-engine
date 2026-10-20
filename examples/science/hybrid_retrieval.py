#!/usr/bin/env python3
"""End-to-end hybrid retrieval with caller-supplied embeddings (A4).

A small fleet: units at sites, maintenance notes about units, one unit
restricted to an admin. The caller embeds every note with its own encoder
(here a deterministic word-hashing toy, standing in for a real model) and
writes the vectors as embedding facts. Then:

1. graph to vector: the notes nearest to an incident, restricted to the
   2-hop neighborhood of the affected unit and valid at the incident time;
2. vector to graph: the units and sites behind the nearest notes, in one plan;
3. candidate relations: similar units that the graph does not link yet;
4. the same questions as the analyst, who never sees the restricted unit.

Prints a JSON summary. Run it with the native binary:

    WEAVE_SCIENCE_BINARY=target/debug/weave-science \\
      python examples/science/hybrid_retrieval.py --output hybrid-output
"""

import argparse
import hashlib
import json
import math
from pathlib import Path

from weave_science import (Engine, asserts, embedding, embedding_space, expand,
                           nearest_embeddings)

DIMENSIONS = 32
DAY = 86_400_000


def encode(text: str) -> list[float]:
    """Toy encoder: signed hashing of words, L2-normalized. Replace with your model."""
    vector = [0.0] * DIMENSIONS
    for word in text.lower().split():
        digest = hashlib.sha256(word.strip(".,:").encode()).digest()
        vector[digest[0] % DIMENSIONS] += 1.0 if digest[1] % 2 else -1.0
    norm = math.sqrt(sum(x * x for x in vector)) or 1.0
    return [x / norm for x in vector]


NOTES = {
    "wo-1": ("unit-1", 10, "Replaced the worn main bearing after vibration alarms"),
    "wo-2": ("unit-1", 40, "Calibrated the temperature sensor and checked coolant flow"),
    "wo-3": ("unit-2", 20, "Bearing noise and vibration at high speed; bearing replaced"),
    "wo-4": ("unit-3", 30, "Vibration alarm on the main bearing; replaced bearing"),
    "wo-5": ("unit-2", 90, "Firmware update and filter change"),
}
UNITS = {"unit-1": ("site-a", []), "unit-2": ("site-a", []), "unit-3": ("site-b", ["admin"])}


def facts():
    nodes = [{"kind": "node", "id": s, "entity_id": s, "space_id": "fleet"} for s in ("site-a", "site-b")]
    edges = []
    for unit, (site, readers) in UNITS.items():
        nodes.append({"kind": "node", "id": unit, "entity_id": unit, "space_id": "fleet", "readers": readers})
        edges.append({"kind": "edge", "id": f"at-{unit}", "predicate": "located_at", "from": unit,
                      "to": site, "valid_time": {"start": 0}})
    for wo, (unit, day, text) in NOTES.items():
        readers = UNITS[unit][1]
        nodes.append({"kind": "node", "id": wo, "entity_id": wo, "space_id": "fleet", "readers": readers,
                      "properties": {"note": text}})
        edges.append({"kind": "edge", "id": f"on-{wo}", "predicate": "performed_on", "from": wo,
                      "to": unit, "valid_time": {"start": day * DAY}})
    return nodes + edges


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--output", type=Path, default=Path("hybrid-output"))
    parser.add_argument("--binary", type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    database = args.output / "hybrid.sqlite"
    if database.exists():
        parser.error(f"{database} already exists; choose a new --output directory")
    writer = Engine(database, actor="loader", write_graphs=["fleet"], binary=args.binary)
    writer.import_facts("fleet", asserts(facts()))

    # Embed with your encoder; Weave stores the vectors as temporal, permissioned facts.
    space = embedding_space("notes", revision="1", encoder="toy-hash", encoder_version="1",
                            preprocessing="lowercase-words/v1", dimensions=DIMENSIONS)
    vectors = [embedding(f"vec-{wo}", space, wo, encode(text), valid_from=day * DAY,
                         readers=UNITS[unit][1])
               for wo, (unit, day, text) in NOTES.items()]
    head = writer.import_facts("fleet", asserts([space] + vectors))
    analyst, admin = (Engine(database, actor=who, binary=args.binary) for who in ("analyst", "admin"))

    incident = encode("Vibration alarm: bearing noise reported on unit-1")
    incident_time = 50 * DAY
    neighborhood = expand("fleet", ["unit-1"], hops=2, direction="both")
    # 1. Graph to vector.
    near = analyst.nearest_embeddings("fleet", ("notes", "1"), vector=incident, k=3,
                                      valid_at=incident_time, candidates=neighborhood)
    # 2. Vector to graph: nearest notes, then their units, then the units' sites.
    behind = analyst.expand("fleet", hops=2, direction="out",
                            predicates=["performed_on", "located_at"],
                            seeds_from=nearest_embeddings("fleet", ("notes", "1"), vector=incident, k=2))
    # 3. Similar notes the graph does not link (candidates are never written).
    similar = {who: engine.similar_candidates("fleet", ("notes", "1"), k=1, exclude_linked=True)
               for who, engine in (("analyst", analyst), ("admin", admin))}
    summary = {
        "embedding_revision": head.revision,
        "nearest_notes_in_neighborhood": [(n["embedding_id"], round(n["score"], 4)) for n in near.neighbors],
        "exactness": near.exactness,
        "note_text": {n["id"]: n["properties"]["note"] for n in near.nodes},
        "units_and_sites_behind_nearest_notes": sorted(n["id"] for n in behind.nodes),
        "candidates": {who: [(c["from"], c["to"], round(c["score"], 4)) for c in result.candidates]
                       for who, result in similar.items()},
        "provenance_methods": sorted({p["provenance"]["method"]["kind"] for p in near.fact_provenance}),
    }
    print(json.dumps(summary, indent=2))
    return summary


if __name__ == "__main__":
    main()
