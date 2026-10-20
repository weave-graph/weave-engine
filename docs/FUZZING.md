# Bounded coverage-guided fuzz smoke tests

The independent `fuzz` workspace uses libFuzzer with AddressSanitizer. It does not add a nightly requirement to normal engine builds. Setup follows the [Rust Fuzz Book](https://rust-fuzz.github.io/book/cargo-fuzz/setup.html); the bounded CI runs follow its [CI guidance](https://rust-fuzz.github.io/book/cargo-fuzz/ci.html).

```sh
rustup toolchain install nightly-2026-09-19 --profile minimal
cargo install cargo-fuzz --version 0.13.2 --locked
cargo run --locked --manifest-path fuzz/Cargo.toml --example check_seeds
mkdir -p fuzz/corpus/program_atomic
cargo +nightly-2026-09-19 fuzz run program_atomic fuzz/corpus/program_atomic fuzz/seeds/program_atomic -- -max_total_time=30 -timeout=5 -rss_limit_mb=1024 -max_len=131072 -seed=190901
```

Replace `program_atomic` with `capsule_receive`, `policy_wire`, `fact_write`, `traversal_plans`, `vector_plans` or `mcp_jsonrpc` for the other targets. CI (`fuzz.yml`) runs every target for 30 seconds. Generated corpora, binaries and failure artifacts are ignored; curated seeds and their semantic seed check are committed. A finding must be minimized and turned into a regression before being considered resolved. Local runs leave raw logs under `fuzz/runlogs`; normalized published logs and measured counters are in [docs/fuzz](fuzz/2026-09-19-smoke.json). The manifest records the actual compiler, source base, seeds, sanitizer and resource limits. CI uses a larger 1536 MiB RSS limit on Linux and publishes artifacts on failure.

The three invariants are deliberately narrow:

- `program_atomic`: a deserializable failed program leaves no published events, and a fresh `fuzz/main` CAS(None) commit still works. This does not independently check every schema/identity registry, other graph heads, or durable process-death behavior.
- `capsule_receive`: importing a capsule never emits accepted graph events; repeating a successful import adds zero revisions. This target does not independently prove failed-import quarantine rollback or all acceptance/governance transitions.
- `policy_wire`: decoding and verification remain bounded and cannot mint trust when the host has no installed roots. Valid delegated chains, request replay and nonce transactions need the separate admission tests; these paths are not claimed to be covered by the empty-root fuzz host.

The first local runs at engine `d184eaa` processed 247,142 plan inputs, 488,311 capsule inputs and 3,318,373 policy inputs in 31 seconds per target, with no sanitizer crash or failed asserted invariant. A further plan run after explicit valid-seed verification is recorded separately. Reported peak RSS was 519, 529 and 671 MiB respectively; these are sanitizer/fuzzer process measurements, not production memory estimates. Full geometry execution and idempotent capsule import are exercised by `check_seeds`.

Many generated inputs are rejected while parsing. Input counts and libFuzzer coverage counters are not complete valid workflows, percentages of semantic coverage, security certification, or proof that no bug exists. These short seeded runs are reproducible smoke evidence for E14. Longer campaigns, additional structured/stateful targets, error-path persistence oracles, supported-platform campaigns and independent security review remain open.

The initial capsule host lacked storage authority, so its 488,311 inputs exercised parsing and rejection rather than successful import replay. Review corrected the target to grant storage only for graph `fuzz`. The authorized follow-up processed 529,194 inputs in 31 seconds (reported RSS 508 MiB), with no crash or failed invariant. The current checked-in target and CI include this correction.

## Phase A targets (A6)

Four targets cover the write and query paths that Phase A added. They share one seeded fact graph (`fuzz/fuzz_targets/common/mod.rs`): three fact writes at recorded times 10, 20 and 30, with nodes, edges, an embedding space, embeddings, a correction and a retraction. Every fact readable only by `admin` has the word `secret` in its ID or content. For the reader `analyst`, no result or error may contain that word unless the fuzz input contains it. This is a narrow noninterference oracle that also covers error messages. `check_seeds` shows that the oracle can fail: through the seeds of each read target, `admin` does see `secret`.

- `fact_write`: decodes a program with `decode_program` and keeps it only if every command is a `write`. It applies the program to a fresh seeded store. A failed write must leave the head and event count unchanged and the line writable. After a successful write, both readers must still be able to read the graph. The same program on a second seeded store must have the same outcome, revision IDs included.
- `traversal_plans`: decodes a program whose commands are all `evaluate` of `expand`, `as_of` or `changes`. Nested expressions are unrestricted. Evaluation as either reader must be deterministic and read-only (head and event count unchanged), and must pass the leak oracle.
- `vector_plans`: the same checks, for `nearest` and `similar_candidates`. This includes `nearest` over an `expand` neighborhood and `expand` seeded from `nearest`.
- `mcp_jsonrpc`: each input line, up to 16 lines per input, is one message to a `weave-mcp` `Server` reading the seeded store (written to a temporary file) as `analyst`. Every response must be one JSON-RPC 2.0 object with exactly one of `result` and `error`, and must echo the request ID unless the envelope itself was rejected. No message may change the store, which a second engine on the same file checks. The leak oracle also applies.

The first local runs at engine `f97c93d` used a fresh corpus plus the committed seeds and ran 60 seconds per target. They were on Linux x86_64 (AMD Ryzen 9 7950X), with `nightly-2026-09-19`, cargo-fuzz 0.13.2, AddressSanitizer, a 5 s per-input timeout, a 1536 MiB RSS limit and seed 201020. No run had a sanitizer crash or a failed invariant:

| Target | Inputs | Seconds | libFuzzer cov / features | Corpus at end | Reported RSS |
|---|---:|---:|---:|---:|---:|
| `fact_write` | 575,281 | 61 | 10,098 / 17,226 | 1,086 (392 KB) | 527 MiB |
| `traversal_plans` | 429,784 | 61 | 12,377 / 30,109 | 1,081 (231 KB) | 539 MiB |
| `vector_plans` | 488,606 | 61 | 12,229 / 26,345 | 1,111 (299 KB) | 540 MiB |
| `mcp_jsonrpc` | 7,275 | 61 | 11,173 / 40,858 | 641 (752 KB) | 585 MiB |

The [manifest](fuzz/2026-10-20-phase-a-smoke.json) records the command, limits and seed hashes. The normalized logs are `docs/fuzz/<target>.log`. `mcp_jsonrpc` is about 60 times slower per input than the others: each input can hold up to 16 tool calls, each a full evaluation on a file-backed store. Its count measures that cost, not a weaker target.

Limits: most generated inputs are still rejected by strict decoding. The plan targets accept only programs whose top-level commands are their operators. The leak oracle detects hidden IDs and content appearing in output; it does not detect influence through counts, scores or ordering, which B2's property harness is for. The write target checks atomicity in memory; it does not check durability after a process dies. As before, these counts are smoke evidence, not semantic coverage or a security review.
