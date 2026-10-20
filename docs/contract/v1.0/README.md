# Weave contract 1.0.0: core v1, frozen

Protocol 1.0.0 is the Phase A release of the contract (A6,
[#16](https://github.com/weave-graph/weave-engine/issues/16)). It freezes
[core contract v1](../core-v1.md) as it stood in 0.24.0: the base graph
model, per-fact provenance, node valid time and fact-level writes (0.22),
k-hop `expand`, `as_of` and `changes` (0.23), and embedding facts with exact
`nearest` and `similar_candidates` (0.24).

## What changes

- **No wire feature is added or removed.** A 1.0.0 program has exactly the
  feature set of 0.24.0. Every request type, field, default, error code and
  canonical encoding is unchanged, so stored data, content hashes and
  revision IDs are unchanged.
- **Version identifiers.** `weave_contract::VERSION` and the contract crate
  are 1.0.0. Results report the producing engine's version, 1.0.0. New
  handler and view artifacts record protocol 1.0.0, and artifacts recorded
  with older protocols stay accepted under the same rules as before.
- **Accepted versions.** The engine accepts 0.1.0 through 0.24.0 and 1.0.0,
  and runs each with the semantics it was published with. 1.1.0 and 2.0.0
  fail with `E_VERSION`, before any structural decoding.
- **One version table.** `weave_contract::versions` holds the ordered list
  `SUPPORTED_VERSIONS` and the helpers `version_at_least(version,
  introduced)`, `version_rank`, `is_supported_version` and `is_current`.
  Every feature gate in the engine and the contract names the version that
  introduced its feature. A future minor version is added to the table, and
  no gate changes. `tests/version_gates.rs` checks that every gate accepts
  exactly the versions it accepted before the table, for every supported
  version, and runs each gated feature under every version.

## Compatibility from 1.0.0 on

As defined in [core v1 §11](../core-v1.md#11-compatibility-policy):

- MINOR (1.1.0) adds only what §11.3 allows: optional fields omitted when
  absent, new variants rejected under older versions with `E_VERSION`, new
  result fields and new error codes.
- MAJOR (2.0.0) is required for anything in §11.4, with an ADR and a
  migration.
- Saved plans keep their declared version and its semantics.

## Fixtures

- `positive/protocol-1-0-0.json` runs a 1.0.0 program that writes facts and
  an embedding space, then evaluates `nearest` and `expand`, with golden
  engine outcomes.
- `positive/legacy-version-accepted.json` still accepts an old version.
- `negative/unknown-version.json` (1.1.0) and
  `negative/unknown-version-checked-first.json` (2.0.0) fail with
  `E_VERSION`.
- The manifest protocol is 1.0.0.

## For consumers

The language repository vendors `crates/weave-contract/` byte for byte. Its
1.0.0 vendor update is the list of changed files in the 1.0.0 release notes
([CHANGELOG](../../../CHANGELOG.md)).
