# Weave contract 1.3.0: review provenance

Protocol 1.3.0 is an additive minor version (B5,
[#12](https://github.com/weave-graph/weave-engine/issues/12),
[REVIEW.md](../../REVIEW.md)). It adds one optional engine-recorded field to
`FactProvenance`. Semantics are in core v1 §6,
[`provenance.review`](../core-v1.md#6-provenance).

## What changes

- **`FactProvenance.review`**: `ReviewProvenance { proposal_id, decision_id,
  reviewers, proposed_at_ms, decided_at_ms }`, present on facts that a
  reviewed graph published through an accepted proposal and omitted
  otherwise. `writer` and `on_behalf_of` remain the proposer's. `reviewers`
  are 1 to 32 sorted, distinct principal IDs that never include the writer,
  and `decided_at_ms` equals `recorded_at_ms`. A record that breaks these
  rules fails validation with `E_PROVENANCE`.
- **Results.** `fact_provenance` of 1.3.0 programs carries `review`;
  results of older programs never do, whatever the store holds.
- **Clients cannot send it.** `ClaimedProvenance` has no `review` field and
  rejects one with `E_UNKNOWN_FIELD`.
- **Versions.** `weave_contract::VERSION` and the contract crate are 1.3.0.
  The table accepts 0.1.0 through 0.24.0 and 1.0.0 through 1.3.0. 1.4.0 and
  2.0.0 fail with `E_VERSION` before structural decoding.

Review itself (policies, proposals, votes) is a trusted engine API, not a
`Program` command, so programs gain no command. The store moves to 33 for
the review tables ([REVIEW.md](../../REVIEW.md)). Results report the
producing engine's version, 1.3.0.

## Fixtures

- `positive/fact-provenance-review.json`: an agent's fact accepted by two
  reviewers.
- `negative/fact-provenance-self-review.json`: a writer listed among its
  own reviewers fails with `E_PROVENANCE`.
- `negative/claimed-provenance-forged-review.json`: a client-supplied
  `review` fails with `E_UNKNOWN_FIELD`.
- `negative/unknown-version.json` now uses 1.4.0, because 1.3.0 is real.
- The manifest protocol is 1.3.0.

## For consumers

The language vendors `crates/weave-contract/` byte for byte. The 1.3.0
vendor update is the list of files in the B5 part 1 entry of the
[CHANGELOG](../../../CHANGELOG.md). A compiler needs no change to emit 1.3.0
programs; a consumer that decodes engine results with `deny_unknown_fields`
needs the new field.
