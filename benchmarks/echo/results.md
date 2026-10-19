# Aiko TCP echo results

Measured on 19 October 2026 at commit [`7de956e`](https://github.com/core-rs/core-scheduler/commit/7de956e450bd0abeb92c49412ecdc46656adb0ba), with a clean working tree. [Raw samples and machine metadata](results/aiko-2026-10-19.json) preserve all 72 measurements. The [benchmark guide](readme.md) describes the workload and how to reproduce it.

Aiko has an AMD Ryzen 9 9950X3D (16 physical cores, 32 hardware threads), running Linux 7.2.6 and Rust 1.99.0. The server used CPU 0 for one worker and CPUs 0–3 for four workers. The client used four Tokio workers on CPUs 4–7. These are separate physical cores sharing one last-level cache. The runner did not change frequency policy or machine-wide settings.

Each persistent TCP connection sent one request at a time over IPv4 loopback with `TCP_NODELAY`. Each sample had one second of warmup and three seconds of measurement. Every case ran three times in shuffled order, using optimized builds and the checked-in lockfile. Payload validation, client scheduling, and loopback networking are part of the workload.

## Throughput

Median complete round trips per second across three repetitions:

| Server workers | Connections | Payload | Pure Tokio | Tokio adapter | Native io_uring |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 64 B | 271,566 | 253,477 | 164,843 |
| 1 | 1 | 4,096 B | 234,596 | 231,639 | 154,944 |
| 1 | 32 | 64 B | 703,028 | 587,294 | 483,448 |
| 1 | 32 | 4,096 B | 624,975 | 520,543 | 440,771 |
| 4 | 1 | 64 B | 267,526 | 252,725 | 159,530 |
| 4 | 1 | 4,096 B | 237,031 | 232,804 | 154,011 |
| 4 | 32 | 64 B | 1,724,577 | 1,558,793 | 1,314,847 |
| 4 | 32 | 4,096 B | 1,618,402 | 1,466,707 | 1,177,404 |

At 32 connections, the Tokio adapter delivered 83–91% of pure Tokio throughput, and Native io_uring delivered 69–76%. Moving from one to four server workers increased throughput by about 2.5–2.8× for these cases. With one connection, the Tokio adapter was much closer to pure Tokio; adding server workers did not improve that serial workload.

The throughput range `(maximum − minimum) / median` was 0.2–4.0% for the 32-connection cases and reached 7.0% for single-connection cases. Three short repetitions provide a useful initial baseline, not a precise confidence interval.

## Round-trip latency

Each cell is the median of the three runs’ p50 / p99 latencies, in microseconds. These are client-observed closed-loop latencies, not pooled percentiles or server-only service times.

| Server workers | Connections | Payload | Pure Tokio | Tokio adapter | Native io_uring |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 64 B | 3.56 / 4.33 | 3.63 / 5.73 | 5.95 / 6.65 |
| 1 | 1 | 4,096 B | 3.91 / 5.44 | 3.99 / 5.48 | 6.31 / 7.44 |
| 1 | 32 | 64 B | 46.49 / 53.41 | 55.74 / 61.92 | 65.47 / 75.45 |
| 1 | 32 | 4,096 B | 52.32 / 59.71 | 62.91 / 69.76 | 71.94 / 82.11 |
| 4 | 1 | 64 B | 3.58 / 4.89 | 3.63 / 6.08 | 6.15 / 11.49 |
| 4 | 1 | 4,096 B | 3.88 / 5.80 | 3.97 / 5.63 | 6.30 / 8.14 |
| 4 | 32 | 64 B | 17.50 / 52.64 | 19.41 / 55.87 | 23.04 / 56.96 |
| 4 | 32 | 4,096 B | 18.86 / 59.81 | 20.67 / 60.48 | 25.21 / 67.33 |

## Client placement sensitivity

A supplementary run increased the client from four to eight workers and expanded its CPU set from 4–7 to 4–7,12–15. The server stayed on CPUs 0–3 with four workers and 32 connections. The added client cores belong to a second last-level cache, so this changes locality as well as available client capacity. [Raw supplementary samples](results/aiko-2026-10-19-client-placement.json) preserve its 18 measurements.

Median round trips per second, with the minimum–maximum range of the three repetitions:

| Payload | Pure Tokio | Tokio adapter | Native io_uring |
| ---: | ---: | ---: | ---: |
| 64 B | 1,024,316 (351,122–1,042,838) | 963,456 (344,828–968,970) | 303,597 (301,023–867,144) |
| 4,096 B | 314,673 (314,017–929,204) | 834,399 (309,636–855,935) | 790,302 (272,925–813,485) |

Throughput decreased and variation increased substantially. This is evidence that the experiment is sensitive to client placement and concurrency; it is not evidence that the baseline client has spare capacity. The main table therefore reports end-to-end throughput for the stated configuration, not isolated server capacity. A stronger capacity claim needs client/server CPU measurements and a controlled locality comparison.

To reproduce the supplementary run:

```sh
python3 benchmarks/echo/run.py --workers 4 --connections 32 --client-workers 8 --server-cpus 0-3 --client-cpus 4-7,12-15 --output /tmp/echo-client-placement.json
```

## Interpretation

This baseline compares the current concrete implementations and their ownership paths. It does not isolate the cost of the scheduler traits, explain where time is spent, or establish a general advantage of readiness over completion I/O. The io_uring server uses real completion-based socket reads and writes with per-worker rings; listener handling still uses the readiness fallback.

The workload has no pipelining, connection churn, TLS, physical network, or open-loop arrival process. Its latency does not correct for coordinated omission. The next useful step for explaining a difference would be profiling a fixed case, with separate attention to client CPU time, server scheduling, and operation submission/completion costs.
