# Phase A exit scenario check

The [Phase A exit scenario](ROADMAP.md#phase-a-rock-solid-shared-core) runs
as one automated check, `scripts/check_phase_a_exit.py`, with one pass or
fail. CI runs it in the `phase-a-exit` job of `ci.yml` on Linux, macOS and
Windows, at the exit size of 5,000 units and 1,000 updates, with an optimized
build. Each run uploads a JSON report as the `phase-a-exit-<os>` artifact.

```sh
python3 scripts/check_phase_a_exit.py                  # 5,000 units, 1,000 updates
python3 scripts/check_phase_a_exit.py --units 150 --updates 150 --report small.json
```

The script reimplements no part of the scenario or its oracles. It runs three
integration tests over the same generated fleet (the seeded A5 generator) at
the requested size:

| Test | What it checks |
|---|---|
| `crates/weave-hardware/tests/fleet_scenario.rs` | Imports the base export from CSV (parts, firmware, sites, owners, series references, maintenance notes), re-imports it with zero facts written, and applies each update as its own fact write (question 1). Answers question 2 in both forms against an independent oracle built from the rows. Hides restricted units from the analyst (question 5). Checks that answers cite their source facts and are identical after the store is reopened |
| `crates/weave-hardware/tests/fleet_hybrid.rs` | Embeds every note, incident and unit profile as embedding facts. Answers question 3 (`nearest` over the 2-hop `expand` neighborhood, valid at the incident time) and question 4 (`similar_candidates` with `exclude_linked`) against an independent ranking. Covers question 5, the embedding provenance of each hit, and reopening |
| `crates/weave-mcp/tests/fleet_exit_questions.rs` | A scripted MCP client asks questions 2 to 4 through the real `weave-mcp` binary over stdio, using only the preview tools. The binary opens the store after the writer has closed it. The test checks results against the `weave-hardware` helpers and the same independent rankings, and checks provenance (including every `what_changed` entry) and question 5 |

Each test prints one `PHASE_A_EXIT {json}` line, and only after all of its
assertions have passed. The script fails unless all of the following hold:

- each test exits successfully and prints its line, at the requested size;
- every scenario clause is reported by the tests listed for it in the
  script's `CLAUSES` table: the fleet and its embeddings, questions 1 to 5,
  source-fact explanations, reproducibility after reopening, and questions 2
  to 4 through MCP;
- all three tests report the same dataset digest and the same head revision
  after the 1,000 updates. This shows that they answered over one fleet state
  (content-derived revision IDs), not three similar ones.

The report records each test's command, exit code and duration, the clause
map, the dataset digest, the head revision, the commit and the host.

**Limits.** The three tests each build their own copy of the fleet store from
the same seed. Equal digests and heads show equal content; the script does not
share one database file between them. The test embedder is a deterministic
word-hashing function, not a language model, so questions 3 and 4 test
retrieval semantics, not embedding quality. The size and duration are those
of the scenario as written. They are not capacity or latency promises. On one
local Linux host (AMD Ryzen 9 7950X), the three tests took about 100 s, 130 s
and 70 s at the exit size.
