# Native baseline measurements

These are small reproducible local workloads, not a service SLO or a capacity claim. Source commit `193cc7e608b592213680b556619e92547f491c92`, Rust 1.94.0 release build, Apple M4 Max, macOS/aarch64. The process uses a temporary local SQLite WAL database, one principal, no networking and warm operating-system caches. The [raw measurement JSON](measurements/2026-09-19-native-baseline.json) records exact counts, source hash and limitations; [time output](measurements/2026-09-19-native-time.txt) records whole-process resource observations.

| Workload | Samples after 3 warmups | Median | Observed p95 |
|---|---:|---:|---:|
| Commit 256 nodes/1024 positive edges, changed property generation | 30 |7.072ms|10.890ms|
| Authorized exact predicate/time query,256 nodes/1024 edges | 30 |2.833ms|2.953ms|
| Available navigation hierarchy,64 nodes/63 chain links | 10 |13.037ms|23.850ms|

The query value serializes to 554,127 bytes. The navigation graph has 128 nodes/127 edges and serializes to 5,371,300 bytes, reflecting conservative repeated whole-input influence proofs. Whole executable maximum resident set was 99,516,416 bytes, with 95,846,832 bytes reported peak memory footprint; these include fixture construction, revisions, cache state and final serialization and are not per-operation allocations. One reopen took 1.906 ms and preserved 34 events and exact query results. Small-sample percentiles do not characterize tail latency under production contention.

Reproduce from the corresponding source commit:

```sh
cargo build --release --locked -p weave-engine --example benchmark_baseline
/usr/bin/time -l target/release/examples/benchmark_baseline
```

On Linux use the platform's resource-reporting equivalent; the JSON-producing executable itself is portable across native supported hosts. Commit timing includes program construction/decoding inside the helper; read/navigation timing excludes final JSON serialization but includes runtime validation and output byte-budget checks. Three warmups per operation are excluded. No mobile/browser, network, concurrent-reader/writer, large-dataset, clustering-quality or cryptographic review result is implied. The bounds and duplicated provenance motivate future compact proof representation and incremental maintenance; they do not justify weakening authorization checks.
