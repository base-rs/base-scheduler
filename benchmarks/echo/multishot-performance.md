# Multishot receive comparison

`socket_recvmsg_multishot` keeps a receive submitted while the kernel selects buffers from a bounded provided-buffer ring. Completed buffers become owned leases; dropping a lease returns it to its original ring. Owner-thread returns publish directly, while cross-thread returns use that ring’s inbox. Exhaustion pauses reception until a lease returns, and only a final original CQE allows retirement or resubmission.

On this closed-loop TCP echo workload, multishot does **not beat pure Tokio**. Median throughput is 2.4–4.7% lower in six configurations and 9.0–9.6% lower with one worker and 32 connections. Against Native `_poll`, multishot improves the 64-byte, single-connection cases by 5.8–14.2% and the one-worker, 32-connection cases by 8.0–11.9%; the other configurations are 1.0–3.3% lower. These are results for this experiment, not a universal ranking of the I/O mechanisms.

These measurements predate the portable `SocketRecvMsgMultishot` interface. The same generic echo loop now also runs as `tokio-adapter-multishot` using polling emulation; that backend is smoke-tested but is not measured in this report.

## Method

Measured on Aiko on 2026-10-21 from clean implementation commit `7b3e925d4ca02e66afed13cdda84f3dc17c8c606`. The machine is an AMD Ryzen 9 9950X3D with Linux 7.2.6-arch2-1, Rust 1.99.0, and LLVM 23.1.1. Server CPUs 0–3 and client CPUs 4–7 are separate physical cores sharing one last-level cache; the client has four Tokio workers. All builds, tests, and measurements ran under the exclusive `/tmp/core-scheduler-perf.lock`.

The runner builds with `cargo build --release --locked`, using the default release profile and no additional `RUSTFLAGS`. Every backend uses the same executable, including the client. Five repetitions cover one or four server workers, one or 32 persistent connections, and 64-byte or 4096-byte payloads. Backend and case order are randomized with seed 20261021. Each sample has a 0.5-second warmup and a two-second measurement. All 120 samples, latency histograms summarized as percentiles, source provenance, binary hash, toolchain, CPU topology, and host load are retained in the [raw results](results/recvmsg-multishot/measurements.json).

Multishot uses eight 64 KiB payload buffers per connection, plus metadata, compared with the other cases’ single 64 KiB buffer. It sends directly from each completed lease using the same borrowed send path as `_poll`, handles short writes, and then returns the lease. Each persistent receiver holds one ring admission slot until retirement; all Native cases reserve at least twice the connection count, with a minimum capacity of 256 per ring.

## Throughput

Values are median thousands of completed round trips per second; brackets show the minimum and maximum across five repetitions. Percentage differences compare medians.

| Workers | Connections | Bytes | Tokio kreq/s [range] | Native `_poll` kreq/s [range] | Multishot kreq/s [range] | Multishot vs Tokio | Multishot vs `_poll` |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 1 | 64 | 270.5 [268.2–271.3] | 249.2 [240.2–252.2] | 263.6 [262.2–268.1] | -2.55% | +5.76% |
| 1 | 1 | 4096 | 237.0 [232.3–241.4] | 233.8 [231.1–236.1] | 226.0 [223.8–228.7] | -4.66% | -3.34% |
| 1 | 32 | 64 | 691.4 [686.1–702.2] | 562.4 [560.8–563.1] | 629.2 [628.5–629.3] | -8.99% | +11.89% |
| 1 | 32 | 4096 | 606.0 [603.9–611.0] | 507.0 [505.2–507.6] | 547.8 [544.3–550.9] | -9.61% | +8.04% |
| 4 | 1 | 64 | 269.0 [267.6–271.0] | 228.0 [223.8–229.8] | 260.4 [256.0–262.4] | -3.19% | +14.22% |
| 4 | 1 | 4096 | 235.4 [234.7–239.2] | 231.7 [230.1–234.3] | 229.4 [229.0–231.9] | -2.52% | -0.98% |
| 4 | 32 | 64 | 1740.4 [1731.8–1741.9] | 1720.9 [1696.9–1744.4] | 1698.9 [1665.5–1723.2] | -2.39% | -1.28% |
| 4 | 32 | 4096 | 1610.0 [1594.8–1619.9] | 1579.4 [1561.1–1597.6] | 1546.4 [1502.6–1558.5] | -3.95% | -2.09% |

## Tail latency

Median per-sample p99 round-trip latency, in microseconds. The raw results also retain p50, p95, maximum, and mean latency.

| Workers | Connections | Bytes | Tokio p99 µs | Native `_poll` p99 µs | Multishot p99 µs |
| --- | --- | --- | --- | --- | --- |
| 1 | 1 | 64 | 4.37 | 5.68 | 4.45 |
| 1 | 1 | 4096 | 5.37 | 5.40 | 5.53 |
| 1 | 32 | 64 | 54.11 | 61.12 | 55.81 |
| 1 | 32 | 4096 | 61.60 | 68.73 | 63.74 |
| 4 | 1 | 64 | 4.67 | 9.38 | 4.73 |
| 4 | 1 | 4096 | 5.55 | 5.89 | 5.95 |
| 4 | 32 | 64 | 52.70 | 54.98 | 52.13 |
| 4 | 32 | 4096 | 60.54 | 62.75 | 60.70 |

## Interpretation and verification

Tests verify that 20 successive stream receives reuse one kernel submission, and that local lease returns repeatedly advance the provided-buffer ring without resubmission. Kernel rearming works, but it does not remove completion delivery, task wakeups, scheduling, or lease bookkeeping. This benchmark alone does not attribute the remaining gap among those costs.

Each connection sends one payload and waits for its complete echo before sending another. Consequently, the workload offers little opportunity to receive a burst of messages on one connection before its consumer runs. Pipelined TCP, bursty datagrams, connection churn, and physical-network performance remain unmeasured. The ranges above describe observed variation, not statistical confidence intervals.

The io_uring implementation has 100% measured region coverage (2911/2911 regions), with 102 unit tests and 180 conformance tests passing. The Linux echo package has 100% measured region coverage (585/585 regions), including its multishot smoke test. Tests exercise exhaustion and rearming, short payload capacities, empty datagrams, sender addresses and truncation, stream EOF, explicit cancellation, abandonment, owner migration and remote returns, driver failures, and leases retained through shutdown.

## Reproduction

Run on Aiko, or adjust the runner’s CPU sets for another host:

```sh
flock -x /tmp/core-scheduler-perf.lock env CARGO_TARGET_DIR="$HOME/.cache/core-scheduler-recvmsg-multishot/target" python3 benchmarks/echo/results/recvmsg-multishot/run.py --repeats 5 --seconds 2 --output /tmp/recvmsg-multishot.json
```

The [runner](results/recvmsg-multishot/run.py) records source status and executable identity. Reproduce from the clean implementation commit above; later documentation-only commits include the report and raw output.
