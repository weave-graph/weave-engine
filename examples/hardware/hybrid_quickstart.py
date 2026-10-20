#!/usr/bin/env python3
"""Hybrid retrieval over an imported hardware fleet (README quickstart).

Run this after `weave-hardware generate`, `import` and `apply-updates` (see
the README). The script:

1. embeds every maintenance note, every incident description and a profile
   of every unit, using a small deterministic word-hashing embedder;
2. writes those vectors as embedding facts through `weave-science import`,
   keeping each row's readers;
3. answers two questions as the reader you choose:
   - which maintenance notes are similar to the latest incident, within two
     hops of the affected unit and valid when the incident opened (one
     `nearest` plan over an `expand` neighborhood);
   - which two units are similar but not linked in the graph (one
     `similar_candidates` plan).

Every hit is printed with its source: the note text and the writer and
method recorded for its embedding.

The hashing embedder is a stand-in so the example needs no model or network.
Weave does not generate embeddings. Use your own encoder's vectors the same
way. Standard library only.
"""
import argparse
import csv
import json
import math
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

GRAPH = "fleet"
DIMS = 64
ENCODER = "quickstart-hash-embedder"
EMBEDDER = "note-embedder"


def embed(text):
    """Signed feature hashing of lowercase words, L2-normalized."""
    v = [0.0] * DIMS
    word = ""
    for ch in text.lower() + " ":
        if ch.isalnum():
            word += ch
            continue
        if len(word) > 2:
            h = 0xCBF29CE484222325
            for b in word.encode():
                h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
            v[h % DIMS] += 1.0 if (h >> 40) & 1 == 0 else -1.0
        word = ""
    norm = math.sqrt(sum(x * x for x in v))
    if norm == 0:
        v[0] = 1.0
        return v
    return [x / norm for x in v]


def millis(text):
    if "T" not in text:
        text += "T00:00:00Z"
    return int(datetime.strptime(text, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc).timestamp() * 1000)


def readers(row):
    return [r for r in (row.get("readers") or "").split(";") if r]


def load_rows(fleet):
    """The final rows: the base export with every update applied, by key."""
    keys = {"maintenance": "work_order", "incidents": "incident_id", "units": "serial",
            "firmware_installs": "install_id"}
    tables = {t: {} for t in keys}
    for table, key in keys.items():
        with open(fleet / "base" / f"{table}.csv", newline="", encoding="utf-8") as f:
            for row in csv.DictReader(f):
                tables[table][row[key]] = {k: v for k, v in row.items() if v != ""}
    with open(fleet / "updates.jsonl", encoding="utf-8") as f:
        for line in f:
            update = json.loads(line)
            for table, rows in update["tables"].items():
                if table in keys:
                    for row in rows:
                        tables[table][row[keys[table]]] = row
    return tables


class Science:
    def __init__(self, binary, db):
        self.binary, self.db = binary, db

    def call(self, actor, request, writes=()):
        args = [self.binary, "--db", str(self.db), "--actor", actor]
        for graph in writes:
            args += ["--write", graph]
        r = subprocess.run(args, input=json.dumps(request), capture_output=True, text=True)
        response = json.loads(r.stdout) if r.stdout.strip() else {"ok": False, "error": r.stderr}
        if not response.get("ok"):
            sys.exit(f"weave-science failed: {response.get('error')}")
        return response["result"]

    def evaluate(self, actor, expression):
        from_version = self.call(actor, {"operation": "capabilities"})["engine_contract_version"]
        program = {"version": from_version, "commands": [{"op": "evaluate", "value": expression}]}
        return self.call(actor, {"operation": "execute", "program": program})["results"][0]["result"]


