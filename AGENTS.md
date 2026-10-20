# Working in weave-engine

Instructions for AI agents and human contributors. `CLAUDE.md` imports this file.

## Read first

1. [VISION.md](VISION.md): the three goals (temporal graph DB for hardware,
   permissioned agent fact store, time-aware hybrid RAG) and the non-goals.
2. [docs/ROADMAP.md](docs/ROADMAP.md): Phases A, B and C, the current epic and
   its dependencies. Work on the earliest phase's ready epics unless the owner
   says otherwise.
3. [docs/STATUS.md](docs/STATUS.md): what is supported today.
4. The GitHub issue for the epic you are working on. Its acceptance criteria are
   the definition of the work.

`docs/IMPLEMENTATION_PLAN.md`, `docs/WORKFLOW.md` and `docs/RECONCILIATION.md`
record the original white-paper plan. They are background, not the work queue.

## Architecture overview

Rust workspace. `weave-engine` is the semantic and storage authority; every
other interface calls into it. There is no second implementation.

| Crate | Role | Phase relevance |
|---|---|---|
| `crates/weave-contract` | Canonical versioned types: IDs, nodes, edges, assertions, metadata, intervals, plans (`GraphExpression`), commands, results. Vendored by weave-language | Core (A1) |
| `crates/weave-engine` | Runtime: SQLite store (`lib.rs`, store version `STORAGE_VERSION`), queries and algebra, recorded history, authorization and influence, metadata, rules, views, governance, dispatch, capsules and more | Core (A2–A4) plus peripheral subsystems |
| `crates/weave-science` | `weave-science` JSON CLI: `import`, `execute`, `analyze` (degree, components, paths, PageRank, exact kNN) | Current user-facing API |
| `crates/weave-hardware` | Hardware reference model, `SeriesRef`, CSV/JSON importers, seeded fleet generator and CLI | A5 |
| `crates/weave-mcp` | `weave-mcp` read-only stdio MCP server over one store, as a host-fixed reader; a thin adapter over `weave-science` | A7 (B4 builds on it) |
| `crates/weave-service` | `weave-service` authenticated multi-tenant HTTP/JSON server over TLS: one store per tenant, API tokens, audit log; a thin adapter over the engine | B3 |
| `python/` | `weave_science` Python client (subprocess to the CLI), tables, experiments, replay | Current user-facing API |
| `crates/weave-policy` | ed25519 capability chains and signed admission | B1/B3 building block |
| `crates/weave-spaces` | Typed geometry and embedding spaces, descriptors, pairwise operations | A4 building block |
| `crates/weave-cluster` | Topology clustering (experimental) | Deferred |
| `crates/weave-host`, `crates/weave-native`, `hosts/swift` | Trusted Rust facade, C ABI, Swift wrapper | Embedding interfaces |
| `crates/weave-browser-host`, `examples/browser-*` | Experimental browser transport | Deferred |
| `crates/weave-conformance` | Fixed portable semantic vectors (native versus WASM parity) | Regression |
| `fuzz/` | cargo-fuzz targets | Hardening |
| `scripts/` | Independent acceptance, migration and recovery checks (Python) | Evidence |

Inside `crates/weave-engine/src`:

- **Core graph, store and query:** `lib.rs`, `snapshot.rs`, `assertions.rs`,
  `metadata.rs`, `recorded_history.rs`, `operation_clock.rs`, `typed*.rs`,
  `whole_visibility.rs`, `authorization.rs`, `influence.rs`, `selection.rs`,
  `views.rs`, `retention.rs`, `geometry.rs`.
- **Phase B building blocks:** `admission.rs`, `governance*.rs`,
  `identity_acceptance.rs`.
- **Peripheral, maintained but not extended without an epic:** dispatch, actors,
  compiled handlers, effects, capsules, mounts, image host.

Current limits that Phase A changes (see the [audit](docs/AUDIT_2026-10-17.md)):
full-snapshot commits, a 100k-object per-graph cap, no k-hop operator and no
plan-level kNN.

## Build and test

Requires stable Rust (CI uses `dtolnay/rust-toolchain@stable`), rustfmt and
Clippy, Python 3.10+, and a C toolchain for bundled SQLite. Run from the
repository root.

