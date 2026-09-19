# Native CLI baseline

The [raw measurements](2026-09-19-protocol-0.3.json) cover engine commit `938cde7629d5acb5b381b393cf67af8752a7f253`, built with `cargo build --release --locked -p weave-engine` using Rust 1.94.0. This is an experimental protocol 0.3 baseline, not a performance guarantee or full engine conformance result.

Each workload is a ring of public nodes and directed edges, with equal node and edge counts, one space, half-open valid time, no metadata, no adapters, no peer transport, and one visibility partition. Each of five samples starts a fresh CLI process and database, commits the graph, queries all valid objects, and serializes the result. The harness checks exact returned counts and complete coverage. Timing includes CLI startup, parsing, SQLite work, querying, and output; excludes compilation, input generation, and harness verification. Filesystem caches are not flushed.

| Objects | Outcome | Median elapsed | Sample p95 | Maximum child RSS |
| --- | --- | --- | --- | --- |
| 10,000 | Passed, all five samples | 34.1 ms | 448.9 ms | 28.4 MiB |
| 100,000 | Passed, all five samples | 318.7 ms | 325.0 ms | 235.5 MiB |
| 1,000,000 | Rejected, all five samples | Not a capacity result | Not applicable | 19.9 MiB during rejection |

The million-object input is 77,444,603 bytes and is rejected by the documented 16 MiB CLI input limit before graph execution. No million-object capacity is claimed. The 10,000-object tail includes the first invocation; these five samples are too few to characterize production tail latency. Host OS, architecture, logical CPU count, each raw sample, output size, and rejection diagnostic are preserved in the JSON. Peak RSS is measured for each child via POSIX `wait4`.

To reproduce after checking out the recorded engine revision and building its release executable, run the benchmark harness from a revision containing this script:

```sh
python3 scripts/root_benchmark.py \
  --engine /path/to/recorded-checkout/target/release/weave-engine \
  --revision 938cde7629d5acb5b381b393cf67af8752a7f253 \
  --output baseline.json
```

The caller must ensure the binary matches the supplied revision. This POSIX harness does not measure mobile energy, metadata fan-out, partitioned visibility, clustering quality, adapter amplification, or recovery. Those remain separate acceptance work under E14.
