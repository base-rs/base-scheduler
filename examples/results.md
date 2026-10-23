# Canonical recipe baseline

Measured on Aiko on 2026-10-23 using commit [`4d15853`](https://github.com/core-rs/core-scheduler/commit/4d15853baeac1d8ee5d7126b9c3e8ceca1c63522). These are application-loop baselines for the recommended compositions. This change does not modify any scheduler implementation or claim a runtime speedup.

## Environment and method

- AMD Ryzen 9 9950X3D, 16 physical cores / 32 logical CPUs; Linux `7.2.8-arch1-2`.
- Rust `1.99.0`, LLVM `23.1.1`; Cargo's ordinary release profile, empty `RUSTFLAGS`, no custom affinity or target-CPU tuning.
- Clean source checkout at the recorded commit. Aiko was nearly idle before the run (load averages `0.14, 0.07, 0.02`). An earlier exploratory run on Hana was discarded from this report because that host is regularly busy.
- Five independent invocations for each of four backends, four recipes, and one/four workers: **160 validated samples**. Each invocation performs a complete warmup before timing the second pass. Configuration order is shuffled with seed zero.
- TCP is loopback with one sender thread and one receiver task; files are page-cache-hot and do not synchronize durability. Increasing worker count does not add connections or file copies. UDP dispatches up to eight independent processing tasks.
- Setup creates payloads, sockets/files, pools, and buffers before timing. Timing includes the recipe's single monitor/receiver lifecycle, application validation, and packet completion notifications. Exclusive resource close and peer join are outside timing.
- Frames retain two owners, validate them again on later frame callbacks, and validate the final owners after receiver close. This extra validation is application work in the frame case, not scheduler overhead. Packet validation checks sequence numbers, duplicates, payload bytes, and sender addresses.

The standard matrix uses 256 MiB of stream bytes, 32,768 frames of 4,093 bytes (about 127.9 MiB), 16,384 datagrams of 1,200 bytes (18.75 MiB), and 256 MiB file copies. Receive/copy storage is 64 KiB. Pooled network cases have sixteen shared slots, independent of worker count; the file loop alternates two vectors. The TCP sender writes the complete prepared byte stream, so receive chunking remains a backend/OS outcome.

Reproduce with `python3 examples/benchmark.py --repeats 5 --output /tmp/core-scheduler-examples.jsonl`. The [raw samples](results/aiko.jsonl) and [build metadata](results/aiko.metadata.json) preserve exact counts, intervals, compiler details, and lockfile identity. The CPU model above was obtained separately with `lscpu`; Python's processor field was empty on this host.

## Throughput

Each cell is **median MiB/s [minimum–maximum]** across five invocations. Compare a row across backends; different recipes perform different ownership and validation work.

| Recipe | Workers | Direct | Tokio adapter | Native readiness | Native io_uring |
| --- | ---: | ---: | ---: | ---: | ---: |
| stream | 1 | 16,006.0 [13,975.7–16,344.5] | 16,652.0 [15,179.4–17,282.1] | 16,946.3 [15,749.3–17,549.8] | 16,546.5 [12,044.0–16,942.4] |
| stream | 4 | 10,373.4 [7,337.0–14,627.5] | 15,224.9 [13,373.9–17,032.5] | 16,773.3 [15,769.9–17,671.0] | 13,237.1 [6,949.5–17,255.2] |
| frames | 1 | 1,626.7 [480.4–4,095.2] | 11,155.3 [9,749.9–12,854.6] | 11,268.9 [7,021.4–13,807.2] | 11,181.6 [5,907.2–12,625.5] |
| frames | 4 | 496.1 [465.7–1,397.0] | 13,347.3 [12,068.2–14,818.3] | 8,203.2 [7,044.5–12,251.9] | 9,829.9 [5,668.8–10,182.8] |
| datagrams | 1 | 6.3 [4.4–16.8] | 988.9 [947.4–1,055.5] | 756.7 [553.9–780.1] | 957.2 [591.5–994.0] |
| datagrams | 4 | 24.7 [23.1–27.3] | 1,018.3 [1,001.6–1,045.9] | 822.4 [502.4–907.0] | 812.6 [548.8–913.9] |
| files | 1 | 6,401.0 [6,323.0–6,876.1] | 6,081.3 [5,063.9–8,600.2] | 6,857.3 [5,203.6–9,104.4] | 5,906.4 [5,543.5–6,556.2] |
| files | 4 | 5,977.4 [5,781.2–7,915.7] | 8,045.7 [4,830.6–8,908.1] | 8,816.5 [5,209.2–9,136.7] | 6,792.9 [5,833.4–7,122.6] |

## Interpretation

The borrowed stream loop reaches similar one-worker median throughput on Tokio and both Native backends. Retained-frame medians are also close with one worker, while the four-worker results vary more. Tokio has the highest median in this particular packet-handoff workload; the Native backends remain substantially faster than the Direct reference. File results vary enough that these samples do not establish a stable backend ordering.

Intervals range from about 8.6 ms to 4.3 s, and several configurations have wide ranges despite the quiet host. These are short baseline transfers, not latency distributions or evidence for a small performance advantage. Use larger record counts and repeated controlled runs before treating a small difference as a regression. A single sequential connection also does not measure many-connection scaling.

These measurements do not compare pooling against borrowed receives under identical ownership requirements. The stream recipe consumes bytes immediately; the frame recipe retains views; the datagram recipe transfers ownership to tasks. They support keeping those application behaviors explicit in the standard workloads. They do not justify selecting registered buffers, multishot, or a different pool policy in generic protocol code. The recommended starting points remain persistent monitors for borrowed I/O and pooled receivers for independently owned data.
