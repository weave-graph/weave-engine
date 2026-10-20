# Hardware reference model

The hardware reference model (roadmap A5, [#36](https://github.com/weave-graph/weave-engine/issues/36))
is a ready-made schema for a hardware fleet knowledge base. It covers
assets, parts, firmware, sites, sensors, series references, users and
organizations. It ships with:

- CSV and JSON importers that turn tabular exports (for example from ERP,
  PLM or a fleet-operations tool) into individual fact writes;
- a deterministic, seeded fleet generator sized for the Phase A exit
  scenario;
- query helpers that answer fleet questions with the existing operators.

It is implemented in the `weave-hardware` crate (library and CLI) on top of
the A2 fact store ([ADR-0004](architecture/ADR-0004-fact-log-and-storage-backend.md),
[ADR-0005](architecture/ADR-0005-per-fact-provenance.md)). It needs no
contract change: the model is an ordinary typed graph schema, and a
`SeriesRef` is a typed metadata value.

## Quickstart

```sh
cargo build --release --locked -p weave-hardware
B=target/release/weave-hardware
$B generate --out fleet --units 5000 --updates 1000 --seed 42
$B import --db fleet.db --actor etl-importer --graph fleet --csv fleet/base
$B import --db fleet.db --actor etl-importer --graph fleet --csv fleet/base   # writes 0 facts
$B apply-updates --db fleet.db --actor etl-importer --graph fleet --updates fleet/updates.jsonl
$B schema                                                                   # the graph schema as JSON
```

Each command prints one JSON object, `{"ok": true, "result": ...}` or
`{"ok": false, "error": {"code", "message"}}`, and exits nonzero on error.
`--actor` is trusted local authority, as with `weave-science`; the CLI grants
it write access to `--graph` only. `import` reads a directory of
`<table>.csv` files (`--csv`) or one JSON batch (`--json`). `--source` sets the
claimed source for rows without a `source` column.

The import report counts what happened:

```json
{"facts": 151622, "asserted": 0, "corrected": 0, "unchanged": 151622,
 "revisions": [], "initialized": false, "head": "fact:sha256:…"}
```

The store can then be read with `weave-science` or the Python client, like
any other graph (for example `{"op": "query", "query": {"graph_id": "fleet",
"predicate": "hw:runs", "to": "fw:compressor-2.2.0", "valid_at": 1717200000000}}`).

## Model

All nodes are in space `hw`. A node's `type_id` names its type and its ID is
`<prefix>:<key>`, where the key comes from the row (for example
`unit:SN-000123`). The entity ID equals the node ID: the model has one
manifestation per entity. Edge `type_id` equals the predicate.

### Node types

| Type | ID prefix | Properties (required in bold) | Meaning |
|---|---|---|---|
| `hw.Organization` | `org` | **name**, **kind** | A customer, operator, manufacturer or internal organization |
| `hw.Team` | `team` | **name** | A team inside an organization |
| `hw.User` | `user` | **name**, principal, role | A person. `principal` is the Weave principal used in `readers` |
| `hw.Site` | `site` | **name**, region, timezone | A physical location where units operate |
| `hw.Model` | `model` | **name**, manufacturer, product_line | A product model |
| `hw.FirmwareVersion` | `fw` | **product**, **version**, released_at | A firmware or software version |
| `hw.Configuration` | `config` | **name**, uri, digest | A named configuration, stored outside the graph |
| `hw.Unit` | `unit` | **serial**, name | A physical asset, identified by its serial number. Node valid time runs from commissioning to decommissioning |
| `hw.Part` | `part` | **part_serial**, **part_number**, description | A serialized physical part |
| `hw.Sensor` | `sensor` | **kind**, unit_of_measure, sample_rate_hz | A sensor. Its samples stay in a series store |
| `hw.MaintenanceEvent` | `maint` | **kind** | A maintenance work order. Node valid time is when the work happened |
| `hw.Incident` | `incident` | **severity**, **title** | An incident. Node valid time runs from open to close |

Serial numbers are identity keys (`serial`, `part_serial`), not separate
nodes. Time properties (`released_at`) are integer Unix-epoch milliseconds.
`sample_rate_hz` is a float. All other properties are strings.

### Relations

Every relation is an edge with a valid time `[start, end)`. Intervals that
are open (`end` absent) are still true.

| Predicate | From → to | Valid time |
|---|---|---|
| `hw:part_of` | Team → Organization | always (start 0) |
| `hw:member_of` | User → Team | membership period, start 0 when not given |
| `hw:operated_by` | Site → Organization | always |
| `hw:has_model` | Unit → Model | from commissioning |
| `hw:located_at` | Unit → Site | placement period |
| `hw:owned_by` | Unit → Organization | ownership period |
| `hw:configured_with` | Unit → Configuration | assignment period |
| `hw:installed_in` | Part → Unit | install period. Property `work_order` |
| `hw:replaced` | Part → Part (new → old) | from the replacement. Property `work_order` |
| `hw:runs` | Unit → FirmwareVersion | install period |
| `hw:measured_by` | Unit → Sensor | mount period |
| `hw:performed_on` | MaintenanceEvent → Unit | from the start of the work |
| `hw:performed_by` | MaintenanceEvent → User | from the start of the work |
| `hw:affected` | Incident → Unit | from when the incident opened |

### Attachments

| Key | Host | Value | Valid time |
|---|---|---|---|
| `hw:series` | a sensor | a `SeriesRef` (below) | exactly the referenced range |
| `hw:note` | a maintenance event or an incident | the note or description text (string literal) | the host's valid time |

A note's ID is `note:<host node ID>` (for example `note:maint:WO-000123`).
That stable ID and its text are the hook for A4 embeddings
([#13](https://github.com/weave-graph/weave-engine/issues/13)). The importer
writes no embedding facts: the caller embeds the notes and writes them as
[embedding facts](EMBEDDINGS.md) on the maintenance or incident node, as
`tests/fleet_hybrid.rs` does.

### Enforcement

The importer creates the graph with the reference schema `weave.hardware`,
revision `1`, as its first (snapshot) revision. Every later fact write is
validated against it, so the engine rejects a node with an undeclared or
mistyped property and an edge whose endpoint types do not match the
relation, whoever writes it. Importing into an existing graph without this
schema fails with `E_HW_SCHEMA`.

## `SeriesRef`

A `SeriesRef` references a half-open time range `[start, end)` of one series
in an external store. No raw samples are stored in the graph, following the
C1 leaning for the telemetry boundary (ADR-0009, not yet written).

| Field | Meaning |
|---|---|
| `store_uri` | Where the series store is (Parquet dataset, TSDB) |
| `series_id` | The series inside that store |
| `range` | `{start, end}` in Unix-epoch milliseconds. `end` is required and exclusive |
| `digest` | Digest of the referenced samples, so a reader can detect changed data |

It is stored as an `hw:series` attachment on a sensor. The value is a typed
literal:

```json
{"kind": "literal", "value": {
  "type": "weave.hw.series_ref", "version": 1,
  "store_uri": "parquet://fleet-telemetry/vibration/year=2024",
  "series_id": "SN-000123-V",
  "range": {"start": 1704067200000, "end": 1719792000000},
  "digest": "sha256:…"}}
```

The attachment's `valid_time` must equal `range`. Decoding
(`SeriesRef::from_attachment`) rejects a wrong type tag or version, unknown
fields, empty fields, an unbounded or empty range and a valid time that
differs from the range, all with `E_HW_SERIES`.

Because the attachment's valid time is the range, `valid_at` selection
applies to series references directly. `query::series_overlapping` returns
the references whose range overlaps a window `[start, end)` (shares at least
one instant), for one unit or the whole fleet. For each one it lists the
units the sensor was mounted on during the overlap.

## Tables and columns

Each table maps rows to facts. The **key** column is the row's stable
identity. Every fact ID the row produces derives from it, which makes the
key the idempotency key: the same row always maps to the same facts. Keys
are 1 to 256 bytes with no whitespace.

| Table | Key | Other columns | Facts per row |
|---|---|---|---|
| `organizations` | `org_id` | name, kind | `hw.Organization` |
| `teams` | `team_id` | name, org_id | `hw.Team`, `hw:part_of` |
| `users` | `user_id` | name, principal, role, team_id, valid_from, valid_to | `hw.User`, `hw:member_of` when team_id is set |
| `sites` | `site_id` | name, region, timezone, org_id | `hw.Site`, `hw:operated_by` when org_id is set |
| `models` | `model_id` | name, manufacturer, product_line | `hw.Model` |
| `firmware` | `firmware_id` | product, version, released_at | `hw.FirmwareVersion` |
| `configurations` | `config_id` | name, uri, digest | `hw.Configuration` |
| `units` | `serial` | name, model_id, commissioned_at, decommissioned_at | `hw.Unit`, `hw:has_model` |
| `placements` | `placement_id` | serial, site_id, valid_from, valid_to | `hw:located_at` |
| `ownerships` | `ownership_id` | serial, org_id, valid_from, valid_to | `hw:owned_by` |
| `configuration_assignments` | `assignment_id` | serial, config_id, valid_from, valid_to | `hw:configured_with` |
| `parts` | `part_serial` | part_number, description | `hw.Part` |
| `part_installs` | `install_id` | part_serial, serial, valid_from, valid_to, replaces, work_order | `hw:installed_in`, `hw:replaced` when replaces is set |
| `firmware_installs` | `install_id` | serial, firmware_id, valid_from, valid_to | `hw:runs` |
| `sensors` | `sensor_id` | kind, unit_of_measure, sample_rate_hz | `hw.Sensor` |
| `sensor_mounts` | `mount_id` | sensor_id, serial, valid_from, valid_to | `hw:measured_by` |
| `series` | `series_ref_id` | sensor_id, store_uri, series_id, start, end, digest | `hw:series` attachment |
| `maintenance` | `work_order` | kind, serial, performed_by, started_at, ended_at, note | `hw.MaintenanceEvent`, `hw:performed_on`, `hw:performed_by`, `hw:note` |
| `incidents` | `incident_id` | severity, title, serial, opened_at, closed_at, description | `hw.Incident`, `hw:affected`, `hw:note` |

Every table also accepts:

| Column | Meaning |
|---|---|
| `readers` | Semicolon-separated principals (a JSON list in JSON input). Every fact of the row gets these `readers`; empty means public |
| `source` | Claimed provenance source for the row's facts |
| `evidence_uri`, `evidence_digest` | An artifact the row cites (for example a work-order PDF), recorded as claimed evidence |

Fact IDs: `<edge prefix>:<key>` for edges (`runs:FW-SN-000123-2`,
`located_at:PL-SN-000123-1`), `series:<key>` for series references and
`note:<host>` for notes. The edge prefix is the predicate without `hw:`.

**Times** are integer Unix-epoch milliseconds, `YYYY-MM-DD`, or
`YYYY-MM-DDTHH:MM:SS[.mmm]Z` (UTC only). An empty end means open. An end at
or before its start fails with `E_HW_ROW`.

**Input checking** is strict: an unknown table, file or column, a repeated
column or a record with the wrong number of fields fails with `E_HW_INPUT`;
a missing required value, a malformed time or number, or two rows that
produce different facts with one ID fail with `E_HW_ROW`. A row naming a node
that does not exist fails in the engine with `E_SCHEMA_ENDPOINT`. A failing
row means no fact of the batch is written; only the schema header of a first
import may already exist (see also the limit on chunked imports below).

CSV follows RFC 4180: a header row, comma separators, `"` quoting with `""`
escapes, and quoted fields may hold commas and line breaks. A JSON batch is
`{"source"?: "...", "tables": {"<table>": [{"<column>": value}]}}`; values
are strings, numbers, booleans or null (absent).

## Incremental, idempotent and attributable import

An import maps every row to facts and reconciles them against the branch
head (`Engine::reconcile_facts`):

- a fact that is not live is **asserted**;
- a live fact with the same ID that differs is **corrected**, which writes a
  new version and keeps the old one readable at earlier revisions;
- an identical live fact is **unchanged** and nothing is written for it.

The changes go out as one A2 `write` per chunk of at most 100,000 changes,
guarded by `expected_head`, so a concurrent writer causes `E_CONFLICT`
instead of a lost update. An identical second run therefore writes **zero
facts and no revision**. The reconciliation needs write authority and
reveals no more than a write would: whether each fact is absent, identical
or different.

Every fact carries a claimed provenance (ADR-0005):

```json
{"source": "erp", "method": {"kind": "import", "importer": "weave-hardware", "version": "1"}}
```

The source is the row's `source` column, else the batch source, else the
table default (`directory`, `erp`, `plm`, `fleet-ops`, `telemetry` or
`field-service`). The method's `version` is the schema and mapping revision,
not the binary version, so upgrading the binary does not re-attribute
unchanged data. The engine records the writer (the `--actor`) and recorded
time; a client cannot set them. Provenance is part of a fact's content: a row
is one claim, so re-sending a row with another source corrects every fact of
that row.

**Valid time and corrections.** Firmware installs, placements, ownerships,
configuration assignments, part installs and sensor mounts are valid-time
intervals. A firmware rollout is two changed rows: the current install gets
its `valid_to` (a correction of that edge) and a new install row starts at
the rollout time (an assertion). A part replacement closes the old install
and adds a new part, a new install with `replaces`, and a work order. A
wrong date or attribute is fixed by re-sending the corrected row; the
correction is a new fact version, never an edit of history.

A row cannot change the structural identity of its facts (for example an
install row that now names another unit). That fails with `E_CONFLICT`.
Write a new row with a new key instead.

## Generated fleet dataset

`weave-hardware generate` (and `generate::generate`) produces a synthetic but
realistic fleet. The default, `--units 5000 --updates 1000 --seed 42`, is the
Phase A exit-scenario size: about 151,600 facts.

- Two Northwind organizations (manufacturer, field service), one customer
  per 250 units with an operations team and operator, one technician per 25
  units, and one site per 80 units, each operated by a customer.
- Six models in three product lines, each line with seven or eight firmware
  versions released from 2020 to 2025, and three configurations per model.
- Units commissioned from 2021 to March 2024, about 2% decommissioned. Each
  has a placement (8% moved once), an ownership, a configuration, four parts,
  a firmware history with rollouts after releases, a vibration and a
  temperature sensor, and one `SeriesRef` per sensor and calendar year.
- Maintenance work orders with realistic note text, part replacements (10%),
  and incidents (4%).
- **Reader restrictions.** Every fifth customer (C003, C008, …) is
  customer-restricted: every row about its units has
  `readers = fleet-admin;customer:<id>`. The principal `fleet-analyst` sees
  only unrestricted units. This is what A6 exit check 5 exercises.
- **Updates.** `updates.jsonl` holds 1,000 updates reported between July 2024
  and July 2025. Each is one fact write: firmware rollouts (45%), corrections
  of a rollout date (10%), relocations (8%), part replacements (12%),
  maintenance (12%), incidents (5%) and attribute corrections (8%). Each line
  has `seq`, `kind`, `reported_at` (when the source reported it; the engine
  still assigns recorded time) and a batch. The percentages are the draw
  weights; a draw that does not apply to the drawn unit is redrawn.

The output is `base/<table>.csv`, `updates.jsonl` and `manifest.json` (the
configuration and the SHA-256 of every file). The generator uses its own
SplitMix64 generator, integer time arithmetic and ordered maps, so the same
configuration gives byte-identical files on every platform. The committed
fixture `crates/weave-hardware/fixtures/fleet-small` (seed 7, 12 units, 24
updates) must equal a fresh run, and the default dataset's digest is pinned
in `crates/weave-hardware/tests/generator.rs`. A change to the output bumps
`GENERATOR_VERSION`. Generated datasets are not committed; regenerate them.

## Fleet questions

`weave_hardware::query` answers these with ordinary engine reads under the
caller's authority, so `readers` apply to every answer:

- `units_at_site_running(site, firmware, valid_at, at)`: which units at site
  S ran firmware F at valid time D, as known at `at`. Two indexed reads
  (`hw:runs` to F, `hw:located_at` to S, both at D) intersected by unit. The
  answer lists the supporting edge of each unit and their engine-recorded
  provenance.
- `unit_changes(unit, before, after)`: what changed for unit U between two
  states: the unit node, its edges and attachments, each added, removed or
  corrected.
- `series_overlapping(unit?, start, end, at)`: series references by overlap.

`at` selects the state: `Head`, `Revision(id)` (for example from an import
report), `Recorded(unix_millis)` (`recorded_query` with `local_time`) or
`AsOf(unix_millis)` (the A3 `as_of` expression, which works for restricted
readers).

## Phase A scenario

`crates/weave-hardware/tests/fleet_scenario.rs` runs the parts of the Phase A
exit scenario that the current operators reach, on the generated fleet:
import the base export from CSV, re-import it (zero facts), apply every update
as its own fact write with a test clock at its reported time, then answer
question 2 (both forms) for 12 rollouts, before and after each one was
recorded, for `fleet-admin` and `fleet-analyst`, and compare every answer with
an independent oracle computed from the rows. It also checks that every
answer cites the provenance of its facts, that restricted units never reach
the analyst, and that answers are identical after reopening the store. CI
runs it with 150 units in the main suite and with 5,000 units and 1,000
updates in the `phase-a-exit` job on Linux, macOS and Windows, as part of
the [Phase A exit scenario check](PHASE_A_EXIT.md).

Measured on one Linux host (AMD Ryzen 9 7950X, optimized build, on-disk
store), 5,000 units: the base import (151,622 facts, two fact revisions)
took about 10 s, the identical re-import (zero facts) about 4.6 s, and the
1,000 updates about 2.3 s in total. This is a description of one run, not a
capacity promise.

Questions 3 and 4 are answered by `tests/fleet_hybrid.rs` (A4) on the same
generated fleet. A deterministic test embedder (signed feature hashing of
words into 64 dimensions; no network and no model) embeds every maintenance
note and incident description into the `hw-notes` space and a profile of
every unit (model, firmware, notes, incident titles) into `hw-units`,
written as embedding facts with the rows' readers and `method: model`
provenance. Question 3 is one `nearest` plan whose candidates are the
`expand` 2-hop neighborhood of the affected unit, valid at the incident
time; question 4 is one `similar_candidates` plan with `exclude_linked`.
Both are compared with an independent oracle for `fleet-admin` and
`fleet-analyst`, return the note text and the embeddings' provenance, keep
restricted units from the analyst, and are reproduced after reopening the
store. CI runs it with 150 units in the main suite and at 5,000 units in the
`phase-a-exit` job.

## Example

This batch shows every node type, relation and attachment of the model. A
test imports it and checks that coverage, so it stays valid.

```json
{
  "source": "example",
  "tables": {
    "organizations": [
      {"org_id": "NWM", "name": "Northwind Machines", "kind": "manufacturer"},
      {"org_id": "C001", "name": "Customer 001 Logistics", "kind": "customer"}
    ],
    "teams": [{"team_id": "C001-OPS", "name": "Customer 001 Operations", "org_id": "C001"}],
    "users": [
      {"user_id": "T0001", "name": "Technician 0001", "principal": "tech-0001",
       "role": "technician", "team_id": "C001-OPS", "valid_from": "2023-01-01"}
    ],
    "sites": [{"site_id": "S001", "name": "Plant 001", "region": "eu-north",
               "timezone": "Europe/Stockholm", "org_id": "C001"}],
    "models": [{"model_id": "CX-200", "name": "CX-200 compressor",
                "manufacturer": "Northwind Machines", "product_line": "compressor"}],
    "firmware": [
      {"firmware_id": "compressor-2.1.0", "product": "compressor", "version": "2.1.0", "released_at": "2023-05-22"},
      {"firmware_id": "compressor-2.2.0", "product": "compressor", "version": "2.2.0", "released_at": "2024-01-15"}
    ],
    "configurations": [{"config_id": "CX-200-standard", "name": "CX-200 standard",
                        "uri": "git://configs/CX-200/standard.toml", "digest": "sha256:5f1c"}],
    "units": [{"serial": "SN-000001", "name": "CX-200-000001", "model_id": "CX-200",
               "commissioned_at": "2023-06-01", "readers": ["fleet-admin", "customer:C001"]}],
    "placements": [{"placement_id": "PL-SN-000001-1", "serial": "SN-000001", "site_id": "S001",
                    "valid_from": "2023-06-01", "readers": ["fleet-admin", "customer:C001"]}],
    "ownerships": [{"ownership_id": "OW-SN-000001-1", "serial": "SN-000001", "org_id": "C001",
                    "valid_from": "2023-06-01", "readers": ["fleet-admin", "customer:C001"]}],
    "configuration_assignments": [{"assignment_id": "CA-SN-000001-1", "serial": "SN-000001",
                                   "config_id": "CX-200-standard", "valid_from": "2023-06-01"}],
    "parts": [
      {"part_serial": "PT-0000001", "part_number": "PN-C-B6", "description": "CX-200 main bearing"},
      {"part_serial": "PT-0000002", "part_number": "PN-C-B6", "description": "CX-200 main bearing"}
    ],
    "part_installs": [
      {"install_id": "PI-0000001", "part_serial": "PT-0000001", "serial": "SN-000001",
       "valid_from": "2023-06-01", "valid_to": "2024-03-04T13:00:00Z"},
      {"install_id": "PI-0000002", "part_serial": "PT-0000002", "serial": "SN-000001",
       "valid_from": "2024-03-04T13:00:00Z", "replaces": "PT-0000001", "work_order": "WO-000002"}
    ],
    "firmware_installs": [
      {"install_id": "FW-SN-000001-1", "serial": "SN-000001", "firmware_id": "compressor-2.1.0",
       "valid_from": "2023-06-01", "valid_to": "2024-02-12"},
      {"install_id": "FW-SN-000001-2", "serial": "SN-000001", "firmware_id": "compressor-2.2.0",
       "valid_from": "2024-02-12", "source": "fleet-ops"}
    ],
    "sensors": [{"sensor_id": "SN-000001-V", "kind": "vibration", "unit_of_measure": "mm/s",
                 "sample_rate_hz": 2560}],
    "sensor_mounts": [{"mount_id": "SM-SN-000001-V-1", "sensor_id": "SN-000001-V",
                       "serial": "SN-000001", "valid_from": "2023-06-01"}],
    "series": [{"series_ref_id": "SN-000001-V-2024", "sensor_id": "SN-000001-V",
                "store_uri": "parquet://fleet-telemetry/vibration/year=2024",
                "series_id": "SN-000001-V", "start": "2024-01-01", "end": "2024-07-01",
                "digest": "sha256:9a0e"}],
    "maintenance": [{"work_order": "WO-000002", "kind": "replacement", "serial": "SN-000001",
                     "performed_by": "T0001", "started_at": "2024-03-04T13:00:00Z",
                     "ended_at": "2024-03-04T16:00:00Z",
                     "note": "Checked the main bearing: vibration elevated at 2x running speed; replaced the worn component and ran a 30 minute test. Readings back within limits.",
                     "evidence_uri": "erp://work-orders/WO-000002.pdf", "evidence_digest": "sha256:77c2"}],
    "incidents": [{"incident_id": "INC-00001", "severity": "high", "title": "Abnormal vibration alarm",
                   "serial": "SN-000001", "opened_at": "2024-03-02T03:00:00Z",
                   "closed_at": "2024-03-04T16:00:00Z",
                   "description": "Abnormal vibration alarm on SN-000001: vibration elevated at 2x running speed."}]
  }
}
```

The unit row above produces these two facts (the importer adds the
`hw:has_model` edge with the same `readers` and claim):

```json
{"kind": "node", "id": "unit:SN-000001", "entity_id": "unit:SN-000001", "space_id": "hw",
 "type_id": "hw.Unit", "valid_time": {"start": 1685577600000, "end": null},
 "properties": {"serial": "SN-000001", "name": "CX-200-000001"},
 "readers": ["customer:C001", "fleet-admin"],
 "provenance": {"source": "example", "method": {"kind": "import", "importer": "weave-hardware", "version": "1"}}}
{"kind": "edge", "id": "has_model:SN-000001", "type_id": "hw:has_model", "predicate": "hw:has_model",
 "from": "unit:SN-000001", "to": "model:CX-200", "valid_time": {"start": 1685577600000, "end": null}, "...": "..."}
```

## Limits

- **Chunked imports are not atomic.** An import above 100,000 changes is
  written as several fact revisions, one per chunk. Each chunk is atomic and
  head-guarded; a failure in a later chunk leaves the earlier chunks
  written. Re-running the import completes it, because it is idempotent.
- **No deletion by omission.** A row missing from a later export is not
  retracted; the importer only asserts and corrects. End a relation by
  setting its `valid_to`.
- **Re-importing an old export reverts.** The importer makes the store match
  the rows it is given. Importing an older export again corrects facts back
  to the older values (as new versions, with history kept).
- **Recorded-time selection and restricted readers.** `At::Recorded`
  (`recorded_query`) needs the reader to see the whole selected revision
  (ADR-0002), so a reader who cannot see the restricted units gets
  `E_HISTORY_UNAVAILABLE`. `At::AsOf` (A3 `as_of`) selects the same revision
  and returns the visible subset to that reader, equal to a read pinned to
  it. The scenario checks all three forms for both readers.
- **Overlap is filtered after an indexed read.** There is no interval-overlap
  index; `series_overlapping` reads the `hw:measured_by` edges (by unit when
  given) with their sensors and attachments, then filters. Fleet-wide calls
  are bounded by the read budgets.
- **Interim helpers.** `units_at_site_running` and `unit_changes` compose
  filtered reads until the A3 `expand` and `changes` operators exist.
- **Fixed model.** Properties outside the schema are rejected. Extending the
  model is a schema revision with a new mapping version.