```sh
# Focused development checks (example)
cargo test --locked -p weave-science
cargo test --locked -p weave-engine --test root_temporal

# Full native suite: required before opening a PR that touches Rust
cargo fmt --all -- --check
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

# Python client and independent science acceptance
cargo build --locked -p weave-science
python3 -m venv .venv && .venv/bin/python -m pip install ./python
WEAVE_SCIENCE_BINARY="$PWD/target/debug/weave-science" \
  .venv/bin/python -m unittest discover -s python/tests -v
.venv/bin/python scripts/check_science.py --engine target/debug/weave-science
```

The full workspace suite has about 615 tests and takes several minutes on a cold
build. Python integration tests **skip** without `WEAVE_SCIENCE_BINARY`; a
skipped test is not evidence.

**CI** (`.github/workflows/`):

- `ci.yml` runs the suite above on Linux, macOS and Windows for every PR.
- `compatibility.yml` (store migrations), `integration.yml` (language
  compiler) and `fuzz.yml` run when their paths change.
- `browser.yml` is manual only.
- `docs-check.yml` enforces the docs rule below.

If linking fails with `cc: unknown command`, a non-compiler `cc` is first on
`PATH`. Use `CC=gcc CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=gcc`.

## Conventions

- **Semantics:**
  - Keep entity, manifestation (node), edge, assertion, graph, revision, branch
    and replica identities distinct.
  - Facts are immutable; corrections are new facts.
  - Valid time and recorded time are separate axes. Clients never supply
    authoritative recorded time.
  - Similarity is never identity. Vector neighbors are candidates with
    provenance.
  - Unknown, partial, denied and empty are distinguishable; never return an
    empty "complete" answer for unavailable input.
- **Permissions:** they apply to everything derived: results, counts, ranks,
  explanations, clusters, embeddings, indexes and error messages. Any new
  operator needs a paired test showing that hidden input does not change visible
  output.
- **Contract:**
  - `weave-contract` is shared with weave-language.
  - Changes need a version decision, golden fixtures, and a coordinated vendor
    update in the language repo (`vendor/manifest.json`).
  - Never silently reinterpret saved plans or stored data.
- **Storage:** bump `STORAGE_VERSION` for any schema change, with a
  crash-tested migration from the previous version and an old-binary refusal
  test.
- **Style:**
  - Follow the surrounding code.
  - Strict Clippy, with no warnings allowed.
  - Bound all inputs (sizes, depth, work) with explicit errors (`E_BUDGET`,
    `E_CONFLICT` and so on).
  - Test observable behavior, not private structure.
- **Decisions:** architectural choices named in the roadmap need an ADR in
  `docs/architecture/` (the next number is ADR-0004) before the implementing PR
  merges.
- **Do not commit** credentials, personal paths, build outputs or private
  conversation material. Keep `docs/source/` byte-identical (hash manifest).

## Documentation rule: docs ship with code

**Every PR that changes behavior updates the affected docs in the same PR.**
This includes:

- the user guide or reference for the feature;
- `docs/STATUS.md` when supported behavior or limits change;
- `docs/ROADMAP.md` when an epic's scope or status changes;
- the README when the quickstart or what Weave does changes.

A PR with no doc changes must say `[no-docs]` in its description and give a
reason, for example "test-only" or "internal refactor". `docs-check.yml` fails
PRs that change code without docs or the marker.

Write docs for the current state. Put dated evidence in `docs/benchmarks/`,
`docs/measurements/` or `VERIFICATION_*.md`, not in reference docs or issue
bodies. Mark proposals as proposals, and add "Superseded by" when they ship.

## Definition of done

A change is done when:

1. Each acceptance criterion it claims has a linked test or reproducible
   measurement.
2. The full native suite (fmt, test, Clippy) passes, and the Python and science
   checks pass if those paths changed.
3. Docs are updated per the rule above.
4. Limits and non-guarantees are written down.
5. Contract or storage changes carry fixtures, migration tests and the
   language vendor update.

An **epic** closes only when every criterion in its issue is met. A useful
subset is progress, not completion; note it as a comment on the issue, not in
the issue body.

## Process

- Branch from `main`. One focused PR per change. Reference the epic issue. Do
  not merge your own PR without owner approval.
- Do not close, retitle or create GitHub issues unless the owner asked for it.
- Report what you ran and its actual result. Never relabel skipped or unrun
  checks as passing.
