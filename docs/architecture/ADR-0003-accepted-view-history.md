# ADR 0003: replica-local governed acceptance history

The original papers distinguish valid time, a named replica's recording time,
revision ancestry and a governed view's acceptance time. Copying a graph or
receiving a capsule does not create an accepted-view occurrence.

Native `query_accepted_history_for` selects an explicit local observer and view
using an exact genuine decision or a local acceptance-time criterion. It returns
its protected occurrence, source pin, branch and acceptance time, together with
the ordinary authority-carrying accepted query result. The observing identity
comes from the actual runtime. Claimed metadata and copied remote governance
graphs cannot install an occurrence or local policy authority.

Date selection follows the current decision's actual predecessor chain rather
than filtering SQL timestamps. Each link must agree with the original proposal,
its signed approvals/quorum, historical policy validity at acceptance, the exact
receipt and the protected occurrence body. Equal milliseconds follow ancestry.
Missing, corrupt, cyclic, reparented and time-regressed links cannot be skipped.
New acceptances reject a clock regression relative to the persistent prior
acceptance; no failed decision, receipt or event is published.

Historical approval validity establishes the past occurrence. Current policy,
identity and whole source/dependency authority still govern every returned value.
An expired historical approval does not expire an otherwise currently authorized
read, and a historical approval never replaces current authority. Foreign,
unknown and unavailable selections fail closed. Ranges return the authorized
state at the start plus all occurrences in the half-open interval in ancestry
order, including occurrences exactly at the start. They never omit an inaccessible
version to return an apparently complete range.

One SQLite snapshot and trusted clock sample span nested selection and reads.
Walks/ranges have a 1,000-entry bound, share the 128 MiB / 4,096-read operation
budget and bound the aggregate serialized range to 32 MiB. Limits are serialized
work bounds, not a process RSS or execution-isolation claim. Invalid future cuts,
open ranges and invalid limits are rejected. No global sequence, actor roster,
policy member list or parent cursor is returned.

This native extension keeps protocol0.20, store20 and capsule0.4. It adds no schema,
reseals no historical row and invents no old acceptance time. Canonical/source
accepted-time and range selectors remain separate mandatory work. Retention, GC,
expired replay, global coordination and hardware antirollback are not implemented
by this profile. The original requirement gates remain open.
