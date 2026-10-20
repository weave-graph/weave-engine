# Weave contract 1.2.0: write-time propagation

Protocol 1.2.0 is an additive minor version (B2,
[#5](https://github.com/weave-graph/weave-engine/issues/5),
[ADR-0008](../../architecture/ADR-0008-write-time-propagation-and-release-policies.md)).
It adds one optional field and two write rules for 1.2.0 programs, and its
engine treats live versions a writer cannot read as absent for every program
version. Semantics are in core v1 §4,
[`fact.write` and `fact.propagation`](../core-v1.md#4-facts).

## What changes

- **`widen_readers`** on a `correct` change: a boolean, omitted when false.
  Under programs older than 1.2.0 it fails with `E_VERSION` at execution.
- **Widening corrections (1.2.0 programs).** A correction that would make a
  fact readable by principals its live version does not admit fails with
  `E_READERS_WIDENED` unless it sets `widen_readers: true`. The comparison is
  of the `readers` and `reader_groups` lists, never of group membership.
- **Declared derivations (1.2.0 programs).** A fact citing restricted
  provenance `evidence` that the writer can see must carry it as an influence
  carrier or be readable only by a subset of its readers and groups, else
  `E_DERIVED_READERS`.
- **Hidden live versions (every version).** For `correct`, `retract` and
  `expected`, a live version the writer cannot read is absent, so the write
  fails as for a missing fact. An `assert` of its ID fails with `E_CONFLICT`
  "fact ID is not available", whatever its kind or content. In a registry
  store, references to records the writer cannot read fail as references to
  missing ones. In a store without a registry, a writer can also act on a
  version it recorded itself.
- **Versions.** `weave_contract::VERSION` and the contract crate are 1.2.0.
  The table accepts 0.1.0 through 0.24.0, 1.0.0, 1.1.0 and 1.2.0. 1.3.0 and
  2.0.0 fail with `E_VERSION` before structural decoding.

Nothing stored changes, so the store stays at 32. Results report the
producing engine's version, 1.2.0.

## Error codes for clients

| Code | Message | When |
|---|---|---|
| `E_READERS_WIDENED` | a correction may not widen readers without widen_readers | 1.2.0 correction that admits more readers |
| `E_DERIVED_READERS` | a derived fact's readers are broader than an input it cites | 1.2.0 fact broader than restricted evidence it cites |
| `E_CONFLICT` | fact ID is not available | `assert` of an ID a live fact the writer cannot read holds |
| `E_CONFLICT` | corrected fact is not live / retracted fact is not live | the fact is missing, or the writer cannot read it |
| `E_DEPENDENT_FACT` | a retracted fact still has live dependent facts | a live dependent remains, visible or not |

## Fixtures

- `negative/readers-widened.json`: a 1.2.0 correction that drops the readers
  of a restricted fact fails with `E_READERS_WIDENED`.
- `positive/readers-widened-explicitly.json`: the same correction with
  `widen_readers: true` commits.
- `negative/widen-readers-version-gated.json`: the flag under 1.1.0 fails
  with `E_VERSION`.
- `negative/derived-readers-broader.json`: a public fact citing a restricted
  node as evidence fails with `E_DERIVED_READERS`.
- `negative/hidden-fact-id-unavailable.json`: asserting the ID of a fact the
  writer cannot read fails with `E_CONFLICT`, even with identical content.
- `negative/unknown-version.json` now uses 1.3.0, because 1.2.0 is real.
- The manifest protocol is 1.2.0.

## For consumers

The language vendors `crates/weave-contract/` byte for byte. The 1.2.0
vendor update is the list of changed files in the B2 part 2 entry of the
[CHANGELOG](../../../CHANGELOG.md), including the B2 part 1 changes to
`src/rules.rs` and `src/carrier_algebra.rs`. A compiler that emits 1.2.0
corrections must copy a fact's `readers` and `reader_groups`, or set
`widen_readers` when it means to publish.
