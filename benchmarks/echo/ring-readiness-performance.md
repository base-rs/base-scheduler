# Worker-ring socket readiness

The native io_uring selector now attempts borrowed socket operations directly with a nonblocking syscall. Only `WouldBlock` submits a readiness request to the selected worker's ring. This replaces the shared `async-io` readiness handoff measured in the [previous comparison](poll-performance.md).

## Implementation

`socket_recv_poll`, `socket_send_poll`, `socket_recvmsg_poll`, and `socket_sendmsg_poll` call synchronous nonblocking helpers on `NativeSocket`. On Linux these enforce `MSG_DONTWAIT`. An immediately ready operation bypasses ring routing, capacity admission, request allocation, and completion channels. Borrowed I/O vectors reach the syscall directly.

After `WouldBlock`, the operation submits a one-shot `PollAdd` and retries the syscall when it completes. The first wait selects the ring; retries keep that ring if the task moves to another worker. Readiness requests retain the socket, capacity permit, and completion sender, never borrowed payload or vector pointers. Cancellation waits for the original readiness request to retire. Dropping the future requests cancellation; forgetting it may retain the socket until readiness or shutdown, without retaining access to its old payload.

`socket_wait` uses the same ring mechanism. Socket creation, accept/connect, and nonblocking retry waits for owned completion transfers still use the shared fallback. The owned echo path still submits `Recv` / `Send` payload operations to io_uring. These remaining paths are separate opportunities; this change does not replace the portable readiness selector.

## Repeated results

Median complete round trips per second across five repetitions:

| Workers | Connections | Bytes | Old fallback | Ring readiness | Change | Native owned | Pure Tokio |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 64 | 157,766 | 180,050 | +14.1% | 234,813 | 271,786 |
| 1 | 1 | 4096 | 140,995 | 202,625 | +43.7% | 187,498 | 233,910 |
| 1 | 32 | 64 | 403,369 | 552,075 | +36.9% | 515,181 | 694,651 |
| 1 | 32 | 4096 | 358,727 | 497,975 | +38.8% | 468,489 | 614,539 |
| 4 | 32 | 64 | 1,343,846 | 1,705,874 | +26.9% | 1,485,888 | 1,734,055 |
| 4 | 32 | 4096 | 1,162,361 | 1,554,970 | +33.8% | 1,361,198 | 1,597,755 |

Ring readiness improves median throughput over the old fallback in all six cases, by 14–44%. Every new-path repetition exceeds every corresponding fallback repetition. With four workers and 32 connections it reaches 98.4% of pure Tokio throughput for 64-byte messages and 97.3% for 4-KiB messages. The 64-byte sample ranges overlap; these results do not establish a precise statistical gap.

The new path beats owned completions in five of six median comparisons, but the single-connection 64-byte case remains 23.3% slower than owned io_uring and 33.8% slower than pure Tokio. With one worker and 32 connections, it remains about 19–21% below pure Tokio. Removing the fallback handoff therefore helps substantially without making readiness universally faster than completions. This experiment measures the whole new path; it does not isolate individual syscall, allocation, or wakeup costs.

Minimum–maximum throughput across the five repetitions:

| Workers | Connections | Bytes | Old fallback | Ring readiness | Native owned | Pure Tokio |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 64 | 156,474–162,226 | 172,677–182,215 | 231,489–236,620 | 269,302–274,505 |
| 1 | 1 | 4096 | 140,807–147,114 | 190,518–209,902 | 186,603–197,154 | 231,194–242,342 |
| 1 | 32 | 64 | 400,749–404,065 | 550,613–554,248 | 513,324–516,391 | 684,076–705,412 |
| 1 | 32 | 4096 | 356,155–363,324 | 493,830–501,160 | 466,485–470,693 | 607,136–618,312 |
| 4 | 32 | 64 | 1,314,554–1,348,937 | 1,684,812–1,712,266 | 1,437,914–1,502,076 | 1,689,438–1,756,238 |
| 4 | 32 | 4096 | 1,147,739–1,178,247 | 1,545,812–1,574,940 | 1,308,527–1,363,829 | 1,590,103–1,628,378 |

Median of each run’s mean / p50 / p99 round-trip latency, in microseconds. Percentiles are not pooled across runs:

