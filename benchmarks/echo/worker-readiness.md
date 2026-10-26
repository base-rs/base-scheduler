# Worker-owned readiness measurements

These are historical measurements from the downstream, unreleased Base HTTP hello benchmark on an Apple Silicon macOS host. They explain the choices in this change stack; they are not measurements rerun against the extracted Git commits. Source and binary hashes in the cohort environments identify the measured snapshots. Extracted commits also include tests and documentation prepared after measurement.

The diagnostic raw server has one application worker, one spawned supervisor, a `FuturesUnordered` connection set capped at 1,024, and TCP_NODELAY. Each connection reads arbitrary bytes into a 16 KiB borrowed buffer and writes one fixed 78-byte hello response in one call. A short write fails validation. It deliberately has no HTTP parser or pipelining support. The Tokio task control spawns its supervisor once too. The full HTTP variants include parsing and body handling; raw results cannot predict full HTTP throughput.

Each cohort records its duration, repetitions, concurrency, warmup, variant hashes, and interleaved samples in `results/worker-readiness/`. Longer confirmation runs use wrk with four load threads, 64 connections, a one-second warmup and five eight-second samples per variant. User and system CPU are server-process time divided by completed requests. Host activity and core placement were uncontrolled; kernel time dominates this loopback workload. Percentages from different cohorts must not be added together. No result establishes TLS, io_uring, non-macOS or multi-worker performance.

The initial change moves readiness delivery from the independent reactor onto executor workers, and retains edge generations and read-byte/EOF hints. Its extraction has no new isolated performance measurement; subsequent cohorts compare refinements of that implementation.

## Fused notifier

The notifier confirmation reduces median raw user CPU from 0.37438 to 0.33881 microseconds/request (9.5%) and full HTTP from 0.86195 to 0.81298 (5.7%). Median throughput is 133,733 to 134,884 requests/second raw and 129,130 to 129,877 HTTP. All samples report zero errors. This cohort includes the `Cancellation::never()` path and removal of redundant closure listeners; it does not isolate those pieces individually. See [notifier summaries](results/worker-readiness/notifier/summary.json) and [samples](results/worker-readiness/notifier/samples.jsonl).
