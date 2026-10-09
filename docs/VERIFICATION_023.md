# Native lifecycle verification

Requirements R12/R13/R36/R38; gates E05/E06/E14. Exact published revisions and
clean-archive evidence are recorded separately; a passing native subset is not
full original Weave completion.

`root_delivery_cancellation` observes actual pending leases, graph heads,
immutable preparations, checkpoints and receipt rows. It tests stale native
and compiled output CAS, source-policy expiry, renewed and expired leases,
foreign owner denial, exact retry/body conflict, no output execution, stateful
rebuild fencing, modern marker downgrade rejection, actual unknown-effect
exclusion and valid-JSON corruption before collection.

`root_projection_migration` checks state/checkpoint equality through upgrade/reopen
and a subsequent real completion. Rollback restores the actual recorded prior
artifact/state/checkpoint into a fresh namespace; duplicates after newer work
never rewind it. Schema, scope, effects, pending work, missing rebuild and expired
rollback epoch fail closed. Unrelated private events advance only the actual
private coordinate, leaving public transfer inputs unchanged. A panic before
commit leaves the source, new namespace, state and audit receipt unchanged.

`scripts/check_lifecycle_recovery.py` runs24 native processes and8 controlled
deaths. The old host first creates a genuine store22 with an erased orphan,
retention tombstone, actual opaque projection state, completion receipt and
private checkpoint. Schema migration death/restart preserves every original
schema and row; the old runtime subsequently refuses marker23. The existing
completion retry returns its exact receipt without rewinding rebuilt state.
Upgrade and rollback each exercise both process-death boundaries and exact
duplicates. A real stale compiled preparation survives cancellation; heads,
events and original preparation bytes remain unchanged and the canceled
occurrence cannot prepare again. Raw responses, actual databases and SQL snapshots
can be preserved with `--evidence-dir`.

The entire workspace, strict all-target/all-feature lint and format remain
required. CI retains older populated migrations and exact old SDK journals,
adds populated store22 snapshot/handler/effect/accepted-history upgrade and runs
the four lifecycle process-death pairs on Ubuntu, macOS and Windows. Portable
semantic parity and browser image persistence remain separate regression
profiles; they do not attest source/browser/mobile lifecycle applications.

Source bindings, compiled and effectful state migration, receipt expiry, complete
actor lifecycle, causal feedback controls, resource isolation, event taxonomy/lag,
full applications and all remaining original assurance gates remain mandatory.
