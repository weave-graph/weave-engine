# Validation and performance evidence

Start with the current native science reports. The [validation guide](../SCIENCE_VALIDATION.md)
explains their commands, independent references, measurement boundaries and limits.
For implementation status and platform verification, read [status](../STATUS.md).
This directory preserves earlier workloads as historical evidence; their sizes
and timing boundaries differ from the current science workload.

## Fact-level writes (A2)

| Report | Recorded scope | Result |
|---|---|---|
| [Fact writes, 19 October 2026](fact-writes-2026-10-19.json) | `cargo run --release --example fact_write_benchmark`; contract 0.22.0, store 30; graphs of 10k, 100k and 1M facts (half nodes, half edges); 200 single-fact commits each on an on-disk WAL store; one AMD Ryzen 9 7950X Linux host | Median single-fact commit 0.89–0.90 ms at every size; mean growth 2.5–2.6 KB per commit |

## Recorded-time selection (A3)

| Report | Recorded scope | Result |
|---|---|---|
| [Embedding upsert and exact nearest, 20 October 2026](embeddings-2026-10-20.json) | `cargo run --release --example embedding_benchmark -- 100000 1536 1000`; contract 0.24.0, store 31; 100,000 embeddings of 1,536 dimensions (cosine) on 100,000 nodes, written in 100 fact writes of 1,000, then 5 single-embedding writes; 5 exact top-10 `nearest` over the whole space by a reader who sees 90% of it, with and without `valid_at`; one AMD Ryzen 9 7950X Linux host | Load 34.7 s (330 ms per 1,000-embedding write, flat from first to last); single-embedding upsert 1.19 ms median at full size; whole-space exact top-10 7.09 s median, 3.62 s with `valid_at` selecting half; 2.1 GB store; peak RSS 265 MB, not raised by the rankings |
| [As-of selection, 20 October 2026](as-of-selection-2026-10-20.json) | `cargo run --release --example as_of_benchmark`; contract 0.23.0, store 30; 10k and 1M single-fact revisions on one branch of an on-disk WAL store; 500 `expand` reads `as_of` uniformly random recorded times, each checked against the state recorded then, plus the same read pinned to the head; one AMD Ryzen 9 7950X Linux host | Median as-of read 0.43 ms at 10k and 0.46 ms at 1M revisions (pinned 0.46 and 0.48 ms): selection cost does not grow with history length beyond index depth |

## Current native science baseline

| Report | Recorded scope | Result |
|---|---|---|
| [Scientific acceptance, 10 October 2026](science-acceptance-2026-10-10.json) | Optimized science API 0.1.0 / engine contract 0.21.0; real native store and independent graph/vector/time/permission checks | 16 grouped cases passed, 158 native process operations |
| [Science workloads, 10 October 2026](science-benchmark-2026-10-10.json) | 1k/10k input nodes, temporal multigraph, eight-dimensional vectors, shared metadata, two visibility partitions; separate import/query/analytics samples | Both sizes passed; 10k query/analytics medians 172–194 ms including full JSON/provenance output |

