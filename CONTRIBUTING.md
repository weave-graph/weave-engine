# Contributing

Use stable Rust. During development, run the focused crate or client checks for
the behavior being changed. At integration, run `cargo fmt --all -- --check`,
`cargo test --workspace --all-features --locked` and
`cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`.
Native science changes also run the installed Python client tests and
`scripts/check_science.py`; see [validation](docs/SCIENCE_VALIDATION.md). Include
requirement and workflow gate IDs with behavior tests and evidence.

Normal CI validates current native behavior on Linux, macOS and Windows.
Historical migration/compiler checks run when those implementation paths change;
the full prior-version suite remains available through the Historical
compatibility workflow. Browser persistence is manual while browser/mobile
application delivery is deferred. Do not rebuild every archived runtime for a
science-only client change.

Contract changes affect the companion language repository and require coordinated review, fixtures and vendor manifest updates. Preserve backwards compatibility explicitly or change the contract version. Do not claim complete platform, security or white-paper support from a successful local build.

Small focused pull requests are welcome. Read `AGENTS.md` and `docs/STATUS.md` for the current boundaries.
