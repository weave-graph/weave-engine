# Explicit compiled snapshot reconstruction

Mandatory engine3.4/4.1/4.4 requirements; R12/R13/R36, E05/E06/E14.
Compatible source-compiled version transfer starts from native24. Expired replay
still needs actual reconstruction, rather than relabeling a checkpoint.

The registered pure stateless recipe must execute on the kernel's current whole
authorized input/metadata closure. An explicit owner request binds the real
artifact, current epoch, current input revisions and expected output head. The
adapter must be paused/drained without pending or unresolved effects. Output,
immutable reconstruction receipt, replay-ready state and private checkpoint
commit together. The checkpoint skips only history superseded by this explicit
whole snapshot reconstruction. Public bindings exclude private event coordinates.

Historical retries validate current authority and actual immutable registration,
retained inputs and earlier output; they never overwrite a newer materialization
or rewind the current checkpoint. Later actual event completion and nonmatching
private scans advance their paired private replay state atomically. Compatible
version transfer keeps the actual readiness proof; incompatible or expired epochs
need a new explicit reconstruction. Cancellation of state-bound work fences
delivery until rebuilding again.

Native25 adds separately fenced immutable receipt and current replay-state
tables. Their actual artifact/epoch/input/output/receipt/checkpoint bindings are
checked before collection. Genuine native24 initialization preserves every
prior schema and row. Opaque state, effectful actors and external retained host
journals remain separate mandatory profiles, never accepted as stateless recipes.

The bounded implementation and source controller cover real compiler recipes, current input/output CAS, authority
and budget failures, output equivalence, receipt/private-scan noninterference,
later event completion/version transfer, cancellation fencing, exact duplicates,
populated native24 migration and pre/postcommit process deaths. The source controller passes66 processes and8 controlled deaths. Full frozen
archive and hosted acceptance remain pending; no full-project claim is made.