These reports describe one Apple M4 Max / arm64 Mac host, an optimized executable
and the recorded dirty integration checkout. They preserve exact binary and
source-tree identities; the base commit is not an exact clean science release
mapping. They are local measurements, not hosted Linux/Windows performance
evidence or a production service-level objective. The same executable digest
appears in both reports. Report SHA-256 values and the full measured table are in
[the recorded baseline](../SCIENCE_VALIDATION.md#recorded-optimized-baseline-10-october-2026).

Reproduce from the repository root:

```sh
cargo build --release --locked -p weave-science
python3 scripts/check_science.py \
  --engine target/release/weave-science \
  --report ../weave-validation/science-acceptance.json --seed 1337
python3 scripts/benchmark_science.py \
  --engine target/release/weave-science \
  --report ../weave-validation/science-benchmark.json \
  --sizes 1000 10000 --samples 3 --dimensions 8 --seed 1337
```

Use `python` and `weave-science.exe` on Windows; the validation guide provides
PowerShell commands. New runs create new evidence and should retain their own
host, source and binary identities. Inspect report `status`: benchmark budget
rejection is not a passing capacity measurement, even when the harness exits
normally. Individual operation timers exclude fixture generation and Python
decoding, while the overall elapsed time includes fixture/store setup and verification.
Filesystem caches are not flushed.

## Historical profiles

| Profile | Evidence | How its measurement differs |
|---|---|---|
| Protocol 0.3 public ring, 19 September 2026 | [Raw report](2026-09-19-protocol-0.3.json), details below | One process commits and queries a public ring; sizes count total nodes plus edges; no metadata/vectors/visibility partitioning |
| Protocol 0.11 native helper, 19 September 2026 | [Measurement guide](../PERFORMANCE_BASELINE.md), [raw report](../measurements/2026-09-19-native-baseline.json) | Warm in-process commit/read/navigation samples; read timings exclude final JSON serialization; resource observations cover the whole helper |
| Retained incremental selection fixture, date/revision not recorded | [Raw debug report](selection-local.json), [selection semantics](../INCREMENTAL_SELECTION.md) | Two nodes and 256 claims in memory, five correction samples; work counters and refresh cost, not a large-data throughput result |

Compare a new implementation against a reproduced matching workload before
claiming a speedup. Object/node counts, query semantics, visibility, metadata,
storage, build profile and serialization boundaries all matter. These reports
do not support a direct speedup ratio between the September ring and October
science profiles. The selection fixture's median incremental refresh was slower
than its full refresh; its improved membership counters are not an end-to-end
speedup claim.

### Historical protocol 0.3 public ring

The [raw measurements](2026-09-19-protocol-0.3.json) cover engine commit `938cde7629d5acb5b381b393cf67af8752a7f253`, built with `cargo build --release --locked -p weave-engine` using Rust 1.94.0. This is an experimental protocol 0.3 baseline, not a performance guarantee or full engine conformance result.

Each workload is a ring of public nodes and directed edges, with equal node and edge counts, one space, half-open valid time, no metadata, no adapters, no peer transport, and one visibility partition. Each of five samples starts a fresh CLI process and database, commits the graph, queries all valid objects, and serializes the result. The harness checks exact returned counts and complete coverage. Timing includes CLI startup, parsing, SQLite work, querying, and output; excludes compilation, input generation, and harness verification. Filesystem caches are not flushed.

| Total graph objects | Outcome | Median elapsed | Sample p95 | Maximum child RSS |
| --- | --- | --- | --- | --- |
| 10,000 | Passed, all five samples | 34.1 ms | 448.9 ms | 28.4 MiB |
| 100,000 | Passed, all five samples | 318.7 ms | 325.0 ms | 235.5 MiB |
| 1,000,000 | Rejected, all five samples | Not a capacity result | Not applicable | 19.9 MiB during rejection |

The million-object input is 77,444,603 bytes and is rejected by the documented 16 MiB CLI input limit before graph execution. No million-object capacity is claimed. The 10,000-object tail includes the first invocation; these five samples are too few to characterize production tail latency. Host OS, architecture, logical CPU count, each raw sample, output size, and rejection diagnostic are preserved in the JSON. Peak RSS is measured for each child via POSIX `wait4`.

To reproduce this historical profile, check out the recorded engine revision in
a separate checkout and build its release executable. Run the harness from a
checkout containing `scripts/root_benchmark.py`:

```sh
python3 scripts/root_benchmark.py \
  --engine /path/to/recorded-checkout/target/release/weave-engine \
  --revision 938cde7629d5acb5b381b393cf67af8752a7f253 \
  --output ../weave-validation/protocol-0.3-baseline.json
```

The supplied revision is a caller-verified label. This POSIX harness requires
`wait4`; it does not measure metadata fan-out, partitioned visibility, clustering
quality, adapter amplification, recovery or mobile energy. Those remain separate
acceptance topics under the original E14 roadmap.
