# Protocol 0.19.0: alternative carriers and temporal graph values

Unpublished implementation stage. Protocol0.19, SQLite marker18 and capsule0.4 form
one compatibility boundary. The native and portable acceptance evidence must be
complete before pairing/publication; enum decoding alone is not support.

The existing flat restriction fields remain AND gates. Nonempty `derivations`
lists on GraphInfluence, Node and MetadataAttachment add OR-of-AND alternatives.
Derivation.snapshot_premises contains exact whole-snapshot restrictions within one
branch; input_snapshots remains descriptive. Absent/empty new fields are omitted,
so historical stored/signed bytes and identifiers are not rewritten.

New carriers bound raw groups at128 and aggregate gate references at1000, before
deduplication. Existing edge/assertion groups without new snapshot premises retain
their historical validation profile. Shared traversal work, active recursion,
materialization bytes and read limits apply. Failed alternatives cannot reset a
budget; an unavailable nonempty list is never converted into an unrestricted empty
carrier. Whole-snapshot services reject redacted alternatives conservatively.

`Window { input, window }` clips half-open validity intervals into derived occurrence
records with distinct local identities when clipping changes the input. A window
that changes no edge or attachment interval retains record identities after the
same context/proof validation and preserves whole-value carriers, so repeated
application is idempotent. `Sequence { left, right, window, relation,
match_on }` supports entity-space endpoint matching with Before, Meets, Overlaps and
Within. Sequence matches positive occurrences; Window also preserves negative evidence. Relations compare the original intervals before clipping. Output records
retain each event's individual interval; no hull is asserted as a simultaneous fact.
Protected derivation parameters describe the match. Original real metadata remains
navigable on current returned wrapper IDs; original local IDs are not generic aliases.
Neither operator chooses an authority clock, remote observer or recorded-time range.

Persisted copies retain whole-value/path restrictions in their generated records,
including empty metadata followed by scalar construction. Readers and explanation
indexes never replace proof checks. Newly materialized grouped original legacy edges
and attachments include their own exact record premise in each alternative, preserving
the original record's readers alongside its derivation premises. This strengthens new
materializations/copies. Migration preserves historical outputs and cannot reconstruct
authority absent from an old derived record; no retroactive rewrite or privacy recall
is claimed.

Old Programs reject typed new fields/operators before any command writes; similarly
named literal JSON keys stay inert. Genuine older capsules remain accepted, while
capsule0.4 is required for actual new carriers. Historical0.16/.17/.18 view templates,
0.18 handler registrations/preparations and signed export receipts retain their exact
identities and bytes, subject to current retained authorization and replay semantics.
Historical duplicate receipts are not freshly evaluated temporal queries.

Full temporal operators are not an incremental-maintenance claim. Query/Filter
membership maintenance falls back atomically when new proof carriers/operators are
ineligible. Source lowering, native persistence, current-policy service paths and
populated store17→18 recovery are joint acceptance requirements. See the
[integration matrix](../../proposals/TEMPORAL_CARRIER_INTEGRATION.md).