def provenance(result, embedding_id):
    for entry in result.get("fact_provenance", []):
        if entry["fact"].get("embedding_id") == embedding_id:
            p = entry["provenance"]
            return f"{p['writer']} via {p['method']['kind']} {p['method'].get('model', '')}".strip()
    return "unknown"


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--science", required=True, help="path to the weave-science binary")
    p.add_argument("--db", required=True, type=Path)
    p.add_argument("--fleet", required=True, type=Path, help="directory written by weave-hardware generate")
    p.add_argument("--reader", default="fleet-analyst", help="principal to answer as (fleet-admin sees every customer)")
    a = p.parse_args()
    science = Science(a.science, a.db)
    tables = load_rows(a.fleet)

    # 1-2. Embed and write: one embedding fact per note, incident and unit.
    space = lambda sid, prep: {"change": "assert", "fact": {
        "kind": "embedding_space", "id": sid, "revision": "1", "encoder": ENCODER, "encoder_version": "1",
        "preprocessing": prep, "dimensions": DIMS, "metric": "cosine_distance"}}
    changes = [space("hw-notes", "lowercase-words/v1"), space("hw-units", "unit-profile/v1")]
    profiles = {}

    def fact(eid, sid, subject, text, start, end, row):
        return {"change": "assert", "fact": {
            "kind": "embedding", "id": eid, "space": {"id": sid, "revision": "1"},
            "subject": {"kind": "node", "node_id": subject}, "vector": embed(text),
            "valid_time": {"start": start, "end": end}, "readers": readers(row),
            "provenance": {"method": {"kind": "model", "model": ENCODER, "version": "1"}, "source": "quickstart"}}}

    for row in tables["maintenance"].values():
        subject = f"maint:{row['work_order']}"
        changes.append(fact(f"emb:note:{subject}", "hw-notes", subject, row["note"], millis(row["started_at"]), None, row))
        profiles[row["serial"]] = profiles.get(row["serial"], "") + " " + row["note"]
    incidents = []
    for row in tables["incidents"].values():
        subject = f"incident:{row['incident_id']}"
        changes.append(fact(f"emb:note:{subject}", "hw-notes", subject, row["description"], millis(row["opened_at"]), None, row))
        incidents.append(row)
        profiles[row["serial"]] = profiles.get(row["serial"], "") + " " + row["title"]
    for row in tables["firmware_installs"].values():
        profiles[row["serial"]] = profiles.get(row["serial"], "") + " " + row["firmware_id"]
    for row in tables["units"].values():
        end = millis(row["decommissioned_at"]) if row.get("decommissioned_at") else None
        text = row["model_id"] + profiles.get(row["serial"], "")
        changes.append(fact(f"emb:profile:unit:{row['serial']}", "hw-units", f"unit:{row['serial']}", text,
                            millis(row["commissioned_at"]), end, row))
    for start in range(0, len(changes), 2000):
        science.call(EMBEDDER, {"operation": "import", "graph_id": GRAPH, "changes": changes[start:start + 2000]}, [GRAPH])
    print(f"wrote {len(changes) - 2} embedding facts as {EMBEDDER} (a re-run writes nothing new)")

    # 3a. Notes similar to the latest incident the reader can see on a unit with
    # maintenance history, within two hops of that unit.
    def sees(row):
        return not readers(row) or a.reader in readers(row)

    def has_history(r):  # the unit has visible maintenance notes before the incident
        return any(m["serial"] == r["serial"] and sees(m) and millis(m["started_at"]) <= millis(r["opened_at"])
                   for m in tables["maintenance"].values())

    visible = [r for r in incidents if sees(r) and sees(tables["units"][r["serial"]]) and has_history(r)]
    incident = max(visible, key=lambda r: (millis(r["opened_at"]), r["incident_id"]))
    opened = millis(incident["opened_at"])
    result = science.evaluate(a.reader, {
        "kind": "nearest", "graph_id": GRAPH, "space": {"id": "hw-notes", "revision": "1"},
        "query": {"kind": "embedding", "embedding_id": f"emb:note:incident:{incident['incident_id']}"},
        "k": 5, "valid_at": opened,
        "candidates": {"kind": "expand", "graph_id": GRAPH, "seeds": [f"unit:{incident['serial']}"],
                       "hops": 2, "direction": "both"}})
    notes = {x["id"]: x["value"].get("value") for x in result["graph"].get("attachments", [])}
    print(f"\nas {a.reader}: notes like {incident['incident_id']} ({incident['title']}) "
          f"within 2 hops of {incident['serial']}, valid at {incident['opened_at']}:")
    for n in result.get("neighbors", []):
        subject = n["embedding_id"].removeprefix("emb:note:")
        text = notes.get(f"note:{subject}", "")
        print(f"  {n['score']:.3f}  {subject}: {text[:90]}")
        print(f"         source: {provenance(result, n['embedding_id'])}")

    if not result.get("neighbors"):
        sys.exit("no similar notes found")

    # 3b. Similar units that the graph does not link.
    result = science.evaluate(a.reader, {
        "kind": "similar_candidates", "graph_id": GRAPH, "space": {"id": "hw-units", "revision": "1"},
        "k": 1, "exclude_linked": True})
    print(f"\nas {a.reader}: similar-unit candidates absent from the graph:")
    for c in result.get("candidates", [])[:2]:
        origins = ", ".join(o["embedding_id"] for o in c["origins"])
        print(f"  {c['score']:.3f}  {c['from']} {c['predicate']} {c['to']}  (from {origins})")
    if not result.get("candidates"):
        sys.exit("no candidates")


if __name__ == "__main__":
    main()
