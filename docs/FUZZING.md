# Bounded coverage-guided fuzz smoke tests

The independent `fuzz` workspace uses libFuzzer with AddressSanitizer. It does not add a nightly requirement to normal engine builds. Setup follows the [Rust Fuzz Book](https://rust-fuzz.github.io/book/cargo-fuzz/setup.html); the bounded CI runs follow its [CI guidance](https://rust-fuzz.github.io/book/cargo-fuzz/ci.html).

```sh
rustup toolchain install nightly-2026-09-19 --profile minimal
cargo install cargo-fuzz --version 0.13.2 --locked
cargo run --locked --manifest-path fuzz/Cargo.toml --example check_seeds
mkdir -p fuzz/corpus/program_atomic
cargo +nightly-2026-09-19 fuzz run program_atomic fuzz/corpus/program_atomic fuzz/seeds/program_atomic -- -max_total_time=30 -timeout=5 -rss_limit_mb=1024 -max_len=131072 -seed=190901
```

Replace `program_atomic` with `capsule_receive` or `policy_wire` for the other targets. Generated corpora, binaries and failure artifacts are ignored; curated seeds and their semantic seed check are committed. A finding must be minimized and turned into a regression before being considered resolved. Local runs leave raw logs under `fuzz/runlogs`; normalized published logs and measured counters are in [docs/fuzz](fuzz/2026-09-19-smoke.json). The manifest records the actual compiler, source base, seeds, sanitizer and resource limits. CI uses a larger 1536 MiB RSS limit on Linux and publishes artifacts on failure.

The three invariants are deliberately narrow:

- `program_atomic`: a deserializable failed program leaves no published events, and a fresh `fuzz/main` CAS(None) commit still works. This does not independently check every schema/identity registry, other graph heads, or durable process-death behavior.
- `capsule_receive`: importing a capsule never emits accepted graph events; repeating a successful import adds zero revisions. This target does not independently prove failed-import quarantine rollback or all acceptance/governance transitions.
- `policy_wire`: decoding and verification remain bounded and cannot mint trust when the host has no installed roots. Valid delegated chains, request replay and nonce transactions need the separate admission tests; these paths are not claimed to be covered by the empty-root fuzz host.

The first local runs at engine `d184eaa` processed 247,142 plan inputs, 488,311 capsule inputs and 3,318,373 policy inputs in 31 seconds per target, with no sanitizer crash or failed asserted invariant. A further plan run after explicit valid-seed verification is recorded separately. Reported peak RSS was 519, 529 and 671 MiB respectively; these are sanitizer/fuzzer process measurements, not production memory estimates. Full geometry execution and idempotent capsule import are exercised by `check_seeds`.

Many generated inputs are rejected while parsing. Input counts and libFuzzer coverage counters are not complete valid workflows, percentages of semantic coverage, security certification, or proof that no bug exists. These short seeded runs are reproducible smoke evidence for E14. Longer campaigns, additional structured/stateful targets, error-path persistence oracles, supported-platform campaigns and independent security review remain open.

The initial capsule host lacked storage authority, so its 488,311 inputs exercised parsing and rejection rather than successful import replay. Review corrected the target to grant storage only for graph `fuzz`. The authorized follow-up processed 529,194 inputs in 31 seconds (reported RSS 508 MiB), with no crash or failed invariant. The current checked-in target and CI include this correction.
