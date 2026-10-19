# Contributing

Start with [AGENTS.md](AGENTS.md) (architecture, commands, conventions and the
definition of done), [VISION.md](VISION.md) and the [phased roadmap](docs/ROADMAP.md).
Work is organized as Phase A, B and C epics on GitHub; reference the epic issue
in your PR. The native data-science profile [DS01–DS09](docs/NATIVE_SCIENCE_PLAN.md)
is the currently supported release.

## Set up and make a focused change

Use stable Rust with Cargo, rustfmt and Clippy, and Python 3.10 or newer. The
[README quickstart](README.md#run-your-first-experiment) builds the science binary
and installs the Python package into a virtual environment. Run commands from
the repository root. Cargo may need your platform's native compiler/linker tools.

During development, run checks for the behavior you changed. For example:

```sh
cargo test --locked -p weave-science
WEAVE_SCIENCE_BINARY="$PWD/target/debug/weave-science" \
  .venv/bin/python -m unittest discover -s python/tests -v
```

Build the binary first with `cargo build --locked -p weave-science` if it is not
present. On Windows PowerShell, set `$env:WEAVE_SCIENCE_BINARY` to the absolute
`target/debug/weave-science.exe` path and use `.venv\Scripts\python.exe` for
Python commands. Native integration tests skip when the binary environment
variable is absent; a skipped result is not native integration evidence.

**Docs ship with code.** A PR that changes behavior updates the affected docs,
`docs/STATUS.md` and, if scope changes, `docs/ROADMAP.md` in the same PR. A PR
without doc changes must state `[no-docs]` and a reason; CI enforces this.

Documentation changes should keep examples runnable against the supported
profile, verify local links and distinguish measured evidence from proposals.
Use a fresh temporary database or output directory for examples with mutations.
Preserve the unchanged source papers and their hash manifest.

## Validate integration

For a native implementation change, run the complete current suite once at
integration, after focused checks pass:

```sh
cargo fmt --all -- --check
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

Science changes also build the binary, run the installed Python client tests
with its explicit path, and run `scripts/check_science.py`. The
[validation guide](docs/SCIENCE_VALIDATION.md) gives commands, independent oracles
and benchmark methods. Benchmarks describe their actual binary, host and workload;
do not infer capacity from rejected inputs or relabel skipped tests as passes.

Normal CI checks the current native profile on Linux, macOS and Windows.
Historical migration/compiler workflows run for changes to their implementation
paths and remain available by manual dispatch. Browser persistence is manual while
browser/mobile delivery is deferred. Rebuilding every archived runtime is not a
prerequisite for a documentation or science-only client edit.

## Contract and review boundaries

Contract changes affect the companion language repository. Coordinate review,
golden fixtures and vendor-manifest updates. Follow the compatibility policy in
[core contract v1](docs/contract/core-v1.md#11-compatibility-policy): preserve
compatibility explicitly or advance the contract version, and never silently
reinterpret saved plans.
Contract, store, capsule and host-request versions identify different boundaries.

Keep pull requests focused. Explain the concrete behavior change and include
relevant validation and remaining limits. A successful local build does not
establish complete platform, security or white-paper acceptance. Parent
orchestration owns repository publication and final release actions.
