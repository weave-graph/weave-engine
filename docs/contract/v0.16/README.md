# Contract 0.16.0: exact governed reads and host view artifacts

This candidate implements the native protocol 0.16.0 boundary. Source pairing and independent release verification are recorded separately; a passing interface test is not full-system acceptance.

`GraphExpression::AcceptedGraph { selection: AcceptedGraphSelection { view_id, decision_id } }` requires an exact accepted occurrence. Historical occurrence identity never bypasses current policy, source authorization or the trusted operation clock.

`GraphExpression::CurrentView { selection: CurrentViewSelection { view_id, definition_digest, time } }` requires a matching compiled definition and a current authorized materialization. `ViewReadTime` uses a `kind` tag: `fixed` or `tick` with integer `valid_at`. It never refreshes or installs a view. Definition/time/authority checks share the operation snapshot. Native view registrations recursively reject both new reads until their dynamic dependencies can be tracked.

`CompiledViewTemplate` has required fields `format`, `protocol`, `name`, `revision`, `expression`, `clock`, `source_revisions`, `definition_digest`. Format is `weave-view-registration/1`; protocol is `0.16.0`. `ViewClock` preserves the existing native unit-string encoding `fixed`/`tick`. Artifacts deny unknown fields and contain no host authority, enrollment, scheduler instructions or instance ID.

The first recipe profile accepts one live Query (`revision: null`, `include_metadata: false`) beneath at most 32 Filter layers. No captured values, metadata expansion, accepted/current reads or function-body capture is supported. Identifiers are bounded to 512 UTF-8 bytes; each serialized artifact is bounded to 1 MiB and its manifest to 1000 entries.

The portable `view_registration::seal_template` adds a normalized compiled-recipe source revision, canonicalizes source manifests with conflict rejection and calculates the definition digest. The own source label is `weave:view-template:` plus a domain-separated framed kind/name SHA-256. Its content digest includes protocol/name/revision/expression/clock. Final definition identity includes format, protocol, name, revision, expression, clock and complete canonical source manifest, excluding the host instance ID and digest field. Literal strings, operator order and fact times are preserved. `validate_template` verifies the whole sealed artifact. These hashes are attribution and identity, never authentication.

Native APIs are `register_compiled_view(instance_id, &template, tick, host)` and `read_current_view(&selection, host)`, returning the existing `ViewSnapshot`. Legacy source-less registrations cannot impersonate a compiled definition. All evaluation paths retain source manifests before result/cache/processed identity is computed. Source compilation must return separate host artifacts explicitly; existing APIs that return only a Program must reject sources containing templates rather than discard them.

Both new expression variants require protocol 0.16 recursively before any Program writes. Existing 0.15 data encodings and omitted defaults remain unchanged. No remote arbitrary-program admission, accepted-head following, automatic registration/refresh/subscription or full incremental reactor language is added.

## Native persistence and replay

SQLite marker14 adds a nullable compiled-source digest on `live_views` and a private `view_sources` artifact registry. Both copies bind the complete validated artifact to the immutable owner/instance definition. Registration stores result, definition, source identity and initial transition atomically. Legacy rows remain source-less; missing or inconsistent compiled bindings fail closed. Older marker13 binaries refuse this store before initialization. Existing graph rows/revision bytes are unchanged.

Sources enter initial/full/fallback results and incremental rematerializations before comparison and storage. Kernel and scheduler definition fingerprints include the compiled artifact identity. A Program source-label conflict rolls back every tentative command in that operation. Source identities are descriptive; unchanged manifests confer no continuing access.

Fresh `CurrentView` execution matches definition, clock mode, requested fact tick, pending tick and dependency heads under one SQL snapshot, then applies current authority. It does not refresh. A duplicate adapter handler completion returns its **historical receipt**, marked `duplicate`, after current authorization checks; it does not re-execute CurrentView or promise fresh heads/ticks. The existing signed facade admits bounded QueryPlan operations, not arbitrary expression Programs or registration artifacts.

The whole snapshot load and output repinning costs remain O(input)/O(output) for the selection profile. These operators add source access to bounded native services, not general incremental evaluation or a background reactive scheduler.

## Candidate verification

Engine verification passed 344 workspace tests/doctests with all features, strict all-target/all-feature lint, formatting and the portable contract WASM build check. This includes three independent native boundary tests and focused source binding corruption, clock, fallback/scheduler and exact accepted-expression tests. Actual preserved-binary marker13→14 and marker12→14 scripts passed process-death rollback, restart, old-reader refusal and idempotence. Compiler fixture acceptance and independent root review are separate release gates; these native counts do not claim that those gates have passed.
