# Weave contract 1.1.0: group readers and delegation provenance

Protocol 1.1.0 is the first additive minor version after the 1.0.0 freeze
(B1, [#38](https://github.com/weave-graph/weave-engine/issues/38),
[ADR-0007](../../architecture/ADR-0007-identity-groups-and-tenancy.md)). It
activates the name `reader_groups`, reserved since 0.22.0, and adds one
engine-recorded provenance field. Semantics are in
[core v1 §4 `fact.reader_groups`](../core-v1.md#4-facts) and
[§6](../core-v1.md#6-provenance); the user guide is
[REGISTRY.md](../../REGISTRY.md).

## What changes

- **`reader_groups`** on `Node`, `Edge`, `StructuralEdge`, `Assertion`,
  `MetadataAttachment` and `EmbeddingFact` (embedding spaces have no
  readers). A list of group IDs, omitted from encodings when empty, so every
  record without groups keeps its canonical bytes, content hash and revision
  ID. It widens access alongside `readers` (a disjunction), except that a
  record with empty `readers` and nonempty `reader_groups` is not public.
  Groups resolve through the store's principal registry when each request
  runs.
- **`FactProvenance.on_behalf_of`**: the ownership chain of an agent writer,
  owner first, at most 8 principal IDs; omitted when empty. The engine
  records it on fact writes by agents; results of programs older than 1.1.0
  never carry it.
- **Versions.** `weave_contract::VERSION` and the contract crate are 1.1.0.
  The table accepts 0.1.0 through 0.24.0, 1.0.0 and 1.1.0. 1.2.0 and 2.0.0
  fail with `E_VERSION` before structural decoding.
- **Gates.** `reader_groups` in a `write` or `commit` under a program older
  than 1.1.0 fails with `E_VERSION` at execution. Decoding accepts it under
  every version, because decoding is structural only (core v1 §14); a 1.0.0
  program that never names a group decodes and runs exactly as before.

No command, expression or result envelope changes. The registry and ACL
changes are a trusted engine API (`weave_engine::RegistryChange`), not wire
types (ADR-0007 amendment 2).

## Fixtures

- `positive/reader-groups-omitted-when-empty.json`: an empty list re-encodes
  without the field.
- `negative/reader-groups-unknown-group.json`: a 1.1.0 write naming a group in
  a store without a registry fails with `E_UNAVAILABLE`.
- `negative/reader-groups-version-gated.json`: the same field under 1.0.0
  decodes and fails with `E_VERSION`. It replaces
  `negative/reserved-node-reader-groups.json`, which expected
  `E_UNKNOWN_FIELD` while the name was reserved.
- `positive/fact-provenance-on-behalf-of.json`: a provenance record with a
  delegation chain.
- `negative/unknown-version.json` now uses 1.2.0, because 1.1.0 is real.
- The manifest protocol is 1.1.0.

## For consumers

The language vendors `crates/weave-contract/` byte for byte. The 1.1.0
vendor update is the list of changed files in the B1 entry of the
[CHANGELOG](../../../CHANGELOG.md). A compiler that emits 1.0.0 programs
keeps working unchanged; results report the producing engine's version,
1.1.0.