| Workers | Connections | Bytes | Old fallback | Ring readiness | Native owned | Pure Tokio |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 64 | 6.28 / 5.99 / 12.21 | 5.50 / 5.42 / 9.54 | 4.22 / 4.08 / 5.70 | 3.65 / 3.56 / 4.33 |
| 1 | 1 | 4096 | 6.99 / 6.93 / 12.29 | 4.86 / 4.97 / 5.99 | 5.26 / 5.01 / 7.70 | 4.22 / 3.94 / 5.41 |
| 1 | 32 | 64 | 79.27 / 85.63 / 120.96 | 57.90 / 57.76 / 63.97 | 62.04 / 61.57 / 70.08 | 46.00 / 47.07 / 53.98 |
| 1 | 32 | 4096 | 89.10 / 94.66 / 134.27 | 64.16 / 64.03 / 70.59 | 68.20 / 67.78 / 76.67 | 51.97 / 53.25 / 60.61 |
| 4 | 32 | 64 | 23.76 / 18.22 / 53.79 | 18.72 / 17.44 / 54.88 | 21.49 / 20.18 / 56.64 | 18.41 / 17.57 / 52.99 |
| 4 | 32 | 4096 | 27.45 / 19.28 / 63.04 | 20.50 / 18.94 / 62.88 | 23.43 / 22.05 / 64.70 | 19.95 / 18.96 / 60.35 |

Mean and median latency improve over the old fallback in all six cases. Tail latency is not uniformly better: with four workers and 64-byte messages, median per-run p99 rises from 53.79 to 54.88 µs, while mean latency falls from 23.76 to 18.72 µs. The single-connection 64-byte case also retains worse mean, median, and p99 latency than owned io_uring. These differences remain visible alongside the throughput gains.

## Method

The [runner](results/ring-readiness/run.py) records all 120 samples in the [raw results](results/ring-readiness/aiko.json). Measurements use Aiko's Ryzen 9 9950X3D, Linux 7.2.6, and Rust 1.99.0. Each of six worker/connection/payload configurations runs five times in each of four modes. The entire matrix is shuffled with seed `20261022`, using one second of warmup and three seconds of measurement per sample.

The old fallback server is the unchanged executable from `7cd6af8725fedebc2dffd52473380d51d7d7fc5e`. The candidate server is built at `85f296f0854319ed71ebba4b72237eadd5925174`; its three modes are new ring readiness, owned completions, and pure Tokio. Both server builds use `cargo build --release --locked -p core-scheduler-echo-benchmark` with the default release profile, without explicit cross-crate LTO or `target-cpu=native`. The unchanged client is the executable retained from `5736301fded46c4e5f11d5387950ae2ee0807b94`. The old-server and client hashes match those in the previous borrowed-socket comparison; the raw results record all three executable hashes.

Servers use CPU 0 or CPUs 0–3; the four-worker client uses CPUs 4–7. These are separate physical cores sharing one last-level cache. Only one workload runs at a time, without concurrent builds or profilers on the host. CPU totals include the entire process lifecycle, including warmup and shutdown, rather than just the measurement window.

The comparison uses persistent TCP connections, default socket flags, `TCP_NODELAY`, one reusable 64-KiB buffer per connection, and one request in flight per connection. It measures closed-loop end-to-end echo throughput and round-trip latency, including the client and loopback network. It does not isolate server capacity, correct for coordinated omission, or establish a general ranking of readiness and completion I/O. Five short repetitions show direction and variation, not confidence intervals. See the [benchmark methodology](readme.md) for further details.

Build the two source revisions into separate target directories, retain the original client executable, then run:

```sh
python3 benchmarks/echo/results/ring-readiness/run.py \
  --baseline /tmp/core-echo-poll-fallback-baseline \
  --candidate target/release/core-scheduler-echo-benchmark \
  --client /tmp/core-scheduler-profile-baseline/release/core-scheduler-echo-benchmark \
  --source 85f296f0854319ed71ebba4b72237eadd5925174 \
  --output /tmp/core-ring-readiness-results.json
```

## Validation

Regression tests verify zero allocation for immediately ready scalar and vectored I/O even when ring capacity is full, readiness arriving before submission, spurious readiness, migration between threads, cancellation, backpressure, capacity exhaustion, and selector/driver shutdown. Borrowed-operation conformance suites run for IPv4 and IPv6 against independently driven rings, native composition, and per-worker rings.

The Linux workspace passed 18,258/18,258 measured source regions, with the existing single invariant `unreachable!` exclusion. macOS passed 15,802/15,802 regions without exclusions. Formatting, Clippy, and API documentation checks pass. GitHub Actions also passes the Windows coverage gate and Rust 1.85 compatibility tests. Linux coverage excludes only distro standard-library paths under `/usr/src/debug/rust/`, as in the earlier investigation; no project sources are excluded.

The follow-up [socket-path profiling investigation](readiness-profiling.md) examines the remaining latency and CPU-efficiency gaps with fresh controls and kernel evidence.
