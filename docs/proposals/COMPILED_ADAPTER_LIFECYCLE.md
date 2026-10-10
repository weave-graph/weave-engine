# Compiled stateless adapter version transfer

Required source: engine4.1–4.4; R12/R13/R36/R38, E05/E06/E14.
Store23 native actual-state migration is the accepted starting profile. This is
implemented as the bounded native24 profile; hosted acceptance remains pending.

A sealed pure stateless handler can transfer its actual checkpoint into a new
immutable namespace after the old version is paused/drained with no pending
occurrence or unresolved effect. The destination artifact must be a real validated
compiler template, with the same compatible event protocol, input/metadata scope,
output slot and destination. New artifact/configuration identity and effective
owner/output authority are explicit. Installation, private checkpoint transfer,
old-version retirement and immutable audit receipt commit together. Existing
outputs, preparations and completed receipts remain byte-for-byte historical.

The transfer captures actual explicit primary input revision and current authorized metadata closure and actual
registered artifact identity, not a caller's claim about either. Public bindings
exclude private event coordinates. A later duplicate returns its original receipt
without resetting a checkpoint advanced by new work. Rollback restores the prior
recorded artifact/configuration/checkpoint pair into another fresh namespace.
Native opaque state and external retained host journals cannot be treated as
stateless compiled state. Effects, incompatible schemas or scopes, pending work
and expired replay fail closed until their explicit reconstruction profile exists.

Store24 fences older readers from ignoring the new registry. The root tracer
validates immutable compiled migration records and keeps genuine input pins
and prior registrations. Actual populated store23 migration preserves every
retention/lifecycle row and add no fictitious version history.

Acceptance uses actual source compiler output for two handler versions, real
completion after transfer and rollback, stale/pending/CAS cleanup, private scan
noninterference, exact historical duplicates, missing/corrupt bindings, and
pre/postcommit process death. The full old-store migration and source/peer/effect
profiles remain regressions. Compiled reconstruction, stateful/effectful actors,
source/portable lifecycle controls and all remaining original gates stay required.
