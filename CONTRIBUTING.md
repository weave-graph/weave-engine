# Contributing

Use stable Rust. Run `cargo fmt --all -- --check`, `cargo test --workspace --locked` and `cargo clippy --workspace --all-targets --locked -- -D warnings` before proposing a change. Include requirement and workflow gate IDs with behavior tests and evidence.

Contract changes affect the companion language repository and require coordinated review, fixtures and vendor manifest updates. Preserve backwards compatibility explicitly or change the contract version. Do not claim complete platform, security or white-paper support from a successful local build.

Small focused pull requests are welcome. Read `AGENTS.md` and `docs/STATUS.md` for the current boundaries.
