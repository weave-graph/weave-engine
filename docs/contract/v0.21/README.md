# Weave contract 0.21.0: accepted history and history ranges

`AcceptedHistory` selects a genuine local governed view through `LocalTime` or an
explicit observer/decision. It returns an ordinary graph value with descriptive
`accepted_observations`: the actual local observer, view/decision, acceptance time,
protected occurrence, exact source and source branch. Those descriptions never
install governance authority. Runtime reads verify the signed original decision
and protected occurrence, then apply the current policy and whole input authority.

Empty outputs retain their observations and both exact input pins. Union, Diff,
projection, temporal operations and explanations preserve all selections and
reject conflicting descriptions. Cached reuse revalidates the local registry,
exact descriptions and current source/policy authority. Old values omit the new
empty field, preserving historical serialized bytes.

`RecordedRange` and `AcceptedRange` are named read commands. They return
`HistoryRanged` with a typed `HistoryRangeValue`: the axis, finite half-open interval,
authorized state at the start and every actual occurrence in `[start,end)` in
ancestry order. An occurrence exactly at start is also in `changes`; equal times
retain each genuine transition. Each state is a normal graph result carrying its
selected observation. The optional fact-time filter applies independently to every
state, so empty values still retain the history selection. Collections are not
program-local graph bindings, and no hidden or unavailable version is omitted to
make a range appear complete.

Selectors require bounded identifiers and nonnegative observed times. Intervals
are finite and limits are1–1,000. One SQLite snapshot and trusted operation clock
span selection and evaluation. Walks share the128 MiB/4,096-read budget; serialized
range values and total program results remain bounded to32 MiB. Limit exhaustion
fails closed. These serialized work bounds do not establish process isolation.

Programs carrying the new expressions or commands require0.21 before any earlier
write can execute. Existing0.20 recorded queries, sealed view/handler templates,
retained recipes and signed export bindings remain supported with their original
identities. Capsule0.4 is unchanged. Store21 prevents older runtimes from dropping
acceptance manifests in persisted query results. Its transactional marker upgrade
preserves every original store20 table, row, observation and receipt without
adding baseline history or resealing decisions. Older baseline migrations retain
their original rules.

Accepted-history reads are explicitly bound source values. They cannot be hidden
inside pure handlers/functions, and the current sealed-view registration profile
continues to support live/recorded Query/Filter recipes. Retention/GC, expired replay,
reactor/lifecycle upgrades, broader incremental operators, network transport, full
portable application services and the original assurance gates remain required.
This extension advances R04/E02 and R08/E03; it does not complete either paper.
